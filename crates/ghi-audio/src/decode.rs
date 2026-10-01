// SPDX-License-Identifier: Apache-2.0
//! Import decoders: any supported audio (or video with an audio track) file to
//! 16 kHz `f32` blocks, streamed so that a three-hour file never sits in memory.
//!
//! Routing, in order:
//!
//! 1. Ogg Opus (including what `ghi record` writes) goes to the `opus` crate
//!    ([`Backend::OggOpus`]): Symphonia has no Opus decoder.
//! 2. Everything else is tried with Symphonia ([`Backend::Symphonia`]: WAV,
//!    AIFF, MP3, FLAC, Ogg Vorbis, AAC / ALAC in MP4 / M4A, ADTS, MKV).
//! 3. On macOS, when Symphonia rejects the file (unknown container or codec,
//!    unreadable first packet), AVFoundation takes over
//!    ([`Backend::AvFoundation`], the `ghi_mac_decode_*` C ABI): CAF, AC-3,
//!    video containers and whatever else the OS can play. Elsewhere the error
//!    is [`DecodeError::Unsupported`] ("format not supported on this platform yet").
//!
//! Every backend resamples to 16 kHz per source channel (stereo stays two
//! channels, the caller splits or mixes). Positions are indices on the 16 kHz
//! timeline, the same one the capture pipeline writes.

use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use ogg::reading::PacketReader;
use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{Async, FixedAsync, Resampler as _, SincInterpolationParameters, WindowFunction};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder as SymDecoder, DecoderOptions};
use symphonia::core::errors::{Error as SymError, SeekErrorKind};
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use crate::SAMPLE_RATE;

/// Blocks are at least this long (0.5 s) unless the file ends first, and
/// rarely much longer, so memory stays bounded by a few seconds of audio.
const MIN_BLOCK: usize = SAMPLE_RATE as usize / 2;
/// Input frames per resampler call.
const RS_CHUNK: usize = 1024;
/// Consecutive undecodable packets tolerated before the file counts as corrupt.
const MAX_BAD_PACKETS: u32 = 100;
/// Opus decoder warm-up before a seek target (80 ms of 48 kHz granules).
const OPUS_PREROLL: u64 = 3840;

/// Which decoder produced the audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Symphonia,
    /// macOS only (`AVAssetReader`).
    AvFoundation,
    OggOpus,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Symphonia => "symphonia",
            Backend::AvFoundation => "avfoundation",
            Backend::OggOpus => "ogg-opus",
        }
    }
}

/// What a file contains, as reported by the backend before resampling.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    /// Source channels (the blocks carry this many).
    pub channels: u16,
    /// Native sample rate of the source in Hz.
    pub sample_rate: u32,
    /// Length, when the container says so.
    pub duration_ms: Option<u64>,
    /// Short codec name (`pcm_s16le`, `mp3`, `aac`, `opus`, ...).
    pub codec: String,
    /// Container guessed from the file header (`wav`, `mp4`, `ogg`, ...).
    pub container: String,
    pub backend: Backend,
}

#[derive(Debug)]
pub enum DecodeError {
    NotFound,
    /// Not a format any available backend can read.
    Unsupported {
        format: String,
    },
    /// Recognised, but the data is damaged.
    Corrupt(String),
    Io(io::Error),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::NotFound => write!(f, "file not found"),
            DecodeError::Unsupported { format } => write!(f, "unsupported format: {format}"),
            DecodeError::Corrupt(m) => write!(f, "corrupt audio: {m}"),
            DecodeError::Io(e) => write!(f, "i/o error: {e}"),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<io::Error> for DecodeError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::NotFound => DecodeError::NotFound,
            io::ErrorKind::UnexpectedEof => DecodeError::Corrupt("truncated file".into()),
            _ => DecodeError::Io(e),
        }
    }
}

impl From<SymError> for DecodeError {
    fn from(e: SymError) -> Self {
        match e {
            SymError::IoError(e) => e.into(),
            SymError::DecodeError(m) => DecodeError::Corrupt(m.to_owned()),
            SymError::Unsupported(m) => DecodeError::Unsupported {
                format: m.to_owned(),
            },
            other => DecodeError::Corrupt(other.to_string()),
        }
    }
}

/// A run of decoded audio.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Timeline index at 16 kHz of the first sample.
    pub pos_16k: u64,
    /// One vector per source channel, all the same length, 16 kHz.
    pub channels: Vec<Vec<f32>>,
}

