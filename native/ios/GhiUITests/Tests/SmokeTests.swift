// SPDX-License-Identifier: Apache-2.0
// Spikes 16-B (c) and (e): XCUITest drives the installed app, and the app
// keeps recording through home + lock. Needs a build from
// `build-ios.sh --sim --test-hooks`, a booted simulator with the app installed,
// the microphone granted (`sim.sh grant`) and models (`sim.sh models`).

import XCTest

final class SmokeTests: XCTestCase {
    override func setUp() { continueAfterFailure = false }

    func testLaunchShowsTheApp() {
        let app = Ghira.app()
        app.launch()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 20))
        // The webview content is reached through the accessibility tree: the
        // first screen is onboarding, or the tab bar once it is done.
        Ghira.unlockIfNeeded(app) // whatever state an earlier run left (lock on, onboarding done or not)
        let first = app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "Which languages")).firstMatch
        XCTAssertTrue(first.waitForExistence(timeout: 30) || app.buttons["Record"].exists, app.debugDescription)
        add(XCTAttachment(screenshot: app.screenshot()))
    }

    /// Record with the fake mic, go home, lock for GHI_BG_SECONDS (default
    /// 120), come back: the timer must have kept running (not suspended).
    func testRecordingSurvivesHomeAndLock() throws {
        let fakeMic = ProcessInfo.processInfo.environment["GHI_FAKE_MIC_PATH"] ?? ""
        let seconds = Double(ProcessInfo.processInfo.environment["GHI_BG_SECONDS"] ?? "") ?? 120
        let app = Ghira.app(env: fakeMic.isEmpty ? [:] : ["GHI_FAKE_MIC": fakeMic])
        app.launch()
        Ghira.completeOnboarding(app)
        Ghira.openRecordTab(app)
        if !Ghira.stopButton(app).exists {
            XCTAssertTrue(Ghira.recordButton(app).waitForExistence(timeout: 20), app.debugDescription)
            Ghira.recordButton(app).tap()
        }
        let consent = app.buttons["Everyone knows, start recording"]
        if consent.waitForExistence(timeout: 3) { consent.tap() }
        XCTAssertTrue(Ghira.stopButton(app).waitForExistence(timeout: 20), "recording did not start\n" + app.debugDescription)
        sleep(3)

        XCUIDevice.shared.press(.home)
        sleep(2)
        // A device has a passcode: no lock button dance, the app just stays in the background.
        if !Ghira.onDevice { Ghira.pressLockButton() }
        sleep(UInt32(seconds))
        if !Ghira.onDevice { Ghira.pressLockButton() }
        sleep(2)
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 20))

        // Elapsed time shows as mm:ss; after `seconds` it is at least that.
        let timer = app.staticTexts.matching(NSPredicate(format: "label MATCHES %@", "^[0-9]{1,2}:[0-9]{2}$")).firstMatch
        XCTAssertTrue(timer.waitForExistence(timeout: 10))
        let parts = timer.label.split(separator: ":").compactMap { Double($0) }
        let elapsed = parts.count == 2 ? parts[0] * 60 + parts[1] : 0
        add(XCTAttachment(screenshot: app.screenshot()))
        XCTAssertGreaterThanOrEqual(elapsed, seconds, "timer shows \(timer.label)")
        Ghira.stopButton(app).tap()
    }

    /// Record with the fake mic and stay in the foreground for GHI_REC_SECONDS
    /// (default 100), logging once a second what the screen shows (numbers
    /// only: transcript text nodes and their characters). Compare with the
    /// app's live.jsonl `ui` windows to tell a stalled engine from a stalled screen.
    func testForegroundTranscriptKeepsMoving() throws {
        let fakeMic = ProcessInfo.processInfo.environment["GHI_FAKE_MIC_PATH"] ?? ""
        let seconds = Double(ProcessInfo.processInfo.environment["GHI_REC_SECONDS"] ?? "") ?? 100
        let app = Ghira.app(env: ["GHI_KEYSTORE": "keychain", "GHI_IGNORE_THERMAL": "1"].merging(fakeMic.isEmpty ? [:] : ["GHI_FAKE_MIC": fakeMic]) { $1 })
        app.launch()
        Ghira.completeOnboarding(app)
        Ghira.openRecordTab(app)
        if !Ghira.stopButton(app).exists {
            XCTAssertTrue(Ghira.recordButton(app).waitForExistence(timeout: 20), app.debugDescription)
            Ghira.recordButton(app).tap()
        }
        let consent = app.buttons["Everyone knows, start recording"]
        if consent.waitForExistence(timeout: 3) { consent.tap() }
        XCTAssertTrue(Ghira.stopButton(app).waitForExistence(timeout: 20), "recording did not start\n" + app.debugDescription)
        let start = Date()
        var lastChars = -1
        var lastChange = 0.0
        var maxStill = 0.0
        while Date().timeIntervalSince(start) < seconds {
            let t = Date().timeIntervalSince(start)
            func texts(_ e: XCUIElementSnapshot) -> [String] {
                (e.elementType == .staticText && !e.label.isEmpty ? [e.label] : []) + e.children.flatMap { texts($0) }
            }
            // One atomic snapshot: the tree changes under a query by element.
            guard let snap = try? app.snapshot() else { sleep(1); continue }
            let labels = texts(snap)
            let chars = labels.reduce(0) { $0 + $1.count }
            if chars != lastChars { lastChars = chars; lastChange = t }
            maxStill = max(maxStill, t - lastChange)
            print("UIPROBE t=\(Int(t)) nodes=\(labels.count) chars=\(chars) still=\(Int(t - lastChange))")
            sleep(1)
        }
        print("UIPROBE max_still_s=\(Int(maxStill))")
        add(XCTAttachment(screenshot: app.screenshot()))
        Ghira.stopButton(app).tap()
    }

    /// Record, leave for the home screen for a few seconds and come back: the
    /// transcript must go on changing (text nodes on screen), not stay where
    /// it was. Logs `UIPROBE` lines like the foreground test.
    func testTranscriptGoesOnAfterHome() throws {
        let fakeMic = ProcessInfo.processInfo.environment["GHI_FAKE_MIC_PATH"] ?? ""
        let away = Double(ProcessInfo.processInfo.environment["GHI_AWAY_SECONDS"] ?? "") ?? 8
        let app = Ghira.app(env: ["GHI_KEYSTORE": "keychain", "GHI_IGNORE_THERMAL": "1"].merging(fakeMic.isEmpty ? [:] : ["GHI_FAKE_MIC": fakeMic]) { $1 })
        app.launch()
        Ghira.completeOnboarding(app)
        Ghira.openRecordTab(app)
        if !Ghira.stopButton(app).exists {
            XCTAssertTrue(Ghira.recordButton(app).waitForExistence(timeout: 20), app.debugDescription)
            Ghira.recordButton(app).tap()
        }
        let consent = app.buttons["Everyone knows, start recording"]
        if consent.waitForExistence(timeout: 3) { consent.tap() }
        XCTAssertTrue(Ghira.stopButton(app).waitForExistence(timeout: 20), "recording did not start\n" + app.debugDescription)

        func chars() -> Int {
            func texts(_ e: XCUIElementSnapshot) -> Int {
                (e.elementType == .staticText ? e.label.count : 0) + e.children.reduce(0) { $0 + texts($1) }
            }
            guard let snap = try? app.snapshot() else { return -1 }
            return texts(snap)
        }
        func watch(_ name: String, _ seconds: Double) -> Double {
            let start = Date()
            var last = chars(), lastChange = 0.0, maxStill = 0.0
            while Date().timeIntervalSince(start) < seconds {
                let t = Date().timeIntervalSince(start)
                let c = chars()
                if c != last { last = c; lastChange = t }
                maxStill = max(maxStill, t - lastChange)
                print("UIPROBE \(name) t=\(Int(t)) chars=\(c) still=\(Int(t - lastChange))")
                sleep(1)
            }
            return maxStill
        }
        let before = watch("before", 25)
        XCUIDevice.shared.press(.home)
        sleep(UInt32(away))
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 20))
        let after = watch("after", 30)
        print("UIPROBE max_still_before=\(Int(before)) max_still_after=\(Int(after))")
        Ghira.stopButton(app).tap()
    }
}
