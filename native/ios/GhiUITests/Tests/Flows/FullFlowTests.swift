// SPDX-License-Identifier: Apache-2.0
// Phase 16-K: the whole app on the Simulator, end to end, deterministic and
// without models: a `build-ios.sh --sim --test-hooks` build with GHI_FAKE_MIC
// (synthetic samples) and GHI_FAKE_ENGINES (scripted ASR + diarization, lines
// "câu số N hello there"). Start from a fresh install: run via flows.sh.
//
// One test, in order, because each step needs the state the last one left:
// onboarding -> record -> lock/unlock (catch-up) -> stop -> final pass ->
// meeting in the list -> search -> share-inbox import -> app lock -> delete all
// (back to onboarding). Labels are the English strings of
// packages/i18n/locales/mobile.

import XCTest

final class FullFlowTests: XCTestCase {
    override func setUp() { continueAfterFailure = false }

    private func staticText(_ app: XCUIApplication, _ s: String) -> XCUIElement {
        app.staticTexts.containing(NSPredicate(format: "label CONTAINS[c] %@", s)).firstMatch
    }

    private func button(_ app: XCUIApplication, startingWith s: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", s)).firstMatch
    }

    private func step(_ name: String, _ body: () throws -> Void) rethrows {
        try XCTContext.runActivity(named: name) { _ in try body() }
    }

    private func shot(_ app: XCUIApplication, _ name: String) {
        let a = XCTAttachment(screenshot: app.screenshot())
        a.name = name
        a.lifetime = .keepAlways
        add(a)
    }

    /// Opens a settings row by its label prefix (scrolling to it).
    private func openRow(_ app: XCUIApplication, _ prefix: String) {
        let row = button(app, startingWith: prefix)
        XCTAssertTrue(row.waitForExistence(timeout: 10), "no '\(prefix)' row\n" + app.debugDescription)
        for _ in 0..<4 where !row.isHittable { app.swipeUp() }
        row.tap()
    }

