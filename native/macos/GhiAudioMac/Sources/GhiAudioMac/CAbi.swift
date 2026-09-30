// SPDX-License-Identifier: Apache-2.0
//
// C ABI exports (include/ghi_audio_mac.h). Keep in sync with the header.

import Foundation

private let abiVersion: UInt32 = 1

@_cdecl("ghi_mac_abi_version")
public func ghi_mac_abi_version() -> UInt32 { abiVersion }

@_cdecl("ghi_mac_start")
public func ghi_mac_start(
    _ flags: UInt32,
    _ tapPids: UnsafePointer<Int32>?,
    _ nTapPids: UInt32,
    _ audio: (@convention(c) (UnsafeMutableRawPointer?, UInt32, UnsafePointer<Float>?, UInt32, Double, UInt64) -> Void)?,
    _ event: (@convention(c) (UnsafeMutableRawPointer?, UInt32, Int64, UnsafePointer<CChar>?) -> Void)?,
    _ ctx: UnsafeMutableRawPointer?,
    _ outSession: UnsafeMutablePointer<UnsafeMutableRawPointer?>?
) -> Int32 {
    guard flags & 0x3 != 0, flags & ~UInt32(0x3) == 0, let audio, let event, let outSession,
          nTapPids == 0 || tapPids != nil else { return GhiStatus.invalid }
    guard #available(macOS 14.2, *) else { return GhiStatus.unsupported }

    let pids = tapPids.map { Array(UnsafeBufferPointer(start: $0, count: Int(nTapPids))) } ?? []
    let session = CaptureSession(flags: flags, tapPIDs: pids, audio: audio, event: event, userData: ctx)
    let status = session.start()
    guard status == GhiStatus.ok else { return status }
    outSession.pointee = Unmanaged.passRetained(session).toOpaque()
    return GhiStatus.ok
}

private func session(_ pointer: UnsafeMutableRawPointer?) -> CaptureSession? {
    pointer.map { Unmanaged<CaptureSession>.fromOpaque($0).takeUnretainedValue() }
}

@_cdecl("ghi_mac_pause")
public func ghi_mac_pause(_ handle: UnsafeMutableRawPointer?) -> Int32 {
    session(handle)?.pause() ?? GhiStatus.invalid
}

@_cdecl("ghi_mac_resume")
public func ghi_mac_resume(_ handle: UnsafeMutableRawPointer?) -> Int32 {
    session(handle)?.resume() ?? GhiStatus.invalid
}

@_cdecl("ghi_mac_stop")
public func ghi_mac_stop(_ handle: UnsafeMutableRawPointer?) {
    guard let handle else { return }
    let owned = Unmanaged<CaptureSession>.fromOpaque(handle).takeRetainedValue()
    owned.stop()
}

@_cdecl("ghi_mac_mic_permission")
public func ghi_mac_mic_permission() -> Int32 { MicPermission.status() }

@_cdecl("ghi_mac_request_mic_permission")
public func ghi_mac_request_mic_permission() -> Int32 { MicPermission.request() }

@_cdecl("ghi_mac_route")
public func ghi_mac_route(
    _ outputKind: UnsafeMutablePointer<UInt32>?, _ inputBluetooth: UnsafeMutablePointer<UInt32>?
) -> Int32 {
    guard let outputKind, let inputBluetooth else { return GhiStatus.invalid }
    let route = Route.current()
    outputKind.pointee = route.kind
    inputBluetooth.pointee = route.inputBluetooth ? 1 : 0
    return GhiStatus.ok
}

@_cdecl("ghi_mac_audio_processes")
public func ghi_mac_audio_processes() -> UnsafeMutablePointer<CChar>? {
    guard #available(macOS 14.2, *), let json = AudioProcesses.json() else { return nil }
    return strdup(json)
}

@_cdecl("ghi_mac_free")
public func ghi_mac_free(_ string: UnsafeMutablePointer<CChar>?) {
    free(string)
}
