// SPDX-License-Identifier: Apache-2.0
// With the web view's contentInsetAdjustmentBehavior = .never the keyboard must
// still keep the focused field visible: tap a field, type, and assert the field's
// frame lies fully above the keyboard's and the typed text shows. Nothing is
// saved or submitted. The Simulator needs the software keyboard
// (Simulator > I/O > Keyboard > Connect Hardware Keyboard off, i.e.
// `defaults write com.apple.iphonesimulator ConnectHardwareKeyboard 0`).
// There is no speaker rename field on iOS yet; the transcript-line editor needs a
// recorded meeting, so (c) is the lowest field of Settings (the consent message).

import XCTest

final class KeyboardTests: XCTestCase {
    override func setUp() { continueAfterFailure = false }

    private func launch() -> XCUIApplication {
        let app = Ghira.app()
        app.launch()
        Ghira.unlockIfNeeded(app)
        Ghira.completeOnboarding(app)
        return app
    }

    private func openSettingsRow(_ app: XCUIApplication, _ title: String) {
        Ghira.tapTab(app, "Settings")
        let row = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", title)).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10), "no '\(title)' row\n" + app.debugDescription)
        for _ in 0..<5 where !row.isHittable { app.swipeUp() }
        row.tap()
    }

    /// Taps the field, types, and asserts it sits above the keyboard with the text visible.
    private func assertVisibleAboveKeyboard(_ app: XCUIApplication, _ field: XCUIElement, _ name: String, typed: String = "abc") {
        XCTAssertTrue(field.waitForExistence(timeout: 10), "\(name): field not found\n" + app.debugDescription)
        for _ in 0..<5 where !field.isHittable { app.swipeUp() }
        field.tap()
        let keyboard = app.keyboards.element
        XCTAssertTrue(keyboard.waitForExistence(timeout: 10), "\(name): no keyboard (Simulator: software keyboard off?)")
        field.typeText(typed)
        sleep(1) // the scroll that keeps the field visible animates
        let f = field.frame, k = keyboard.frame
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = "keyboard-\(name)"; shot.lifetime = .keepAlways; add(shot)
        if let dir = ProcessInfo.processInfo.environment["GHI_CAPTURE_DIR"], !dir.isEmpty {
            try? app.screenshot().pngRepresentation.write(to: URL(fileURLWithPath: dir).appendingPathComponent("keyboard-\(name).png"))
        }
        print("KEYBOARD \(name) field=\(f) keyboard=\(k) value=\(String(describing: field.value))")
        XCTAssertLessThanOrEqual(f.maxY, k.minY + 0.5, "\(name): field \(f) is under the keyboard \(k)")
        XCTAssertGreaterThanOrEqual(f.minY, 0, "\(name): field \(f) scrolled off the top")
        XCTAssertTrue(((field.value as? String) ?? "").contains(typed), "\(name): typed text not shown, value=\(String(describing: field.value))")
    }

    func testSearchFieldStaysAboveKeyboard() {
        let app = launch()
        Ghira.tapTab(app, "Search")
        let field = app.searchFields.firstMatch.exists ? app.searchFields.firstMatch : app.textFields.firstMatch
        assertVisibleAboveKeyboard(app, field, "search")
    }

    func testVocabularyFieldStaysAboveKeyboard() {
        let app = launch()
        openSettingsRow(app, "Custom vocabulary")
        assertVisibleAboveKeyboard(app, app.textFields["New term"], "vocabulary")
    }

    func testConsentMessageFieldStaysAboveKeyboard() {
        let app = launch()
        openSettingsRow(app, "Consent message")
        let areas = app.textViews
        XCTAssertTrue(areas.firstMatch.waitForExistence(timeout: 10), app.debugDescription)
        assertVisibleAboveKeyboard(app, areas.element(boundBy: areas.count - 1), "consent-vi")
    }
}