impl Block {
    pub fn frames(&self) -> usize {
        self.channels.first().map_or(0, Vec::len)
    }
}

/// Reads only the file's header and metadata.
pub fn probe(path: &Path) -> Result<Info, DecodeError> {
    Ok(Decoder::open(path)?.info)
}

/// A streaming decoder: [`next_block`](Decoder::next_block) until `None`.
pub struct Decoder {
    info: Info,
    src: Box<dyn Source + Send>,
    /// Timeline index of the next sample `src` will produce.
    pos: u64,
    done: bool,
}

impl Decoder {
    /// Opens `path`, choosing the backend as described in the module docs.
    pub fn open(path: &Path) -> Result<Decoder, DecodeError> {
        Self::open_with(path, None)
    }

    /// Like [`open`](Self::open), forcing a backend. Used by tests and by
    /// callers that want to retry a file with another decoder.
    pub fn open_with(path: &Path, force: Option<Backend>) -> Result<Decoder, DecodeError> {
        let mut head = [0u8; 64];
        let n = File::open(path)?.read(&mut head)?;
        let head = &head[..n];
        let container = sniff(head, path);

        let (info, src) = match force {
            Some(Backend::OggOpus) => open_opus(path)?,
            Some(Backend::Symphonia) => open_symphonia(path, container)?,
            Some(Backend::AvFoundation) => open_native(path, container)?,
            None if is_ogg_opus(head) => open_opus(path)?,
            None => match open_symphonia(path, container) {
                Ok(v) => v,
                Err(first @ (DecodeError::Unsupported { .. } | DecodeError::Corrupt(_))) => {
                    // Prefer the OS decoder's result; keep Symphonia's error
                    // when the OS cannot read the file either.
                    open_native(path, container).map_err(|second| match second {
                        DecodeError::Io(_) => second,
                        _ => first,
                    })?
                }
                Err(e) => return Err(e),
            },
        };
        Ok(Decoder {
            info,
            src,
            pos: 0,
            done: false,
        })
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    /// The next block of at least 0.5 s (less at the end of the file), or
    /// `None` at the end.
    pub fn next_block(&mut self) -> Result<Option<Block>, DecodeError> {
        let mut out: Vec<Vec<f32>> = Vec::new();
        let pos = self.pos;
        while !self.done && out.first().map_or(0, Vec::len) < MIN_BLOCK {
            match self.src.pull()? {
                Some(piece) => {
                    if out.is_empty() {
                        out = piece;
                    } else {
                        for (o, p) in out.iter_mut().zip(piece) {
                            o.extend_from_slice(&p);
                        }
                    }
                }
                None => self.done = true,
            }
        }
        let frames = out.first().map_or(0, Vec::len);
        if frames == 0 {
            return Ok(None);
        }
        self.pos += frames as u64;
        Ok(Some(Block {
            pos_16k: pos,
            channels: out,
        }))
    }

    /// Repositions so that the next block starts at `pos_16k` or, where the
    /// format cannot seek exactly, at the packet boundary before it. The next
    /// [`Block::pos_16k`] reports where decoding really resumed.
    pub fn seek(&mut self, pos_16k: u64) -> Result<(), DecodeError> {
        self.pos = self.src.seek(pos_16k)?;
        self.done = false;
        Ok(())
    }
}

/// One decoder behind [`Decoder`]. Pieces are in order and gapless.
trait Source {
    /// The next piece of 16 kHz planar audio, `None` at the end.
    fn pull(&mut self) -> Result<Option<Vec<Vec<f32>>>, DecodeError>;
    /// Seeks and returns the timeline index the next piece starts at.
    fn seek(&mut self, pos_16k: u64) -> Result<u64, DecodeError>;
}

type Opened = (Info, Box<dyn Source + Send>);

// ---- container sniffing ----------------------------------------------------

fn sniff(h: &[u8], path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if h.starts_with(b"RIFF") && h.get(8..12) == Some(b"WAVE") {
        "wav"
    } else if h.starts_with(b"FORM") {
        "aiff"
    } else if h.starts_with(b"fLaC") {
        "flac"
    } else if h.starts_with(b"OggS") {
        "ogg"
    } else if h.starts_with(b"caff") {
        "caf"
    } else if h.get(4..8) == Some(b"ftyp") {
        match ext.as_deref() {
            Some("mov") => "mov",
            _ => "mp4",
        }
    } else if h.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        "matroska"
    } else if h.starts_with(b"ID3") || (h.len() > 1 && h[0] == 0xFF && h[1] & 0xE0 == 0xE0) {
        match ext.as_deref() {
            Some("aac") => "adts",
            _ => "mp3",
        }
    } else {
        "unknown"
    }
}

