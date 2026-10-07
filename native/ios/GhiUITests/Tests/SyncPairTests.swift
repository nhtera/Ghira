// SPDX-License-Identifier: Apache-2.0
// Phase 15-H/J: the native pairing scanner, driven through the test hooks of a
// `build-ios.sh --sim --test-hooks` build (the Simulator has no camera, so the
// sheet opens without a preview). `qr-open` starts the scan (what the web UI's
// scan button does through Rust); `qr` injects a scan result (GHI_FAKE_QR or a
// fixed string). The app writes qr-injected.txt in the App Group once the text
// was handed to Rust. Needs the app installed and onboarding done.
//
// The end-to-end test pairs with a Mac's `ghi sync serve --print-qr` (the code
// is a secret: it is read from the 0600 file named by GHI_FAKE_QR_FILE, handed
// to the app's environment and never printed or asserted on):
//   ghi sync serve --dir <scratch> --bind <en0 ip> --name "Test Mac" --print-qr > <0600 file> &
//   TEST_RUNNER_GHI_FAKE_QR_FILE=<file> native/ios/GhiUITests/run.sh -only-testing:GhiUITests/SyncPairTests/testInjectedScanPairs

import XCTest

final class SyncPairTests: XCTestCase {
    override func setUp() { continueAfterFailure = false }

    private func openScanner(_ app: XCUIApplication) -> XCUIElement {
        Ghira.hook("qr-open")
        return app.buttons["ghira.qr-cancel"]
    }

    /// The scanner opens full-screen with guidance and Cancel; Cancel closes it.
    func testScannerOpensAndCancelCloses() {
        let app = Ghira.app()
        app.launch()
        Ghira.completeOnboarding(app)
        let cancel = openScanner(app)
        // The camera prompt on a real phone (once).
        if Ghira.onDevice { Ghira.allowSystemAlert() }
        XCTAssertTrue(cancel.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertTrue(app.staticTexts["ghira.qr-guidance"].exists)
        cancel.tap()
        XCTAssertTrue(Ghira.waitUntil(10) { !cancel.exists }, "the scanner stayed open\n" + app.debugDescription)
    }

    /// The scanner stays up while the camera runs (on a phone it once closed by itself after ~10 s).
    /// A screenshot every 2 s is attached, so a failure shows what took its place.
    func testScannerStaysOpen() {
        let app = Ghira.app()
        app.launch()
        Ghira.completeOnboarding(app)
        let cancel = openScanner(app)
        if Ghira.onDevice { Ghira.allowSystemAlert() }
        XCTAssertTrue(cancel.waitForExistence(timeout: 10), app.debugDescription)
        for second in stride(from: 0, through: 20, by: 2) {
            let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
            shot.name = "scanner-\(second)s"
            shot.lifetime = .keepAlways
            add(shot)
            XCTAssertTrue(cancel.exists, "the scanner closed by itself after \(second) s\n" + app.debugDescription)
            sleep(2)
        }
        cancel.tap()
        XCTAssertTrue(Ghira.waitUntil(10) { !cancel.exists }, "the scanner stayed open\n" + app.debugDescription)
    }

    /// An injected scan closes the scanner and reaches Rust exactly once.
    func testInjectedScanDismissesTheScanner() throws {
        try XCTSkipIf(Ghira.groupDir == nil, "GHI_GROUP_DIR not set (run via run.sh)")
        let app = Ghira.app()
        app.launch()
        Ghira.completeOnboarding(app)
        let marker = Ghira.groupDir!.appendingPathComponent("qr-injected.txt")
        try? FileManager.default.removeItem(at: marker)
        let cancel = openScanner(app)
        XCTAssertTrue(cancel.waitForExistence(timeout: 10), app.debugDescription)
        Ghira.hook("qr")
        XCTAssertTrue(Ghira.waitUntil(10) { !cancel.exists }, "the scan did not close the scanner\n" + app.debugDescription)
        XCTAssertTrue(Ghira.waitUntil(5) { FileManager.default.fileExists(atPath: marker.path) }, "the scan never reached Rust")
        try? FileManager.default.removeItem(at: marker)
        // A second injection with no scan active is ignored.
        Ghira.hook("qr")
        sleep(1)
        XCTAssertFalse(FileManager.default.fileExists(atPath: marker.path), "a scan was delivered with no scanner open")
    }

    /// The injected string pairs with a hub on this Mac and a recording reaches it: the
    /// app answers its own scan with GHI_FAKE_QR, shows the paired computer, and the session
    /// that follows marks it synced. Skips without a code file (needs a running hub).
    func testInjectedScanPairs() throws {
        // A phone cannot read the host's file: run.sh hands a device run the code itself.
        let env = ProcessInfo.processInfo.environment
        let code: String
        if let given = env["GHI_FAKE_QR"], !given.isEmpty {
            code = given.trimmingCharacters(in: .whitespacesAndNewlines)
        } else if let path = env["GHI_FAKE_QR_FILE"], !path.isEmpty {
            code = try String(contentsOfFile: path, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
        } else {
            throw XCTSkip("GHI_FAKE_QR_FILE not set (a 0600 file with the pairing code of `ghi sync serve --print-qr`)")
        }
        XCTAssertFalse(code.isEmpty, "the code file is empty")
        let app = Ghira.app(env: ["GHI_FAKE_QR": code])
        app.launch()
        Ghira.completeOnboarding(app)
        Ghira.tapTab(app, "Settings")
        let row = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Sync with computer")).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 10), app.debugDescription)
        row.tap()
        // Not paired yet: the button starts the scan, which the environment answers at once. (A phone
        // that paired in the onboarding's pair step, which scans by itself, is paired already.)
        let scan = app.buttons["Scan the code"]
        if scan.waitForExistence(timeout: 5) { scan.tap() }
        // A real phone asks once for the local network when the app first
        // looks for the computer.
        if Ghira.onDevice { Ghira.allowSystemAlert(timeout: 8) }
        // Paired: the computer's card, then the first session's "Last synced".
        XCTAssertTrue(app.staticTexts["Paired computer"].waitForExistence(timeout: 30), "not paired\n" + app.debugDescription)
        XCTAssertTrue(
            app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Last synced")).firstMatch.waitForExistence(timeout: 60),
            "no session finished\n" + app.debugDescription)

        // A short recording syncs to the computer on its own (rows, then its audio).
        Ghira.tapTab(app, "Record")
        let record = Ghira.recordButton(app)
        XCTAssertTrue(record.waitForExistence(timeout: 10), app.debugDescription)
        record.tap()
        let consent = app.buttons["Everyone knows, start recording"]
        if consent.waitForExistence(timeout: 3) { consent.tap() }
        sleep(6)
        let stop = Ghira.stopButton(app)
        XCTAssertTrue(stop.waitForExistence(timeout: 10), app.debugDescription)
        stop.tap()
        // The change goes out within seconds; the Mac side checks the store afterwards.
        sleep(20)
    }
}
