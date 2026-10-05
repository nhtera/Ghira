// SPDX-License-Identifier: Apache-2.0
// The web view must fill the screen: the page lays itself out under the status
// bar and the home indicator and pads for them with env(safe-area-inset-*)
// (viewport-fit=cover). WKWebView's default (.automatic) also adds the safe-area
// insets to its scroll view, so the page ended a safe-area short of the bottom
// and the tab bar floated above an empty band. Every WKWebView in the app gets
// .never as soon as its window is on screen, and is laid out again: the layout
// viewport is measured once, so without the relayout it stays a safe area short.
import UIKit
import WebKit

enum WebViewInsets {
    private static var observers: [NSObjectProtocol] = []

    /// Main thread only (Rust's setup runs there, before the window exists).
    static func install() {
        guard observers.isEmpty else { return }
        let nc = NotificationCenter.default
        let names: [Notification.Name] = [UIWindow.didBecomeVisibleNotification, UIApplication.didBecomeActiveNotification]
        observers = names.map { name in
            nc.addObserver(forName: name, object: nil, queue: .main) { _ in apply() }
        }
        // The webview is created right after setup; look again once it is in.
        for delay in [0.1, 0.5, 1.5, 3.0] {
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) { apply() }
        }
    }

    static func apply() {
        for scene in UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }) {
            for window in scene.windows { fix(window) }
        }
    }

    private static func fix(_ view: UIView) {
        if let web = view as? WKWebView {
            if web.scrollView.contentInsetAdjustmentBehavior != .never {
                web.scrollView.contentInsetAdjustmentBehavior = .never
                web.setNeedsLayout()
                web.superview?.layoutIfNeeded()
            }
        }
        for sub in view.subviews { fix(sub) }
    }
}
