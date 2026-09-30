// SPDX-License-Identifier: Apache-2.0
//
// Microphone permission. System audio (process tap) has no status API: a denied
// tap either fails with kAudioDevicePermissionsError or delivers silence, which
// the host detects from the RMS of the system track.

import AVFoundation

enum MicPermission {
    /// GHI_MAC_PERM_*; the values match AVAuthorizationStatus.
    static func status() -> Int32 {
        Int32(AVCaptureDevice.authorizationStatus(for: .audio).rawValue)
    }

    /// Prompts when undetermined and blocks until answered. On the main thread
    /// it only reports the current status: the completion handler is delivered
    /// on the main queue, so waiting there would deadlock.
    static func request() -> Int32 {
        guard status() == 0, !Thread.isMainThread else { return status() }
        let answered = DispatchSemaphore(value: 0)
        AVCaptureDevice.requestAccess(for: .audio) { _ in answered.signal() }
        answered.wait()
        return status()
    }
}
