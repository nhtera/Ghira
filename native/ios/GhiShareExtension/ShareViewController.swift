// SPDX-License-Identifier: Apache-2.0
// "Open in Ghira" share extension (M5). A stub until slice 16-E: it accepts
// the share and closes. 16-E copies the shared audio into the App Group inbox
// (group.com.nhtera.ghira) and hands off to the app.

import UIKit

final class ShareViewController: UIViewController {
    override func viewDidLoad() {
        super.viewDidLoad()
        extensionContext?.completeRequest(returningItems: [], completionHandler: nil)
    }
}
