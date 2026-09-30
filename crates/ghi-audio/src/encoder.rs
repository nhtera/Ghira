// SPDX-License-Identifier: Apache-2.0
//! Ogg Opus encoding of the raw 16 kHz tracks, and crash-tolerant decoding.
//!
//! One Ogg stream per track: `OpusHead` and `OpusTags` pages, then audio pages
//! of about one second (50 packets of 20 ms) so that a crash loses at most the
//! current page. Pages are handed to a [`PageSink`], the stand-in for the
//! phase 5 bundle writer, which owns files, encryption and fsync policy.
//!
//! Granule positions follow RFC 7845: the 48 kHz sample count the decoder has
//! produced up to the page's last packet, including the encoder's `pre_skip`
//! priming; the final page is trimmed so that the decoded length is exact.

use std::io::{self, Read};

use ogg::writing::{PacketWriteEndInfo, PacketWriter};
use opus::{Application, Bitrate, Channels, Decoder, Encoder};

use crate::pipeline::FrameSink;
use crate::{Marker, MarkerKind, SAMPLE_RATE, Track};

/// Samples per Opus packet at 16 kHz (20 ms).
pub const PACKET_SAMPLES: usize = 320;
/// Granule units (48 kHz) per input sample (16 kHz).
const GRANULE_PER_SAMPLE: u64 = 3;

/// Receives the encoder's output. Implemented by the bundle writer (phase 5)
/// and by the file sinks of `ghi record`.
pub trait PageSink {
    /// Appends one complete Ogg page of `track`'s stream.
    fn write_page(&mut self, track: Track, page: &[u8]) -> io::Result<()>;
    /// Makes what was written to `track` survive a crash. `durable` asks for
    /// the strong flavour (`F_FULLFSYNC`); otherwise a write barrier suffices.
    fn sync(&mut self, track: Track, durable: bool) -> io::Result<()>;
    /// Records a marker (pause, gap, AEC span...) at its timeline position.
    fn marker(&mut self, marker: &Marker) -> io::Result<()>;
}

#[derive(Debug, Clone, Copy)]
pub struct EncoderConfig {
    pub application: Application,
    pub bitrate_bps: i32,
    /// Packets per page (50 = 1 s).
    pub packets_per_page: u32,
    /// Every n-th page gets a durable sync, the others a barrier.
    pub full_sync_every_pages: u32,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            application: Application::Voip,
            bitrate_bps: 24_000,
            packets_per_page: 50,
            full_sync_every_pages: 2,
        }
    }
}

fn opus_err(e: opus::Error) -> io::Error {
    io::Error::other(format!("opus: {e}"))
}

/// Encodes one track to an Ogg Opus byte stream, page by page.
pub struct TrackEncoder {
    enc: Encoder,
    pw: PacketWriter<'static, Vec<u8>>,
    serial: u32,
    packets_per_page: u32,
    pre_skip: u32,
    lookahead: usize,
    pending: Vec<f32>,
    /// Encoded, not yet written (a page can only be ended after its last packet is known).
    held: Option<(Vec<u8>, u64)>,
    in_page: u32,
    /// 16 kHz samples handed to `push`.
    real_samples: u64,
    /// 16 kHz samples turned into packets.
    encoded_samples: u64,
    buf: Vec<u8>,
    finished: bool,
}

