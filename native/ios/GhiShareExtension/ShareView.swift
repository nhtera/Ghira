// SPDX-License-Identifier: Apache-2.0
// The share sheet: file name and length, language, where to process, Import.

import SwiftUI

struct ShareView: View {
    @ObservedObject var model: ShareModel

    var body: some View {
        NavigationStack {
            content
                .navigationTitle(Text("mobile.ios.share.title"))
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("mobile.ios.share.cancel") { model.cancel() }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("mobile.ios.share.import") { model.importFile() }
                            .disabled(model.phase != .ready)
                    }
                }
        }
    }

    @ViewBuilder
    private var content: some View {
        switch model.phase {
        case .loading, .importing:
            VStack(spacing: 12) {
                ProgressView()
                Text("mobile.ios.share.importing").foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .done:
            Label("mobile.ios.share.done", systemImage: "checkmark.circle.fill")
                .font(.headline)
                .foregroundStyle(.green)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .rejected(let message):
            Text(message)
                .multilineTextAlignment(.center)
                .padding()
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        case .ready:
            form
        }
    }

    private var form: some View {
        Form {
            Section {
                HStack {
                    Image(systemName: "waveform")
                    Text(model.fileName).lineLimit(2)
                    Spacer()
                    if let d = model.duration {
                        Text(Self.format(d)).monospacedDigit().foregroundStyle(.secondary)
                    }
                }
            }
            Section {
                Picker("mobile.ios.share.language", selection: $model.language) {
                    Text("mobile.ios.share.languageAuto").tag(ShareLanguage.auto)
                    Text(verbatim: "English").tag(ShareLanguage.en)
                    Text(verbatim: "Tiếng Việt").tag(ShareLanguage.vi)
                }
            }
            Section("mobile.ios.share.processOn") {
                targetRow("mobile.ios.share.thisPhone", .phone)
                HStack {
                    VStack(alignment: .leading) {
                        Text("mobile.ios.share.myDesktop")
                        Text("mobile.ios.share.myDesktopDisabled").font(.caption)
                    }
                    Spacer()
                }
                .foregroundStyle(.secondary)
                .accessibilityElement(children: .combine)
                .accessibilityAddTraits(.isStaticText)
                targetRow("mobile.ios.share.cloudNotes", .cloud)
            }
        }
    }

    private func targetRow(_ title: LocalizedStringKey, _ value: ShareTarget) -> some View {
        Button {
            model.target = value
        } label: {
            HStack {
                Text(title).foregroundStyle(.primary)
                Spacer()
                if model.target == value { Image(systemName: "checkmark").foregroundStyle(.tint) }
            }
        }
        .accessibilityAddTraits(model.target == value ? .isSelected : [])
    }

    private static func format(_ seconds: TimeInterval) -> String {
        let total = Int(seconds.rounded())
        let (h, m, s) = (total / 3600, total % 3600 / 60, total % 60)
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, s) : String(format: "%d:%02d", m, s)
    }
}
