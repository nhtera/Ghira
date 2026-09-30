// SPDX-License-Identifier: Apache-2.0
// iOS audio capture and app lifecycle for the Ghira spike (phase 7).
//
// - AVAudioSession `.record` / `.measurement` with `UIBackgroundModes: audio`,
//   so recording continues with the screen locked.
// - An AVAudioEngine input tap hands mono float PCM (hardware rate) to Rust,
//   which resamples, writes Ogg Opus and feeds the live engine.
// - On resign-active Rust stops starting engine steps; on entering the
//   background it marks any step that was running then as suspect, since iOS
//   silently refuses GPU work from the background (RT-5); that step is redone.
//   On becoming active again the engine catches up.
// - Thermal state and audio interruptions are forwarded to Rust.
//
// Rust calls in through the `ghi_swift_*` functions (@_cdecl); Swift calls
// Rust through `GhiIOS` (include/ghi_ios.h).

import AVFoundation
import GhiIOS
import UIKit

final class GhiAudio {
    static let shared = GhiAudio()

    private let engine = AVAudioEngine()
    private let lock = NSLock()
    private var running = false
    private var observers: [NSObjectProtocol] = []
    private var ticksToNs: Double = 1
    /// UIDevice is main-thread only; Rust reads this copy from any thread.
    private var battery: Float = -1

    private init() {
        var tb = mach_timebase_info_data_t()
        mach_timebase_info(&tb)
        ticksToNs = Double(tb.numer) / Double(tb.denom)
    }

    func install() {
        UIDevice.current.isBatteryMonitoringEnabled = true
        updateBattery()
        GhiLiveActivity.shared.endStale()
        // Ask at launch, so Record never blocks on the permission prompt.
        AVAudioApplication.requestRecordPermission { _ in }
        let nc = NotificationCenter.default
        observers = [
            nc.addObserver(forName: UIApplication.willResignActiveNotification, object: nil, queue: .main) { _ in
                ghi_ios_suspend()
            },
            nc.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { _ in
                // Synchronous and non-blocking: ordered before the next didBecomeActive.
                if !ghi_ios_entered_background() {
                    NSLog("ghira: an engine step overlapped the move to the background; it will be redone")
                }
            },
            nc.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { [weak self] _ in
                ghi_ios_resume()
                // An interruption that ended without `shouldResume` left capture off.
                self?.resumeIfStopped(reason: "becoming active")
            },
            nc.addObserver(forName: UIDevice.batteryLevelDidChangeNotification, object: nil, queue: .main) { [weak self] _ in
                self?.updateBattery()
            },
            nc.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: nil, queue: .main) { [weak self] _ in
                self?.resumeIfStopped(reason: "a media services reset")
            },
            nc.addObserver(forName: ProcessInfo.thermalStateDidChangeNotification, object: nil, queue: nil) { _ in
                ghi_ios_thermal_changed(Int32(ProcessInfo.processInfo.thermalState.rawValue))
            },
            nc.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] note in
                self?.interruption(note)
            },
            nc.addObserver(forName: .AVAudioEngineConfigurationChange, object: engine, queue: .main) { [weak self] _ in
                // The input format may have changed (route change): re-install the tap.
                self?.restart(reason: "configuration change")
            },
            nc.addObserver(forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main) { note in
                let reason = (note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt) ?? 0
                NSLog("ghira: audio route changed (reason \(reason))")
            },
        ]
    }

    /// 0 on success; 1 no microphone permission, 2 session error, 3 engine error.
    func start() -> Int32 {
        lock.lock()
        defer { lock.unlock() }
        if running { return 0 }
        guard AVAudioApplication.shared.recordPermission == .granted else { return 1 }
        let session = AVAudioSession.sharedInstance()
        do {
            try session.setCategory(.record, mode: .measurement, options: [])
            try session.setPrefersNoInterruptionsFromSystemAlerts(true)
            try session.setActive(true)
        } catch {
            NSLog("ghira: audio session: \(error)")
            return 2
        }
        do {
            try startEngine()
        } catch {
            NSLog("ghira: audio engine: \(error)")
            try? session.setActive(false, options: .notifyOthersOnDeactivation)
            return 3
        }
        running = true
        return 0
    }

    /// Installs the tap in the input's current format and starts the engine.
    private func startEngine() throws {
        let input = engine.inputNode
        input.removeTap(onBus: 0)
        let format = input.outputFormat(forBus: 0)
        // During a call or a route change the input can report no format;
        // installTap would raise an (uncatchable) exception.
        guard format.sampleRate > 0, format.channelCount > 0 else {
            throw NSError(domain: "ghira.audio", code: 1, userInfo: [NSLocalizedDescriptionKey: "no input format"])
        }
        let rate = format.sampleRate
        let toNs = ticksToNs
        input.installTap(onBus: 0, bufferSize: 4800, format: format) { buffer, time in
            guard let data = buffer.floatChannelData, buffer.frameLength > 0 else { return }
            let hostNs = time.isHostTimeValid ? UInt64(Double(time.hostTime) * toNs) : 0
            // Channel 0 only: the phone's (mono) microphone.
            ghi_ios_push_pcm(data[0], Int(buffer.frameLength), rate, hostNs)
        }
        engine.prepare()
        do {
            try engine.start()
        } catch {
            input.removeTap(onBus: 0)
            throw error
        }
    }

    @discardableResult
    private func restart(reason: String) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard running else { return false }
        engine.stop()
        do {
            try AVAudioSession.sharedInstance().setActive(true)
            try startEngine()
            return true
        } catch {
            NSLog("ghira: could not restart audio after \(reason): \(error)")
            return false
        }
    }

    /// Recording, but the engine stopped (interruption without `shouldResume`,
    /// media services reset): try to start it again.
    private func resumeIfStopped(reason: String) {
        lock.lock()
        let stalled = running && !engine.isRunning
        lock.unlock()
        if stalled, restart(reason: reason) {
            ghi_ios_interruption(false)
        }
    }

    private func updateBattery() {
        let level = UIDevice.current.batteryLevel
        lock.lock()
        battery = level
        lock.unlock()
    }

    func batteryLevel() -> Float {
        lock.lock()
        defer { lock.unlock() }
        return battery
    }

    func stop() {
        lock.lock()
        defer { lock.unlock() }
        guard running else { return }
        engine.stop()
        engine.inputNode.removeTap(onBus: 0)
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        running = false
    }

    private func interruption(_ note: Notification) {
        guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
              let type = AVAudioSession.InterruptionType(rawValue: raw)
        else { return }
        switch type {
        case .began:
            // The engine has stopped; Rust marks a pause in the recording.
            ghi_ios_interruption(true)
        case .ended:
            let options = (note.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt)
                .map(AVAudioSession.InterruptionOptions.init(rawValue:)) ?? []
            guard options.contains(.shouldResume) else {
                NSLog("ghira: interruption ended without shouldResume; resuming when the app is active")
                return
            }
            if restart(reason: "an interruption") {
                ghi_ios_interruption(false)
            }
        @unknown default:
            break
        }
    }

    static func memoryFootprint() -> UInt64 {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
        let kr = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return kr == KERN_SUCCESS ? info.phys_footprint : 0
    }
}

