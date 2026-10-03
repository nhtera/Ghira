// SPDX-License-Identifier: Apache-2.0
// Shared helpers: the app under test is addressed by bundle id, never built here.

import XCTest

enum Ghira {
    static let bundleId = "com.nhtera.ghira"

    static func app(env: [String: String] = [:], args: [String] = []) -> XCUIApplication {
        let app = XCUIApplication(bundleIdentifier: bundleId)
        app.launchEnvironment.merge(env) { $1 }
        app.launchArguments += args
        return app
    }

    static var springboard: XCUIApplication {
        XCUIApplication(bundleIdentifier: "com.apple.springboard")
    }

    /// The simulator has no Lock command in simctl; the private selector works.
    static func pressLockButton() {
        XCUIDevice.shared.perform(NSSelectorFromString("pressLockButton"))
    }
}
