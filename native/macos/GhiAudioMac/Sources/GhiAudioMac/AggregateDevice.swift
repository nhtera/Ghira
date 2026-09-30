// SPDX-License-Identifier: Apache-2.0
//
// One capture graph: a private aggregate device holding the default input device
// (mic) and/or a process tap (system audio), read by a single IOProc so both
// tracks share one clock. A graph is immutable once built; any device or format
// change is handled by tearing it down and building a new one.

import CoreAudio
import Foundation

struct GraphConfig {
    var captureMic: Bool
    var captureSystem: Bool
    var tapPIDs: [Int32]
    var audio: AudioCallback
    var userData: UnsafeMutableRawPointer?
    /// Reports non-fatal problems (skipped PIDs) as GHI_MAC_EV_ERROR.
    var report: (Int64, String) -> Void
}

/// What identifies a graph to the host: its devices and sample rate.
struct GraphSignature: Equatable {
    let inputDevice: AudioDeviceID
    let outputDevice: AudioDeviceID
    let rate: Double
}

final class CaptureGraph {
    let inputDevice: AudioDeviceID
    let outputDevice: AudioDeviceID
    private(set) var rate: Double = 0
    let hasSystem: Bool
    let micName: String
    let systemName: String

    var signature: GraphSignature {
        GraphSignature(inputDevice: inputDevice, outputDevice: outputDevice, rate: rate)
    }

    private var tapID = AudioObjectID(kAudioObjectUnknown)
    private var aggregateID = AudioObjectID(kAudioObjectUnknown)
    private var procID: AudioDeviceIOProcID?
    private let ioQueue = DispatchQueue(label: "app.ghira.audio-mac.io", qos: .userInteractive)
    private var listeners: [PropertyListener] = []
    private var running = false

