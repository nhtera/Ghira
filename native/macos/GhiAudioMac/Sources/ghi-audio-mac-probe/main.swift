// SPDX-License-Identifier: Apache-2.0
//
// Live check for GhiAudioMac: drives the C ABI and prints per-track statistics.
//   ghi-audio-mac-probe [--seconds N] [--mic] [--system] [--list] [--route]
// Nothing is written to disk; audio is only measured.

import Foundation
import GhiAudioMac

/// Per-track counters. Written from the audio callback, read after stop.
final class TrackStats {
    var frames: UInt64 = 0
    var calls: UInt64 = 0
    var sumSquares: Double = 0
    var peak: Float = 0
    var rate: Double = 0
    var rates = Set<Double>()
    var firstHost: UInt64 = 0
    var lastHost: UInt64 = 0
    var lastFrames: UInt32 = 0
    var maxGapMs: Double = 0
    var lastEndNs: UInt64 = 0
    var firstWall: Double = 0
    var lastWall: Double = 0
}

let stats = [TrackStats(), TrackStats()]
let start = Date()

func stamp() -> String { String(format: "%6.3f", Date().timeIntervalSince(start)) }

let onAudio: @convention(c) (UnsafeMutableRawPointer?, UInt32, UnsafePointer<Float>?, UInt32, Double, UInt64) -> Void = {
    _, track, samples, frames, rate, host in
    guard track < 2, let samples else { return }
    let s = stats[Int(track)]
    let wall = Date().timeIntervalSince(start)
    if s.calls == 0 { s.firstHost = host; s.firstWall = wall }
    s.lastWall = wall
    if s.lastEndNs != 0 {
        let gap = (Double(host) - Double(s.lastEndNs)) / 1e6
        s.maxGapMs = max(s.maxGapMs, abs(gap))
    }
    s.lastEndNs = host + UInt64(Double(frames) / rate * 1e9)
    s.lastHost = host
    s.calls += 1
    s.frames += UInt64(frames)
    s.rate = rate
    s.rates.insert(rate)
    s.lastFrames = frames
    for i in 0..<Int(frames) {
        let v = samples[i]
        s.sumSquares += Double(v * v)
        s.peak = max(s.peak, abs(v))
    }
}

let kindNames = [1: "TRACK_STARTED", 2: "ROUTE_CHANGED", 3: "MIC_DEVICE_CHANGED", 4: "SYSTEM_RESTARTED",
                 5: "TRACK_LOST", 6: "SLEEP", 7: "WAKE", 8: "ERROR"]
let onEvent: @convention(c) (UnsafeMutableRawPointer?, UInt32, Int64, UnsafePointer<CChar>?) -> Void = {
    _, kind, code, detail in
    let text = detail.map { String(cString: $0) } ?? "-"
    print("[\(stamp())] event \(kindNames[Int(kind)] ?? "\(kind)") code=\(code) detail=\(text)")
}

func routeName(_ kind: UInt32) -> String {
    ["unknown", "speakers", "headphones", "bluetooth", "external"][Int(min(kind, 4))]
}

var seconds = 5.0
var pids: [Int32] = []
var flags: UInt32 = 0
var list = false, route = false, pauseTest = false
var args = CommandLine.arguments.dropFirst()
while let arg = args.popFirst() {
    switch arg {
    case "--seconds": seconds = Double(args.popFirst() ?? "") ?? seconds
    case "--mic": flags |= 1
    case "--system": flags |= 2
    case "--list": list = true
    case "--route": route = true
    case "--pause": pauseTest = true
    case "--pid": pids.append(Int32(args.popFirst() ?? "") ?? 0)
    default:
        print("usage: ghi-audio-mac-probe [--seconds N] [--mic] [--system] [--list] [--route] [--pause] [--pid PID]...")
        exit(2)
    }
}

print("abi version \(ghi_mac_abi_version()), mic permission \(ghi_mac_mic_permission())")

if route {
    var kind: UInt32 = 0, bt: UInt32 = 0
    let st = ghi_mac_route(&kind, &bt)
    print("route status=\(st) output=\(routeName(kind)) input_bluetooth=\(bt)")
}
if list {
    if let raw = ghi_mac_audio_processes() {
        print(String(cString: raw))
        ghi_mac_free(raw)
    } else {
        print("audio processes: failed")
    }
}

if flags != 0 {
    var handle: UnsafeMutableRawPointer?
    let st = ghi_mac_start(flags, pids, UInt32(pids.count), onAudio, onEvent, nil, &handle)
    print("[\(stamp())] start status=\(st)")
    if st == 0, let handle {
        if pauseTest {
            Thread.sleep(forTimeInterval: seconds / 2)
            print("[\(stamp())] pause -> \(ghi_mac_pause(handle))")
            Thread.sleep(forTimeInterval: 1)
            print("[\(stamp())] resume -> \(ghi_mac_resume(handle))")
            Thread.sleep(forTimeInterval: seconds / 2)
        } else {
            Thread.sleep(forTimeInterval: seconds)
        }
        ghi_mac_stop(handle)
        print("[\(stamp())] stopped")
        let names = ["mic", "system"]
        for (i, s) in stats.enumerated() where s.calls > 0 {
            let rms = s.frames > 0 ? (s.sumSquares / Double(s.frames)).squareRoot() : 0
            let db = rms > 0 ? 20 * log10(rms) : -Double.infinity
            let span = Double(s.lastHost - s.firstHost) / 1e9
            print(String(
                format: "%@: frames=%llu calls=%llu rates=%@ rms=%.6f (%.1f dBFS) peak=%.4f span=%.3fs "
                    + "expected=%.3fs wall=[%.3f..%.3f] firstHost=%llu lastHost=%llu maxGapMs=%.3f lastCallFrames=%u",
                names[i], s.frames, s.calls, s.rates.sorted().description, rms, db, s.peak, span,
                Double(s.frames) / max(s.rate, 1), s.firstWall, s.lastWall, s.firstHost, s.lastHost, s.maxGapMs, s.lastFrames))
        }
        if stats[0].calls > 0 && stats[1].calls > 0 {
            let offset = (Double(stats[1].firstHost) - Double(stats[0].firstHost)) / 1e6
            print(String(format: "start offset system-mic: %.3f ms", offset))
        }
    }
}