/// An Ogg whose first packet is `OpusHead`.
fn is_ogg_opus(h: &[u8]) -> bool {
    if !h.starts_with(b"OggS") || h.len() < 28 {
        return false;
    }
    let body = 27 + h[26] as usize;
    h.get(body..body + 8) == Some(b"OpusHead")
}

// ---- resampling -----------------------------------------------------------

/// Streaming fixed-ratio resampler for all channels of one file. Gapless: the
/// start-up delay is dropped and the tail flushed, so the output length is
/// `round(input * 16000 / rate)`.
struct Resampler {
    /// `None` when the source already is 16 kHz.
    rs: Option<Async<f32>>,
    ratio: f64,
    channels: usize,
    inbuf: Vec<Vec<f32>>,
    outbuf: Vec<Vec<f32>>,
    /// Output frames of start-up delay still to drop.
    skip: usize,
    fed: u64,
    emitted: u64,
}

impl Resampler {
    fn new(rate: u32, channels: usize) -> Result<Self, DecodeError> {
        let ratio = SAMPLE_RATE as f64 / rate as f64;
        let rs = if rate == SAMPLE_RATE {
            None
        } else {
            let params = SincInterpolationParameters::new(64, WindowFunction::BlackmanHarris2)
                .oversampling_factor(128);
            Some(
                Async::new_sinc(ratio, 1.1, &params, RS_CHUNK, channels, FixedAsync::Input)
                    .map_err(|e| DecodeError::Unsupported {
                        format: format!("sample rate {rate} Hz: {e}"),
                    })?,
            )
        };
        let mut r = Resampler {
            rs,
            ratio,
            channels,
            inbuf: vec![Vec::new(); channels],
            outbuf: Vec::new(),
            skip: 0,
            fed: 0,
            emitted: 0,
        };
        r.reset();
        Ok(r)
    }

    /// Forgets the stream so far (after a seek).
    fn reset(&mut self) {
        for b in &mut self.inbuf {
            b.clear();
        }
        self.fed = 0;
        self.emitted = 0;
        if let Some(rs) = self.rs.as_mut() {
            rs.reset();
            self.skip = rs.output_delay();
            self.outbuf = vec![vec![0.0; rs.output_frames_max()]; self.channels];
        }
    }

    fn push(&mut self, planar: &[Vec<f32>]) -> Result<Vec<Vec<f32>>, DecodeError> {
        self.fed += planar.first().map_or(0, Vec::len) as u64;
        self.run(planar)
    }

    fn run(&mut self, planar: &[Vec<f32>]) -> Result<Vec<Vec<f32>>, DecodeError> {
        let Resampler {
            rs,
            channels,
            inbuf,
            outbuf,
            skip,
            emitted,
            ..
        } = self;
        let Some(rs) = rs.as_mut() else {
            *emitted += planar.first().map_or(0, Vec::len) as u64;
            return Ok(planar.to_vec());
        };
        let fail =
            |e: &dyn fmt::Display| DecodeError::Io(io::Error::other(format!("resample: {e}")));
        for (b, p) in inbuf.iter_mut().zip(planar) {
            b.extend_from_slice(p);
        }
        let mut out = vec![Vec::new(); *channels];
        loop {
            let need = rs.input_frames_next();
            if inbuf[0].len() < need {
                break;
            }
            let max = rs.output_frames_max();
            let (used, made) = {
                let inp = SequentialSliceOfVecs::new(&inbuf[..], *channels, need)
                    .map_err(|e| fail(&e))?;
                let mut outp = SequentialSliceOfVecs::new_mut(&mut outbuf[..], *channels, max)
                    .map_err(|e| fail(&e))?;
                rs.process_into_buffer(&inp, &mut outp, None)
                    .map_err(|e| fail(&e))?
            };
            let drop = (*skip).min(made);
            *skip -= drop;
            for c in 0..*channels {
                out[c].extend_from_slice(&outbuf[c][drop..made]);
                inbuf[c].drain(..used);
            }
            *emitted += (made - drop) as u64;
        }
        Ok(out)
    }

