// SPDX-License-Identifier: Apache-2.0
// The recording Live Activity: lock screen banner and Dynamic Island, with a
// running timer, the state, and Stop/Mark buttons (iOS 17 App Intents).

import ActivityKit
import AppIntents
import SwiftUI
import WidgetKit

@main
struct GhiLiveActivityBundle: WidgetBundle {
    var body: some Widget {
        RecordingLiveActivity()
    }
}

struct RecordingLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: RecordingAttributes.self) { context in
            LockScreenView(context: context)
                .padding()
                .activityBackgroundTint(Color.black.opacity(0.75))
                .activitySystemActionForegroundColor(.white)
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    Label("Ghira", systemImage: "waveform")
                        .font(.caption.bold())
                }
                DynamicIslandExpandedRegion(.trailing) {
                    Elapsed(context: context).frame(maxWidth: 72)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    HStack {
                        Text(context.state.label).font(.caption)
                        Spacer()
                        if !context.state.stopped {
                            Buttons(marks: context.state.marks)
                        }
                    }
                }
            } compactLeading: {
                Image(systemName: "record.circle").foregroundStyle(.red)
            } compactTrailing: {
                Elapsed(context: context).frame(maxWidth: 48)
            } minimal: {
                Image(systemName: "record.circle").foregroundStyle(.red)
            }
        }
    }
}

private struct LockScreenView: View {
    let context: ActivityViewContext<RecordingAttributes>

    var body: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Label(context.state.stopped ? "Stopped" : "Recording", systemImage: "record.circle")
                    .font(.headline)
                    .foregroundStyle(context.state.stopped ? .gray : .red)
                Text(context.state.label)
                    .font(.caption)
                    .foregroundStyle(.white.opacity(0.8))
            }
            Spacer()
            Elapsed(context: context)
                .font(.title2)
                .foregroundStyle(.white)
                .frame(maxWidth: 96)
            if !context.state.stopped {
                Buttons(marks: context.state.marks)
            }
        }
    }
}

/// Running time while recording; nothing once stopped.
private struct Elapsed: View {
    let context: ActivityViewContext<RecordingAttributes>

    var body: some View {
        if context.state.stopped {
            Image(systemName: "stop.circle")
        } else {
            Text(context.attributes.startedAt, style: .timer).monospacedDigit()
        }
    }
}

private struct Buttons: View {
    let marks: Int

    var body: some View {
        HStack(spacing: 8) {
            Button(intent: MarkMomentIntent()) {
                Label(marks > 0 ? "Mark \(marks)" : "Mark", systemImage: "bookmark")
            }
            .tint(.orange)
            Button(intent: StopRecordingIntent()) {
                Label("Stop", systemImage: "stop.fill")
            }
            .tint(.red)
        }
        .font(.caption.bold())
        .labelStyle(.iconOnly)
        .buttonStyle(.borderedProminent)
    }
}
