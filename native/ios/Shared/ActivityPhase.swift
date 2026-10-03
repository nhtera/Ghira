// SPDX-License-Identifier: Apache-2.0
// The Live Activity's phase, shared by the app and the widget. The raw values
// are the C ABI contract with Rust: `ActivityPhase` in
// apps/mobile/src-tauri/src/platform.rs mirrors this enum case for case (a
// Rust test pins the numbers).

import Foundation

enum ActivityPhase: Int, Codable {
    case loading = 0
    case live = 1
    case locked = 2
    case catchingUp = 3
    case hot = 4
    case interrupted = 5
    case finishing = 6
    case done = 7
    case recordOnly = 8
    /// Interrupted by something other than a call, or paused by the user.
    case paused = 9

    /// Capture has stopped (finishing, done): no timer, no Stop/Mark.
    var stopped: Bool { self == .finishing || self == .done }

    /// While locked there is no live transcript: just "Recording".
    var label: LocalizedStringResource {
        switch self {
        case .live: return "mobile.ios.activity.phase.live"
        case .catchingUp: return "mobile.ios.activity.phase.catchingUp"
        case .interrupted: return "mobile.ios.activity.phase.pausedForCall"
        case .paused: return "mobile.ios.activity.phase.paused"
        case .finishing, .done: return "mobile.ios.activity.phase.stopped"
        case .loading, .locked, .hot, .recordOnly: return "mobile.ios.activity.phase.recording"
        }
    }
}