    /// The tail: flushes the resampler with silence and trims to the exact length.
    fn finish(&mut self) -> Result<Vec<Vec<f32>>, DecodeError> {
        let mut out = vec![Vec::new(); self.channels];
        let Some(need) = self.rs.as_ref().map(|r| r.input_frames_next()) else {
            return Ok(out);
        };
        let expected = (self.fed as f64 * self.ratio).round() as u64;
        for _ in 0..64 {
            if self.emitted >= expected {
                break;
            }
            let zeros = vec![vec![0.0f32; need]; self.channels];
            for (o, p) in out.iter_mut().zip(self.run(&zeros)?) {
                o.extend_from_slice(&p);
            }
        }
        let excess = (self.emitted.saturating_sub(expected) as usize).min(out[0].len());
        for o in &mut out {
            o.truncate(o.len() - excess);
        }
        self.emitted = expected;
        Ok(out)
    }
}

// ---- Symphonia --------------------------------------------------------------

struct Sym {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn SymDecoder>,
    track_id: u32,
    rate: u32,
    channels: usize,
    rs: Resampler,
    scratch: Option<SampleBuffer<f32>>,
    /// Source frames of the next packets to drop (exact seeks).
    skip_src: u64,
    /// The first buffer decoded at open (it told us the real layout).
    stash: Option<Vec<Vec<f32>>>,
    /// The resampler tail was already emitted.
    flushed: bool,
}

fn open_symphonia(path: &Path, container: &str) -> Result<Opened, DecodeError> {
    let file = File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(
        &hint,
        mss,
        &FormatOptions {
            enable_gapless: true,
            ..Default::default()
        },
        &MetadataOptions::default(),
    )?;
    let format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| DecodeError::Unsupported {
            format: format!("{container}: no audio track"),
        })?;
    let params = track.codec_params.clone();
    let track_id = track.id;
    let decoder = symphonia::default::get_codecs().make(&params, &DecoderOptions::default())?;
    let codec = symphonia::default::get_codecs()
        .get_codec(params.codec)
        .map_or("unknown", |d| d.short_name)
        .to_owned();

    let mut sym = Sym {
        format,
        decoder,
        track_id,
        rate: params.sample_rate.unwrap_or(0),
        channels: params.channels.map_or(0, |c| c.count()),
        rs: Resampler::new(SAMPLE_RATE, 1)?,
        scratch: None,
        skip_src: 0,
        stash: None,
        flushed: false,
    };
    // The first buffer is the proof that the codec works, and fills in a rate
    // or channel layout the container left out.
    let first = sym
        .decode_next()?
        .ok_or_else(|| DecodeError::Corrupt("no audio data".into()))?;
    sym.rs = Resampler::new(sym.rate, sym.channels)?;
    sym.stash = Some(first);
    let duration_ms = params.n_frames.map(|n| n * 1000 / sym.rate as u64);
    let info = Info {
        channels: sym.channels as u16,
        sample_rate: sym.rate,
        duration_ms,
        codec,
        container: container.to_owned(),
        backend: Backend::Symphonia,
    };
    Ok((info, Box::new(sym)))
}

impl Sym {
    /// The next decoded packet of our track as source-rate planar audio.
    fn decode_next(&mut self) -> Result<Option<Vec<Vec<f32>>>, DecodeError> {
        let mut bad = 0u32;
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(SymError::ResetRequired) => return Ok(None),
                Err(e) => return Err(e.into()),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                // Recoverable (a bad frame): skip it, as Symphonia advises.
                Err(SymError::DecodeError(m)) => {
                    bad += 1;
                    if bad > MAX_BAD_PACKETS {
                        return Err(DecodeError::Corrupt(m.to_owned()));
                    }
                    continue;
                }
                Err(SymError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(e) => return Err(e.into()),
            };
            bad = 0;
            let spec = *decoded.spec();
            let frames = decoded.frames();
            if frames == 0 {
                continue;
            }
            let ch = spec.channels.count();
            // MP4 often leaves these out of the container.
            if self.rate == 0 {
                self.rate = spec.rate;
            }
            if self.channels == 0 {
                self.channels = ch;
            }
            if self
                .scratch
                .as_ref()
                .is_none_or(|s| s.capacity() < frames * ch)
            {
                self.scratch = Some(SampleBuffer::new(decoded.capacity() as u64, spec));
            }
            let sb = self.scratch.as_mut().expect("scratch was just set");
            sb.copy_planar_ref(decoded);
            let mut planar: Vec<Vec<f32>> = sb
                .samples()
                .chunks_exact(frames)
                .take(ch)
                .map(<[f32]>::to_vec)
                .collect();
            if self.skip_src > 0 {
                let drop = (self.skip_src as usize).min(frames);
                self.skip_src -= drop as u64;
                for p in &mut planar {
                    p.drain(..drop);
                }
                if drop == frames {
                    continue;
                }
            }
            return Ok(Some(planar));
        }
    }
}

