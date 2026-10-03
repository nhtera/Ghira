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
        var phase: ActivityPhase
        var marks: Int
    }

    var startedAt: Date
}

extension RecordingAttributes.ContentState {
    var stopped: Bool { phase.stopped }
    var label: LocalizedStringResource { phase.label }
}

struct StopRecordingIntent: LiveActivityIntent {
    static let title: LocalizedStringResource = "mobile.ios.activity.stop"

    func perform() async throws -> some IntentResult {
        #if GHI_APP
        ghi_ios_stop_requested()
        #endif
        return .result()
    }
}

struct MarkMomentIntent: LiveActivityIntent {
    static let title: LocalizedStringResource = "mobile.ios.activity.mark"

    func perform() async throws -> some IntentResult {
        #if GHI_APP
        ghi_ios_mark_requested()
        #endif
        return .result()
    }
}
