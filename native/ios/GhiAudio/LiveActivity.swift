// SPDX-License-Identifier: Apache-2.0
// Starts, updates and ends the recording Live Activity (app side).

import ActivityKit
import Foundation

final class GhiLiveActivity {
    static let shared = GhiLiveActivity()
    private let queue = DispatchQueue(label: "ghira.live-activity")
    private var activity: Activity<RecordingAttributes>?
    /// Updates and the end run one after another, in the order they were asked for.
    private var last: Task<Void, Never>?

    /// Runs `work` after everything queued before it (on `queue`).
    private func chain(_ work: @escaping @Sendable () async -> Void) {
        let previous = last
        last = Task {
            await previous?.value
            await work()
        }
    }

    func start(phase: Int) {
        queue.async {
            guard ActivityAuthorizationInfo().areActivitiesEnabled else {
                NSLog("ghira: Live Activities are off in Settings")
                return
            }
            // End activities a crashed or killed session left behind (iOS
            // caps how many an app may have).
            self.endAll()
            let state = RecordingAttributes.ContentState(phase: phase, marks: 0)
            do {
                self.activity = try Activity.request(
                    attributes: RecordingAttributes(startedAt: Date()),
                    content: .init(state: state, staleDate: nil)
                )
            } catch {
                NSLog("ghira: Live Activity: \(error)")
            }
        }
    }

    /// Ends every activity of this app (on the queue).
    private func endAll() {
        for stale in Activity<RecordingAttributes>.activities {
            chain { await stale.end(nil, dismissalPolicy: .immediate) }
        }
        activity = nil
    }

    /// At launch: nothing is recording yet, so any activity is stale.
    func endStale() {
        queue.async { self.endAll() }
    }

    func update(phase: Int, marks: Int) {
        queue.async {
            guard let activity = self.activity else { return }
            let state = RecordingAttributes.ContentState(phase: phase, marks: marks)
            self.chain { await activity.update(.init(state: state, staleDate: nil)) }
        }
    }

    func end() {
        queue.async {
            guard let activity = self.activity else { return }
            self.activity = nil
            let state = RecordingAttributes.ContentState(phase: 7, marks: 0)
            self.chain { await activity.end(.init(state: state, staleDate: nil), dismissalPolicy: .default) }
        }
    }
}

@_cdecl("ghi_swift_activity_start")
public func ghiSwiftActivityStart(_ phase: Int32) {
    GhiLiveActivity.shared.start(phase: Int(phase))
}

@_cdecl("ghi_swift_activity_update")
public func ghiSwiftActivityUpdate(_ phase: Int32, _ marks: UInt32) {
    GhiLiveActivity.shared.update(phase: Int(phase), marks: Int(marks))
}

@_cdecl("ghi_swift_activity_end")
public func ghiSwiftActivityEnd() {
    GhiLiveActivity.shared.end()
}