impl Source for Sym {
    fn pull(&mut self) -> Result<Option<Vec<Vec<f32>>>, DecodeError> {
        loop {
            if self.flushed {
                return Ok(None);
            }
            let piece = match self.stash.take() {
                Some(p) => Some(p),
                None => self.decode_next()?,
            };
            let out = match piece {
                Some(p) => self.rs.push(&p)?,
                None => {
                    self.flushed = true;
                    self.rs.finish()?
                }
            };
            if !out[0].is_empty() {
                return Ok(Some(out));
            }
            if self.flushed {
                return Ok(None);
            }
        }
    }

    fn seek(&mut self, pos_16k: u64) -> Result<u64, DecodeError> {
        let secs = pos_16k as f64 / SAMPLE_RATE as f64;
        let to = SeekTo::Time {
            time: Time::new(secs.trunc() as u64, secs.fract()),
            track_id: Some(self.track_id),
        };
        let seeked = match self.format.seek(SeekMode::Accurate, to) {
            Ok(s) => s,
            Err(SymError::SeekError(SeekErrorKind::OutOfRange)) => {
                // Past the end: nothing left to read.
                self.stash = None;
                self.flushed = true;
                return Ok(pos_16k);
            }
            Err(SymError::SeekError(_) | SymError::Unsupported(_)) => {
                return Err(DecodeError::Unsupported {
                    format: "seeking in this file".into(),
                });
            }
            Err(e) => return Err(e.into()),
        };
        self.decoder.reset();
        // Timestamps are in the track's time base, source frames by default.
        let to_frames = |ts: u64| -> u64 {
            let tb = self
                .format
                .tracks()
                .iter()
                .find(|t| t.id == self.track_id)
                .and_then(|t| t.codec_params.time_base);
            match tb {
                Some(tb) => {
                    let t = tb.calc_time(ts);
                    ((t.seconds as f64 + t.frac) * self.rate as f64).round() as u64
                }
                None => ts,
            }
        };
        let (want, got) = (to_frames(seeked.required_ts), to_frames(seeked.actual_ts));
        self.skip_src = want.saturating_sub(got);
        self.stash = None;
        self.flushed = false;
        self.rs.reset();
        Ok(if got > want {
            // The container landed after the target (rare): report the truth.
            got * SAMPLE_RATE as u64 / self.rate as u64
        } else {
            pos_16k
        })
    }
}

// ---- Ogg Opus ---------------------------------------------------------------

struct OpusSrc {
    rdr: PacketReader<BufReader<File>>,
    dec: opus::Decoder,
    serial: u32,
    channels: usize,
    /// Decoder start-up samples at 16 kHz to drop.
    pre_skip: u64,
    /// Length of the audible stream (from the last page), when known.
    end: Option<u64>,
    /// Timeline index of the next sample to be returned.
    pos: u64,
    /// Decoded samples still to drop before `pos`.
    skip: u64,
    /// A whole page decoded by a seek, ready to hand out.
    stash: Option<Vec<Vec<f32>>>,
    frame: Vec<f32>,
}

fn opus_err(e: opus::Error) -> DecodeError {
    DecodeError::Corrupt(format!("opus: {e}"))
}

fn ogg_err(e: ogg::OggReadError) -> DecodeError {
    match e {
        ogg::OggReadError::ReadError(e) => e.into(),
        other => DecodeError::Corrupt(format!("ogg: {other}")),
    }
}

