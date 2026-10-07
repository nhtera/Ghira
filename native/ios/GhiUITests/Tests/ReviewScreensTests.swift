// SPDX-License-Identifier: Apache-2.0
// The cloud model menu and Transcribe again, checked on the installed app with
// screenshots attached (a real phone keeps its own meetings and settings: what
// the test turns on for a look, it turns off again, and it never confirms a
// new transcript). Needs onboarding done; the meeting test needs a meeting.
//   native/ios/GhiUITests/run.sh -only-testing:GhiUITests/ReviewScreensTests

import XCTest

final class ReviewScreensTests: XCTestCase {
    override func setUp() { continueAfterFailure = false }

    /// The menu per provider, in the order of crates/ghi-llm/prices.toml.
    private let menus: [String: [String]] = [
        "OpenAI": ["gpt-6.1-sol", "gpt-6-luna", "gpt-6-astra", "gpt-4.1-mini"],
        "Anthropic": ["claude-sonnet-5-5", "claude-haiku-4-5", "claude-opus-5-5"],
        "Gemini": ["gemini-3.8-flash", "gemini-3.5-flash-lite", "gemini-3.1-flash-lite"],
    ]

    private func shot(_ name: String) {
        let a = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        a.name = name
        a.lifetime = .keepAlways
        add(a)
    }

    private func button(_ app: XCUIApplication, startingWith s: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", s)).firstMatch
    }

    private func scrollTo(_ app: XCUIApplication, _ e: XCUIElement, swipes: Int = 6) {
        for _ in 0..<swipes where !(e.exists && e.isHittable) { app.swipeUp() }
    }

