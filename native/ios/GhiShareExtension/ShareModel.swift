// SPDX-License-Identifier: Apache-2.0
// The share extension's state: finds the shared audio, copies it into the App
// Group `inbox-staging/<uuid>/`, writes `manifest.json` (confirmed: false) and
// renames the folder into `inbox/<uuid>/` (apps/mobile/src-tauri/src/inbox.rs),
// so a sheet that is dismissed early still leaves the item in the app's inbox
// list. Import rewrites the manifest with confirmed: true (temp file + rename);
// Cancel removes the folder.
//
// Memory: the file is only ever copied (the system clones it on APFS), and
// AVURLAsset reads the duration from the header; nothing is decoded here.

import AVFoundation
import SwiftUI
import UniformTypeIdentifiers

enum ShareLanguage: String, CaseIterable, Identifiable {
    case auto, en, vi
    var id: String { rawValue }
}

enum ShareTarget: String {
    case phone, cloud
}

@MainActor
final class ShareModel: ObservableObject {
    enum Phase: Equatable {
        case loading
        case ready
        case importing
        case done
        /// Not importable; the message is a localized reason.
        case rejected(String)
    }

    @Published var phase: Phase = .loading
    @Published var fileName = ""
    @Published var duration: TimeInterval?
    @Published var language: ShareLanguage = .auto
    @Published var target: ShareTarget = .phone

    private let context: NSExtensionContext?
    private let id = UUID().uuidString
    /// The shared file inside its `inbox/<uuid>/` folder once it has been handed over.
    private var staged: URL?

    init(context: NSExtensionContext?) {
        self.context = context
    }

    private var provider: NSItemProvider? {
        let items = (context?.inputItems as? [NSExtensionItem]) ?? []
        return items.flatMap { $0.attachments ?? [] }
            .first { $0.hasItemConformingToTypeIdentifier(UTType.audio.identifier) }
    }

    func load() {
        guard let provider else {
            phase = .rejected(String(localized: "mobile.ios.share.unsupported"))
            return
        }
        Self.removeStaleStaging()
        let typeId = provider.registeredTypeIdentifiers.first {
            UTType($0)?.conforms(to: .audio) == true
        } ?? UTType.audio.identifier
        let fallbackExt = UTType(typeId)?.preferredFilenameExtension
        let suggested = provider.suggestedName
        let id = id
        provider.loadFileRepresentation(forTypeIdentifier: typeId) { [weak self] url, _ in
            // The file at `url` is only valid inside this handler: copy it now.
            let result = Self.stage(url, suggested: suggested, fallbackExt: fallbackExt, id: id)
            Task { @MainActor in self?.staged(result) }
        }
    }

    private enum Staged {
        case ok(URL)
        case caf
        case unsupported
        case failed
    }

    private nonisolated static func stage(_ url: URL?, suggested: String?, fallbackExt: String?, id: String) -> Staged {
        guard let url, let staging = InboxShared.stagingURL() else { return .failed }
        var name = url.lastPathComponent
        var ext = url.pathExtension.lowercased()
        if !InboxShared.extensions.contains(ext), ext != "caf", let fallbackExt {
            // A provider-made temp name can lack the extension: use the type's.
            let base = (suggested ?? name) as NSString
            name = base.deletingPathExtension + "." + fallbackExt
            ext = fallbackExt.lowercased()
        }
        if ext == "caf" { return .caf }
        guard InboxShared.extensions.contains(ext) else { return .unsupported }
        let dir = staging.appendingPathComponent(id, isDirectory: true)
        let dest = dir.appendingPathComponent(sanitized(name))
        do {
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            try FileManager.default.copyItem(at: url, to: dest)
            return .ok(dest)
        } catch {
            try? FileManager.default.removeItem(at: dir)
            return .failed
        }
    }

    /// A plain file name the app's inbox accepts: no separators, controls or leading dots.
    nonisolated static func sanitized(_ name: String) -> String {
        let ext = (name as NSString).pathExtension
        var base = (name as NSString).deletingPathExtension
        base = String(base.unicodeScalars.map { s in
            s == "/" || s == "\\" || s.properties.generalCategory == .control ? "_" : Character(s)
        })
        while base.hasPrefix(".") { base.removeFirst() }
        if base.isEmpty { base = "Recording" }
        // Keep the whole name well under the 255-byte limit.
        while base.utf8.count > 120 { base.removeLast() }
        return ext.isEmpty ? base : base + "." + ext
    }

