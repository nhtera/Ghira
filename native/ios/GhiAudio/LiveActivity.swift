// SPDX-License-Identifier: Apache-2.0
// Starts, updates and ends the recording Live Activity (app side).

import ActivityKit
import Foundation

final class GhiLiveActivity {
    static let shared = GhiLiveActivity()
    private let queue = DispatchQueue(label: "ghira.live-activity")
    private var activity: Activity<RecordingAttributes>?
    /// Timer bookkeeping (on `queue`): start, time spent paused so far, and when the current pause began.
    private var startedAt = Date()
    private var pausedTotal: TimeInterval = 0
    private var pausedAt: Date?

    /// Content state for `phase`: freezes the elapsed time while paused/interrupted and
    /// moves the timer's start forward by the paused time when capture resumes.
    private func state(phase: ActivityPhase, marks: Int) -> RecordingAttributes.ContentState {
        let pausing = phase == .paused || phase == .interrupted
        if pausing, pausedAt == nil { pausedAt = Date() }
        if !pausing, let began = pausedAt {
            pausedTotal += Date().timeIntervalSince(began)
            pausedAt = nil
        }
        let start = startedAt.addingTimeInterval(pausedTotal)
        let frozen = pausedAt.map { max(0, $0.timeIntervalSince(start)) }
        return .init(phase: phase, marks: marks, timerStart: start, frozen: frozen)
    }
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

    func start(phase: ActivityPhase) {
        queue.async {
            guard ActivityAuthorizationInfo().areActivitiesEnabled else {
                NSLog("ghira: Live Activities are off in Settings")
                return
            }
            // End activities a crashed or killed session left behind (iOS
            // caps how many an app may have).
            self.endAll()
            self.startedAt = Date()
            self.pausedTotal = 0
            self.pausedAt = nil
            let state = self.state(phase: phase, marks: 0)
            do {
                self.activity = try Activity.request(
                    attributes: RecordingAttributes(startedAt: self.startedAt),
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

    func update(phase: ActivityPhase, marks: Int) {
        queue.async {
            guard let activity = self.activity else { return }
            let state = self.state(phase: phase, marks: marks)
            self.chain { await activity.update(.init(state: state, staleDate: nil)) }
        }
    }

    func end() {
        queue.async {
            guard let activity = self.activity else { return }
            self.activity = nil
            let state = RecordingAttributes.ContentState(phase: .done, marks: 0)
            self.chain { await activity.end(.init(state: state, staleDate: nil), dismissalPolicy: .default) }
        }
    }
}

@_cdecl("ghi_swift_activity_start")
public func ghiSwiftActivityStart(_ phase: Int32) {
    GhiLiveActivity.shared.start(phase: ActivityPhase(rawValue: Int(phase)) ?? .locked)
}

@_cdecl("ghi_swift_activity_update")
public func ghiSwiftActivityUpdate(_ phase: Int32, _ marks: UInt32) {
    GhiLiveActivity.shared.update(phase: ActivityPhase(rawValue: Int(phase)) ?? .locked, marks: Int(marks))
}

@_cdecl("ghi_swift_activity_end")
public func ghiSwiftActivityEnd() {
    GhiLiveActivity.shared.end()
}
