// SPDX-License-Identifier: Apache-2.0
//
// State used on the Core Audio IO thread. Everything is allocated up front; the
// IO path takes no locks, allocates nothing and sends no ObjC messages.

import CoreAudio
import Foundation

typealias AudioCallback = @convention(c) (
    UnsafeMutableRawPointer?, UInt32, UnsafePointer<Float>?, UInt32, Double, UInt64
) -> Void

typealias EventCallback = @convention(c) (
    UnsafeMutableRawPointer?, UInt32, Int64, UnsafePointer<CChar>?
) -> Void

/// mach_absolute_time ticks -> nanoseconds, precomputed once.
private let timebase: mach_timebase_info_data_t = {
    var info = mach_timebase_info_data_t()
    mach_timebase_info(&info)
    return info
}()

private func hostTimeToNanos(_ ticks: UInt64) -> UInt64 {
    let numer = UInt64(timebase.numer), denom = UInt64(timebase.denom)
    // Split to avoid overflowing ticks * numer on long uptimes.
    return (ticks / denom) * numer + (ticks % denom) * numer / denom
}

final class IOContext: @unchecked Sendable {
    static let micTrack: UInt32 = 0
    static let systemTrack: UInt32 = 1

    private let audio: AudioCallback
    private let userData: UnsafeMutableRawPointer?
    private let captureMic: Bool
    private let captureSystem: Bool
    private let rate: Double
    /// Input buffers the aggregate must present (mic streams + the tap stream).
    private let expectedBuffers: Int
    private let scratch: UnsafeMutablePointer<Float>
    private let scratchFrames: Int

    init(
        audio: @escaping AudioCallback, userData: UnsafeMutableRawPointer?,
        captureMic: Bool, captureSystem: Bool, rate: Double, expectedBuffers: Int, maxFrames: Int
    ) {
        self.audio = audio
        self.userData = userData
        self.captureMic = captureMic
        self.captureSystem = captureSystem
        self.rate = rate
        self.expectedBuffers = expectedBuffers
        self.scratchFrames = max(maxFrames, 4096)
        self.scratch = .allocate(capacity: scratchFrames)
        scratch.initialize(repeating: 0, count: scratchFrames)
        _ = hostTimeToNanos(0) // force the lazy timebase now, not on the IO thread
    }

    deinit { scratch.deallocate() }

    /// Input buffer layout of the aggregate: the sub-device streams first (mic at
    /// index 0), the tap stream last.
    func process(_ inputTime: UnsafePointer<AudioTimeStamp>, _ input: UnsafePointer<AudioBufferList>) {
        let list = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: input))
        guard list.count > 0 else { return }
        let stamp = inputTime.pointee
        let ns = hostTimeToNanos(stamp.mHostTime)
        if captureMic {
            deliver(list[0], track: IOContext.micTrack, hostNanos: ns)
        }
        // The tap stream is last. Without the full complement it is not there, and
        // the last buffer would be the mic: never deliver that as system audio.
        if captureSystem, list.count >= expectedBuffers {
            deliver(list[list.count - 1], track: IOContext.systemTrack, hostNanos: ns)
        }
    }

    /// Mixes one interleaved buffer down to mono and hands it to the host.
    private func deliver(_ buffer: AudioBuffer, track: UInt32, hostNanos: UInt64) {
        let channels = Int(buffer.mNumberChannels)
        guard channels > 0, let raw = buffer.mData else { return }
        let total = Int(buffer.mDataByteSize) / (MemoryLayout<Float>.size * channels)
        guard total > 0 else { return }
        let samples = raw.assumingMemoryBound(to: Float.self)

        if channels == 1 {
            audio(userData, track, samples, UInt32(total), rate, hostNanos)
            return
        }
        let scale = 1 / Float(channels)
        var done = 0
        while done < total {
            let n = min(scratchFrames, total - done)
            for i in 0..<n {
                var sum: Float = 0
                let base = (done + i) * channels
                for c in 0..<channels { sum += samples[base + c] }
                scratch[i] = sum * scale
            }
            let offset = UInt64(Double(done) / rate * 1e9)
            audio(userData, track, scratch, UInt32(n), rate, hostNanos + offset)
            done += n
        }
    }
}
