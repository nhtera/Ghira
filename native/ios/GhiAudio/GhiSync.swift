// SPDX-License-Identifier: Apache-2.0
// LAN sync, Swift side (phase 15, doc 07 §4.1): the camera scan of the desktop's
// pairing code and the Bonjour browse for its service. C ABI in
// include/ghi_ios.h. The scanned text carries a secret: it goes to Rust only
// (`ghi_ios_qr_scanned`) and is never logged. Failures reach the same callback
// as a text starting with `errorPrefix` + a code (`denied`, `unavailable`,
// `cancelled`), which Rust tells apart from a scan by the prefix.
//
// The browse uses NWBrowser only (no outgoing connection): the addresses come from the
// service's TXT record (`v=1`, `a=<ip:port,…>`) and Rust re-checks every one
// with `is_lan` before use. This file is the one place the egress check allows
// NWBrowser.

import AVFoundation
import Foundation
import Network
import UIKit
import GhiIOS

private let errorPrefix = "\u{1}ghi-error:"

private func deliver(_ text: String) {
    text.withCString { ghi_ios_qr_scanned($0) }
}

// MARK: - QR scanner

/// Main thread only.
private final class QrScanner: NSObject, AVCaptureMetadataOutputObjectsDelegate {
    static let shared = QrScanner()

    private var controller: QrViewController?
    private var starting = false

    var isActive: Bool { controller != nil || starting }

    func start() {
        guard !isActive else { return }
        starting = true
        #if GHI_TEST_HOOKS
        // The Simulator has no camera: the sheet still opens so tests can drive it.
        let hasCamera = AVCaptureDevice.default(for: .video) != nil
        if !hasCamera { return present(session: nil) }
        #endif
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            present(session: makeSession())
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { granted in
                DispatchQueue.main.async {
                    guard self.starting else { return }
                    if granted { self.present(session: self.makeSession()) } else { self.fail("denied") }
                }
            }
        default:
            fail("denied")
        }
    }

    func stop() {
        starting = false
        guard let c = controller else { return }
        controller = nil
        c.teardown()
        c.dismiss(animated: true)
    }

    /// The scan result or the cancel: exactly once, then the sheet closes.
    func finish(_ text: String) {
        guard isActive else { return }
        stop()
        deliver(text)
    }

    private func fail(_ code: String) {
        starting = false
        deliver(errorPrefix + code)
    }

    private func makeSession() -> AVCaptureSession? {
        guard let device = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: device)
        else { return nil }
        let session = AVCaptureSession()
        let output = AVCaptureMetadataOutput()
        guard session.canAddInput(input), session.canAddOutput(output) else { return nil }
        session.addInput(input)
        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: .main)
        guard output.availableMetadataObjectTypes.contains(.qr) else { return nil }
        output.metadataObjectTypes = [.qr]
        return session
    }

    private func present(session: AVCaptureSession?) {
        #if !GHI_TEST_HOOKS
        guard let session else { return fail("unavailable") }
        #endif
        guard starting, let top = topViewController() else { return fail("unavailable") }
        starting = false
        let c = QrViewController(session: session)
        c.modalPresentationStyle = .fullScreen
        controller = c
        top.present(c, animated: true)
    }

    func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject],
                        from connection: AVCaptureConnection) {
        guard let text = objects.compactMap({ ($0 as? AVMetadataMachineReadableCodeObject)?.stringValue }).first,
              !text.isEmpty
        else { return }
        finish(text)
    }

    private func topViewController() -> UIViewController? {
        let scene = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .first { $0.activationState == .foregroundActive }
        var top = scene?.windows.first(where: \.isKeyWindow)?.rootViewController
        while let presented = top?.presentedViewController { top = presented }
        return top
    }
}

private final class QrViewController: UIViewController {
    private let session: AVCaptureSession?
    private var preview: AVCaptureVideoPreviewLayer?

    init(session: AVCaptureSession?) {
        self.session = session
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) is not used") }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        view.accessibilityIdentifier = "ghira.qr-scanner"
        if let session {
            let layer = AVCaptureVideoPreviewLayer(session: session)
            layer.videoGravity = .resizeAspectFill
            view.layer.addSublayer(layer)
            preview = layer
        }
        let guidance = UILabel()
        guidance.text = NSLocalizedString("sync.qr.guidance", tableName: "Sync", value: "Point the camera at the code on your Mac", comment: "")
        guidance.textColor = .white
        guidance.font = .preferredFont(forTextStyle: .headline)
        guidance.adjustsFontForContentSizeCategory = true
        guidance.numberOfLines = 0
        guidance.textAlignment = .center
        guidance.accessibilityIdentifier = "ghira.qr-guidance"
        let cancel = UIButton(type: .system)
        cancel.setTitle(NSLocalizedString("sync.qr.cancel", tableName: "Sync", value: "Cancel", comment: ""), for: .normal)
        cancel.titleLabel?.font = .preferredFont(forTextStyle: .title3)
        cancel.setTitleColor(.white, for: .normal)
        cancel.accessibilityIdentifier = "ghira.qr-cancel"
        cancel.addTarget(self, action: #selector(cancelTapped), for: .touchUpInside)
        let stack = UIStackView(arrangedSubviews: [guidance, cancel])
        stack.axis = .vertical
        stack.spacing = 24
        stack.alignment = .center
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
            stack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24),
            stack.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -32),
        ])
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        preview?.frame = view.bounds
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        if let session {
            DispatchQueue.global(qos: .userInitiated).async { session.startRunning() }
        }
    }

    func teardown() {
        if let session {
            DispatchQueue.global(qos: .userInitiated).async { session.stopRunning() }
        }
    }

    @objc private func cancelTapped() {
        QrScanner.shared.finish(errorPrefix + "cancelled")
    }
}

