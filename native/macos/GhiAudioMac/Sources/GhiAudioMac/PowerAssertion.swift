// SPDX-License-Identifier: Apache-2.0
//
// Keeps the Mac out of idle sleep while capturing: on idle sleep the audio
// hardware powers down and a recording would silently stop delivering audio.
// A closed lid still sleeps (reported through PowerObserver).

import Foundation
import IOKit.pwr_mgt

final class PowerAssertion {
    private var id = IOPMAssertionID(kIOPMNullAssertionID)

    func acquire(reason: String) {
        guard id == kIOPMNullAssertionID else { return }
        var created = IOPMAssertionID(kIOPMNullAssertionID)
        let status = IOPMAssertionCreateWithName(
            kIOPMAssertPreventUserIdleSystemSleep as CFString,
            IOPMAssertionLevel(kIOPMAssertionLevelOn),
            reason as CFString, &created)
        if status == kIOReturnSuccess { id = created }
    }

    func release() {
        guard id != kIOPMNullAssertionID else { return }
        IOPMAssertionRelease(id)
        id = IOPMAssertionID(kIOPMNullAssertionID)
    }

    deinit { release() }
}