    /// The model rows on screen (their labels start with the model id).
    private func modelRows(_ app: XCUIApplication) -> [String] {
        let all = menus.values.flatMap { $0 }
        return app.buttons.allElementsBoundByIndex.map(\.label).filter { label in
            label.range(of: #"^(gpt|claude|gemini|o[0-9])[-.0-9a-z]*"#, options: .regularExpression) != nil
                || all.contains { label.hasPrefix($0) }
        }
    }

    func testCloudModelMenuIsCurrent() {
        let app = Ghira.app()
        app.launch()
        Ghira.completeOnboarding(app)
        Ghira.tapTab(app, "Settings")
        let row = button(app, startingWith: "Cloud notes")
        XCTAssertTrue(row.waitForExistence(timeout: 10), app.debugDescription)
        scrollTo(app, row)
        row.tap()
        let offer = app.switches["Offer cloud notes"]
        XCTAssertTrue(offer.waitForExistence(timeout: 10), app.debugDescription)
        let wasOff = (offer.value as? String) != "1"
        if wasOff { offer.tap() }
        let none = button(app, startingWith: "None")
        XCTAssertTrue(none.waitForExistence(timeout: 10), app.debugDescription)
        let chosen = menus.keys.first { name in
            app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@ AND label CONTAINS %@", name, "Selected")).firstMatch.exists
        }
        // A phone with a provider set keeps it: only its menu is read. Otherwise each is picked in turn, then None again.
        let names = chosen.map { [$0] } ?? ["OpenAI", "Anthropic", "Gemini"]
        for name in names {
            if chosen == nil {
                let provider = button(app, startingWith: name)
                XCTAssertTrue(provider.waitForExistence(timeout: 5), "no \(name) row\n" + app.debugDescription)
                provider.tap()
            }
            let first = button(app, startingWith: menus[name]![0])
            XCTAssertTrue(first.waitForExistence(timeout: 10), "\(name): no \(menus[name]![0])\n" + app.debugDescription)
            scrollTo(app, button(app, startingWith: menus[name]!.last!), swipes: 3)
            shot("cloud-\(name)")
            let rows = modelRows(app)
            XCTAssertEqual(rows.count, menus[name]!.count, "\(name) lists \(rows)")
            for (label, model) in zip(rows, menus[name]!) {
                XCTAssertTrue(label.hasPrefix(model), "\(name) lists \(rows)")
            }
            app.swipeDown()
            app.swipeDown()
        }
        if chosen == nil {
            none.tap()
            XCTAssertTrue(Ghira.waitUntil(5) { self.modelRows(app).isEmpty }, app.debugDescription)
        }
        if wasOff {
            for _ in 0..<3 where !offer.isHittable { app.swipeDown() }
            offer.tap()
            XCTAssertTrue(Ghira.waitUntil(5) { (offer.value as? String) != "1" }, "cloud notes were left on")
        }
    }

    /// The newest meeting's notes and action items, as screenshots (notes written on the phone).
    func testNewestMeetingNotes() throws {
        let app = Ghira.app()
        app.launch()
        Ghira.completeOnboarding(app)
        _ = try openFirstMeeting(app)
        for (tab, name) in [("Notes", "notes"), ("Actions", "actions")] {
            let button = app.buttons[tab]
            XCTAssertTrue(button.waitForExistence(timeout: 10), app.debugDescription)
            button.tap()
            sleep(1)
            shot("\(name)-top")
            app.swipeUp()
            shot("\(name)-more")
            app.swipeDown(velocity: .fast)
            app.swipeDown(velocity: .fast)
        }
    }

    /// Settings → Models on an 8 GB phone: the notes model's own section.
    func testNotesModelSection() {
        let app = Ghira.app()
        app.launch()
        Ghira.completeOnboarding(app)
        Ghira.tapTab(app, "Settings")
        let row = button(app, startingWith: "Models")
        XCTAssertTrue(row.waitForExistence(timeout: 10), app.debugDescription)
        row.tap()
        let header = app.staticTexts["Notes on this phone"]
        for _ in 0..<4 where !(header.exists && header.isHittable) { app.swipeUp() }
        XCTAssertTrue(header.exists, "no notes model section\n" + app.debugDescription)
        shot("models-notes")
    }

    /// TEST_RUNNER_GHI_WRITE_NOTES_IN=<text in a meeting row> (a throwaway meeting without notes):
    /// "Write notes" in its Notes tab, then the notes written on the phone.
    func testWriteNotesOnThePhone() throws {
        let match = ProcessInfo.processInfo.environment["GHI_WRITE_NOTES_IN"] ?? ""
        try XCTSkipIf(match.isEmpty, "GHI_WRITE_NOTES_IN not set")
        let app = Ghira.app()
        app.launch()
        Ghira.completeOnboarding(app)
        Ghira.tapTab(app, "Meetings")
        let meeting = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", match)).firstMatch
        for _ in 0..<6 where !(meeting.exists && meeting.isHittable) { app.swipeUp() }
        XCTAssertTrue(meeting.exists, "no meeting with \(match)\n" + app.debugDescription)
        meeting.tap()
        app.buttons["Notes"].tap()
        let write = app.buttons["Write notes"]
        XCTAssertTrue(write.waitForExistence(timeout: 10), app.debugDescription)
        shot("notes-empty")
        write.tap()
        XCTAssertTrue(app.staticTexts["Writing notes…"].waitForExistence(timeout: 10), app.debugDescription)
        shot("notes-writing")
        XCTAssertTrue(app.staticTexts["SUMMARY"].waitForExistence(timeout: 300) || app.staticTexts["Summary"].exists,
                      "no notes after 5 min\n" + app.debugDescription)
        sleep(1)
        shot("notes-written")
    }

    /// Opens the first meeting of the list: a webview button between the search field and the tab bar.
    private func openFirstMeeting(_ app: XCUIApplication) throws -> String {
        Ghira.tapTab(app, "Meetings")
        sleep(2)
        let tabBar = app.otherElements["Tabs, navigation"].frame.minY
        let meeting = app.buttons.allElementsBoundByIndex.first { b in
            b.isHittable && b.frame.minY > 200 && b.frame.maxY < tabBar && !b.label.hasPrefix("Delete")
                && !b.label.hasPrefix("Search") && b.label.count > 3
        }
        let open = try XCTUnwrap(meeting, "no meeting in the list\n" + app.debugDescription)
        let label = open.label
        open.tap()
        return label
    }

    func testTranscribeAgainOffersTheLanguage() throws {
        let app = Ghira.app()
        app.launch()
        Ghira.completeOnboarding(app)
        _ = try openFirstMeeting(app)
        let tab = app.buttons["Transcript"]
        XCTAssertTrue(tab.waitForExistence(timeout: 10), app.debugDescription)
        tab.tap()
        // The card at the end of the transcript: "Transcribe again" + its Start button.
        let start = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Transcribe again")).firstMatch
        for _ in 0..<40 where !(start.exists && start.isHittable) { app.swipeUp(velocity: .fast) }
        shot("transcript-end")
        try XCTSkipUnless(start.exists, "this meeting has no audio on the phone or the computer (sensitive or removed)")
        guard start.isEnabled else {
            // Audio on the computer, or still being processed: the card's line says why.
            return
        }
        start.tap()
        let sheet = app.otherElements["Transcribe again, web dialog"]
        XCTAssertTrue(sheet.waitForExistence(timeout: 10), app.debugDescription)
        // The import sheet's language choice: toggle buttons (switches to XCUITest), value 1 when chosen.
        let chosen: (String) -> Bool = { name in
            let b = sheet.switches[name]
            return b.isSelected || (b.value as? String) == "1"
        }
        for name in ["Auto", "EN", "VI"] {
            XCTAssertTrue(sheet.switches[name].exists, "no \(name)\n" + app.debugDescription)
        }
        XCTAssertEqual(["Auto", "EN", "VI"].filter(chosen).count, 1, "one language is chosen\n" + app.debugDescription)
        XCTAssertTrue(sheet.buttons["Cancel"].exists, app.debugDescription)
        shot("transcribe-again-sheet")
        // TEST_RUNNER_GHI_CONFIRM_RETRANSCRIBE=1 (only for a throwaway meeting): Vietnamese, confirmed, then the new transcript.
        if !(ProcessInfo.processInfo.environment["GHI_CONFIRM_RETRANSCRIBE"] ?? "").isEmpty {
            sheet.switches["VI"].tap()
            XCTAssertTrue(Ghira.waitUntil(3) { chosen("VI") }, app.debugDescription)
            shot("transcribe-again-vi")
            sheet.buttons["Transcribe again"].tap()
            XCTAssertTrue(Ghira.waitUntil(10) { !sheet.exists }, "the sheet stayed open\n" + app.debugDescription)
            // Processing: the card's button waits; done when it can be pressed again.
            Ghira.waitUntil(15) { !start.isEnabled }
            shot("transcribing")
            XCTAssertTrue(Ghira.waitUntil(300) { start.exists && start.isEnabled }, "the final pass did not finish\n" + app.debugDescription)
            sleep(2)
            shot("transcribed-again")
            return
        }
        // Cancel: the meeting keeps its transcript.
        sheet.buttons["Cancel"].tap()
        XCTAssertTrue(Ghira.waitUntil(5) { !sheet.exists }, "the sheet stayed open\n" + app.debugDescription)
    }
}
