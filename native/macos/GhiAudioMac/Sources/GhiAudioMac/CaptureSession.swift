// SPDX-License-Identifier: Apache-2.0
//
// A capture session: owns the current CaptureGraph, rebuilds it when the default
// devices or the stream format change, and delivers events on one serial queue.
//
// All state is confined to `control`. Graph teardown is synchronous, so the old
// IOProc has fully stopped before a new one starts and audio callbacks for one
// track never overlap.

import CoreAudio
import Foundation

final class CaptureSession {
    private let flags: UInt32
    private let tapPIDs: [Int32]
    private let audio: AudioCallback
    private let event: EventCallback
    private let userData: UnsafeMutableRawPointer?

    private let control = DispatchQueue(label: "app.ghira.audio-mac.control")
    private let events = DispatchQueue(label: "app.ghira.audio-mac.events")

    private var graph: CaptureGraph?
    private var listeners: [PropertyListener] = []
    private var power: PowerObserver?
    private let assertion = PowerAssertion()
    private static let assertionReason = "Ghira is recording"
    private var paused = false
    private var asleep = false
    private var stopped = false
    private var lastRoute: Int64 = 0
    /// Devices and rate of the last graph the host was told about.
    private var announced: GraphSignature?
    /// True after TRACK_LOST for that track until it runs again, so it is reported once.
    private var micLost = false
    private var systemLost = false
    private var retryTicket = 0
    private var retryCount = 0
    /// Bumped by every scheduled reconcile so a burst of notifications runs once.
    private var reconcileTicket = 0

    private var captureMic: Bool { flags & 0x1 != 0 }
    private var captureSystem: Bool { flags & 0x2 != 0 }

    init(
        flags: UInt32, tapPIDs: [Int32], audio: @escaping AudioCallback,
        event: @escaping EventCallback, userData: UnsafeMutableRawPointer?
    ) {
        self.flags = flags
        self.tapPIDs = tapPIDs
        self.audio = audio
        self.event = event
        self.userData = userData
    }

    // MARK: - Public operations (each serialized on `control`)

    func start() -> Int32 {
        control.sync {
            if captureMic {
                switch MicPermission.status() {
                case 0: if MicPermission.request() != 3 { return GhiStatus.micPermission }
                case 3: break
                default: return GhiStatus.micPermission
                }
            }
            lastRoute = Route.eventCode()
            // Listeners first, so a device change during start is not missed.
            installListeners()
            do {
                let built = try startGraph()
                graph = built
                announced = built.signature
                // After the start succeeded; a few buffers may precede the event,
                // each carries its own rate.
                announce(built, mic: captureMic, system: built.hasSystem, started: true)
                if captureSystem && !built.hasSystem { systemDegraded() }
            } catch {
                listeners.forEach { $0.remove() }
                listeners = []
                let failure = error as? CaptureError
                emit(8, Int64(failure?.code ?? GhiStatus.tap), failure?.detail ?? "start failed")
                // No session is returned, so the caller may free its context at once.
                events.sync {}
                return failure?.code ?? GhiStatus.tap
            }
            installPower()
            assertion.acquire(reason: CaptureSession.assertionReason)
            return GhiStatus.ok
        }
    }

    func pause() -> Int32 {
        control.sync {
            guard !stopped else { return GhiStatus.invalid }
            paused = true
            retryTicket += 1
            teardownGraph()
            assertion.release()
            return GhiStatus.ok
        }
    }

    func resume() -> Int32 {
        control.sync {
            guard !stopped else { return GhiStatus.invalid }
            paused = false
            assertion.acquire(reason: CaptureSession.assertionReason)
            if asleep { return GhiStatus.ok }
            return rebuild(announceAlways: false)
        }
    }

    /// Returns only when no callback is running or will run again.
    func stop() {
        var observer: PowerObserver?
        control.sync {
            stopped = true
            retryTicket += 1
            listeners.forEach { $0.remove() }
            listeners = []
            observer = power
            power = nil
            teardownGraph()
            assertion.release()
        }
        // Outside `control`: invalidate waits for the power queue, whose
        // willSleep handler may itself be waiting for `control`.
        observer?.invalidate()
        events.sync {}
    }

    // MARK: - Graph lifecycle (on `control`)

    private func makeGraph(system: Bool) throws -> CaptureGraph {
        let config = GraphConfig(
            captureMic: captureMic, captureSystem: system, tapPIDs: tapPIDs,
            audio: audio, userData: userData,
            report: { [weak self] code, detail in self?.emit(8, code, detail) })
        return try CaptureGraph(
            config: config, changed: { [weak self] in self?.scheduleReconcile() }, changeQueue: control)
    }

    /// Builds and starts a graph. If the tap part fails while the mic is also
    /// wanted, falls back to a mic-only graph so the mic keeps running.
    private func startGraph() throws -> CaptureGraph {
        func attempt(system: Bool) throws -> CaptureGraph {
            let built = try makeGraph(system: system)
            try built.start()
            return built
        }
        do {
            return try attempt(system: captureSystem)
        } catch let failure as CaptureError where captureSystem && captureMic {
            emit(8, Int64(failure.code), "system audio unavailable: \(failure.detail)")
            return try attempt(system: false)
        }
    }

    private func teardownGraph() {
        graph?.stop()
        graph = nil
    }