fn open_opus(path: &Path) -> Result<Opened, DecodeError> {
    let file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut rdr = PacketReader::new(BufReader::new(file));
    let head = rdr
        .read_packet()
        .map_err(ogg_err)?
        .ok_or_else(|| DecodeError::Corrupt("empty ogg stream".into()))?;
    let h = &head.data;
    if h.len() < 19 || !h.starts_with(b"OpusHead") {
        return Err(DecodeError::Unsupported {
            format: "ogg without opus".into(),
        });
    }
    let channels = h[9] as usize;
    let pre_skip48 = u16::from_le_bytes([h[10], h[11]]) as u64;
    let input_rate = u32::from_le_bytes([h[12], h[13], h[14], h[15]]);
    if h[18] != 0 || !(1..=2).contains(&channels) {
        return Err(DecodeError::Unsupported {
            format: format!("opus channel mapping {} with {channels} channels", h[18]),
        });
    }
    let serial = head.stream_serial();
    let dec = opus::Decoder::new(
        SAMPLE_RATE,
        if channels == 1 {
            opus::Channels::Mono
        } else {
            opus::Channels::Stereo
        },
    )
    .map_err(opus_err)?;
    // The comment header.
    match rdr.read_packet().map_err(ogg_err)? {
        Some(p) if p.data.starts_with(b"OpusTags") => {}
        _ => return Err(DecodeError::Corrupt("missing OpusTags".into())),
    }

    let pre_skip = pre_skip48 / 3;
    let end = last_granule(path, len).map(|g| (g / 3).saturating_sub(pre_skip));
    let info = Info {
        channels: channels as u16,
        sample_rate: if input_rate == 0 { 48_000 } else { input_rate },
        duration_ms: end.map(|e| e * 1000 / SAMPLE_RATE as u64),
        codec: "opus".into(),
        container: "ogg".into(),
        backend: Backend::OggOpus,
    };
    let src = OpusSrc {
        rdr,
        dec,
        serial,
        channels,
        pre_skip,
        end,
        pos: 0,
        skip: pre_skip,
        stash: None,
        frame: vec![0.0; 5760 * channels],
    };
    Ok((info, Box::new(src)))
}

fn is_opus_header(data: &[u8]) -> bool {
    data.starts_with(b"OpusHead") || data.starts_with(b"OpusTags")
}

/// Granule position of the last page that has one, found in the file's tail.
fn last_granule(path: &Path, len: u64) -> Option<u64> {
    let mut f = File::open(path).ok()?;
    let start = len.saturating_sub(65_536);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = Vec::new();
    f.take(65_536).read_to_end(&mut tail).ok()?;
    let mut i = tail.len().checked_sub(27)?;
    loop {
        if tail[i..].starts_with(b"OggS") {
            let g = i64::from_le_bytes(tail[i + 6..i + 14].try_into().ok()?);
            if g >= 0 {
                return Some(g as u64);
            }
        }
        i = i.checked_sub(1)?;
    }
}

impl OpusSrc {
    /// Decodes one packet to planar 16 kHz audio.
    fn decode(&mut self, data: &[u8]) -> Result<Vec<Vec<f32>>, DecodeError> {
        let n = self
            .dec
            .decode_float(data, &mut self.frame, false)
            .map_err(opus_err)?;
        let ch = self.channels;
        Ok((0..ch)
            .map(|c| (0..n).map(|i| self.frame[i * ch + c]).collect())
            .collect())
    }

    /// Drops `skip` leading samples, then clips at the end of the stream.
    /// Returns `None` when nothing is left to emit.
    fn trim(&mut self, mut planar: Vec<Vec<f32>>) -> Option<Vec<Vec<f32>>> {
        let n = planar[0].len();
        let drop = (self.skip as usize).min(n);
        self.skip -= drop as u64;
        if drop > 0 {
            for p in &mut planar {
                p.drain(..drop);
            }
        }
        let mut n = n - drop;
        if let Some(end) = self.end {
            let room = end.saturating_sub(self.pos) as usize;
            if n > room {
                n = room;
                for p in &mut planar {
                    p.truncate(room);
                }
            }
        }
        self.pos += n as u64;
        (n > 0).then_some(planar)
    }
}