// MARK: - C ABI called by Rust (apps/mobile/src-tauri/src/platform.rs)

@_cdecl("ghi_swift_init")
public func ghiSwiftInit() {
    DispatchQueue.main.async { GhiAudio.shared.install() }
}

@_cdecl("ghi_swift_audio_start")
public func ghiSwiftAudioStart() -> Int32 {
    GhiAudio.shared.start()
}

@_cdecl("ghi_swift_audio_stop")
public func ghiSwiftAudioStop() {
    GhiAudio.shared.stop()
}

@_cdecl("ghi_swift_thermal_state")
public func ghiSwiftThermalState() -> Int32 {
    Int32(ProcessInfo.processInfo.thermalState.rawValue)
}

@_cdecl("ghi_swift_memory_footprint")
public func ghiSwiftMemoryFootprint() -> UInt64 {
    GhiAudio.memoryFootprint()
}

@_cdecl("ghi_swift_battery_level")
public func ghiSwiftBatteryLevel() -> Float {
    // -1 when unknown (simulator, or monitoring not ready yet).
    GhiAudio.shared.batteryLevel()
}

/// Keeps `path` (the app's data: recordings, models, logs) out of iCloud and
/// Finder device backups. Returns false on failure.
@_cdecl("ghi_swift_exclude_from_backup")
public func ghiSwiftExcludeFromBackup(_ path: UnsafePointer<CChar>) -> Bool {
    var url = URL(fileURLWithPath: String(cString: path), isDirectory: true)
    var values = URLResourceValues()
    values.isExcludedFromBackup = true
    do {
        try url.setResourceValues(values)
        return true
    } catch {
        NSLog("ghira: backup exclusion: \(error)")
        return false
    }
}
