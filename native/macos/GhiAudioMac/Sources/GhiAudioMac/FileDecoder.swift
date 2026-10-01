// SPDX-License-Identifier: Apache-2.0
//
// File decoding through AVFoundation (import fallback for what Symphonia cannot
// read: CAF, AC-3, video containers...). `AVAssetReader` converts to planar
// float32 at 16 kHz per source channel, so the OS does the resampling. C ABI:
// ghi_mac_decode_* in include/ghi_audio_mac.h.

import AVFoundation
import CoreMedia
import Foundation

enum DecodeStatus {
    static let ok: Int32 = 0
    static let invalid: Int32 = -1
    static let noAudio: Int32 = -7
    static let readFailed: Int32 = -8
    static let notFound: Int32 = -9
}

private let outRate = 16_000

final class FileDecoder {
    let channels: Int
    let sourceRate: Int
    let durationMs: Int64
    let codec: String

    private let asset: AVURLAsset
    private let track: AVAssetTrack
    private var reader: AVAssetReader?
    private var output: AVAssetReaderTrackOutput?
    /// Decoded, not yet delivered: one array per channel.
    private var planes: [[Float]]
    private var offset = 0
    /// 16 kHz index of the first undelivered frame (`planes[c][offset]`).
    private var nextPos: UInt64 = 0
    /// Frames before this index are dropped after a seek.
    private var target: UInt64 = 0
    private var fresh = true

    init?(path: String, status: inout Int32) {
        guard FileManager.default.fileExists(atPath: path) else {
            status = DecodeStatus.notFound
            return nil
        }
        asset = AVURLAsset(url: URL(fileURLWithPath: path))
        // Synchronous property reads: this runs on the caller's import thread.
        guard let track = asset.tracks(withMediaType: .audio).first,
              let format = (track.formatDescriptions as? [CMFormatDescription])?.first,
              let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(format)?.pointee,
              asbd.mChannelsPerFrame > 0
        else {
            status = DecodeStatus.noAudio
            return nil
        }
        self.track = track
        channels = Int(asbd.mChannelsPerFrame)
        sourceRate = Int(asbd.mSampleRate)
        codec = FileDecoder.fourCC(asbd.mFormatID)
        let seconds = CMTimeGetSeconds(track.timeRange.duration)
        durationMs = seconds.isFinite && seconds >= 0 ? Int64((seconds * 1000).rounded()) : -1
        planes = Array(repeating: [], count: channels)
        guard startReader(at: 0) else {
            status = DecodeStatus.noAudio
            return nil
        }
        status = DecodeStatus.ok
    }

    deinit { reader?.cancelReading() }

    private static func fourCC(_ id: AudioFormatID) -> String {
        let bytes = [24, 16, 8, 0].map { UInt8((id >> $0) & 0xFF) }
        let printable = bytes.allSatisfy { $0 >= 0x20 && $0 < 0x7F }
        let text = printable ? String(decoding: bytes, as: UTF8.self) : "\(id)"
        return text.trimmingCharacters(in: .whitespaces).lowercased()
    }

    /// (Re)creates the reader positioned at `pos` (16 kHz frames).
    private func startReader(at pos: UInt64) -> Bool {
        reader?.cancelReading()
        reader = nil
        output = nil
        planes = Array(repeating: [], count: channels)
        offset = 0
        guard let reader = try? AVAssetReader(asset: asset) else { return false }
        let settings: [String: Any] = [
            AVFormatIDKey: kAudioFormatLinearPCM,
            AVSampleRateKey: outRate,
            AVNumberOfChannelsKey: channels,
            AVLinearPCMBitDepthKey: 32,
            AVLinearPCMIsFloatKey: true,
            AVLinearPCMIsBigEndianKey: false,
            AVLinearPCMIsNonInterleaved: true,
        ]
        let output = AVAssetReaderTrackOutput(track: track, outputSettings: settings)
        output.alwaysCopiesSampleData = false
        guard reader.canAdd(output) else { return false }
        reader.add(output)
        if pos > 0 {
            reader.timeRange = CMTimeRange(
                start: CMTime(value: CMTimeValue(pos), timescale: CMTimeScale(outRate)),
                duration: .positiveInfinity)
        }
        guard reader.startReading() else { return false }
        self.reader = reader
        self.output = output
        target = pos
        fresh = true
        return true
    }

    func seek(to pos: UInt64) -> Int32 {
        startReader(at: pos) ? DecodeStatus.ok : DecodeStatus.readFailed
    }

    private enum Fill { case ok, end, failed }

