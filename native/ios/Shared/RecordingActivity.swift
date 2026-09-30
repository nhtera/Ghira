// SPDX-License-Identifier: Apache-2.0
// Shared by the app and the Live Activity widget extension: the activity's
// data and the Stop/Mark intents. The intents run in the app's process
// (LiveActivityIntent), which is alive while recording (background audio).

import ActivityKit
import AppIntents
import Foundation
#if GHI_APP
import GhiIOS
#endif

struct RecordingAttributes: ActivityAttributes {
    struct ContentState: Codable, Hashable {
        /// `Phase` code from Rust (platform.rs `phase_code`).
        var phase: Int
        var marks: Int
    }

    var startedAt: Date
}

extension RecordingAttributes.ContentState {
    /// Capture has stopped (Finishing, Done): no timer, no Stop/Mark.
    var stopped: Bool { phase == 6 || phase == 7 }

    /// While locked there is no live transcript (RT-5): just "Recording".
    var label: String {
        switch phase {
        case 0: return "Loading models"
        case 1: return "Live transcript"
        case 2, 8: return "Recording"
        case 3: return "Catching up"
        case 4: return "Recording (phone is hot)"
        case 5: return "Paused: interrupted"
        case 6: return "Stopped · open Ghira to finish"
        default: return "Done"
        }
    }
}

struct StopRecordingIntent: LiveActivityIntent {
    static let title: LocalizedStringResource = "Stop recording"

    func perform() async throws -> some IntentResult {
        #if GHI_APP
        ghi_ios_stop_requested()
        #endif
        return .result()
    }
}

struct MarkMomentIntent: LiveActivityIntent {
    static let title: LocalizedStringResource = "Mark this moment"

    func perform() async throws -> some IntentResult {
        #if GHI_APP
        ghi_ios_mark_requested()
        #endif
        return .result()
    }
}
