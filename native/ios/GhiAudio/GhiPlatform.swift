// SPDX-License-Identifier: Apache-2.0
// Device and system facts for Rust (phase 16 contracts, C ABI in
// include/ghi_ios.h). Trivial ones are real; the rest are stubs that 16-E
// fills in (marked TODO(16-E)).

import AVFoundation
import GhiIOS
import UIKit

/// Copies `s` into a C buffer; returns the length written (NUL-terminated).
private func copyOut(_ s: String, _ buf: UnsafeMutablePointer<CChar>?, _ cap: Int) -> Int {
    guard let buf, cap > 0 else { return 0 }
    let bytes = Array(s.utf8.prefix(cap - 1))
    for (i, b) in bytes.enumerated() { buf[i] = CChar(bitPattern: b) }
    buf[bytes.count] = 0
    return bytes.count
}

/// Dynamic Type as a multiplier of the Large size, capped at 2.0.
func ghiTextScale(_ category: UIContentSizeCategory = UIApplication.shared.preferredContentSizeCategory) -> Float {
    let table: [UIContentSizeCategory: Float] = [
        .extraSmall: 0.82, .small: 0.88, .medium: 0.94, .large: 1.0,
        .extraLarge: 1.12, .extraExtraLarge: 1.24, .extraExtraExtraLarge: 1.36,
        .accessibilityMedium: 1.6, .accessibilityLarge: 1.8, .accessibilityExtraLarge: 2.0,
        .accessibilityExtraExtraLarge: 2.0, .accessibilityExtraExtraExtraLarge: 2.0,
    ]
    return min(2.0, table[category] ?? 1.0)
}

/// Dynamic Type for any thread: UIApplication is main-thread only, so the value
/// is cached and refreshed on the main thread (at install and on
/// `UIContentSizeCategory.didChangeNotification`), never read with a blocking hop.
final class TextScaleCache {
    static let shared = TextScaleCache()
    private let lock = NSLock()
    private var value: Float = 1.0

    /// Main thread only.
    @discardableResult
    func refresh() -> Float {
        let v = ghiTextScale()
        lock.lock()
        value = v
        lock.unlock()
        return v
    }

    var current: Float {
        lock.lock()
        defer { lock.unlock() }
        return value
    }
}

@_cdecl("ghi_swift_call_active")
public func ghiSwiftCallActive() -> Bool {
    // TODO(16-E): CXCallObserver.
    false
}

@_cdecl("ghi_swift_device_model")
public func ghiSwiftDeviceModel(_ buf: UnsafeMutablePointer<CChar>?, _ cap: Int) -> Int {
    if let simulated = ProcessInfo.processInfo.environment["SIMULATOR_MODEL_IDENTIFIER"] {
        return copyOut(simulated, buf, cap)
    }
    var info = utsname()
    uname(&info)
    let id = withUnsafePointer(to: &info.machine) {
        $0.withMemoryRebound(to: CChar.self, capacity: Int(_SYS_NAMELEN)) { String(cString: $0) }
    }
    return copyOut(id, buf, cap)
}

@_cdecl("ghi_swift_physical_memory")
public func ghiSwiftPhysicalMemory() -> UInt64 {
    ProcessInfo.processInfo.physicalMemory
}

@_cdecl("ghi_swift_text_scale")
public func ghiSwiftTextScale() -> Float {
    TextScaleCache.shared.current
}

@_cdecl("ghi_swift_share_file")
public func ghiSwiftShareFile(_ path: UnsafePointer<CChar>) -> Bool {
    // TODO(16-E): UIActivityViewController over the key window's top controller.
    false
}

@_cdecl("ghi_swift_open_settings")
public func ghiSwiftOpenSettings() {
    DispatchQueue.main.async {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
        UIApplication.shared.open(url)
    }
}

@_cdecl("ghi_swift_begin_bg_task")
public func ghiSwiftBeginBgTask(_ name: UnsafePointer<CChar>) -> UInt64 {
    // TODO(16-E): UIApplication.beginBackgroundTask with a token table.
    0
}

@_cdecl("ghi_swift_end_bg_task")
public func ghiSwiftEndBgTask(_ token: UInt64) {
    // TODO(16-E)
}

@_cdecl("ghi_swift_set_privacy_cover")
public func ghiSwiftSetPrivacyCover(_ on: Bool) {
    // TODO(16-E): a cover view on the key window.
}

@_cdecl("ghi_swift_continuous_ns")
public func ghiSwiftContinuousNs() -> UInt64 {
    var tb = mach_timebase_info_data_t()
    mach_timebase_info(&tb)
    return mach_continuous_time() * UInt64(tb.numer) / UInt64(tb.denom)
}

@_cdecl("ghi_swift_mic_permission")
public func ghiSwiftMicPermission() -> Int32 {
    switch AVAudioApplication.shared.recordPermission {
    case .granted: return 1
    case .denied: return 2
    default: return 0
    }
}

@_cdecl("ghi_swift_request_mic_permission")
public func ghiSwiftRequestMicPermission() {
    AVAudioApplication.requestRecordPermission { _ in }
}
