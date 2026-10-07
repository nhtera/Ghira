// SPDX-License-Identifier: Apache-2.0
// Device and system facts for Rust (phase 16 contracts, C ABI in
// include/ghi_ios.h): calls, network cost, sharing, background tasks, the
// privacy cover. UIKit state is main-thread only, so Rust-facing getters read
// cached copies; nothing here blocks on the main thread.

import AVFoundation
import CallKit
import GhiIOS
import Network
import UIKit

/// Copies `s` into a C buffer; returns the length written (NUL-terminated).
private func copyOut(_ s: String, _ buf: UnsafeMutablePointer<CChar>?, _ cap: Int) -> Int {
    guard let buf, cap > 0 else { return 0 }
    let bytes = Array(s.utf8.prefix(cap - 1))
    for (i, b) in bytes.enumerated() { buf[i] = CChar(bitPattern: b) }
    buf[bytes.count] = 0
    return bytes.count
}

/// Dynamic Type as a multiplier of the Large size, capped at 2.0.
func ghiTextScale(_ category: UIContentSizeCategory = UIApplication.shared.preferredContentSizeCategory) -> Float {
    let table: [UIContentSizeCategory: Float] = [
        .extraSmall: 0.82, .small: 0.88, .medium: 0.94, .large: 1.0,
        .extraLarge: 1.12, .extraExtraLarge: 1.24, .extraExtraExtraLarge: 1.36,
        .accessibilityMedium: 1.6, .accessibilityLarge: 1.8, .accessibilityExtraLarge: 2.0,
        .accessibilityExtraExtraLarge: 2.0, .accessibilityExtraExtraExtraLarge: 2.0,
    ]
    return min(2.0, table[category] ?? 1.0)
}

/// Dynamic Type for any thread: UIApplication is main-thread only, so the value
/// is cached and refreshed on the main thread (at install and on
/// `UIContentSizeCategory.didChangeNotification`), never read with a blocking hop.
final class TextScaleCache {
    static let shared = TextScaleCache()
    private let lock = NSLock()
    private var value: Float = 1.0

    /// Main thread only.
    @discardableResult
    func refresh() -> Float {
        let v = ghiTextScale()
        lock.lock()
        value = v
        lock.unlock()
        return v
    }

    var current: Float {
        lock.lock()
        defer { lock.unlock() }
        return value
    }
}

/// Whether the app is active and whether it was launched in the background.
/// `applicationState` is main-thread only: `install()` runs there and the
/// notifications keep `active` current.
final class AppLifecycleState {
    static let shared = AppLifecycleState()
    private let lock = NSLock()
    private var active = false
    private var launchedInBackground = false
    private var observers: [NSObjectProtocol] = []

    private var seenState = false

    /// Main thread only.
    func install() {
        captureLaunchState()
        set(UIApplication.shared.applicationState == .active)
        let nc = NotificationCenter.default
        observers = [
            nc.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: nil) { [weak self] _ in
                self?.set(true)
            },
            nc.addObserver(forName: UIApplication.willResignActiveNotification, object: nil, queue: nil) { [weak self] _ in
                self?.set(false)
            },
        ]
    }

    /// Called from `ghi_swift_launched_in_background` on the main thread
    /// (Rust's setup runs there) so the answer is right before `install`.
    func captureLaunchState() {
        lock.lock()
        if !seenState {
            launchedInBackground = UIApplication.shared.applicationState == .background
            seenState = true
        }
        lock.unlock()
    }

    private func set(_ value: Bool) {
        lock.lock()
        active = value
        lock.unlock()
    }

    var isActive: Bool {
        lock.lock()
        defer { lock.unlock() }
        return active
    }

    var wasLaunchedInBackground: Bool {
        lock.lock()
        defer { lock.unlock() }
        return launchedInBackground
    }
}

/// CXCallObserver: any call that has not ended (ringing, dialing, held, connected).
final class CallMonitor: NSObject, CXCallObserverDelegate {
    static let shared = CallMonitor()
    private let observer = CXCallObserver()
    private let queue = DispatchQueue(label: "ghira.calls")
    private let lock = NSLock()
    private var started = false
    private var isActive = false
    #if GHI_TEST_HOOKS
    /// The call-active / call-ended test hooks stand in for a real call.
    private var forced: Bool?
    #endif

    func start() {
        lock.lock()
        defer { lock.unlock() }
        guard !started else { return }
        started = true
        observer.setDelegate(self, queue: queue)
        isActive = observer.calls.contains { !$0.hasEnded }
    }

    var active: Bool {
        start()
        lock.lock()
        defer { lock.unlock() }
        #if GHI_TEST_HOOKS
        if let forced { return forced }
        #endif
        return isActive
    }

    /// Records the new state and tells Rust when it changed.
    private func update(_ now: Bool) {
        lock.lock()
        let changed = now != isActive
        isActive = now
        lock.unlock()
        if changed { ghi_ios_call_active_changed(now) }
    }

