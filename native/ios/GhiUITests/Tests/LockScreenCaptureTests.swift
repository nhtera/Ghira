// SPDX-License-Identifier: Apache-2.0
// Screenshots of the recording Live Activity (Dynamic Island compact/expanded,
// lock screen) in a few states, for comparing against the design. Opt-in: set
// TEST_RUNNER_GHI_CAPTURE_DIR (and _TAG) on the host; skipped otherwise.
// Simulator only; needs a test-hooks build, the microphone granted and
// onboarding done, like NativeLifecycleTests.

import XCTest

final class LockScreenCaptureTests: XCTestCase {
    override func setUp() { continueAfterFailure = false }

    private func shot(_ name: String) {
        let env = ProcessInfo.processInfo.environment
        let dir = env["GHI_CAPTURE_DIR"] ?? ""
        let tag = env["GHI_CAPTURE_TAG"] ?? "run"
        let png = Ghira.springboard.screenshot().pngRepresentation
        try? png.write(to: URL(fileURLWithPath: dir).appendingPathComponent("app-\(name)-\(tag).png"))
    }

    private func lockAndWake() {
        Ghira.pressLockButton()
        sleep(2)
        Ghira.pressLockButton()
        sleep(3)
    }

    /// iOS asks (and later re-asks) whether the app may show Live Activities.
    private func allowLiveActivities() {
        let ok = NSPredicate(format: "label IN %@", ["Allow", "Always Allow", "Cho phép", "Luôn cho phép"])
        let allow = Ghira.springboard.buttons.matching(ok).firstMatch
        if allow.waitForExistence(timeout: 3) { allow.tap(); sleep(2) }
    }

    func testCaptureLiveActivity() throws {
        try XCTSkipIf(Ghira.onDevice, "simulator only")
        let dir = ProcessInfo.processInfo.environment["GHI_CAPTURE_DIR"] ?? ""
        try XCTSkipIf(dir.isEmpty, "set TEST_RUNNER_GHI_CAPTURE_DIR to capture")
        // The app UI stays English (the test taps its labels); the widget follows the device language.
        let app = Ghira.app(args: ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"])
        app.launch()
        Ghira.completeOnboarding(app)
        Ghira.openRecordTab(app)
        let record = Ghira.recordButton(app)
        if !Ghira.stopButton(app).exists {
            XCTAssertTrue(record.waitForExistence(timeout: 20))
            record.tap()
        }
        let consent = app.buttons["Everyone knows, start recording"]
        if consent.waitForExistence(timeout: 3) { consent.tap() }
        XCTAssertTrue(Ghira.stopButton(app).waitForExistence(timeout: 20), "recording did not start")

        // Dynamic Island, app in the background (screen on).
        XCUIDevice.shared.press(.home)
        sleep(4)
        shot("island-compact")
        let top = Ghira.springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.04))
        top.press(forDuration: 1.5)
        sleep(2)
        shot("island-expanded")
        Ghira.springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.7)).tap()
        sleep(1)

        // Lock screen: recording, then paused (interruption), then a call.
        lockAndWake()
        allowLiveActivities()
        shot("lock-recording")
        Ghira.hook("interrupt-begin")
        sleep(4)
        allowLiveActivities()
        shot("lock-paused")
        sleep(6)
        shot("lock-paused-later") // the frozen timer shows the same time
        Ghira.hook("interrupt-end")
        Ghira.hook("call-active")
        sleep(4)
        allowLiveActivities()
        shot("lock-call")
        Ghira.hook("call-ended")
        // The user's Resume (in the app) continues the timer from where it froze.
        app.activate()
        let resume = app.buttons.matching(NSPredicate(format: "label CONTAINS[c] %@", "Resume")).firstMatch
        if resume.waitForExistence(timeout: 8) {
            resume.tap()
            sleep(2)
            XCUIDevice.shared.press(.home)
            lockAndWake()
            allowLiveActivities()
            shot("lock-resumed")
        }

        app.activate()
        if Ghira.stopButton(app).waitForExistence(timeout: 10) { Ghira.stopButton(app).tap() }
    }
}