    /// Tells the host about a running graph. Blocks until delivered.
    private func announce(_ built: CaptureGraph, mic: Bool, system: Bool, started: Bool) {
        if mic {
            emit(started ? 1 : 3, started ? Int64(IOContext.micTrack) : 0, built.micName, wait: true)
        }
        if system {
            emit(started ? 1 : 4, started ? Int64(IOContext.systemTrack) : 0, built.systemName, wait: true)
        }
    }

    /// Full teardown and rebuild. With `announceAlways` false the host is told
    /// only if the devices or rate differ from the previous graph (or a lost
    /// track came back). A failed rebuild is retried with backoff.
    @discardableResult
    private func rebuild(announceAlways: Bool) -> Int32 {
        teardownGraph()
        do {
            let built = try startGraph()
            graph = built
            let all = announceAlways || announced != built.signature
            if captureMic && (all || micLost) {
                emit(3, 0, built.micName, wait: true)
            }
            if built.hasSystem && (all || systemLost) {
                emit(4, 0, built.systemName, wait: true)
            }
            announced = built.signature
            micLost = false
            if built.hasSystem { systemLost = false }
            if captureSystem && !built.hasSystem {
                systemDegraded()
            } else {
                retryTicket += 1
                retryCount = 0
            }
            return GhiStatus.ok
        } catch {
            let failure = error as? CaptureError
            emit(8, Int64(failure?.code ?? GhiStatus.tap), failure?.detail ?? "rebuild failed")
            if captureMic && !micLost { micLost = true; emit(5, Int64(IOContext.micTrack), nil) }
            if captureSystem && !systemLost { systemLost = true; emit(5, Int64(IOContext.systemTrack), nil) }
            scheduleRetry()
            return failure?.code ?? GhiStatus.tap
        }
    }

    /// The mic runs but the system tap could not be built.
    private func systemDegraded() {
        if !systemLost { systemLost = true; emit(5, Int64(IOContext.systemTrack), nil) }
        scheduleRetry()
    }

    /// Retries the rebuild after 0.5, 1, 2, 4 s, then every 5 s.
    private func scheduleRetry() {
        retryTicket += 1
        let ticket = retryTicket
        let delays: [Double] = [0.5, 1, 2, 4]
        let delay = retryCount < delays.count ? delays[retryCount] : 5
        retryCount += 1
        control.asyncAfter(deadline: .now() + delay) { [weak self] in
            guard let self, ticket == self.retryTicket,
                  !self.stopped, !self.paused, !self.asleep else { return }
            // Mic-only fallback running: rebuild (which interrupts the mic) only
            // once a tap can actually be created.
            if self.graph != nil, !CaptureGraph.canCreateTap(pids: self.tapPIDs) {
                self.scheduleRetry()
                return
            }
            self.rebuild(announceAlways: false)
        }
    }

    // MARK: - Change handling

    private func installListeners() {
        let system = AudioObjectID(kAudioObjectSystemObject)
        for selector in [kAudioHardwarePropertyDefaultInputDevice, kAudioHardwarePropertyDefaultOutputDevice] {
            listeners.append(PropertyListener(
                object: system, selector: selector, queue: control,
                handler: { [weak self] in self?.scheduleReconcile() }))
        }
    }

    private func installPower() {
        power = PowerObserver(
            queue: DispatchQueue(label: "app.ghira.audio-mac.power"),
            onSleep: { [weak self] in self?.willSleep() },
            onWake: { [weak self] in self?.didWake() })
    }

    /// Device notifications come in bursts (a Bluetooth switch fires several);
    /// wait for them to settle, then compare against the running graph.
    private func scheduleReconcile() {
        reconcileTicket += 1
        let ticket = reconcileTicket
        control.asyncAfter(deadline: .now() + 0.4) { [weak self] in
            guard let self, ticket == self.reconcileTicket else { return }
            self.reconcile()
        }
    }

    private func reconcile() {
        guard !stopped, !paused, !asleep else { return }

        let route = Route.eventCode()
        if route != lastRoute {
            lastRoute = route
            emit(2, route, nil)
        }

        let input = CAUtil.defaultDevice(input: true) ?? 0
        let output = CAUtil.defaultDevice(input: false) ?? 0
        let stale: Bool
        if let graph {
            stale = (captureMic && input != graph.inputDevice)
                || (captureSystem && (output != graph.outputDevice || !graph.hasSystem))
                || (graph.currentRate().map { $0 != graph.rate } ?? true)
        } else {
            stale = true // a previous rebuild failed; try again
        }
        if stale { rebuild(announceAlways: true) }
    }

    private func willSleep() {
        control.sync {
            guard !stopped else { return }
            asleep = true
            retryTicket += 1
            teardownGraph()
            emit(6, 0, nil, wait: true)
        }
    }

    private func didWake() {
        // Give the audio stack a moment to re-enumerate devices.
        control.asyncAfter(deadline: .now() + 1.0) { [weak self] in
            guard let self, !self.stopped else { return }
            self.asleep = false
            // WAKE goes out before IO restarts: the host rebases on the first
            // post-wake block.
            self.emit(7, 0, nil, wait: true)
            if !self.paused { self.rebuild(announceAlways: false) }
        }
    }

    // MARK: - Events

    /// Queues an event for the host. With `wait` the call returns once the host
    /// callback has run (used so ordering against audio is well defined).
    private func emit(_ kind: UInt32, _ code: Int64, _ detail: String?, wait: Bool = false) {
        let deliver = { [event, userData] in
            if let detail {
                detail.withCString { event(userData, kind, code, $0) }
            } else {
                event(userData, kind, code, nil)
            }
        }
        if wait { events.sync(execute: deliver) } else { events.async(execute: deliver) }
    }
}
