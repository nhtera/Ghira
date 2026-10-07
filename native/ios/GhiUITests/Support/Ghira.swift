// SPDX-License-Identifier: Apache-2.0
// Shared helpers: the app under test is addressed by bundle id, never built here.

import CoreFoundation
import XCTest

enum Ghira {
    static let bundleId = "com.nhtera.ghira"

    /// Set by run.sh with GHI_DEVICE_UDID: a real iPhone (no simctl, no Face ID simulation, no App Group access).
    static var onDevice: Bool { !(ProcessInfo.processInfo.environment["GHI_ON_DEVICE"] ?? "").isEmpty }

    /// Scripted engines on a live-tier phone by default (CI has no models); set
    /// GHI_REAL_ENGINES=1 on the host to run against the models in the app.
    static func app(env: [String: String] = [:], args: [String] = []) -> XCUIApplication {
        let app = XCUIApplication(bundleIdentifier: bundleId)
        // The phone's data was sealed with the Keychain key store (release app): a debug test-hooks build
        // would otherwise look for its file key and report "the key for this store is missing".
        // A phone whose data a test-hooks build created uses its key file instead: GHI_KEYSTORE=file on the host.
        if onDevice { app.launchEnvironment["GHI_KEYSTORE"] = ProcessInfo.processInfo.environment["GHI_KEYSTORE"] ?? "keychain" }
        if (ProcessInfo.processInfo.environment["GHI_REAL_ENGINES"] ?? "").isEmpty {
            app.launchEnvironment["GHI_FAKE_ENGINES"] = "1"
            app.launchEnvironment["GHI_DEVICE_TIER"] = "live"
        }
        if let mic = ProcessInfo.processInfo.environment["GHI_FAKE_MIC_PATH"], !mic.isEmpty {
            app.launchEnvironment["GHI_FAKE_MIC"] = mic
            // Real engines need the WAV's speech through the audio tap, not the tone.
            if !(ProcessInfo.processInfo.environment["GHI_REAL_ENGINES"] ?? "").isEmpty {
                app.launchEnvironment["GHI_FAKE_MIC_TAP"] = "1"
            }
        }
        app.launchEnvironment.merge(env) { $1 }
        app.launchArguments += args
        return app
    }

    static var springboard: XCUIApplication {
        XCUIApplication(bundleIdentifier: "com.apple.springboard")
    }

    /// Accepts a system permission alert (camera, local network) if one shows
    /// within `timeout`: a real phone asks once, the Simulator never does.
    static func allowSystemAlert(timeout: TimeInterval = 5) {
        let labels = ["Allow", "OK", "Cho phép"]
        let allow = springboard.buttons.matching(NSPredicate(format: "label IN %@", labels)).firstMatch
        if allow.waitForExistence(timeout: timeout) { allow.tap() }
    }

    /// The simulator has no Lock command in simctl; the private selector works.
    static func pressLockButton() {
        XCUIDevice.shared.perform(NSSelectorFromString("pressLockButton"))
    }

