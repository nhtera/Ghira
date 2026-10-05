// SPDX-License-Identifier: Apache-2.0
// Phase 15-H: the native pairing scanner, driven through the test hooks of a
// `build-ios.sh --sim --test-hooks` build (the Simulator has no camera, so the
// sheet opens without a preview). `qr-open` is what the web UI's Pair button
// will do through Rust once 15-J lands; `qr` injects a scan result (GHI_FAKE_QR
// or a fixed string). The app writes qr-injected.txt in the App Group once the
// text was handed to Rust. Needs the app installed and onboarding done.

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
        XCTAssertTrue(cancel.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertTrue(app.staticTexts["ghira.qr-guidance"].exists)
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

    /// Pairing from the injected string to a paired device needs the sync
    /// service and its UI (15-J).
    func testInjectedScanPairs() throws {
        throw XCTSkip("end-to-end pairing needs the sync service (15-J) and a desktop peer")
    }
}
