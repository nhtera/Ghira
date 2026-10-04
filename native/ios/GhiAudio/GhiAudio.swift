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
// - Thermal state, audio interruptions, phone calls and route changes are
//   forwarded to Rust. An interruption never restarts the engine by itself:
//   Rust asks the user, and `start()` (the user's Resume) restarts it.
//
// Rust calls in through the `ghi_swift_*` functions (@_cdecl); Swift calls
// Rust through `GhiIOS` (include/ghi_ios.h).

import AVFoundation
import GhiIOS
import UIKit

final class GhiAudio {
    static let shared = GhiAudio()

    /// Replaced after a media services reset (the old engine is dead).
    private var engine = AVAudioEngine()
    private var configObserver: NSObjectProtocol?
    private let lock = NSLock()
    private var running = false
    /// An interruption (or a media services reset) stopped capture; only the
    /// user's Resume (`start()`) restarts it.
    private var interrupted = false
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
        TextScaleCache.shared.refresh()
        AppLifecycleState.shared.install()
        CallMonitor.shared.start()
        NetworkMonitor.shared.start()
        InboxObserver.install()
        #if GHI_TEST_HOOKS
        GhiTestHooks.install()
        #endif
        // The microphone prompt is shown by onboarding (ghi_swift_request_mic_permission).
        let nc = NotificationCenter.default
        observers = [
            nc.addObserver(forName: UIApplication.willResignActiveNotification, object: nil, queue: .main) { _ in
                // Synchronous: Rust raises the privacy cover from here (if app lock is on),
                // before iOS takes the app-switcher snapshot.
                ghi_ios_suspend()
            },
            nc.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) { _ in
                // Synchronous and non-blocking: ordered before the next didBecomeActive.
                if !ghi_ios_entered_background() {
                    NSLog("ghira: an engine step overlapped the move to the background; it will be redone")
                }
            },
            nc.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in
                // Capture is never restarted here: after an interruption only the user's Resume does.
                ghi_ios_resume()
            },
            nc.addObserver(forName: UIDevice.batteryLevelDidChangeNotification, object: nil, queue: .main) { [weak self] _ in
                self?.updateBattery()
            },
            nc.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: nil, queue: .main) { [weak self] _ in
                // The engine and session objects are gone: build a new engine and let the user resume.
                self?.mediaServicesWereReset()
            },
            nc.addObserver(forName: ProcessInfo.thermalStateDidChangeNotification, object: nil, queue: nil) { _ in
                ghi_ios_thermal_changed(Int32(ProcessInfo.processInfo.thermalState.rawValue))
            },
            nc.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: .main) { [weak self] note in
                self?.interruption(note)
            },
            nc.addObserver(forName: AVAudioSession.routeChangeNotification, object: nil, queue: .main) { note in
                let reason = (note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt) ?? 0
                NSLog("ghira: audio route changed (reason \(reason))")
                ghi_ios_route_changed(Int32(reason))
            },
            nc.addObserver(forName: UIContentSizeCategory.didChangeNotification, object: nil, queue: .main) { _ in
                ghi_ios_text_scale_changed(TextScaleCache.shared.refresh())
            },
            nc.addObserver(forName: UIApplication.didReceiveMemoryWarningNotification, object: nil, queue: .main) { _ in
                ghi_ios_memory_warning()
            },
        ]
        observeEngine()
    }

    /// Main thread. (Re-)registers the configuration-change observer for the current engine.
    private func observeEngine() {
        if let old = configObserver { NotificationCenter.default.removeObserver(old) }
        lock.lock()
        let current = engine
        lock.unlock()
        configObserver = NotificationCenter.default.addObserver(
            forName: .AVAudioEngineConfigurationChange, object: current, queue: .main
        ) { [weak self] _ in
            // The input format may have changed (route change): re-install the tap,
            // unless an interruption stopped capture (the user's Resume restarts it).
            self?.restart(reason: "configuration change")
        }
    }

    /// A new engine replaces the one the reset killed; capture stays off until the user resumes.
    private func mediaServicesWereReset() {
        lock.lock()
        if running {
            engine = AVAudioEngine()
        }
        lock.unlock()
        observeEngine()
        interruptionBegan()
    }

    /// Whether the audio engine is running (test probe; 16-E hooks).
    var engineRunning: Bool {
        lock.lock()
        defer { lock.unlock() }
        return engine.isRunning
    }

    #if GHI_TEST_HOOKS
    /// What iOS does at the start of an interruption: the engine stops.
    func testStopEngine() {
        lock.lock()
        engine.stop()
        lock.unlock()
    }
    #endif

    /// 0 on success; 4 a phone call is active (Resume must wait for it); 1 no microphone permission, 2 session error, 3 engine error.
    func start() -> Int32 {
        lock.lock()
        defer { lock.unlock() }
        guard AVAudioApplication.shared.recordPermission == .granted else { return 1 }
        if running {
            // Resume after an interruption: the engine is stopped, the session is not ours any more.
            guard interrupted || !engine.isRunning else { return 0 }
            guard !CallMonitor.shared.active else { return 4 }
            engine.stop()
            do {
                try activateSession()
                try startEngine()
            } catch {
                NSLog("ghira: could not resume audio: \(error)")
                return 3
            }
            interrupted = false
            return 0
        }
        let session = AVAudioSession.sharedInstance()
        do {
            try activateSession()
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
        interrupted = false
        return 0
    }

    private func activateSession() throws {
        let session = AVAudioSession.sharedInstance()
        // `.default`, not `.measurement`: measurement turns off the input's
        // automatic gain, and on an iPhone 15 Pro Max speech then peaked at
        // about -46 dBFS (RMS 0.005), too faint for the voice check and ASR.
        try session.setCategory(.record, mode: .default, options: [])
        try session.setPrefersNoInterruptionsFromSystemAlerts(true)
        try session.setActive(true)
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
            #if GHI_TEST_HOOKS
            // Fake mic: the engine and tap keep running (so background audio
            // stays alive); the samples come from GHI_FAKE_MIC instead.
            if GhiTestHooks.pushFakeMic(frames: Int(buffer.frameLength), rate: rate, hostNs: hostNs) { return }
            #endif
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
        guard running, !interrupted, !CallMonitor.shared.active else { return false }
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
        interrupted = false
    }

    private func interruptionBegan() {
        lock.lock()
        let wasRunning = running
        if running { interrupted = true }
        lock.unlock()
        // The engine has stopped; Rust marks a pause in the recording (and
        // ignores this when nothing is recording).
        if wasRunning { ghi_ios_interruption(true) }
    }

    private func interruption(_ note: Notification) {
        guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
              let type = AVAudioSession.InterruptionType(rawValue: raw)
        else { return }
        switch type {
        case .began:
            interruptionBegan()
        case .ended:
            // Forward only: Rust asks the user (Resume / Stop and save), and
            // the engine restarts in `start()`, never here.
            ghi_ios_interruption(false)
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