    /// Pulls the next sample buffer into `planes`.
    private func fill() -> Fill {
        guard let output else { return .failed }
        guard let sb = output.copyNextSampleBuffer() else {
            return reader?.status == .failed ? .failed : .end
        }
        var size = 0
        guard CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            sb, bufferListSizeNeededOut: &size, bufferListOut: nil, bufferListSize: 0,
            blockBufferAllocator: nil, blockBufferMemoryAllocator: nil, flags: 0,
            blockBufferOut: nil) == noErr, size > 0
        else { return .failed }
        let raw = UnsafeMutableRawPointer.allocate(byteCount: size, alignment: 16)
        defer { raw.deallocate() }
        let list = raw.bindMemory(to: AudioBufferList.self, capacity: 1)
        var block: CMBlockBuffer?
        guard CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
            sb, bufferListSizeNeededOut: nil, bufferListOut: list, bufferListSize: size,
            blockBufferAllocator: nil, blockBufferMemoryAllocator: nil,
            flags: kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
            blockBufferOut: &block) == noErr
        else { return .failed }

        var fresh: [[Float]] = Array(repeating: [], count: channels)
        let buffers = UnsafeMutableAudioBufferListPointer(list)
        if buffers.count == channels {
            // Planar, as requested.
            // Every plane gets the same length (the shortest), so reads
            // never run past one.
            let frames = buffers.map { Int($0.mDataByteSize) / 4 }.min() ?? 0
            for (c, b) in buffers.enumerated() {
                guard let data = b.mData?.assumingMemoryBound(to: Float.self) else { return .failed }
                fresh[c] = Array(UnsafeBufferPointer(start: data, count: frames))
            }
        } else if buffers.count == 1, let data = buffers[0].mData?.assumingMemoryBound(to: Float.self) {
            // Interleaved fallback.
            let total = Int(buffers[0].mDataByteSize) / 4
            let frames = total / channels
            for c in 0..<channels {
                fresh[c] = (0..<frames).map { data[$0 * channels + c] }
            }
        } else {
            return .failed
        }

        var start = nextPos
        if self.fresh {
            // First buffer after open/seek: it says where we really are, and
            // may begin before the requested start.
            let pts = CMSampleBufferGetPresentationTimeStamp(sb)
            let seconds = CMTimeGetSeconds(pts)
            if seconds.isFinite, seconds >= 0 { start = UInt64((seconds * Double(outRate)).rounded()) }
            self.fresh = false
            if start < target {
                let drop = min(Int(target - start), fresh[0].count)
                for c in 0..<channels { fresh[c].removeFirst(drop) }
                start += UInt64(drop)
            }
            nextPos = start
        }
        planes = fresh
        offset = 0
        return .ok
    }

    /// Writes up to `cap` frames planar into `buf` (channel c at `c * cap`).
    /// Returns the frames written (0 at the end) or a negative status.
    func read(into buf: UnsafeMutablePointer<Float>, cap: Int, pos: UnsafeMutablePointer<UInt64>) -> Int64 {
        while offset >= (planes.first?.count ?? 0) {
            // Called from a Rust thread with no run loop: drain the
            // autoreleased sample buffers every time, or a long import grows.
            let r = autoreleasepool { fill() }
            switch r {
            case .ok: continue
            case .end: return 0
            case .failed: return Int64(DecodeStatus.readFailed)
            }
        }
        let n = min(cap, planes[0].count - offset)
        for c in 0..<channels {
            planes[c].withUnsafeBufferPointer { src in
                (buf + c * cap).update(from: src.baseAddress! + offset, count: n)
            }
        }
        pos.pointee = nextPos
        offset += n
        nextPos += UInt64(n)
        return Int64(n)
    }
}

// MARK: C ABI

@_cdecl("ghi_mac_decode_open")
public func ghi_mac_decode_open(
    _ path: UnsafePointer<CChar>?,
    _ channels: UnsafeMutablePointer<UInt32>?,
    _ sampleRate: UnsafeMutablePointer<UInt32>?,
    _ durationMs: UnsafeMutablePointer<Int64>?,
    _ codec: UnsafeMutablePointer<CChar>?,
    _ codecCap: UInt32,
    _ outHandle: UnsafeMutablePointer<UnsafeMutableRawPointer?>?
) -> Int32 {
    guard let path, let channels, let sampleRate, let durationMs, let codec, codecCap > 0, let outHandle
    else { return DecodeStatus.invalid }
    var status = DecodeStatus.ok
    guard let decoder = FileDecoder(path: String(cString: path), status: &status) else { return status }
    channels.pointee = UInt32(decoder.channels)
    sampleRate.pointee = UInt32(decoder.sourceRate)
    durationMs.pointee = decoder.durationMs
    let name = Array(decoder.codec.utf8.prefix(Int(codecCap) - 1))
    for (i, b) in name.enumerated() { codec[i] = CChar(bitPattern: b) }
    codec[name.count] = 0
    outHandle.pointee = Unmanaged.passRetained(decoder).toOpaque()
    return DecodeStatus.ok
}

private func decoder(_ handle: UnsafeMutableRawPointer?) -> FileDecoder? {
    handle.map { Unmanaged<FileDecoder>.fromOpaque($0).takeUnretainedValue() }
}

@_cdecl("ghi_mac_decode_read")
public func ghi_mac_decode_read(
    _ handle: UnsafeMutableRawPointer?,
    _ buf: UnsafeMutablePointer<Float>?,
    _ framesCap: UInt32,
    _ outPos16k: UnsafeMutablePointer<UInt64>?
) -> Int64 {
    guard let d = decoder(handle), let buf, framesCap > 0, let outPos16k else {
        return Int64(DecodeStatus.invalid)
    }
    return d.read(into: buf, cap: Int(framesCap), pos: outPos16k)
}

@_cdecl("ghi_mac_decode_seek")
public func ghi_mac_decode_seek(_ handle: UnsafeMutableRawPointer?, _ pos16k: UInt64) -> Int32 {
    decoder(handle)?.seek(to: pos16k) ?? DecodeStatus.invalid
}

@_cdecl("ghi_mac_decode_close")
public func ghi_mac_decode_close(_ handle: UnsafeMutableRawPointer?) {
    guard let handle else { return }
    Unmanaged<FileDecoder>.fromOpaque(handle).release()
}
