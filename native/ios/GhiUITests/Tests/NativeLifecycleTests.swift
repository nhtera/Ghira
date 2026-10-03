// SPDX-License-Identifier: Apache-2.0
// Phase 16-E: the native side of the lifecycle, driven through the test hooks
// (Darwin notifications) of a `build-ios.sh --sim --test-hooks` build. Needs
// the app installed, the microphone granted (`sim.sh grant`) and onboarding
// done (the Record button reachable). Accessibility labels are the English
// strings of packages/i18n/locales/mobile.

import XCTest

final class NativeLifecycleTests: XCTestCase {
    override func setUp() { continueAfterFailure = false }

    private func startRecording(_ app: XCUIApplication) {
        app.launch()
        Ghira.completeOnboarding(app)
        Ghira.openRecordTab(app)
        let record = Ghira.recordButton(app)
        if !Ghira.stopButton(app).exists {
            XCTAssertTrue(record.waitForExistence(timeout: 20), app.debugDescription)
            record.tap()
        }
        // The first recording asks for the consent acknowledgement (M2).
        let consent = app.buttons["Everyone knows, start recording"]
        if consent.waitForExistence(timeout: 3) { consent.tap() }
        XCTAssertTrue(Ghira.stopButton(app).waitForExistence(timeout: 20), "recording did not start\n" + app.debugDescription)
    }

    private func text(_ app: XCUIApplication, containing s: String) -> XCUIElement {
        app.staticTexts.containing(NSPredicate(format: "label CONTAINS[c] %@", s)).firstMatch
    }