    /// "Record room": the record screen's big button (not the "Record" tab).
    static func recordButton(_ app: XCUIApplication) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Record ")).firstMatch
    }

    static func stopButton(_ app: XCUIApplication) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Stop")).firstMatch
    }

    /// Passes the app-lock gate if it is up (a run that failed half way can leave the lock on).
    static func unlockIfNeeded(_ app: XCUIApplication) {
        let gate = app.buttons["Unlock with Face ID"]
        guard gate.waitForExistence(timeout: 2) else { return }
        // The gate asks for Face ID by itself when it appears; if not, tap it.
        for attempt in 0..<3 {
            if attempt > 0 { gate.tap() }
            faceID(match: true)
            if gate.waitForNonExistence(timeout: 5) { return }
        }
    }

    /// On the Privacy screen: Require Face ID on, locking as soon as the app is
    /// reopened. The delay choice only takes once the Face ID save is done, so it
    /// is repeated until it shows as selected.
    static func enableAppLockImmediately(_ app: XCUIApplication) {
        let toggle = app.switches["Require Face ID"]
        if toggle.value as? String != "1" {
            toggle.tap()
            faceID(match: true)
        }
        let immediately = app.buttons["Only when reopened Selected"]
        for _ in 0..<5 where !immediately.exists {
            _ = waitUntil(2) { immediately.exists }
            app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Only when reopened")).firstMatch.tap()
        }
        XCTAssertTrue(immediately.waitForExistence(timeout: 5), "lock delay not set\n" + app.debugDescription)
    }

    /// Opens the Record tab when the app launched on another one (it restores the last).
    static func openRecordTab(_ app: XCUIApplication) {
        if recordButton(app).waitForExistence(timeout: 3) || stopButton(app).exists { return }
        tapTab(app, "Record")
    }

    /// A tab-bar item (the @ghi/ui TabBar: buttons).
    static func tapTab(_ app: XCUIApplication, _ name: String) {
        for query in [app.buttons, app.links, app.staticTexts] where query[name].waitForExistence(timeout: 3) {
            query[name].tap()
            return
        }
    }

    /// Walks onboarding to the end (first launch only; a no-op once the Record
    /// button is there). Takes the first forward button of each step.
    static func completeOnboarding(_ app: XCUIApplication) {
        unlockIfNeeded(app)
        let forward = ["Continue", "Allow microphone", "Not now", "Download later", "Skip", "Skip for now", "Done", "Start", "Get started", "Finish", "Start recording"]
        for _ in 0..<20 {
            // The last step's "Start recording" lands on a running recording.
            if recordButton(app).waitForExistence(timeout: 3) || stopButton(app).exists { return }
            var tapped = false
            for name in forward where app.buttons[name].exists {
                app.buttons[name].tap()
                tapped = true
                break
            }
            if !tapped {
                // Past onboarding (the app restored another tab): nothing to do.
                if app.staticTexts["Record"].exists { return }
                NSLog("ghira-uitest: onboarding stuck on: \(app.buttons.allElementsBoundByIndex.map(\.label))")
                return
            }
            sleep(1)
        }
    }

    /// Waits for a condition without a fixed sleep (XCTNSPredicateExpectation polls and returns early).
    @discardableResult
    static func waitUntil(_ timeout: TimeInterval, _ condition: @escaping () -> Bool) -> Bool {
        let predicate = NSPredicate { _, _ in condition() }
        let expectation = XCTNSPredicateExpectation(predicate: predicate, object: nil)
        return XCTWaiter().wait(for: [expectation], timeout: timeout) == .completed
    }

    // MARK: - Darwin notifications (simulator-wide; the app's test hooks listen)

    static let hookPrefix = "com.nhtera.ghira.test."

    /// Posts a Darwin notification on this simulator.
    static func post(_ name: String) {
        CFNotificationCenterPostNotification(
            CFNotificationCenterGetDarwinNotifyCenter(), CFNotificationName(name as CFString), nil, nil, true)
    }

    /// A test hook of a `build-ios.sh --sim --test-hooks` build:
    /// interrupt-begin, interrupt-end, thermal-serious, thermal-nominal,
    /// route-change, memory-warning, call-active, call-ended.
    static func hook(_ name: String) { post(hookPrefix + name) }

    /// The simulated Face ID result (enrol first: `sim.sh biometrics enroll`).
    static func faceID(match: Bool) {
        sleep(1) // the system prompt must be up before the result is posted
        post(match ? "com.apple.BiometricKit_Sim.pearl.match" : "com.apple.BiometricKit_Sim.pearl.nomatch")
    }

    // MARK: - Files

    /// The App Group container on the simulator (the host passes it; see run.sh).
    static var groupDir: URL? {
        ProcessInfo.processInfo.environment["GHI_GROUP_DIR"].flatMap { $0.isEmpty ? nil : URL(fileURLWithPath: $0) }
    }

    /// A tiny silent 16 kHz mono WAV.
    static func silentWav(seconds: Int = 1) -> Data {
        let n = 16000 * seconds
        var d = Data()
        func u32(_ v: Int) { withUnsafeBytes(of: UInt32(v).littleEndian) { d.append(contentsOf: $0) } }
        func u16(_ v: Int) { withUnsafeBytes(of: UInt16(v).littleEndian) { d.append(contentsOf: $0) } }
        d.append(contentsOf: Array("RIFF".utf8)); u32(36 + n * 2); d.append(contentsOf: Array("WAVEfmt ".utf8))
        u32(16); u16(1); u16(1); u32(16000); u32(32000); u16(2); u16(16)
        d.append(contentsOf: Array("data".utf8)); u32(n * 2); d.append(Data(count: n * 2))
        return d
    }

    /// What a share extension leaves: `inbox/<uuid>/<file>` then `manifest.json`.
    @discardableResult
    static func dropInboxItem(file: String, confirmed: Bool) throws -> URL {
        guard let group = groupDir else { throw XCTSkip("GHI_GROUP_DIR not set (run via run.sh)") }
        let dir = group.appendingPathComponent("inbox/\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try silentWav().write(to: dir.appendingPathComponent(file))
        let manifest = ["file": file, "lang": "auto", "target": "phone", "source": "test", "confirmed": confirmed] as [String: Any]
        try JSONSerialization.data(withJSONObject: manifest).write(to: dir.appendingPathComponent("manifest.json"))
        post("com.nhtera.ghira.inbox_changed")
        return dir
    }

    /// Dynamic Type for one launch (Accessibility 3 = AX3).
    static let accessibility3 = ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXL"]

    /// The audio engine's state, read through the `probe-engine` test hook
    /// (the app writes engine-running.txt into the App Group).
    static func engineRunning(timeout: TimeInterval = 5) -> Bool? {
        guard let dir = groupDir else { return nil }
        let file = dir.appendingPathComponent("engine-running.txt")
        try? FileManager.default.removeItem(at: file)
        hook("probe-engine")
        var running: Bool?
        waitUntil(timeout) {
            guard let v = try? String(contentsOf: file, encoding: .utf8) else { return false }
            running = v == "1"
            return true
        }
        return running
    }
}