impl TrackEncoder {
    /// Creates the encoder and returns it with the two header pages.
    pub fn new(track: Track, serial: u32, cfg: &EncoderConfig) -> io::Result<(Self, Vec<u8>)> {
        let mut enc =
            Encoder::new(SAMPLE_RATE, Channels::Mono, cfg.application).map_err(opus_err)?;
        enc.set_bitrate(Bitrate::Bits(cfg.bitrate_bps))
            .map_err(opus_err)?;
        enc.set_vbr(true).map_err(opus_err)?;
        let lookahead = enc.get_lookahead().map_err(opus_err)?.max(0) as usize;
        let pre_skip = (lookahead as u64 * GRANULE_PER_SAMPLE) as u32;
        let mut pw = PacketWriter::new(Vec::new());

        let mut head = Vec::with_capacity(19);
        head.extend_from_slice(b"OpusHead");
        head.push(1); // version
        head.push(1); // channels
        head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
        head.extend_from_slice(&SAMPLE_RATE.to_le_bytes()); // original input rate
        head.extend_from_slice(&0i16.to_le_bytes()); // output gain
        head.push(0); // channel mapping family 0
        pw.write_packet(head, serial, PacketWriteEndInfo::EndPage, 0)?;

        let vendor = concat!("ghi-audio ", env!("CARGO_PKG_VERSION"));
        let comment = format!("TRACK={}", track.name());
        let mut tags = Vec::new();
        tags.extend_from_slice(b"OpusTags");
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor.as_bytes());
        tags.extend_from_slice(&1u32.to_le_bytes());
        tags.extend_from_slice(&(comment.len() as u32).to_le_bytes());
        tags.extend_from_slice(comment.as_bytes());
        pw.write_packet(tags, serial, PacketWriteEndInfo::EndPage, 0)?;
        let headers = std::mem::take(pw.inner_mut());

        Ok((
            Self {
                enc,
                pw,
                serial,
                packets_per_page: cfg.packets_per_page.max(1),
                pre_skip,
                lookahead,
                pending: Vec::with_capacity(PACKET_SAMPLES),
                held: None,
                in_page: 0,
                real_samples: 0,
                encoded_samples: 0,
                buf: vec![0u8; 1500],
                finished: false,
            },
            headers,
        ))
    }

    /// Feeds 16 kHz samples; `emit` receives each completed page.
    pub fn push(
        &mut self,
        mut samples: &[f32],
        emit: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        self.real_samples += samples.len() as u64;
        while !samples.is_empty() {
            let take = (PACKET_SAMPLES - self.pending.len()).min(samples.len());
            self.pending.extend_from_slice(&samples[..take]);
            samples = &samples[take..];
            if self.pending.len() == PACKET_SAMPLES {
                self.encode_pending(emit)?;
            }
        }
        Ok(())
    }

    fn encode_pending(&mut self, emit: &mut dyn FnMut(&[u8]) -> io::Result<()>) -> io::Result<()> {
        let n = self
            .enc
            .encode_float(&self.pending, &mut self.buf)
            .map_err(opus_err)?;
        self.pending.clear();
        self.encoded_samples += PACKET_SAMPLES as u64;
        let granule = self.encoded_samples * GRANULE_PER_SAMPLE;
        let pkt = self.buf[..n].to_vec();
        if let Some((p, g)) = self.held.replace((pkt, granule)) {
            self.in_page += 1;
            let end = if self.in_page >= self.packets_per_page {
                self.in_page = 0;
                PacketWriteEndInfo::EndPage
            } else {
                PacketWriteEndInfo::NormalPacket
            };
            self.write(p, end, g, emit)?;
        }
        Ok(())
    }

    fn write(
        &mut self,
        pkt: Vec<u8>,
        end: PacketWriteEndInfo,
        granule: u64,
        emit: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        self.pw.write_packet(pkt, self.serial, end, granule)?;
        if end != PacketWriteEndInfo::NormalPacket {
            let page = std::mem::take(self.pw.inner_mut());
            emit(&page)?;
        }
        Ok(())
    }

    /// Ends the current page now (pause, sleep). Samples of an incomplete
    /// 20 ms packet stay buffered and continue in the next page.
    pub fn flush_page(&mut self, emit: &mut dyn FnMut(&[u8]) -> io::Result<()>) -> io::Result<()> {
        if let Some((p, g)) = self.held.take() {
            self.in_page = 0;
            self.write(p, PacketWriteEndInfo::EndPage, g, emit)?;
        }
        Ok(())
    }

    /// Flushes the tail (padded with the encoder's look-ahead of silence) and
    /// writes the end-of-stream page.
    pub fn finish(&mut self, emit: &mut dyn FnMut(&[u8]) -> io::Result<()>) -> io::Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        let target = self.real_samples + self.lookahead as u64;
        while self.encoded_samples < target || !self.pending.is_empty() {
            let need = PACKET_SAMPLES - self.pending.len();
            self.pending.resize(PACKET_SAMPLES, 0.0);
            let _ = need;
            self.encode_pending(emit)?;
        }
        if self.held.is_none() {
            // Nothing was recorded: an EOS page still needs one packet.
            self.pending.resize(PACKET_SAMPLES, 0.0);
            self.encode_pending(emit)?;
        }
        let (p, _) = self.held.take().unwrap_or_default();
        // Trim the padding: total playable = real samples.
        let granule = self.pre_skip as u64 + self.real_samples * GRANULE_PER_SAMPLE;
        self.write(p, PacketWriteEndInfo::EndStream, granule, emit)
    }

    /// 16 kHz samples fed so far.
    pub fn samples(&self) -> u64 {
        self.real_samples
    }
}

