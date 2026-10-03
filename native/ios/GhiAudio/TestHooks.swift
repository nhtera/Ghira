// SPDX-License-Identifier: Apache-2.0
// Simulator test hooks (phase 16-B). Compiled only with GHI_TEST_HOOKS, which
// `build-ios.sh --sim --test-hooks` sets together with the cargo feature
// `test-hooks`; never in a device or release build.
//
// - Fake mic: launched with GHI_FAKE_MIC=<wav> (via SIMCTL_CHILD_GHI_FAKE_MIC),
//   the audio tap's samples are replaced by the WAV's first channel (16-bit
//   PCM or 32-bit float, any rate, looped). AVAudioEngine still runs, which
//   keeps the background-audio assertion alive.
// - Darwin-notification triggers, posted from the host with
//   `xcrun simctl spawn <udid> notifyutil -p <name>`:
//     com.nhtera.ghira.test.interrupt-begin / interrupt-end
//     com.nhtera.ghira.test.thermal-serious / thermal-nominal
//     com.nhtera.ghira.test.route-change / call-active (logged only until 16-E)

#if GHI_TEST_HOOKS
import Foundation
import notify
import Security
import GhiIOS

enum GhiTestHooks {
    private static let lock = NSLock()
    private static var pcm: [Float] = []
    private static var pcmRate: Double = 16000
    private static var position = 0
    private static var tokens: [Int32] = []

    static func install() {
        NSLog("ghira: TEST HOOKS ENABLED")
        if let path = ProcessInfo.processInfo.environment["GHI_FAKE_MIC"] {
            if let wav = readWav(URL(fileURLWithPath: path)) {
                lock.lock()
                pcm = wav.samples
                pcmRate = wav.rate
                lock.unlock()
                NSLog("ghira: fake mic: \(path) (\(wav.samples.count) samples at \(wav.rate) Hz)")
            } else {
                NSLog("ghira: fake mic: cannot read \(path)")
            }
        }
        if ProcessInfo.processInfo.environment["GHI_SPIKE"] == "storage" { storageSpike() }
        let prefix = "com.nhtera.ghira.test."
        let actions: [(String, () -> Void)] = [
            ("interrupt-begin", { ghi_ios_interruption(true) }),
            ("interrupt-end", { ghi_ios_interruption(false) }),
            ("thermal-serious", { ghi_ios_thermal_changed(2) }),
            ("thermal-nominal", { ghi_ios_thermal_changed(0) }),
            ("route-change", { NSLog("ghira: test hook: route-change") }),
            ("call-active", { NSLog("ghira: test hook: call-active") }),
        ]
        for (name, action) in actions {
            var token: Int32 = 0
            notify_register_dispatch(prefix + name, &token, .main) { _ in
                NSLog("ghira: test hook: \(name)")
                action()
            }
            tokens.append(token)
        }
    }

    /// Spike 16-B(b): App Group container, shared UserDefaults and a
    /// data-protection Keychain item under simulator ad-hoc signing. Writes
    /// Documents/spike-b.json.
    static func storageSpike() {
        var result: [String: Any] = [:]
        let group = "group.com.nhtera.ghira"
        let fm = FileManager.default
        if let dir = fm.containerURL(forSecurityApplicationGroupIdentifier: group) {
            result["groupContainer"] = dir.path
            let file = dir.appendingPathComponent("spike.txt")
            result["groupFileWrite"] = (try? "ok".write(to: file, atomically: true, encoding: .utf8)) != nil
        } else {
            result["groupContainer"] = NSNull()
        }
        if let defaults = UserDefaults(suiteName: group) {
            defaults.set("v", forKey: "spike")
            result["groupDefaults"] = defaults.string(forKey: "spike") == "v"
        }
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "com.nhtera.ghira.spike",
            kSecAttrAccount as String: "k",
            kSecUseDataProtectionKeychain as String: true,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        SecItemDelete(query as CFDictionary)
        var add = query
        add[kSecValueData as String] = Data([1, 2, 3, 4])
        result["keychainAdd"] = Int(SecItemAdd(add as CFDictionary, nil))
        var read = query
        read[kSecReturnData as String] = true
        var out: CFTypeRef?
        result["keychainRead"] = Int(SecItemCopyMatching(read as CFDictionary, &out))
        result["keychainValueOk"] = (out as? Data) == Data([1, 2, 3, 4])
        // Shared-group keychain access group (needs keychain-access-groups; expected to fail ad-hoc).
        var shared = add
        shared[kSecAttrAccount as String] = "g"
        shared[kSecAttrAccessGroup as String] = group
        SecItemDelete(shared as CFDictionary)
        result["keychainGroupAdd"] = Int(SecItemAdd(shared as CFDictionary, nil))
        SecItemDelete(query as CFDictionary)
        SecItemDelete(shared as CFDictionary)
        if let docs = fm.urls(for: .documentDirectory, in: .userDomainMask).first,
           let data = try? JSONSerialization.data(withJSONObject: result, options: [.sortedKeys]) {
            try? data.write(to: docs.appendingPathComponent("spike-b.json"))
        }
        NSLog("ghira: spike b: \(result)")
    }

    /// Pushes fake samples for one tap block (`frames` at the hardware `rate`).
    /// False when no fake mic is loaded: the caller pushes the real samples.
    static func pushFakeMic(frames: Int, rate: Double, hostNs: UInt64) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        guard !pcm.isEmpty, rate > 0 else { return false }
        let count = max(1, Int(Double(frames) * pcmRate / rate))
        var block = [Float](repeating: 0, count: count)
        for i in 0..<count {
            block[i] = pcm[position]
            position = (position + 1) % pcm.count
        }
        block.withUnsafeBufferPointer { ghi_ios_push_pcm($0.baseAddress, count, pcmRate, hostNs) }
        return true
    }

    private static func readWav(_ url: URL) -> (samples: [Float], rate: Double)? {
        guard let d = try? Data(contentsOf: url), d.count > 44,
              d[0..<4] == Data("RIFF".utf8), d[8..<12] == Data("WAVE".utf8)
        else { return nil }
        func u16(_ o: Int) -> Int { Int(d[o]) | Int(d[o + 1]) << 8 }
        func u32(_ o: Int) -> Int { u16(o) | u16(o + 2) << 16 }
        var format = 0, channels = 1, rate = 16000, bits = 16
        var offset = 12
        while offset + 8 <= d.count {
            let id = String(decoding: d[offset..<offset + 4], as: UTF8.self)
            let size = u32(offset + 4)
            let body = offset + 8
            if id == "fmt " {
                format = u16(body)
                channels = max(1, u16(body + 2))
                rate = u32(body + 4)
                bits = u16(body + 14)
            } else if id == "data" {
                let end = min(d.count, body + size)
                let bytes = bits / 8
                let stride = bytes * channels
                guard bytes == 2 || bytes == 4, stride > 0 else { return nil }
                var out: [Float] = []
                out.reserveCapacity((end - body) / stride)
                var o = body
                while o + stride <= end {
                    if bytes == 2 {
                        out.append(Float(Int16(bitPattern: UInt16(u16(o)))) / 32768)
                    } else if format == 3 {
                        out.append(Float(bitPattern: UInt32(u32(o))))
                    } else {
                        out.append(Float(Int32(bitPattern: UInt32(u32(o)))) / 2147483648)
                    }
                    o += stride
                }
                return (out, Double(rate))
            }
            offset = body + size + (size & 1)
        }
        return nil
    }
}
#endif