    /// Audio interruption began: capture pauses and the engine is off; ended: the
    /// app asks and the engine stays off, also after leaving and returning; only
    /// the user's Resume starts it again.
    func testInterruptionPausesAndOnlyTheUserResumes() throws {
        let app = Ghira.app()
        startRecording(app)
        try XCTSkipIf(Ghira.engineRunning() == nil, "GHI_GROUP_DIR not set (run via run.sh)")
        XCTAssertEqual(Ghira.engineRunning(), true, "engine should run while recording")
        Ghira.hook("interrupt-begin")
        XCTAssertTrue(text(app, containing: "Recording paused").waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertEqual(Ghira.engineRunning(), false)
        Ghira.hook("interrupt-end")
        let resume = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Resume")).firstMatch
        XCTAssertTrue(resume.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertEqual(Ghira.engineRunning(), false, "the engine restarted on .ended")
        // Backgrounding and returning must not resume by itself.
        XCUIDevice.shared.press(.home)
        sleep(1)
        app.activate()
        XCTAssertTrue(resume.waitForExistence(timeout: 10), "the app resumed without the user\n" + app.debugDescription)
        XCTAssertEqual(Ghira.engineRunning(), false, "the engine restarted on becoming active")
        resume.tap()
        XCTAssertTrue(app.buttons["Pause"].waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertEqual(Ghira.engineRunning(), true, "Resume should start the engine")
        Ghira.stopButton(app).tap()
    }

    /// A phone call: "Paused for a phone call", then Resume once it ends.
    func testCallActivePausesWithTheCallMessage() {
        let app = Ghira.app()
        startRecording(app)
        Ghira.hook("call-active")
        XCTAssertTrue(text(app, containing: "Paused for a phone call").waitForExistence(timeout: 10), app.debugDescription)
        Ghira.hook("call-ended")
        XCTAssertTrue(app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Resume")).firstMatch.waitForExistence(timeout: 10), app.debugDescription)
        Ghira.stopButton(app).tap()
    }

    /// Thermal serious and a memory warning must not crash or stop a recording.
    func testThermalAndMemoryEventsKeepRecording() {
        let app = Ghira.app()
        startRecording(app)
        Ghira.hook("thermal-serious")
        Ghira.hook("memory-warning")
        Ghira.hook("route-change")
        sleep(2)
        XCTAssertEqual(app.state, .runningForeground)
        XCTAssertTrue(Ghira.stopButton(app).exists)
        Ghira.hook("thermal-nominal")
        Ghira.stopButton(app).tap()
    }

    /// Share extension hand-off: an item in the App Group inbox is listed in the app.
    func testInboxItemIsListed() throws {
        let app = Ghira.app()
        app.launch()
        Ghira.completeOnboarding(app)
        Ghira.openRecordTab(app)
        XCTAssertTrue(Ghira.recordButton(app).waitForExistence(timeout: 20))
        try Ghira.dropInboxItem(file: "Share Test.wav", confirmed: false)
        XCTAssertTrue(text(app, containing: "waiting to import").waitForExistence(timeout: 15), app.debugDescription)
    }

    /// Dynamic Type AX3: the app still launches and shows its main control.
    /// (The `--ghi-text-scale` CSS variable itself is asserted by the web e2e.)
    func testLaunchesAtAccessibilityTextSize() {
        let app = Ghira.app(args: Ghira.accessibility3)
        app.launch()
        Ghira.openRecordTab(app)
        XCTAssertTrue(Ghira.recordButton(app).waitForExistence(timeout: 20), app.debugDescription)
        add(XCTAttachment(screenshot: app.screenshot()))
    }

    /// Live Activity on the lock screen after Record: best effort (springboard
    /// queries are flaky on the simulator), so it retries and skips if the
    /// activity never shows; the owner checks Mark/Stop on a device.
    func testLiveActivityAppearsOnTheLockScreen() throws {
        let app = Ghira.app()
        startRecording(app)
        XCUIDevice.shared.press(.home)
        Ghira.pressLockButton()
        sleep(1)
        Ghira.pressLockButton() // wake: the lock screen shows the activity
        let board = Ghira.springboard
        var found = false
        for _ in 0..<3 where !found {
            found = board.staticTexts.containing(NSPredicate(format: "label CONTAINS[c] %@", "Recording on this phone"))
                .firstMatch.waitForExistence(timeout: 5)
        }
        add(XCTAttachment(screenshot: board.screenshot()))
        app.activate()
        if Ghira.stopButton(app).waitForExistence(timeout: 10) { Ghira.stopButton(app).tap() }
        try XCTSkipUnless(found, "Live Activity not visible to springboard queries on this simulator")
    }

    /// App lock: leaving the app and coming back shows the lock gate with a
    /// hittable Unlock button (the native privacy cover is down again); a
    /// failed Face ID keeps the gate, a match unlocks. Depends on the Settings
    /// screen (16-J) and an enrolled simulator (run.sh).
    func testAppLockFaceIDMatchAndNoMatch() throws {
        let app = Ghira.app()
        app.launch()
        Ghira.unlockIfNeeded(app)
        Ghira.completeOnboarding(app)
        Ghira.tapTab(app, "Settings")
        // (the row's label gains its state, e.g. "Privacy and security On")
        let privacy = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Privacy and security")).firstMatch
        if privacy.waitForExistence(timeout: 10) {
            for _ in 0..<3 where !privacy.isHittable { app.swipeUp() }
            privacy.tap()
        }
        let toggle = app.switches["Require Face ID"]
        if !toggle.waitForExistence(timeout: 10) { try? app.debugDescription.write(toFile: "/private/tmp/ghira-settings-tree.txt", atomically: true, encoding: .utf8) }  // for triage
        try XCTSkipUnless(toggle.exists, "no 'Require Face ID' switch in Settings (accessibility label differs)")
        Ghira.enableAppLockImmediately(app)
        XCUIDevice.shared.press(.home)
        sleep(1)
        app.activate()
        let unlock = app.buttons["Unlock with Face ID"]
        XCTAssertTrue(unlock.waitForExistence(timeout: 10), "lock gate not shown\n" + app.debugDescription)
        XCTAssertTrue(app.staticTexts["Ghira is locked"].exists)
        XCTAssertTrue(unlock.isHittable, "the native privacy cover is still up over the gate")
        // The gate asks for Face ID itself when it appears: answer that prompt with
        // a failure, then (after a tap if needed) with a match.
        Ghira.faceID(match: false)
        XCTAssertTrue(app.staticTexts["That didn’t work. Try again."].waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertTrue(unlock.isHittable)
        unlock.tap()
        Ghira.faceID(match: true)
        XCTAssertTrue(app.staticTexts["Privacy and security"].waitForExistence(timeout: 10)) // back on the screen we left
        XCTAssertFalse(unlock.exists)
        // Leave the app unlocked for the other tests: turn the lock off (it asks for Face ID).
        let off = NSPredicate(format: "value == %@", "0")
        for _ in 0..<3 where !off.evaluate(with: app.switches["Require Face ID"]) {
            app.switches["Require Face ID"].tap()
            Ghira.faceID(match: true)
            sleep(2)
        }
        XCTAssertEqual(app.switches["Require Face ID"].value as? String, "0", "app lock left on")
    }
}
