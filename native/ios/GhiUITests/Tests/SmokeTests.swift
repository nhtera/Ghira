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
        Ghira.pressLockButton()
        sleep(UInt32(seconds))
        Ghira.pressLockButton()
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
}