@_cdecl("ghi_swift_qr_scan_start")
public func ghiSwiftQrScanStart() {
    DispatchQueue.main.async {
        #if GHI_TEST_HOOKS
        GhiSyncHooks.install()
        // GHI_FAKE_QR: a scan without the camera (nothing is presented).
        if let fake = ProcessInfo.processInfo.environment["GHI_FAKE_QR"], !fake.isEmpty {
            return GhiSyncHooks.inject(fake)
        }
        #endif
        QrScanner.shared.start()
    }
}

@_cdecl("ghi_swift_qr_scan_stop")
public func ghiSwiftQrScanStop() {
    DispatchQueue.main.async { QrScanner.shared.stop() }
}

// MARK: - Bonjour browse

private final class Browse {
    static let shared = Browse()
    private let queue = DispatchQueue(label: "com.nhtera.ghira.sync-browse")
    private var browser: NWBrowser?

    func start() {
        queue.async {
            guard self.browser == nil else { return }
            let browser = NWBrowser(for: .bonjourWithTXTRecord(type: "_ghi._tcp", domain: nil), using: .tcp)
            browser.browseResultsChangedHandler = { results, _ in
                Browse.report(results)
            }
            browser.stateUpdateHandler = { [weak self] state in
                // Denied Local Network access or a dead browser: report nothing visible and let a later start retry.
                if case .failed = state {
                    Browse.send([])
                    self?.queue.async { self?.browser = nil }
                }
            }
            self.browser = browser
            browser.start(queue: self.queue)
        }
    }

    func stop() {
        queue.async {
            self.browser?.cancel()
            self.browser = nil
        }
    }

    private static func report(_ results: Set<NWBrowser.Result>) {
        var services: [[String: [String]]] = []
        for result in results {
            guard case .bonjour(let txt) = result.metadata, txt["v"] == "1", let a = txt["a"] else { continue }
            let addrs = parseAddrs(a)
            if !addrs.isEmpty { services.append(["addrs": addrs]) }
        }
        send(services)
    }

    /// The `a` TXT value: comma-separated `ip:port`. Shape only; Rust re-checks with `is_lan`.
    static func parseAddrs(_ a: String) -> [String] {
        a.split(separator: ",").compactMap { part in
            let s = part.trimmingCharacters(in: .whitespaces)
            guard let colon = s.lastIndex(of: ":"), colon != s.startIndex,
                  UInt16(s[s.index(after: colon)...]) != nil
            else { return nil }
            return s
        }
    }

    private static func send(_ services: [[String: [String]]]) {
        guard let data = try? JSONSerialization.data(withJSONObject: services),
              let json = String(data: data, encoding: .utf8)
        else { return }
        json.withCString { ghi_ios_browse_found($0) }
    }
}

@_cdecl("ghi_swift_browse_start")
public func ghiSwiftBrowseStart() { Browse.shared.start() }

@_cdecl("ghi_swift_browse_stop")
public func ghiSwiftBrowseStop() { Browse.shared.stop() }

// MARK: - Test hooks

#if GHI_TEST_HOOKS
import notify

/// Simulator hooks (never in a release build), posted with
/// `xcrun simctl spawn <udid> notifyutil -p <name>`:
///   com.nhtera.ghira.test.qr-open  starts the scan (what the Pair button does)
///   com.nhtera.ghira.test.qr       injects the scan result: GHI_FAKE_QR, else a fixed string
/// An injected scan writes `qr-injected.txt` ("1") into the App Group container
/// after the text reached Rust, so a test can observe it.
enum GhiSyncHooks {
    private static var tokens: [Int32] = []
    static let fallback = "ghira-test-qr"

    static func install() {
        guard tokens.isEmpty else { return }
        let prefix = "com.nhtera.ghira.test."
        let actions: [(String, () -> Void)] = [
            ("qr-open", { ghiSwiftQrScanStart() }),
            ("qr", {
                let env = ProcessInfo.processInfo.environment["GHI_FAKE_QR"]
                inject(env.flatMap { $0.isEmpty ? nil : $0 } ?? fallback)
            }),
        ]
        for (name, action) in actions {
            var token: Int32 = 0
            notify_register_dispatch(prefix + name, &token, .main) { _ in
                NSLog("ghira: test hook: \(name)")
                action()
            }
            tokens.append(token)
        }
    }

    /// Main thread: only while a scan is active, like a real one.
    static func inject(_ text: String) {
        guard QrScanner.shared.isActive else { return }
        QrScanner.shared.finish(text)
        if let dir = InboxShared.containerURL() {
            try? "1".write(to: dir.appendingPathComponent("qr-injected.txt"), atomically: true, encoding: .utf8)
        }
    }
}
#endif
