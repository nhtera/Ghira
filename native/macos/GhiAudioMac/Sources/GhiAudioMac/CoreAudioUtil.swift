// SPDX-License-Identifier: Apache-2.0
//
// Thin typed helpers over the AudioObject property API, plus the status-code
// mapping used by the C ABI.

import CoreAudio
import Foundation

/// A failed Core Audio call, carrying the raw OSStatus.
struct CAError: Error {
    let status: OSStatus
}

/// Status codes of the C ABI (see ghi_audio_mac.h).
enum GhiStatus {
    static let ok: Int32 = 0
    static let invalid: Int32 = -1
    static let unsupported: Int32 = -2
    static let micPermission: Int32 = -3
    static let noDevice: Int32 = -4
    static let tap: Int32 = -5
    static let systemPermission: Int32 = -6

    /// Maps a Core Audio OSStatus to the ABI. Most Core Audio errors are FourCC
    /// codes that are positive as Int32, but the ABI reserves positive values,
    /// so they are negated. `kAudioDevicePermissionsError` becomes
    /// `systemPermission`.
    static func from(osStatus status: OSStatus) -> Int32 {
        if status == OSStatus(bitPattern: UInt32(kAudioDevicePermissionsError)) { return systemPermission }
        return status > 0 ? -status : status
    }
}

/// Failure inside the capture layer, already expressed as an ABI status.
struct CaptureError: Error {
    let code: Int32
    let detail: String
}

enum CAUtil {
    static func address(
        _ selector: AudioObjectPropertySelector,
        scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal
    ) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
    }

    /// Reads a fixed-size value. `initial` supplies the type and the zero value.
    static func get<T>(
        _ object: AudioObjectID,
        _ selector: AudioObjectPropertySelector,
        scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal,
        initial: T
    ) throws -> T {
        var addr = address(selector, scope: scope)
        var size = UInt32(MemoryLayout<T>.size)
        var value = initial
        let status = withUnsafeMutablePointer(to: &value) {
            AudioObjectGetPropertyData(object, &addr, 0, nil, &size, $0)
        }
        guard status == noErr else { throw CAError(status: status) }
        return value
    }

    /// Reads an array-valued property.
    static func getArray<T>(
        _ object: AudioObjectID,
        _ selector: AudioObjectPropertySelector,
        scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal,
        of: T.Type
    ) throws -> [T] {
        var addr = address(selector, scope: scope)
        var size: UInt32 = 0
        var status = AudioObjectGetPropertyDataSize(object, &addr, 0, nil, &size)
        guard status == noErr else { throw CAError(status: status) }
        let count = Int(size) / MemoryLayout<T>.stride
        if count == 0 { return [] }
        let result = try [T](unsafeUninitializedCapacity: count) { buffer, initialized in
            var bytes = size
            status = AudioObjectGetPropertyData(object, &addr, 0, nil, &bytes, buffer.baseAddress!)
            initialized = status == noErr ? Int(bytes) / MemoryLayout<T>.stride : 0
            if status != noErr { throw CAError(status: status) }
        }
        return result
    }

    static func string(
        _ object: AudioObjectID,
        _ selector: AudioObjectPropertySelector,
        scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal
    ) -> String? {
        var addr = address(selector, scope: scope)
        var size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
        var ref: Unmanaged<CFString>?
        let status = AudioObjectGetPropertyData(object, &addr, 0, nil, &size, &ref)
        guard status == noErr, let ref else { return nil }
        return ref.takeRetainedValue() as String
    }

    static func defaultDevice(input: Bool) -> AudioDeviceID? {
        let selector = input ? kAudioHardwarePropertyDefaultInputDevice : kAudioHardwarePropertyDefaultOutputDevice
        guard let id = try? get(AudioObjectID(kAudioObjectSystemObject), selector, initial: AudioDeviceID(0)),
              id != kAudioObjectUnknown else { return nil }
        return id
    }

    static func deviceName(_ device: AudioDeviceID) -> String {
        string(device, kAudioObjectPropertyName) ?? ""
    }

    static func deviceUID(_ device: AudioDeviceID) -> String? {
        string(device, kAudioDevicePropertyDeviceUID)
    }

    /// Number of streams the device exposes in `scope`.
    static func streamIDs(_ device: AudioObjectID, scope: AudioObjectPropertyScope) -> [AudioStreamID] {
        (try? getArray(device, kAudioDevicePropertyStreams, scope: scope, of: AudioStreamID.self)) ?? []
    }
}

/// Owns one AudioObject property listener so it can be removed exactly once.
final class PropertyListener {
    private let object: AudioObjectID
    private var address: AudioObjectPropertyAddress
    private let queue: DispatchQueue
    private let block: AudioObjectPropertyListenerBlock
    private var active = false

    init(
        object: AudioObjectID,
        selector: AudioObjectPropertySelector,
        scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal,
        queue: DispatchQueue,
        handler: @escaping () -> Void
    ) {
        self.object = object
        self.address = CAUtil.address(selector, scope: scope)
        self.queue = queue
        self.block = { _, _ in handler() }
        active = AudioObjectAddPropertyListenerBlock(object, &address, queue, block) == noErr
    }

    func remove() {
        guard active else { return }
        active = false
        AudioObjectRemovePropertyListenerBlock(object, &address, queue, block)
    }

    deinit { remove() }
}