/// Encodes the raw tracks and forwards pages and markers to a [`PageSink`].
/// Implements [`FrameSink`], so a pipeline can drive it directly.
pub struct OpusRecorder<S: PageSink> {
    sink: S,
    enc: [Option<TrackEncoder>; 2],
    pages: [u32; 2],
    full_sync_every: u32,
}

impl<S: PageSink> OpusRecorder<S> {
    /// Starts one stream per entry of `tracks` and writes their header pages.
    pub fn new(mut sink: S, tracks: &[Track], cfg: EncoderConfig) -> io::Result<Self> {
        let mut enc: [Option<TrackEncoder>; 2] = [None, None];
        for &t in tracks {
            let (e, headers) = TrackEncoder::new(t, 0x4748_4900 + t.index() as u32, &cfg)?;
            sink.write_page(t, &headers)?;
            sink.sync(t, false)?;
            enc[t.index()] = Some(e);
        }
        Ok(Self {
            sink,
            enc,
            pages: [0; 2],
            full_sync_every: cfg.full_sync_every_pages.max(1),
        })
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    fn with_track(
        &mut self,
        track: Track,
        f: impl FnOnce(&mut TrackEncoder, &mut dyn FnMut(&[u8]) -> io::Result<()>) -> io::Result<()>,
    ) -> io::Result<()> {
        let i = track.index();
        let Some(enc) = self.enc[i].as_mut() else {
            return Ok(());
        };
        let (sink, pages, every) = (&mut self.sink, &mut self.pages[i], self.full_sync_every);
        let mut emit = |page: &[u8]| -> io::Result<()> {
            sink.write_page(track, page)?;
            *pages += 1;
            sink.sync(track, *pages % every == 0)
        };
        f(enc, &mut emit)
    }

    /// Ends the current page of every track and syncs durably.
    pub fn flush(&mut self) -> io::Result<()> {
        for t in Track::ALL {
            self.with_track(t, |e, emit| e.flush_page(emit))?;
            if self.enc[t.index()].is_some() {
                self.sink.sync(t, true)?;
            }
        }
        Ok(())
    }

    /// Finishes every stream (EOS pages, durable sync) and returns the sink.
    pub fn finish(mut self) -> io::Result<S> {
        for t in Track::ALL {
            self.with_track(t, |e, emit| e.finish(emit))?;
            if self.enc[t.index()].is_some() {
                self.sink.sync(t, true)?;
            }
        }
        Ok(self.sink)
    }
}

impl<S: PageSink> FrameSink for OpusRecorder<S> {
    fn frame(&mut self, track: Track, _pos: u64, samples: &[f32]) -> io::Result<()> {
        self.with_track(track, |e, emit| e.push(samples, emit))
    }

    fn marker(&mut self, marker: &Marker) -> io::Result<()> {
        // Data before a pause or sleep is on disk before the marker is.
        if matches!(
            marker.kind,
            MarkerKind::Pause | MarkerKind::Sleep | MarkerKind::Discard { .. }
        ) {
            self.flush()?;
        }
        self.sink.marker(marker)
    }
}

// --- decoding ---------------------------------------------------------------

/// Ogg CRC-32 (polynomial 0x04C11DB7, no reflection, zero init and xor-out).
fn ogg_crc(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut r = (i as u32) << 24;
            for _ in 0..8 {
                r = if r & 0x8000_0000 != 0 {
                    (r << 1) ^ 0x04C1_1DB7
                } else {
                    r << 1
                };
            }
            *e = r;
        }
        t
    });
    data.iter().fold(0u32, |crc, &b| {
        (crc << 8) ^ table[((crc >> 24) as u8 ^ b) as usize]
    })
}