    func callObserver(_ callObserver: CXCallObserver, callChanged call: CXCall) {
        update(callObserver.calls.contains { !$0.hasEnded })
    }

    #if GHI_TEST_HOOKS
    /// Same path as a real call change (the state change goes to Rust through `update`).
    func testSet(_ active: Bool) {
        lock.lock()
        forced = active
        lock.unlock()
        update(active)
    }
    #endif
}

/// NWPathMonitor: cellular and Personal Hotspot are "expensive", Low Data Mode
/// is "constrained"; the models download is Wi-Fi only.
final class NetworkMonitor {
    static let shared = NetworkMonitor()
    private let monitor = NWPathMonitor()
    private let queue = DispatchQueue(label: "ghira.network")
    private let lock = NSLock()
    private var started = false

    func start() {
        lock.lock()
        defer { lock.unlock() }
        guard !started else { return }
        started = true
        monitor.pathUpdateHandler = { _ in }
        monitor.start(queue: queue)
    }

    var metered: Bool {
        start()
        let path = monitor.currentPath
        return path.isExpensive || path.isConstrained
    }
}

/// Darwin notification from the share extension: new items in the inbox.
enum InboxObserver {
    static func install() {
        CFNotificationCenterAddObserver(
            CFNotificationCenterGetDarwinNotifyCenter(), nil,
            { _, _, _, _, _ in ghi_ios_inbox_changed() },
            InboxShared.changedNotification as CFString, nil, .deliverImmediately)
    }
}

@_cdecl("ghi_swift_call_active")
public func ghiSwiftCallActive() -> Bool {
    CallMonitor.shared.active
}

@_cdecl("ghi_swift_launched_in_background")
public func ghiSwiftLaunchedInBackground() -> Bool {
    // Rust's setup runs on the main thread; elsewhere the install() value is used.
    if Thread.isMainThread { AppLifecycleState.shared.captureLaunchState() }
    return AppLifecycleState.shared.wasLaunchedInBackground
}

@_cdecl("ghi_swift_on_expensive_network")
public func ghiSwiftOnExpensiveNetwork() -> Bool {
    NetworkMonitor.shared.metered
}

@_cdecl("ghi_swift_device_model")
public func ghiSwiftDeviceModel(_ buf: UnsafeMutablePointer<CChar>?, _ cap: Int) -> Int {
    if let simulated = ProcessInfo.processInfo.environment["SIMULATOR_MODEL_IDENTIFIER"] {
        return copyOut(simulated, buf, cap)
    }
    var info = utsname()
    uname(&info)
    let id = withUnsafePointer(to: &info.machine) {
        $0.withMemoryRebound(to: CChar.self, capacity: Int(_SYS_NAMELEN)) { String(cString: $0) }
    }
    return copyOut(id, buf, cap)
}

@_cdecl("ghi_swift_physical_memory")
public func ghiSwiftPhysicalMemory() -> UInt64 {
    ProcessInfo.processInfo.physicalMemory
}

// Free space the system would make available for important data (it counts
// purgeable space, unlike statvfs); negative when unknown.
@_cdecl("ghi_swift_available_capacity")
public func ghiSwiftAvailableCapacity() -> Int64 {
    let home = URL(fileURLWithPath: NSHomeDirectory())
    let key = URLResourceKey.volumeAvailableCapacityForImportantUsageKey
    guard let v = try? home.resourceValues(forKeys: [key]).volumeAvailableCapacityForImportantUsage else {
        return -1
    }
    return v
}

@_cdecl("ghi_swift_text_scale")
public func ghiSwiftTextScale() -> Float {
    TextScaleCache.shared.current
}

/// The view controller to present from (main thread): the top of the foreground
/// scene's key window, else of its visible normal-level window. On an iPhone
/// (iOS 27) the app's window was not reported as key, and the pairing scanner
/// had nothing to present on. The privacy cover sits above normal level and is
/// never picked.
func topViewController() -> UIViewController? {
    let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
    let scene = scenes.first { $0.activationState == .foregroundActive }
        ?? scenes.first { $0.activationState == .foregroundInactive }
    let windows = scene?.windows ?? []
    let window = windows.first(where: \.isKeyWindow)
        ?? windows.first { !$0.isHidden && $0.windowLevel == .normal && $0.rootViewController != nil }
    var top = window?.rootViewController
    while let presented = top?.presentedViewController, !presented.isBeingDismissed { top = presented }
    return top
}

/// Main thread. False (the file is kept for Rust's sweep) when there is nothing to present from.
private func presentShare(_ url: URL) -> Bool {
    guard let top = topViewController() else { return false }
    let sheet = UIActivityViewController(activityItems: [url], applicationActivities: nil)
    // The file was made for this share (an export): gone once the sheet closes
    // (finished, or dismissed with no activity chosen). A hand-off to another
    // app (`activityType` set, not completed) may still be reading it.
    sheet.completionWithItemsHandler = { activityType, completed, _, _ in
        if completed || activityType == nil { try? FileManager.default.removeItem(at: url) }
    }
    top.present(sheet, animated: true)
    return true
}

