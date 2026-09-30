// SPDX-License-Identifier: Apache-2.0
//
// Core Audio process objects: PID translation for per-app taps and the process
// list used by meeting detection.

import CoreAudio
import Foundation

@available(macOS 14.2, *)
enum AudioProcesses {
    struct Entry {
        let pid: Int32
        let bundleID: String
        let runningInput: Bool
        let runningOutput: Bool
    }

    /// The process object for `pid`, or nil when the process has no Core Audio
    /// presence (it never opened audio).
    static func processObject(forPID pid: Int32) -> AudioObjectID? {
        var addr = CAUtil.address(kAudioHardwarePropertyTranslatePIDToProcessObject)
        var qualifier = pid_t(pid)
        var object = AudioObjectID(kAudioObjectUnknown)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        let status = AudioObjectGetPropertyData(
            AudioObjectID(kAudioObjectSystemObject), &addr,
            UInt32(MemoryLayout<pid_t>.size), &qualifier, &size, &object)
        guard status == noErr, object != kAudioObjectUnknown else { return nil }
        return object
    }

    static func list() -> [Entry]? {
        let system = AudioObjectID(kAudioObjectSystemObject)
        guard let objects = try? CAUtil.getArray(
            system, kAudioHardwarePropertyProcessObjectList, of: AudioObjectID.self) else { return nil }
        return objects.compactMap { object in
            guard let pid = try? CAUtil.get(object, kAudioProcessPropertyPID, initial: pid_t(0)) else { return nil }
            let input = (try? CAUtil.get(object, kAudioProcessPropertyIsRunningInput, initial: UInt32(0))) ?? 0
            let output = (try? CAUtil.get(object, kAudioProcessPropertyIsRunningOutput, initial: UInt32(0))) ?? 0
            return Entry(
                pid: pid,
                bundleID: CAUtil.string(object, kAudioProcessPropertyBundleID) ?? "",
                runningInput: input != 0,
                runningOutput: output != 0)
        }
    }

    static func json() -> String? {
        guard let entries = list() else { return nil }
        let array: [[String: Any]] = entries.map {
            ["pid": Int($0.pid), "bundle_id": $0.bundleID, "input": $0.runningInput, "output": $0.runningOutput]
        }
        guard let data = try? JSONSerialization.data(withJSONObject: array, options: [.sortedKeys]) else { return nil }
        return String(data: data, encoding: .utf8)
    }
}
