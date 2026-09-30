// SPDX-License-Identifier: Apache-2.0
//
// Output route classification (speakers / headphones / Bluetooth / external) and
// the Bluetooth (HFP) flag of the default input.

import CoreAudio
import Foundation

enum Route {
    static let unknown: UInt32 = 0
    static let speakers: UInt32 = 1
    static let headphones: UInt32 = 2
    static let bluetooth: UInt32 = 3
    static let external: UInt32 = 4

    /// Set in ROUTE_CHANGED codes when the default input is Bluetooth.
    static let inputBluetoothFlag: Int64 = 0x100

    private static func transport(_ device: AudioDeviceID) -> UInt32 {
        (try? CAUtil.get(device, kAudioDevicePropertyTransportType, initial: UInt32(0))) ?? 0
    }

    static func isBluetooth(_ device: AudioDeviceID) -> Bool {
        let t = transport(device)
        return t == kAudioDeviceTransportTypeBluetooth || t == kAudioDeviceTransportTypeBluetoothLE
    }

    static func outputKind(_ device: AudioDeviceID) -> UInt32 {
        let t = transport(device)
        switch t {
        case kAudioDeviceTransportTypeBluetooth, kAudioDeviceTransportTypeBluetoothLE:
            return bluetooth
        case kAudioDeviceTransportTypeBuiltIn:
            // 'hdpn' is the headphone jack, 'ispk' the internal speakers.
            let source = (try? CAUtil.get(
                device, kAudioDevicePropertyDataSource,
                scope: kAudioObjectPropertyScopeOutput, initial: UInt32(0))) ?? 0
            return source == fourCC("hdpn") ? headphones : speakers
        case kAudioDeviceTransportTypeUSB:
            // A USB device that also captures is a headset; output-only is a speaker/DAC.
            let hasInput = !CAUtil.streamIDs(device, scope: kAudioObjectPropertyScopeInput).isEmpty
            return hasInput ? headphones : external
        case kAudioDeviceTransportTypeHDMI, kAudioDeviceTransportTypeDisplayPort,
             kAudioDeviceTransportTypeAirPlay, kAudioDeviceTransportTypeThunderbolt,
             kAudioDeviceTransportTypeFireWire:
            return external
        default:
            return unknown
        }
    }

    /// The ROUTE_CHANGED code for the current defaults, and the raw pair.
    static func current() -> (kind: UInt32, inputBluetooth: Bool) {
        let kind = CAUtil.defaultDevice(input: false).map(outputKind) ?? unknown
        let bt = CAUtil.defaultDevice(input: true).map(isBluetooth) ?? false
        return (kind, bt)
    }

    static func eventCode() -> Int64 {
        let route = current()
        return Int64(route.kind) | (route.inputBluetooth ? inputBluetoothFlag : 0)
    }

    private static func fourCC(_ s: String) -> UInt32 {
        s.utf8.reduce(0) { ($0 << 8) | UInt32($1) }
    }
}