impl Source for OpusSrc {
    fn pull(&mut self) -> Result<Option<Vec<Vec<f32>>>, DecodeError> {
        if let Some(p) = self.stash.take() {
            return Ok(Some(p));
        }
        loop {
            let packet = match self.rdr.read_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(None),
                // A file cut short ends there, like Symphonia's formats.
                Err(ogg::OggReadError::ReadError(e))
                    if e.kind() == io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(ogg_err(e)),
            };
            if packet.stream_serial() != self.serial || is_opus_header(&packet.data) {
                continue;
            }
            let planar = self.decode(&packet.data)?;
            if let Some(p) = self.trim(planar) {
                return Ok(Some(p));
            }
        }
    }

    fn seek(&mut self, pos_16k: u64) -> Result<u64, DecodeError> {
        self.stash = None;
        self.dec.reset_state().map_err(opus_err)?;
        let goal = (pos_16k + self.pre_skip) * 3;
        if goal <= OPUS_PREROLL {
            // Near the start: replay from the first audio packet.
            self.rdr
                .seek_bytes(SeekFrom::Start(0))
                .map_err(DecodeError::from)?;
            for _ in 0..2 {
                self.rdr.read_packet().map_err(ogg_err)?;
            }
            self.pos = 0;
            self.skip = self.pre_skip + pos_16k;
            return Ok(pos_16k);
        }
        let found = self
            .rdr
            .seek_absgp(Some(self.serial), goal - OPUS_PREROLL)
            .map_err(ogg_err)?;
        if !found {
            // Past the end: nothing more to read.
            self.pos = self.end.unwrap_or(pos_16k);
            self.skip = 0;
            self.rdr
                .seek_bytes(SeekFrom::End(0))
                .map_err(DecodeError::from)?;
            return Ok(self.pos);
        }
        // Decode the page we landed on (it carries the granule, which tells
        // where it starts), then cut it down to the target.
        let mut batch: Vec<Vec<f32>> = vec![Vec::new(); self.channels];
        let mut granule = None;
        while let Some(p) = self.rdr.read_packet().map_err(ogg_err)? {
            if p.stream_serial() != self.serial || is_opus_header(&p.data) {
                continue;
            }
            for (b, d) in batch.iter_mut().zip(self.decode(&p.data)?) {
                b.extend_from_slice(&d);
            }
            if p.last_in_page() {
                granule = (p.absgp_page() != u64::MAX).then(|| p.absgp_page());
                break;
            }
        }
        let n = batch[0].len() as u64;
        // Logical index of the batch's first sample (negative: still priming).
        let start = match granule {
            Some(g) => (g / 3) as i64 - n as i64 - self.pre_skip as i64,
            None => pos_16k as i64,
        };
        let drop = (pos_16k as i64 - start).clamp(0, n as i64) as usize;
        for b in &mut batch {
            b.drain(..drop);
        }
        self.pos = (start + drop as i64).max(0) as u64;
        self.skip = 0;
        self.stash = self.trim(batch);
        // `trim` advanced `pos` past the stash; the block starts before it.
        let at = self.pos - self.stash.as_ref().map_or(0, |s| s[0].len() as u64);
        Ok(at)
    }
}

// ---- AVFoundation -------------------------------------------------------------

#[cfg(target_os = "macos")]
mod native {
    use std::ffi::{CString, c_char, c_void};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    use super::{Backend, DecodeError, Info, Opened, SAMPLE_RATE, Source};

    unsafe extern "C" {
        fn ghi_mac_decode_open(
            path: *const c_char,
            channels: *mut u32,
            sample_rate: *mut u32,
            duration_ms: *mut i64,
            codec: *mut c_char,
            codec_cap: u32,
            out_handle: *mut *mut c_void,
        ) -> i32;
        fn ghi_mac_decode_read(
            handle: *mut c_void,
            buf: *mut f32,
            frames_cap: u32,
            out_pos_16k: *mut u64,
        ) -> i64;
        fn ghi_mac_decode_seek(handle: *mut c_void, pos_16k: u64) -> i32;
        fn ghi_mac_decode_close(handle: *mut c_void);
    }

    // GHI_MAC_ERR_* of the decode functions.
    const ERR_NO_AUDIO: i32 = -7;
    const ERR_NOT_FOUND: i32 = -9;

    /// Frames per read (0.5 s at 16 kHz).
    const READ_FRAMES: usize = SAMPLE_RATE as usize / 2;

    /// A read: the timeline index of its first frame and the planar audio.
    type Landed = (u64, Vec<Vec<f32>>);