    func testFullFlow() throws {
        // A flow that cannot run is a failure, not a skip: a green run must mean it ran.
        let fakeMic = ProcessInfo.processInfo.environment["GHI_FAKE_MIC_PATH"] ?? ""
        XCTAssertFalse(fakeMic.isEmpty, "GHI_FAKE_MIC_PATH not set (run via flows.sh)")
        XCTAssertNotNil(Ghira.groupDir, "GHI_GROUP_DIR not set (run via flows.sh)")
        let app = Ghira.app(env: ["GHI_FAKE_MIC": fakeMic, "GHI_FAKE_ENGINES": "1", "GHI_DEVICE_TIER": "live"])
        app.launch()

        try step("1 onboarding") {
            XCTAssertTrue(staticText(app, "Which languages").waitForExistence(timeout: 30), "not a first launch\n" + app.debugDescription)
            Ghira.completeOnboarding(app)
            shot(app, "after-onboarding")
        }

        step("2 record") {
            Ghira.openRecordTab(app)
            if !Ghira.stopButton(app).exists {
                XCTAssertTrue(Ghira.recordButton(app).waitForExistence(timeout: 20), app.debugDescription)
                Ghira.recordButton(app).tap()
            }
            let consent = app.buttons["Everyone knows, start recording"]
            if consent.waitForExistence(timeout: 3) { consent.tap() }
            XCTAssertTrue(Ghira.stopButton(app).waitForExistence(timeout: 20), "recording did not start\n" + app.debugDescription)
            // The scripted engines speak every 4 s of audio.
            XCTAssertTrue(staticText(app, "hello there").waitForExistence(timeout: 30), "no live line\n" + app.debugDescription)
            shot(app, "recording")
        }

        step("3 lock and unlock") {
            XCUIDevice.shared.press(.home)
            sleep(1)
            Ghira.pressLockButton()
            sleep(12) // audio keeps being captured; the transcript catches up on return
            Ghira.pressLockButton()
            sleep(1)
            app.activate()
            Ghira.unlockIfNeeded(app)
            XCTAssertTrue(Ghira.stopButton(app).waitForExistence(timeout: 20), "recording stopped while locked\n" + app.debugDescription)
            // Lines from the locked stretch (about 12 s = 3 more lines) arrive.
            let lines = app.staticTexts.matching(NSPredicate(format: "label CONTAINS[c] %@", "hello there"))
            Ghira.waitUntil(60) { lines.count >= 4 }
            XCTAssertGreaterThanOrEqual(lines.count, 4, "transcript did not catch up\n" + app.debugDescription)
            shot(app, "after-unlock")
        }

        step("4 stop") {
            Ghira.stopButton(app).tap()
            XCTAssertTrue(staticText(app, "Recording saved").waitForExistence(timeout: 30), app.debugDescription)
        }

        step("5 final pass and the meeting in the list") {
            Ghira.tapTab(app, "Meetings")
            XCTAssertTrue(staticText(app, "Processed on phone").waitForExistence(timeout: 120), "final pass did not finish\n" + app.debugDescription)
            shot(app, "meetings")
        }

        step("6 search finds a word") {
            Ghira.tapTab(app, "Search")
            let field = app.searchFields.firstMatch.exists ? app.searchFields.firstMatch : app.textFields.firstMatch
            XCTAssertTrue(field.waitForExistence(timeout: 10), app.debugDescription)
            field.tap()
            field.typeText("hello")
            XCTAssertTrue(staticText(app, "result").waitForExistence(timeout: 20), "search found nothing\n" + app.debugDescription)
            shot(app, "search")
            // The keyboard covers the tab bar: the web view's accessory bar has Done.
            if app.keyboards.count > 0 { app.buttons["Done"].tap() }
        }

        try step("7 share-inbox import") {
            Ghira.tapTab(app, "Meetings")
            try Ghira.dropInboxItem(file: "Flow Import.wav", confirmed: true)
            // A second meeting, processed on the phone, next to the recording.
            let processed = app.staticTexts.matching(NSPredicate(format: "label == %@", "Processed on phone"))
            Ghira.waitUntil(90) { processed.count >= 2 }
            XCTAssertGreaterThanOrEqual(processed.count, 2, "imported file not listed\n" + app.debugDescription)
            shot(app, "after-import")
        }

        step("8 app lock") {
            Ghira.tapTab(app, "Settings")
            openRow(app, "Privacy and security")
            XCTAssertTrue(app.switches["Require Face ID"].waitForExistence(timeout: 10), app.debugDescription)
            Ghira.enableAppLockImmediately(app)
            XCUIDevice.shared.press(.home)
            sleep(1)
            app.activate()
            let unlock = app.buttons["Unlock with Face ID"]
            XCTAssertTrue(unlock.waitForExistence(timeout: 10), "lock gate not shown\n" + app.debugDescription)
            // The gate asks for Face ID by itself; on a slow machine the
            // prompt comes up after the first simulated result, which is then
            // lost, or it did not ask at all (the CI simulator): repeat each
            // result until it shows, asking with the button from the second try.
            let failed = staticText(app, "didn’t work")
            for attempt in 0..<8 where !failed.exists {
                if attempt > 0 { unlock.tap() }
                Ghira.faceID(match: false)
                _ = Ghira.waitUntil(2) { failed.exists }
            }
            XCTAssertTrue(failed.exists, app.debugDescription)
            XCTAssertTrue(unlock.exists, "unlocked on a failed Face ID")
            for _ in 0..<8 where unlock.exists {
                unlock.tap()
                Ghira.faceID(match: true)
                _ = Ghira.waitUntil(2) { !unlock.exists }
            }
            XCTAssertFalse(unlock.exists, "still locked after a match")
        }

        step("9 delete everything returns to onboarding") {
            // Still in Privacy and security after the unlock.
            openRow(app, "Delete everything")
            let field = app.textFields["Type DELETE to confirm"]
            XCTAssertTrue(field.waitForExistence(timeout: 10), app.debugDescription)
            field.tap()
            field.typeText("DELETE")
            if app.keyboards.count > 0 { app.buttons["Done"].tap() } // the keyboard covers the sheet's button
            // The sheet's button (the row behind it has the same label).
            let confirms = app.buttons.matching(NSPredicate(format: "label == %@ AND enabled == true", "Delete everything"))
            XCTAssertTrue(confirms.firstMatch.waitForExistence(timeout: 5), app.debugDescription)
            confirms.allElementsBoundByIndex.last!.tap()
            XCTAssertTrue(staticText(app, "Which languages").waitForExistence(timeout: 60), "not back at onboarding\n" + app.debugDescription)
            shot(app, "after-delete")
        }
    }
}
