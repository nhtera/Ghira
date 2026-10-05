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

/// The design's palette (Plans/design "Ghi Mobile" M2 lock screen): a dark card
/// in both appearances, teal tile, coral recording colour, gold Mark star.
private enum Palette {
    static let card = Color(red: 18 / 255, green: 22 / 255, blue: 20 / 255).opacity(0.82)
    static let tile = Color(red: 0x4D / 255, green: 0xBF / 255, blue: 0xA5 / 255)
    static let onTile = Color(red: 0x04 / 255, green: 0x20 / 255, blue: 0x1A / 255)
    static let rec = Color(red: 0xF0 / 255, green: 0x69 / 255, blue: 0x5C / 255)
    static let muted = Color(red: 0xA3 / 255, green: 0xAF / 255, blue: 0xA9 / 255)
    static let soft = Color(red: 0xD8 / 255, green: 0xE0 / 255, blue: 0xDC / 255)
    static let mark = Color(red: 0xE3 / 255, green: 0xAA / 255, blue: 0x4C / 255)
    static let idle = Color(red: 0x6B / 255, green: 0x78 / 255, blue: 0x72 / 255)
}

/// Timer and status colour: coral while capturing, muted while paused/stopped.
private func accent(_ state: RecordingAttributes.ContentState) -> Color {
    switch state.phase {
    case .paused, .interrupted, .finishing, .done: return Palette.muted
    default: return Palette.rec
    }
}

struct RecordingLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: RecordingAttributes.self) { context in
            LockScreenView(context: context)
                .padding(.horizontal, 16)
                .padding(.vertical, 14)
                .activityBackgroundTint(Palette.card)
                .activitySystemActionForegroundColor(.white)
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    HStack(spacing: 8) {
                        Tile(size: 26)
                        Text(verbatim: "Ghira")
                            .font(.system(size: 13, weight: .semibold))
                    }
                    // Clear of the island's large top corner radius, or the tile is clipped.
                    .padding(.leading, 10)
                    .padding(.top, 10)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    Elapsed(context: context, size: 20)
                        .padding(.trailing, 10)
                        .padding(.top, 10)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 10) {
                        StatusRow(state: context.state)
                        if !context.state.stopped {
                            Buttons(height: 40)
                        }
                    }
                    .padding(.horizontal, 10)
                }
            } compactLeading: {
                Circle().fill(dot(context.state)).frame(width: 9, height: 9)
            } compactTrailing: {
                Elapsed(context: context, size: 12, width: 44, color: .white)
            } minimal: {
                Circle().fill(dot(context.state)).frame(width: 9, height: 9)
            }
            .keylineTint(Palette.rec)
        }
    }
}

private func dot(_ state: RecordingAttributes.ContentState) -> Color {
    state.phase == .paused || state.phase == .interrupted || state.stopped ? Palette.idle : Palette.rec
}

/// The brand tile: the design's teal rounded square with a serif "g".
private struct Tile: View {
    let size: CGFloat

    var body: some View {
        Text("g")
            .font(.system(size: size * 0.6, weight: .semibold, design: .serif))
            .foregroundStyle(Palette.onTile)
            .frame(width: size, height: size)
            .background(Palette.tile, in: RoundedRectangle(cornerRadius: size * 0.27, style: .continuous))
            .accessibilityHidden(true)
    }
}

/// Status line: a dot and the phase ("Live", "Paused", "Catching up"), then marks.
/// Never transcript text or a meeting title: the lock screen shows no content.
private struct StatusRow: View {
    let state: RecordingAttributes.ContentState

    var body: some View {
        HStack(spacing: 8) {
            Circle().fill(dot(state)).frame(width: 8, height: 8)
            Text(detail(state))
                .font(.system(size: 13))
                .foregroundStyle(Palette.soft)
        }
        .accessibilityElement(children: .combine)
    }
}

private struct LockScreenView: View {
    let context: ActivityViewContext<RecordingAttributes>

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                Tile(size: 30)
                VStack(alignment: .leading, spacing: 1) {
                    Text(String(localized: "mobile.ios.liveActivitySubtitle"))
                        .font(.system(size: 12))
                        .foregroundStyle(Palette.muted)
                        .lineLimit(1)
                        .minimumScaleFactor(0.8)
                    Text(String(localized: context.state.label))
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(.white)
                        .lineLimit(1)
                }
                Spacer(minLength: 8)
                Elapsed(context: context, size: 22)
            }
            if context.state.marks > 0 {
                StatusRow(state: context.state)
            }
            if !context.state.stopped {
                Buttons(height: 44)
            }
        }
    }
}

/// "Live", or "Live · 2 marks".
private func detail(_ state: RecordingAttributes.ContentState) -> String {
    let label = String(localized: state.label)
    guard state.marks > 0 else { return label }
    let marks = String.localizedStringWithFormat(
        NSLocalizedString("mobile.ios.activity.marks", comment: "Mark count"), state.marks)
    return "\(label) · \(marks)"
}

/// Running time (mm:ss, mono, coral) while recording; a stop glyph once stopped.
private struct Elapsed: View {
    let context: ActivityViewContext<RecordingAttributes>
    var size: CGFloat
    var width: CGFloat = 84
    /// Overrides the phase colour (the compact island's timer is white).
    var color: Color?

    var body: some View {
        if context.state.stopped {
            Image(systemName: "stop.circle")
                .font(.system(size: size))
                .foregroundStyle(Palette.muted)
        } else {
            timer
                .font(.system(size: size, weight: .medium, design: .monospaced))
                .foregroundStyle(color ?? accent(context.state))
                .multilineTextAlignment(.trailing)
                .frame(width: width, alignment: .trailing)
        }
    }

    /// Frozen "mm:ss" while paused; otherwise the system timer from the pause-adjusted start.
    @ViewBuilder private var timer: some View {
        if let frozen = context.state.frozen {
            let t = Int(frozen)
            Text(String(format: "%02d:%02d", t / 60, t % 60))
        } else {
            let start = context.state.timerStart ?? context.attributes.startedAt
            Text(timerInterval: start...start.addingTimeInterval(86_400),
                 pauseTime: nil, countsDown: false, showsHours: false)
        }
    }
}

/// Mark (glass pill, gold star) and Stop (coral pill), the design's two buttons.
private struct Buttons: View {
    let height: CGFloat

    var body: some View {
        HStack(spacing: 8) {
            Button(intent: MarkMomentIntent()) {
                pill(Palette.mark, "star.fill", "mobile.ios.activity.mark", fill: Color.white.opacity(0.14), star: true)
            }
            Button(intent: StopRecordingIntent()) {
                pill(.white, "stop", "mobile.ios.activity.stop", fill: Palette.rec, star: false)
            }
        }
        .buttonStyle(.plain)
    }

    private func pill(_ icon: Color, _ symbol: String, _ key: String.LocalizationValue, fill: Color, star: Bool) -> some View {
        HStack(spacing: 6) {
            Image(systemName: symbol)
                .font(.system(size: star ? 15 : 13, weight: .bold))
                .foregroundStyle(icon)
            Text(String(localized: key))
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(.white)
        }
        .frame(maxWidth: .infinity)
        .frame(height: height)
        .background(fill, in: Capsule())
    }
}