    struct Mac {
        handle: *mut c_void,
        channels: usize,
        buf: Vec<f32>,
        /// First read after a seek: where it landed.
        stash: Option<Landed>,
    }

    // The handle is used from one thread at a time (`&mut self`).
    unsafe impl Send for Mac {}

    impl Drop for Mac {
        fn drop(&mut self) {
            // SAFETY: `handle` came from ghi_mac_decode_open and is closed once.
            unsafe { ghi_mac_decode_close(self.handle) }
        }
    }

    pub fn open(path: &Path, container: &str) -> Result<Opened, DecodeError> {
        let c_path = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| DecodeError::Corrupt("path contains NUL".into()))?;
        let (mut channels, mut rate, mut duration_ms) = (0u32, 0u32, -1i64);
        let mut codec = [0 as c_char; 16];
        let mut handle: *mut c_void = std::ptr::null_mut();
        // SAFETY: all out-pointers are valid for the sizes passed.
        let status = unsafe {
            ghi_mac_decode_open(
                c_path.as_ptr(),
                &mut channels,
                &mut rate,
                &mut duration_ms,
                codec.as_mut_ptr(),
                codec.len() as u32,
                &mut handle,
            )
        };
        match status {
            0 if !handle.is_null() && channels > 0 => {}
            ERR_NOT_FOUND => return Err(DecodeError::NotFound),
            ERR_NO_AUDIO => {
                return Err(DecodeError::Unsupported {
                    format: format!("{container}: no audio the OS can decode"),
                });
            }
            code => {
                return Err(DecodeError::Corrupt(format!(
                    "avfoundation could not open the file (status {code})"
                )));
            }
        }
        let codec = {
            // SAFETY: the callee NUL-terminates within the buffer.
            let s = unsafe { std::ffi::CStr::from_ptr(codec.as_ptr()) };
            s.to_string_lossy().into_owned()
        };
        let info = Info {
            channels: channels as u16,
            sample_rate: rate,
            duration_ms: (duration_ms >= 0).then_some(duration_ms as u64),
            codec,
            container: container.to_owned(),
            backend: Backend::AvFoundation,
        };
        let mac = Mac {
            handle,
            channels: channels as usize,
            buf: vec![0.0; READ_FRAMES * channels as usize],
            stash: None,
        };
        Ok((info, Box::new(mac)))
    }

    impl Mac {
        fn read(&mut self) -> Result<Option<Landed>, DecodeError> {
            let mut pos = 0u64;
            // SAFETY: `buf` holds channels * READ_FRAMES floats, planar.
            let n = unsafe {
                ghi_mac_decode_read(
                    self.handle,
                    self.buf.as_mut_ptr(),
                    READ_FRAMES as u32,
                    &mut pos,
                )
            };
            if n < 0 {
                return Err(DecodeError::Corrupt(format!(
                    "avfoundation read failed (status {n})"
                )));
            }
            if n == 0 {
                return Ok(None);
            }
            let n = n as usize;
            let planar = (0..self.channels)
                .map(|c| self.buf[c * READ_FRAMES..c * READ_FRAMES + n].to_vec())
                .collect();
            Ok(Some((pos, planar)))
        }
    }

    impl Source for Mac {
        fn pull(&mut self) -> Result<Option<Vec<Vec<f32>>>, DecodeError> {
            if let Some((_, p)) = self.stash.take() {
                return Ok(Some(p));
            }
            Ok(self.read()?.map(|(_, p)| p))
        }

        fn seek(&mut self, pos_16k: u64) -> Result<u64, DecodeError> {
            self.stash = None;
            // SAFETY: valid handle.
            let status = unsafe { ghi_mac_decode_seek(self.handle, pos_16k) };
            if status != 0 {
                return Err(DecodeError::Corrupt(format!(
                    "avfoundation seek failed (status {status})"
                )));
            }
            match self.read()? {
                Some((at, p)) => {
                    self.stash = Some((at, p));
                    Ok(at)
                }
                None => Ok(pos_16k),
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn open_native(path: &Path, container: &str) -> Result<Opened, DecodeError> {
    native::open(path, container)
}

#[cfg(not(target_os = "macos"))]
fn open_native(_path: &Path, container: &str) -> Result<Opened, DecodeError> {
    Err(DecodeError::Unsupported {
        format: format!("{container}: format not supported on this platform yet"),
    })
}