    /// Creates the tap and the aggregate. Nothing runs until `start()`.
    init(config: GraphConfig, changed: @escaping () -> Void, changeQueue: DispatchQueue) throws {
        let input = CAUtil.defaultDevice(input: true) ?? AudioDeviceID(kAudioObjectUnknown)
        let output = CAUtil.defaultDevice(input: false) ?? AudioDeviceID(kAudioObjectUnknown)
        inputDevice = input
        outputDevice = output
        hasSystem = config.captureSystem
        micName = CAUtil.deviceName(input)
        systemName = CAUtil.deviceName(output)

        // The clock master: the mic when captured, otherwise the output device.
        let clockDevice = config.captureMic ? input : output
        guard clockDevice != kAudioObjectUnknown, let clockUID = CAUtil.deviceUID(clockDevice) else {
            throw CaptureError(code: GhiStatus.noDevice, detail: "no default audio device")
        }

        var aggregate: [String: Any] = [
            kAudioAggregateDeviceNameKey: "Ghira capture",
            kAudioAggregateDeviceUIDKey: "app.ghira.capture.\(UUID().uuidString)",
            kAudioAggregateDeviceMainSubDeviceKey: clockUID,
            kAudioAggregateDeviceIsPrivateKey: true,
            kAudioAggregateDeviceIsStackedKey: false,
            kAudioAggregateDeviceTapAutoStartKey: false,
            kAudioAggregateDeviceSubDeviceListKey: [
                [kAudioSubDeviceUIDKey: clockUID, kAudioSubDeviceDriftCompensationKey: false]
            ],
        ]

        do {
            if config.captureSystem {
                guard #available(macOS 14.2, *) else {
                    throw CaptureError(code: GhiStatus.unsupported, detail: "process taps need macOS 14.2")
                }
                let (id, uid) = try CaptureGraph.makeTap(config)
                tapID = id
                aggregate[kAudioAggregateDeviceTapListKey] = [
                    [kAudioSubTapUIDKey: uid, kAudioSubTapDriftCompensationKey: true]
                ]
            }

            var created = AudioObjectID(kAudioObjectUnknown)
            let status = AudioHardwareCreateAggregateDevice(aggregate as CFDictionary, &created)
            guard status == noErr else {
                throw CaptureError(code: GhiStatus.from(osStatus: status), detail: "create aggregate device")
            }
            aggregateID = created

            // Buffers: every input stream of the clock device, then the tap's.
            let expectedBuffers = CAUtil.streamIDs(clockDevice, scope: kAudioObjectPropertyScopeInput).count
                + (config.captureSystem ? 1 : 0)
            let format = try CaptureGraph.awaitInputFormat(aggregate: created, streams: expectedBuffers)
            rate = format.mSampleRate
            guard format.mFormatID == kAudioFormatLinearPCM, format.mBitsPerChannel == 32,
                  format.mFormatFlags & kAudioFormatFlagIsFloat != 0 else {
                throw CaptureError(code: GhiStatus.tap, detail: "unexpected aggregate stream format")
            }

            var frames = UInt32(0)
            frames = (try? CAUtil.get(created, kAudioDevicePropertyBufferFrameSize, initial: frames)) ?? 4096
            let context = IOContext(
                audio: config.audio, userData: config.userData,
                captureMic: config.captureMic, captureSystem: config.captureSystem,
                rate: rate, expectedBuffers: expectedBuffers, maxFrames: Int(frames) * 4)

            var proc: AudioDeviceIOProcID?
            let ioStatus = AudioDeviceCreateIOProcIDWithBlock(&proc, created, ioQueue) {
                _, input, inputTime, _, _ in
                context.process(inputTime, input)
            }
            guard ioStatus == noErr, proc != nil else {
                throw CaptureError(code: GhiStatus.from(osStatus: ioStatus), detail: "create IOProc")
            }
            procID = proc

            // The aggregate follows its clock device; a rate or stream change
            // means the graph must be rebuilt.
            listeners = [
                PropertyListener(
                    object: created, selector: kAudioDevicePropertyNominalSampleRate,
                    queue: changeQueue, handler: changed),
                PropertyListener(
                    object: created, selector: kAudioDevicePropertyStreamConfiguration,
                    scope: kAudioObjectPropertyScopeInput, queue: changeQueue, handler: changed),
            ]
        } catch {
            destroy()
            throw error
        }
    }

    func start() throws {
        guard let procID else { throw CaptureError(code: GhiStatus.invalid, detail: "graph destroyed") }
        let status = AudioDeviceStart(aggregateID, procID)
        guard status == noErr else {
            throw CaptureError(code: GhiStatus.from(osStatus: status), detail: "start aggregate device")
        }
        running = true
    }

    /// Stops the IOProc and destroys the aggregate and tap. Returns after the IO
    /// thread has finished: AudioDeviceStop and DestroyIOProcID are synchronous.
    func stop() {
        destroy()
    }

    /// The aggregate's current input rate, for detecting a format change.
    func currentRate() -> Double? {
        streamFormat(aggregate: aggregateID)?.mSampleRate
    }

    private func destroy() {
        listeners.forEach { $0.remove() }
        listeners = []
        if let procID {
            if running { AudioDeviceStop(aggregateID, procID) }
            AudioDeviceDestroyIOProcID(aggregateID, procID)
            ioQueue.sync {} // no IO block runs after this returns
        }
        procID = nil
        running = false
        if aggregateID != kAudioObjectUnknown {
            AudioHardwareDestroyAggregateDevice(aggregateID)
            aggregateID = AudioObjectID(kAudioObjectUnknown)
        }
        if tapID != kAudioObjectUnknown {
            if #available(macOS 14.2, *) { AudioHardwareDestroyProcessTap(tapID) }
            tapID = AudioObjectID(kAudioObjectUnknown)
        }
    }

    deinit { destroy() }

    // MARK: - Tap

    /// True if a process tap can be created right now. Used while running
    /// mic-only to decide whether a full rebuild is worth interrupting the mic.
    static func canCreateTap(pids: [Int32]) -> Bool {
        guard #available(macOS 14.2, *) else { return false }
        let config = GraphConfig(
            captureMic: false, captureSystem: true, tapPIDs: pids,
            audio: { _, _, _, _, _, _ in }, userData: nil, report: { _, _ in })
        guard let (tap, _) = try? makeTap(config) else { return false }
        AudioHardwareDestroyProcessTap(tap)
        return true
    }

    @available(macOS 14.2, *)
    private static func makeTap(_ config: GraphConfig) throws -> (AudioObjectID, String) {
        let description: CATapDescription
        if config.tapPIDs.isEmpty {
            // Everything except this process (Ghira's own playback must not loop back).
            let own = AudioProcesses.processObject(forPID: getpid()).map { [$0] } ?? []
            description = CATapDescription(monoGlobalTapButExcludeProcesses: own)
        } else {
            var objects: [AudioObjectID] = []
            for pid in config.tapPIDs {
                if let object = AudioProcesses.processObject(forPID: pid) {
                    objects.append(object)
                } else {
                    config.report(Int64(GhiStatus.invalid), "pid \(pid) has no Core Audio process object")
                }
            }
            guard !objects.isEmpty else {
                throw CaptureError(code: GhiStatus.tap, detail: "none of the requested pids can be tapped")
            }
            description = CATapDescription(monoMixdownOfProcesses: objects)
        }
        description.name = "Ghira system audio"
        description.uuid = UUID()
        description.muteBehavior = .unmuted
        description.isPrivate = true

        var tap = AudioObjectID(kAudioObjectUnknown)
        let status = AudioHardwareCreateProcessTap(description, &tap)
        guard status == noErr else {
            throw CaptureError(code: GhiStatus.from(osStatus: status), detail: "create process tap")
        }
        return (tap, description.uuid.uuidString)
    }

    // MARK: - Aggregate format

    private func streamFormat(aggregate: AudioObjectID) -> AudioStreamBasicDescription? {
        CaptureGraph.inputFormat(aggregate: aggregate, minStreams: 1)
    }

    private static func inputFormat(aggregate: AudioObjectID, minStreams: Int) -> AudioStreamBasicDescription? {
        let streams = CAUtil.streamIDs(aggregate, scope: kAudioObjectPropertyScopeInput)
        guard streams.count >= minStreams, let first = streams.first else { return nil }
        return try? CAUtil.get(first, kAudioStreamPropertyVirtualFormat, initial: AudioStreamBasicDescription())
    }

    /// The aggregate publishes its streams a moment after creation (the tap
    /// stream in particular); wait until all `streams` are there, up to two seconds.
    private static func awaitInputFormat(
        aggregate: AudioObjectID, streams: Int
    ) throws -> AudioStreamBasicDescription {
        let deadline = Date().addingTimeInterval(2)
        while true {
            if let format = inputFormat(aggregate: aggregate, minStreams: max(streams, 1)), format.mSampleRate > 0 {
                return format
            }
            if Date() > deadline {
                throw CaptureError(code: GhiStatus.tap, detail: "aggregate device exposed fewer input streams than expected")
            }
            Thread.sleep(forTimeInterval: 0.02)
        }
    }
}