    private func staged(_ result: Staged) {
        switch result {
        case .ok(let url):
            guard let handedOver = handOver(url) else {
                phase = .rejected(String(localized: "mobile.ios.share.failed"))
                return
            }
            staged = handedOver
            fileName = handedOver.lastPathComponent
            phase = .ready
            Task { duration = await Self.duration(of: handedOver) }
        case .caf:
            phase = .rejected(String(localized: "mobile.ios.share.unsupportedCaf"))
        case .unsupported:
            phase = .rejected(String(localized: "mobile.ios.share.unsupported"))
        case .failed:
            phase = .rejected(String(localized: "mobile.ios.share.failed"))
        }
    }

    /// Writes the unconfirmed manifest and moves the folder into the inbox.
    private func handOver(_ file: URL) -> URL? {
        guard let inbox = InboxShared.inboxURL() else { return nil }
        let dir = file.deletingLastPathComponent()
        do {
            try Self.writeManifest(in: dir, file: file.lastPathComponent, language: .auto, target: .phone, confirmed: false)
            let dest = inbox.appendingPathComponent(id, isDirectory: true)
            try FileManager.default.moveItem(at: dir, to: dest)
            InboxShared.postChanged()
            return dest.appendingPathComponent(file.lastPathComponent)
        } catch {
            try? FileManager.default.removeItem(at: dir)
            return nil
        }
    }

    /// Replaces `manifest.json` in one step (temp file + rename in the same folder).
    nonisolated static func writeManifest(
        in dir: URL, file: String, language: ShareLanguage, target: ShareTarget, confirmed: Bool
    ) throws {
        let manifest: [String: Any] = [
            "file": file,
            "lang": language.rawValue,
            "target": target.rawValue,
            "source": "share",
            "confirmed": confirmed,
        ]
        let data = try JSONSerialization.data(withJSONObject: manifest, options: [.sortedKeys])
        let temp = dir.appendingPathComponent(".manifest.tmp")
        try data.write(to: temp)
        let final = dir.appendingPathComponent("manifest.json")
        if FileManager.default.fileExists(atPath: final.path) {
            _ = try FileManager.default.replaceItemAt(final, withItemAt: temp)
        } else {
            try FileManager.default.moveItem(at: temp, to: final)
        }
    }

    /// From the file header only.
    private nonisolated static func duration(of url: URL) async -> TimeInterval? {
        guard let t = try? await AVURLAsset(url: url).load(.duration), t.isNumeric, t.seconds > 0 else { return nil }
        return t.seconds
    }

    func importFile() {
        guard phase == .ready, let staged else { return }
        phase = .importing
        do {
            try Self.writeManifest(
                in: staged.deletingLastPathComponent(), file: staged.lastPathComponent,
                language: language, target: target, confirmed: true)
        } catch {
            phase = .rejected(String(localized: "mobile.ios.share.failed"))
            return
        }
        InboxShared.postChanged()
        phase = .done
        Task {
            try? await Task.sleep(nanoseconds: 700_000_000)
            context?.completeRequest(returningItems: [], completionHandler: nil)
        }
    }

    func cancel() {
        let removed = discard()
        if removed { InboxShared.postChanged() }
        context?.cancelRequest(withError: NSError(domain: NSCocoaErrorDomain, code: NSUserCancelledError))
    }

    /// Removes the item's folder; true if there was one.
    @discardableResult
    private func discard() -> Bool {
        guard let dir = staged?.deletingLastPathComponent() else { return false }
        try? FileManager.default.removeItem(at: dir)
        staged = nil
        return true
    }

    /// Items an extension that was killed left behind.
    private nonisolated static func removeStaleStaging() {
        guard let staging = InboxShared.stagingURL(),
              let items = try? FileManager.default.contentsOfDirectory(
                at: staging, includingPropertiesForKeys: [.contentModificationDateKey])
        else { return }
        for item in items {
            let modified = (try? item.resourceValues(forKeys: [.contentModificationDateKey]))?.contentModificationDate
            if modified.map({ Date().timeIntervalSince($0) > 24 * 3600 }) ?? true {
                try? FileManager.default.removeItem(at: item)
            }
        }
    }
}