/// What [`read_ogg_opus_detailed`] recovered.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Decoded {
    /// 16 kHz mono samples, priming removed. Pages lost to corruption are
    /// filled with silence so that positions stay true.
    pub samples: Vec<f32>,
    /// The end-of-stream page was seen (the stream was closed cleanly).
    pub complete: bool,
    /// Pages dropped for a bad checksum or structure.
    pub bad_pages: usize,
    /// The data ended inside a page.
    pub truncated: bool,
}

/// Decodes an Ogg Opus stream written by [`TrackEncoder`], tolerating a
/// truncated or corrupt tail (crash recovery). Returns the audio of every
/// intact page.
pub fn read_ogg_opus(mut reader: impl Read) -> io::Result<Vec<f32>> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(decode_ogg_opus(&bytes)?.samples)
}

/// Like [`read_ogg_opus`], with the recovery details.
pub fn read_ogg_opus_detailed(mut reader: impl Read) -> io::Result<Decoded> {
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    decode_ogg_opus(&bytes)
}

fn decode_ogg_opus(bytes: &[u8]) -> io::Result<Decoded> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let mut dec = Decoder::new(SAMPLE_RATE, Channels::Mono).map_err(opus_err)?;
    let mut out = Decoded::default();
    let mut all: Vec<f32> = Vec::new();
    let mut pre_skip16: Option<usize> = None;
    let mut serial: Option<u32> = None;
    let mut last_seq: Option<u32> = None;
    let mut end_granule: Option<u64> = None;
    let mut pos = 0usize;
    let mut frame = vec![0f32; 5760];

    while let Some(rel) = bytes[pos..].windows(4).position(|w| w == b"OggS") {
        let i = pos + rel;
        if bytes.len() < i + 27 {
            out.truncated = true;
            break;
        }
        let h = &bytes[i..i + 27];
        let nsegs = h[26] as usize;
        if h[4] != 0 {
            pos = i + 1;
            continue;
        }
        if bytes.len() < i + 27 + nsegs {
            out.truncated = true;
            break;
        }
        let segs = &bytes[i + 27..i + 27 + nsegs];
        let body_len: usize = segs.iter().map(|&s| s as usize).sum();
        let total = 27 + nsegs + body_len;
        if bytes.len() < i + total {
            out.truncated = true;
            break;
        }
        let page = &bytes[i..i + total];
        let stored = u32::from_le_bytes(page[22..26].try_into().unwrap());
        let mut copy = page.to_vec();
        copy[22..26].fill(0);
        if ogg_crc(&copy) != stored {
            out.bad_pages += 1;
            pos = i + 1;
            continue;
        }
        pos = i + total;

        let flags = page[5];
        let granule = i64::from_le_bytes(page[6..14].try_into().unwrap());
        let page_serial = u32::from_le_bytes(page[14..18].try_into().unwrap());
        let seq = u32::from_le_bytes(page[18..22].try_into().unwrap());
        match serial {
            None => serial = Some(page_serial),
            Some(s) if s != page_serial => continue,
            _ => {}
        }
        let lost_pages = last_seq.is_some_and(|l| seq != l.wrapping_add(1));
        last_seq = Some(seq);

        // Packets: runs of segments ending in one shorter than 255. A packet
        // continued from a lost page, or left open at the end, is dropped.
        let body = &page[27 + nsegs..];
        let (mut off, mut start, mut first) = (0usize, 0usize, true);
        let mut page_samples = 0usize;
        let decoded_before = all.len();
        for &s in segs {
            off += s as usize;
            if s == 255 {
                continue;
            }
            let pkt = &body[start..off];
            start = off;
            let continued = first && flags & 0x01 != 0;
            first = false;
            if continued {
                continue;
            }
            if pkt.starts_with(b"OpusHead") {
                if pkt.len() < 19 {
                    return Err(bad("short OpusHead"));
                }
                let ps = u16::from_le_bytes([pkt[10], pkt[11]]) as usize;
                pre_skip16 = Some(ps / GRANULE_PER_SAMPLE as usize);
                continue;
            }
            if pkt.starts_with(b"OpusTags") {
                continue;
            }
            if pre_skip16.is_none() {
                return Err(bad("audio before OpusHead"));
            }
            match dec.decode_float(pkt, &mut frame, false) {
                Ok(n) => {
                    all.extend_from_slice(&frame[..n]);
                    page_samples += n;
                }
                Err(_) => out.bad_pages += 1,
            }
        }
        if granule >= 0 {
            let g = granule as u64;
            if flags & 0x04 != 0 {
                end_granule = Some(g);
                out.complete = true;
            }
            // Keep positions true across lost pages: the granule says where
            // this page's audio must end.
            let want_end = (g / GRANULE_PER_SAMPLE) as usize;
            let want_start = want_end.saturating_sub(page_samples);
            if (lost_pages || decoded_before < want_start) && decoded_before < want_start {
                let fill = want_start - decoded_before;
                let tail = all.split_off(decoded_before);
                all.resize(decoded_before + fill, 0.0);
                all.extend(tail);
            }
        }
    }

    let skip = pre_skip16.ok_or_else(|| bad("no OpusHead found"))?;
    let mut samples = all.split_off(skip.min(all.len()));
    if let Some(g) = end_granule {
        let real = (g as usize / GRANULE_PER_SAMPLE as usize).saturating_sub(skip);
        samples.truncate(real);
    }
    out.samples = samples;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct MemSink {
        files: BTreeMap<usize, Vec<u8>>,
        page_sizes: Vec<(Track, usize)>,
        syncs: Vec<(Track, bool)>,
        markers: Vec<Marker>,
        /// Fail writes once this many bytes were accepted (disk full).
        limit: Option<usize>,
    }

    impl PageSink for MemSink {
        fn write_page(&mut self, track: Track, page: &[u8]) -> io::Result<()> {
            let f = self.files.entry(track.index()).or_default();
            if self.limit.is_some_and(|l| f.len() + page.len() > l) {
                return Err(io::Error::from_raw_os_error(28)); // ENOSPC
            }
            f.extend_from_slice(page);
            self.page_sizes.push((track, page.len()));
            Ok(())
        }
        fn sync(&mut self, track: Track, durable: bool) -> io::Result<()> {
            self.syncs.push((track, durable));
            Ok(())
        }
        fn marker(&mut self, m: &Marker) -> io::Result<()> {
            self.markers.push(*m);
            Ok(())
        }
    }

    fn tone(secs: f32) -> Vec<f32> {
        let n = (secs * SAMPLE_RATE as f32) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                0.4 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
                    + 0.2 * (2.0 * std::f32::consts::PI * 1250.0 * t).sin()
            })
            .collect()
    }

    fn snr_db(reference: &[f32], got: &[f32]) -> f64 {
        let n = reference.len().min(got.len());
        let (mut s, mut e) = (0.0f64, 0.0f64);
        for i in 0..n {
            s += (reference[i] as f64).powi(2);
            e += (reference[i] as f64 - got[i] as f64).powi(2);
        }
        10.0 * (s / e.max(1e-20)).log10()
    }

    fn record(samples: &[f32], track: Track) -> (Vec<u8>, MemSink) {
        let mut rec =
            OpusRecorder::new(MemSink::default(), &[track], EncoderConfig::default()).unwrap();
        // Uneven chunk sizes, like a pipeline delivering 10 ms frames.
        for c in samples.chunks(160) {
            rec.frame(track, 0, c).unwrap();
        }
        let sink = rec.finish().unwrap();
        (sink.files[&track.index()].clone(), sink)
    }

    /// Byte offsets of every page start.
    fn page_offsets(b: &[u8]) -> Vec<usize> {
        let mut v = Vec::new();
        let mut i = 0;
        while i + 27 <= b.len() && &b[i..i + 4] == b"OggS" {
            v.push(i);
            let n = b[i + 26] as usize;
            let body: usize = b[i + 27..i + 27 + n].iter().map(|&s| s as usize).sum();
            i += 27 + n + body;
        }
        v
    }

    #[test]
    fn ten_second_tone_round_trip() {
        let x = tone(10.0);
        let (bytes, _) = record(&x, Track::Mic);
        let d = read_ogg_opus_detailed(&bytes[..]).unwrap();
        assert!(d.complete && !d.truncated && d.bad_pages == 0);
        assert_eq!(d.samples.len(), x.len(), "decoded length is exact");
        let snr = snr_db(&x, &d.samples);
        let kbps = bytes.len() as f64 * 8.0 / 10.0 / 1000.0;
        eprintln!("round trip: SNR {snr:.1} dB, {kbps:.1} kbps");
        // Priming is trimmed exactly: no other lag correlates better.
        for lag in [-2i32, -1, 1, 2] {
            let (a, b) = if lag > 0 {
                (&x[..x.len() - lag as usize], &d.samples[lag as usize..])
            } else {
                (
                    &x[(-lag) as usize..],
                    &d.samples[..d.samples.len() - (-lag) as usize],
                )
            };
            assert!(snr_db(a, b) < snr, "lag {lag} fits better");
        }
        // Opus is perceptual, not waveform-exact: ~15 dB on two pure tones.
        assert!(snr > 12.0, "SNR {snr}");
        assert!(kbps < 30.0);
    }

    #[test]
    fn stream_is_valid_ogg_opus() {
        let x = tone(3.5);
        let (bytes, _) = record(&x, Track::System);
        let offs = page_offsets(&bytes);
        // head, tags, 3 full pages, tail + EOS.
        assert!(offs.len() >= 6, "{} pages", offs.len());
        assert_eq!(bytes[offs[0] + 5] & 0x02, 0x02, "BOS on the first page");
        assert_eq!(&bytes[offs[0] + 28..offs[0] + 36], b"OpusHead");
        assert_eq!(&bytes[offs[1] + 28..offs[1] + 36], b"OpusTags");
        let last = *offs.last().unwrap();
        assert_eq!(bytes[last + 5] & 0x04, 0x04, "EOS on the last page");
        // Granules: head/tags 0; audio pages advance by 50 packets = 48000.
        let g = |o: usize| u64::from_le_bytes(bytes[o + 6..o + 14].try_into().unwrap());
        assert_eq!((g(offs[0]), g(offs[1])), (0, 0));
        assert_eq!(g(offs[3]) - g(offs[2]), 48_000);
        let head = &bytes[offs[0] + 28..];
        let pre_skip = u16::from_le_bytes([head[10], head[11]]) as u64;
        assert!(pre_skip > 0 && pre_skip.is_multiple_of(3));
        assert_eq!(g(last), pre_skip + x.len() as u64 * 3);
        // An independent reader accepts the stream and finds every packet.
        let mut pr = ogg::PacketReader::new(io::Cursor::new(&bytes));
        let mut n = 0;
        while let Some(p) = pr.read_packet().unwrap() {
            n += 1;
            let _ = p;
        }
        assert!(n > 100);
    }

    #[test]
    fn truncated_mid_page_recovers_the_full_pages() {
        let x = tone(10.0);
        let (bytes, _) = record(&x, Track::Mic);
        let offs = page_offsets(&bytes);
        // audio pages start at offs[2]; cut in the middle of the 10th one.
        let p10 = offs[2 + 9];
        let cut = p10 + (offs[2 + 10] - p10) / 2;
        let d = read_ogg_opus_detailed(&bytes[..cut]).unwrap();
        let secs = d.samples.len() as f64 / SAMPLE_RATE as f64;
        eprintln!("truncated mid page 10: recovered {secs:.3} s of 10 s");
        assert!(d.truncated && !d.complete);
        assert!(secs >= 8.98, "{secs}");
        // What is recovered matches the original.
        assert!(snr_db(&x[..d.samples.len()], &d.samples) > 10.0);
        // The plain API agrees.
        assert_eq!(read_ogg_opus(&bytes[..cut]).unwrap().len(), d.samples.len());
    }

    #[test]
    fn corrupt_middle_page_is_skipped_and_positions_hold() {
        let x = tone(8.0);
        let (bytes, _) = record(&x, Track::Mic);
        let offs = page_offsets(&bytes);
        let mut bad = bytes.clone();
        let mid = offs[2 + 3] + 60; // inside page 4 of the audio
        bad[mid] ^= 0xff;
        let d = read_ogg_opus_detailed(&bad[..]).unwrap();
        assert_eq!(d.bad_pages, 1);
        assert!(d.complete);
        assert_eq!(
            d.samples.len(),
            x.len(),
            "the lost second is silence, not a shift"
        );
        // Before and after the hole the audio still matches.
        let sr = SAMPLE_RATE as usize;
        assert!(snr_db(&x[..3 * sr], &d.samples[..3 * sr]) > 10.0);
        assert!(snr_db(&x[5 * sr..], &d.samples[5 * sr..]) > 10.0);
        // (the hole is offset by the few ms of priming that were trimmed)
        assert!(
            d.samples[3 * sr..4 * sr - 200]
                .iter()
                .all(|&v| v.abs() < 1e-3)
        );
    }

    #[test]
    fn garbage_and_empty_inputs() {
        assert!(read_ogg_opus(&b""[..]).is_err());
        assert!(read_ogg_opus(&b"not an ogg file at all"[..]).is_err());
        // Header only: a valid, empty recording.
        let (e_bytes, _) = record(&[], Track::Mic);
        assert!(read_ogg_opus(&e_bytes[..]).unwrap().is_empty());
    }

    #[test]
    fn recorder_syncs_and_flushes_around_markers() {
        let x = tone(4.0);
        let mut rec = OpusRecorder::new(
            MemSink::default(),
            &[Track::Mic, Track::System],
            EncoderConfig::default(),
        )
        .unwrap();
        for c in x[..2 * 16_000 + 400].chunks(160) {
            rec.frame(Track::Mic, 0, c).unwrap();
            rec.frame(Track::System, 0, c).unwrap();
        }
        let pages_before = rec.sink().page_sizes.len();
        rec.marker(&Marker {
            pos: 32_400,
            kind: MarkerKind::Pause,
        })
        .unwrap();
        assert!(
            rec.sink().page_sizes.len() > pages_before,
            "pause ends the open pages"
        );
        assert_eq!(rec.sink().markers.len(), 1);
        rec.marker(&Marker {
            pos: 32_400,
            kind: MarkerKind::AecOn,
        })
        .unwrap();
        let sink = rec.finish().unwrap();
        assert_eq!(sink.markers.len(), 2);
        // Durable syncs happen, and not on every page.
        let durable = sink.syncs.iter().filter(|s| s.1).count();
        assert!(durable >= 2 && durable < sink.syncs.len());
        // Both streams decode.
        for t in Track::ALL {
            let d = read_ogg_opus_detailed(&sink.files[&t.index()][..]).unwrap();
            assert!(d.complete);
            assert_eq!(d.samples.len(), 2 * 16_000 + 400);
        }
    }

    #[test]
    fn disk_full_surfaces_as_an_error_and_keeps_earlier_data() {
        let x = tone(6.0);
        let mut rec = OpusRecorder::new(
            MemSink {
                limit: Some(12_000),
                ..Default::default()
            },
            &[Track::Mic],
            EncoderConfig::default(),
        )
        .unwrap();
        let mut err = None;
        for c in x.chunks(160) {
            if let Err(e) = rec.frame(Track::Mic, 0, c) {
                err = Some(e);
                break;
            }
        }
        let e = err.expect("write must fail once the limit is hit");
        assert_eq!(e.raw_os_error(), Some(28));
        // Whatever reached the sink is a decodable prefix.
        let d = read_ogg_opus_detailed(&rec.sink().files[&0][..]).unwrap();
        assert!(d.samples.len() >= 16_000, "{}", d.samples.len());
    }
}
