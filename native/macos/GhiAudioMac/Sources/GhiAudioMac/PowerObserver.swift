// SPDX-License-Identifier: Apache-2.0
//
// System sleep/wake via IOKit power notifications. Unlike NSWorkspace's
// willSleep/didWake, these are delivered on a dispatch queue and do not need the
// main run loop, so they also work in a command-line host.

import Foundation
import IOKit
import IOKit.pwr_mgt

// iokit_common_msg(x) values from IOMessage.h (the macros are not imported).
private let messageCanSystemSleep: natural_t = 0xE000_0270
private let messageSystemWillSleep: natural_t = 0xE000_0280
private let messageSystemHasPoweredOn: natural_t = 0xE000_0300

final class PowerObserver {
    private let queue: DispatchQueue
    private let onSleep: () -> Void
    private let onWake: () -> Void
    private var rootPort: io_connect_t = 0
    private var notifyPort: IONotificationPortRef?
    private var notifier: io_object_t = 0

    /// `onSleep` runs before the system is allowed to sleep; it may block briefly.
    init?(queue: DispatchQueue, onSleep: @escaping () -> Void, onWake: @escaping () -> Void) {
        self.queue = queue
        self.onSleep = onSleep
        self.onWake = onWake

        let callback: IOServiceInterestCallback = { refcon, _, messageType, argument in
            guard let refcon else { return }
            Unmanaged<PowerObserver>.fromOpaque(refcon).takeUnretainedValue().handle(messageType, argument)
        }
        rootPort = IORegisterForSystemPower(
            Unmanaged.passUnretained(self).toOpaque(), &notifyPort, callback, &notifier)
        guard rootPort != 0, let notifyPort else { return nil }
        IONotificationPortSetDispatchQueue(notifyPort, queue)
    }

    private func handle(_ message: natural_t, _ argument: UnsafeMutableRawPointer?) {
        let token = Int(bitPattern: argument)
        switch message {
        case messageCanSystemSleep:
            IOAllowPowerChange(rootPort, token)
        case messageSystemWillSleep:
            onSleep()
            IOAllowPowerChange(rootPort, token)
        case messageSystemHasPoweredOn:
            onWake()
        default:
            break
        }
    }

    /// Deregisters; no notification is delivered after this returns.
    func invalidate() {
        if notifier != 0 { IODeregisterForSystemPower(&notifier); notifier = 0 }
        if let port = notifyPort {
            IONotificationPortSetDispatchQueue(port, nil)
            IONotificationPortDestroy(port)
            notifyPort = nil
        }
        if rootPort != 0 { IOServiceClose(rootPort); rootPort = 0 }
        queue.sync {}
    }

    deinit { invalidate() }
}
