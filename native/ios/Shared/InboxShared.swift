// SPDX-License-Identifier: Apache-2.0
// What the app and the share extension agree on (App Group inbox, see
// apps/mobile/src-tauri/src/inbox.rs).

import Foundation

enum InboxShared {
    static let appGroup = "group.com.nhtera.ghira"
    /// Darwin notification the extension posts after moving an item into the inbox.
    static let changedNotification = "com.nhtera.ghira.inbox_changed"
    /// Types the app's decoder reads (inbox.rs `EXTENSIONS`).
    static let extensions: Set<String> = [
        "m4a", "mp3", "wav", "aac", "flac", "ogg", "oga", "opus", "aif", "aiff", "mp4",
    ]

    static func containerURL() -> URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroup)
    }

    /// `<group>/inbox`: items live in `inbox/<uuid>/`. Excluded from backup.
    static func inboxURL() -> URL? {
        guard var url = containerURL()?.appendingPathComponent("inbox", isDirectory: true) else { return nil }
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? url.setResourceValues(values)
        return url
    }

    /// Where the extension builds an item before moving it into `inbox/` (the
    /// app scans `inbox/` only, so it never sees a half-written item).
    static func stagingURL() -> URL? {
        guard var url = containerURL()?.appendingPathComponent("inbox-staging", isDirectory: true) else { return nil }
        try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? url.setResourceValues(values)
        return url
    }

    static func postChanged() {
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(),
            CFNotificationName(changedNotification as CFString), nil, nil, true)
    }
}