@_cdecl("ghi_swift_share_file")
public func ghiSwiftShareFile(_ path: UnsafePointer<CChar>) -> Bool {
    let url = URL(fileURLWithPath: String(cString: path))
    guard FileManager.default.fileExists(atPath: url.path) else { return false }
    if Thread.isMainThread { return presentShare(url) }
    // From a Rust thread: wait briefly for the main thread's answer (never forever:
    // if it is busy the sheet may still appear, so the answer is then "shown").
    let box = ShareResult()
    let done = DispatchSemaphore(value: 0)
    DispatchQueue.main.async {
        box.set(presentShare(url))
        done.signal()
    }
    return done.wait(timeout: .now() + 1) == .timedOut ? true : box.value
}

private final class ShareResult: @unchecked Sendable {
    private let lock = NSLock()
    private var ok = false
    var value: Bool { lock.lock(); defer { lock.unlock() }; return ok }
    func set(_ v: Bool) { lock.lock(); ok = v; lock.unlock() }
}

@_cdecl("ghi_swift_open_settings")
public func ghiSwiftOpenSettings() {
    DispatchQueue.main.async {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
        UIApplication.shared.open(url)
    }
}

/// Background-task tokens handed to Rust (a UIBackgroundTaskIdentifier is not stable across the ABI).
private enum BackgroundTasks {
    static let lock = NSLock()
    static var next: UInt64 = 1
    static var tasks: [UInt64: UIBackgroundTaskIdentifier] = [:]

    static func end(_ token: UInt64) {
        lock.lock()
        let id = tasks.removeValue(forKey: token)
        lock.unlock()
        if let id { UIApplication.shared.endBackgroundTask(id) }
    }
}

@_cdecl("ghi_swift_begin_bg_task")
public func ghiSwiftBeginBgTask(_ name: UnsafePointer<CChar>) -> UInt64 {
    let label = String(cString: name)
    // The table is locked across begin + store, so an expiration handler that
    // fires at once waits for the entry and then ends it.
    BackgroundTasks.lock.lock()
    defer { BackgroundTasks.lock.unlock() }
    let token = BackgroundTasks.next
    BackgroundTasks.next += 1
    // beginBackgroundTask may be called from any thread.
    let id = UIApplication.shared.beginBackgroundTask(withName: label) {
        BackgroundTasks.end(token)
    }
    guard id != .invalid else { return 0 }
    BackgroundTasks.tasks[token] = id
    return token
}

@_cdecl("ghi_swift_end_bg_task")
public func ghiSwiftEndBgTask(_ token: UInt64) {
    BackgroundTasks.end(token)
}

/// One opaque window above everything (the app-switcher snapshot cover), so
/// windows created later are covered too. Main thread only.
private enum PrivacyCover {
    static var window: UIWindow?

    static func set(_ on: Bool) {
        guard on else {
            window?.isHidden = true
            window = nil
            return
        }
        guard window == nil,
              let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first
        else { return }
        let cover = UIWindow(windowScene: scene)
        cover.windowLevel = UIWindow.Level(rawValue: UIWindow.Level.alert.rawValue + 1000)
        let controller = UIViewController()
        controller.view.backgroundColor = .systemBackground
        let mark = UIImageView(image: UIImage(systemName: "lock.fill"))
        mark.tintColor = .secondaryLabel
        mark.translatesAutoresizingMaskIntoConstraints = false
        controller.view.addSubview(mark)
        NSLayoutConstraint.activate([
            mark.centerXAnchor.constraint(equalTo: controller.view.centerXAnchor),
            mark.centerYAnchor.constraint(equalTo: controller.view.centerYAnchor),
        ])
        cover.rootViewController = controller
        cover.isHidden = false
        window = cover
    }
}

@_cdecl("ghi_swift_set_privacy_cover")
public func ghiSwiftSetPrivacyCover(_ on: Bool) {
    // Synchronous when already on the main thread: the lifecycle observer raises
    // it from willResignActive, before the snapshot.
    if Thread.isMainThread { PrivacyCover.set(on) } else { DispatchQueue.main.async { PrivacyCover.set(on) } }
}

@_cdecl("ghi_swift_continuous_ns")
public func ghiSwiftContinuousNs() -> UInt64 {
    var tb = mach_timebase_info_data_t()
    mach_timebase_info(&tb)
    return mach_continuous_time() * UInt64(tb.numer) / UInt64(tb.denom)
}

@_cdecl("ghi_swift_mic_permission")
public func ghiSwiftMicPermission() -> Int32 {
    switch AVAudioApplication.shared.recordPermission {
    case .granted: return 1
    case .denied: return 2
    default: return 0
    }
}

@_cdecl("ghi_swift_request_mic_permission")
public func ghiSwiftRequestMicPermission() {
    AVAudioApplication.requestRecordPermission { _ in }
}
