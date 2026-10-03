// SPDX-License-Identifier: Apache-2.0
// "Open in Ghira" share extension (M5): hosts the SwiftUI sheet. The shared
// audio is copied, never decoded, into the App Group inbox (`ShareModel`).

import SwiftUI
import UIKit

final class ShareViewController: UIViewController {
    override func viewDidLoad() {
        super.viewDidLoad()
        let model = ShareModel(context: extensionContext)
        let host = UIHostingController(rootView: ShareView(model: model))
        addChild(host)
        host.view.frame = view.bounds
        host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(host.view)
        host.didMove(toParent: self)
        model.load()
    }
}
