// SPDX-License-Identifier: Apache-2.0
//! Import decoders against fixtures generated at test time (no audio files in
//! git). WAV is written here, Ogg Opus by ghi-audio's encoder, the rest by
//! `afconvert` (macOS) or `ffmpeg` when present; a test whose tool is missing
//! prints a note and passes.

use std::f32::consts::TAU;
use std::path::{Path, PathBuf};
use std::process::Command;

use ghi_audio::Track;
use ghi_audio::decode::{Backend, Block, DecodeError, Decoder, probe};
use ghi_audio::encoder::{EncoderConfig, TrackEncoder};

const SECS: f32 = 8.0;
const LEFT_HZ: f32 = 440.0;
const RIGHT_HZ: f32 = 1000.0;
/// "A few seconds": blocks must stay well under this.
const MAX_BLOCK: usize = 5 * 16_000;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("ghi-decode-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }

    fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tone(hz: f32, rate: u32, secs: f32) -> Vec<f32> {
    (0..(rate as f32 * secs) as usize)
        .map(|i| 0.5 * (TAU * hz * i as f32 / rate as f32).sin())
        .collect()
}

/// 300 Hz to 3 kHz over `secs`: every position has its own pitch.
fn chirp(rate: u32, secs: f32) -> Vec<f32> {
    let k = (3000.0 - 300.0) / secs;
    (0..(rate as f32 * secs) as usize)
        .map(|i| {
            let t = i as f32 / rate as f32;
            0.5 * (TAU * (300.0 * t + 0.5 * k * t * t)).sin()
        })
        .collect()
}

fn write_wav(path: &Path, rate: u32, channels: &[Vec<f32>]) {
    let spec = hound::WavSpec {
        channels: channels.len() as u16,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..channels[0].len() {
        for c in channels {
            w.write_sample((c[i] * 32767.0) as i16).unwrap();
        }
    }
    w.finalize().unwrap();
}

fn stereo_wav(path: &Path, rate: u32) {
    write_wav(
        path,
        rate,
        &[tone(LEFT_HZ, rate, SECS), tone(RIGHT_HZ, rate, SECS)],
    );
}

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg(if tool == "ffmpeg" { "-version" } else { "-h" })
        .output()
        .is_ok()
}

/// Runs a converter; false (with a note) when it is missing or fails.
fn convert(tool: &str, args: &[&str]) -> bool {
    if !have(tool) {
        eprintln!("note: {tool} not available, skipping");
        return false;
    }
    match Command::new(tool).args(args).output() {
        Ok(o) if o.status.success() => true,
        Ok(o) => {
            eprintln!(
                "note: {tool} {args:?} failed, skipping: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            false
        }
        Err(_) => false,
    }
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn read_all(dec: &mut Decoder) -> Vec<Block> {
    let mut blocks = Vec::new();
    while let Some(b) = dec.next_block().unwrap() {
        blocks.push(b);
    }
    blocks
}

/// Concatenates blocks per channel, asserting contiguity and the size bound.
fn join(blocks: &[Block], channels: usize) -> Vec<Vec<f32>> {
    let mut out = vec![Vec::new(); channels];
    let mut next = blocks.first().map_or(0, |b| b.pos_16k);
    for b in blocks {
        assert_eq!(b.pos_16k, next, "blocks must be contiguous");
        assert_eq!(b.channels.len(), channels);
        assert!(b.frames() <= MAX_BLOCK, "block of {} frames", b.frames());
        for (o, c) in out.iter_mut().zip(&b.channels) {
            assert_eq!(c.len(), b.frames());
            o.extend_from_slice(c);
        }
        next += b.frames() as u64;
    }
    out
}

/// Dominant frequency from zero crossings of the middle of the signal.
fn freq_hz(x: &[f32]) -> f32 {
    let mid = &x[x.len() / 4..x.len() * 3 / 4];
    let crossings = mid.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
    crossings as f32 / (mid.len() as f32 / 16_000.0)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

/// Decodes a stereo (LEFT_HZ / RIGHT_HZ) file and checks everything but the backend.
fn check_stereo(path: &Path, tol: f32) -> Decoder {
    let mut dec = Decoder::open(path).unwrap();
    assert_eq!(dec.info().channels, 2, "{:?}", dec.info());
    let blocks = read_all(&mut dec);
    let ch = join(&blocks, 2);
    let want = SECS * 16_000.0;
    let got = ch[0].len() as f32;
    assert!(
        (got - want).abs() / want <= tol,
        "{:?}: {got} frames, want {want}",
        dec.info()
    );
    let (l, r) = (freq_hz(&ch[0]), freq_hz(&ch[1]));
    assert!((l - LEFT_HZ).abs() < 15.0, "left is {l} Hz, want {LEFT_HZ}");
    assert!(
        (r - RIGHT_HZ).abs() < 30.0,
        "right is {r} Hz, want {RIGHT_HZ}"
    );
    assert!(rms(&ch[0]) > 0.2 && rms(&ch[1]) > 0.2);
    dec
}

/// Seeks to `secs` and checks the resume position and the audio against a
/// full decode (`tol`: allowed mean absolute difference after warm-up).
fn check_seek(path: &Path, secs: f32, max_early: u64, tol: f32) {
    let mut full = Decoder::open(path).unwrap();
    let reference = join(&read_all(&mut full), full.info().channels as usize);

    let mut dec = Decoder::open(path).unwrap();
    let target = (secs * 16_000.0) as u64;
    dec.seek(target).unwrap();
    let b = dec.next_block().unwrap().expect("audio after seek");
    assert!(
        b.pos_16k <= target && target - b.pos_16k <= max_early,
        "resumed at {} for target {target}",
        b.pos_16k
    );
    let warm = 4000;
    let from = b.pos_16k as usize + warm;
    let n = (b.frames() - warm).min(8000);
    let diff: f32 = (0..n)
        .map(|i| (b.channels[0][warm + i] - reference[0][from + i]).abs())
        .sum::<f32>()
        / n as f32;
    assert!(diff < tol, "audio after seek differs by {diff}");
    // And it keeps going to the end with the right total length.
    let mut total = b.pos_16k + b.frames() as u64;
    while let Some(b) = dec.next_block().unwrap() {
        assert_eq!(b.pos_16k, total);
        total += b.frames() as u64;
    }
    let diff = total.abs_diff(reference[0].len() as u64);
    assert!(
        diff <= 16,
        "ends at {total}, full decode has {}",
        reference[0].len()
    );
}

#[test]
fn wav_stereo_44k1() {
    let d = Dir::new("wav");
    let p = d.path("a.wav");
    stereo_wav(&p, 44_100);
    let info = probe(&p).unwrap();
    assert_eq!(
        (
            info.channels,
            info.sample_rate,
            info.backend,
            info.container.as_str()
        ),
        (2, 44_100, Backend::Symphonia, "wav")
    );
    assert_eq!(info.duration_ms, Some(8000));
    assert_eq!(info.codec, "pcm_s16le");
    let dec = check_stereo(&p, 0.001);
    assert_eq!(dec.info().backend, Backend::Symphonia);
}

#[test]
fn wav_native_16k_and_48k_and_mono() {
    let d = Dir::new("rates");
    for rate in [16_000, 48_000, 8_000] {
        let p = d.path(&format!("{rate}.wav"));
        write_wav(&p, rate, &[tone(LEFT_HZ, rate, SECS)]);
        let mut dec = Decoder::open(&p).unwrap();
        assert_eq!(dec.info().channels, 1);
        let ch = join(&read_all(&mut dec), 1);
        assert!(
            (ch[0].len() as i64 - 128_000).abs() <= 2,
            "{rate}: {}",
            ch[0].len()
        );
        assert!((freq_hz(&ch[0]) - LEFT_HZ).abs() < 10.0);
    }
}

#[test]
fn wav_three_channels_keep_all() {
    let d = Dir::new("wav3");
    let p = d.path("a.wav");
    let rate = 32_000;
    write_wav(
        &p,
        rate,
        &[
            tone(300.0, rate, 3.0),
            tone(600.0, rate, 3.0),
            tone(900.0, rate, 3.0),
        ],
    );
    let mut dec = Decoder::open(&p).unwrap();
    assert_eq!(dec.info().channels, 3);
    let ch = join(&read_all(&mut dec), 3);
    for (c, hz) in [300.0, 600.0, 900.0].into_iter().enumerate() {
        assert!((freq_hz(&ch[c]) - hz).abs() < 10.0, "channel {c}");
    }
}

#[test]
fn wav_seek_is_exact() {
    let d = Dir::new("wavseek");
    let p = d.path("a.wav");
    write_wav(&p, 44_100, &[chirp(44_100, SECS)]);
    check_seek(&p, 3.0, 0, 0.01);
    check_seek(&p, 0.0, 0, 0.01);
    // Past the end: nothing to read, no error.
    let mut dec = Decoder::open(&p).unwrap();
    dec.seek(20 * 16_000).unwrap();
    assert!(dec.next_block().unwrap().is_none());
    // Seeking back works after reaching the end.
    dec.seek(16_000).unwrap();
    assert!(dec.next_block().unwrap().is_some());
}

#[test]
fn blocks_are_bounded_for_a_long_file() {
    let d = Dir::new("long");
    let p = d.path("long.wav");
    // 5 minutes of mono 8 kHz (4.8 MB).
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 8_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&p, spec).unwrap();
    for i in 0..8_000 * 300 {
        w.write_sample(((TAU * 300.0 * i as f32 / 8_000.0).sin() * 8000.0) as i16)
            .unwrap();
    }
    w.finalize().unwrap();
    let mut dec = Decoder::open(&p).unwrap();
    let (mut frames, mut max, mut next) = (0u64, 0usize, 0u64);
    while let Some(b) = dec.next_block().unwrap() {
        assert_eq!(b.pos_16k, next);
        next += b.frames() as u64;
        frames += b.frames() as u64;
        max = max.max(b.frames());
    }
    assert!(max <= MAX_BLOCK, "largest block {max} frames");
    assert!((frames as i64 - 300 * 16_000).abs() <= 2);
}

#[test]
fn ogg_opus_from_the_encoder() {
    let d = Dir::new("opus");
    let p = d.path("rec.opus");
    let x = chirp(16_000, SECS);
    let mut enc = TrackEncoder::new(Track::Mic, 7, &EncoderConfig::default()).unwrap();
    let mut bytes = std::mem::take(&mut enc.1);
    let mut emit = |page: &[u8]| {
        bytes.extend_from_slice(page);
        Ok(())
    };
    enc.0.push(&x, &mut emit).unwrap();
    enc.0.finish(&mut emit).unwrap();
    std::fs::write(&p, &bytes).unwrap();

    let info = probe(&p).unwrap();
    assert_eq!((info.channels, info.backend), (1, Backend::OggOpus));
    assert_eq!(info.codec, "opus");
    assert_eq!(info.duration_ms, Some(8000));
    let mut dec = Decoder::open(&p).unwrap();
    let ch = join(&read_all(&mut dec), 1);
    assert_eq!(ch[0].len(), x.len(), "priming and padding are trimmed");
    // Same pitch trajectory: correlate with the source (Opus is lossy but close).
    let corr: f32 = ch[0].iter().zip(&x).map(|(a, b)| a * b).sum::<f32>()
        / (rms(&ch[0]) * rms(&x) * x.len() as f32);
    assert!(corr > 0.9, "correlation {corr}");

    // Seeks land on a page boundary at or before the target and report it.
    check_seek(&p, 3.0, 16_000, 0.05);
    check_seek(&p, 0.2, 16_000, 0.05);
}

#[test]
fn missing_and_garbage_files() {
    let d = Dir::new("bad");
    assert!(matches!(
        Decoder::open(&d.path("nope.wav")),
        Err(DecodeError::NotFound)
    ));
    assert!(matches!(
        probe(&d.path("nope.m4a")),
        Err(DecodeError::NotFound)
    ));

    let junk = d.path("junk.wav");
    std::fs::write(
        &junk,
        (0..4096u32)
            .map(|i| (i * 31 % 251) as u8)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(
        matches!(
            Decoder::open(&junk),
            Err(DecodeError::Unsupported { .. } | DecodeError::Corrupt(_))
        ),
        "garbage is rejected"
    );

    let empty = d.path("empty.mp3");
    std::fs::write(&empty, b"").unwrap();
    assert!(matches!(
        Decoder::open(&empty),
        Err(DecodeError::Unsupported { .. } | DecodeError::Corrupt(_))
    ));

    // A WAV header promising audio, then nothing.
    let hollow = d.path("hollow.wav");
    stereo_wav(&hollow, 16_000);
    let bytes = std::fs::read(&hollow).unwrap();
    std::fs::write(&hollow, &bytes[..40]).unwrap();
    match Decoder::open(&hollow) {
        Err(DecodeError::Unsupported { .. } | DecodeError::Corrupt(_)) => {}
        Ok(mut dec) => {
            let _ = dec.next_block();
        }
        Err(e) => panic!("unexpected {e}"),
    }

    // A truncated Opus file (cut mid-page) plays up to the cut.
    let p = d.path("cut.opus");
    let mut enc = TrackEncoder::new(Track::Mic, 1, &EncoderConfig::default()).unwrap();
    let mut bytes = std::mem::take(&mut enc.1);
    let mut emit = |page: &[u8]| {
        bytes.extend_from_slice(page);
        Ok(())
    };
    enc.0.push(&tone(300.0, 16_000, 4.0), &mut emit).unwrap();
    enc.0.finish(&mut emit).unwrap();
    std::fs::write(&p, &bytes[..bytes.len() * 2 / 3]).unwrap();
    let mut dec = Decoder::open(&p).unwrap();
    let ch = join(&read_all(&mut dec), 1);
    assert!(
        ch[0].len() > 16_000 && ch[0].len() < 64_000,
        "{}",
        ch[0].len()
    );
}

#[test]
fn truncated_wav_plays_what_is_there() {
    let d = Dir::new("trunc");
    let p = d.path("t.wav");
    stereo_wav(&p, 44_100);
    let bytes = std::fs::read(&p).unwrap();
    std::fs::write(&p, &bytes[..bytes.len() / 2]).unwrap();
    let mut dec = Decoder::open(&p).unwrap();
    let ch = join(&read_all(&mut dec), 2);
    let secs = ch[0].len() as f32 / 16_000.0;
    assert!((3.5..4.5).contains(&secs), "{secs} s");
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;

    fn afconvert(wav: &Path, out: &Path, fmt: &[&str]) -> bool {
        let mut args: Vec<&str> = fmt.to_vec();
        args.extend([s(wav), s(out)]);
        convert("afconvert", &args)
    }

    #[test]
    fn m4a_aac_by_symphonia() {
        let d = Dir::new("m4a");
        let (w, p) = (d.path("a.wav"), d.path("a.m4a"));
        stereo_wav(&w, 44_100);
        if !afconvert(&w, &p, &["-f", "m4af", "-d", "aac", "-b", "128000"]) {
            return;
        }
        let info = probe(&p).unwrap();
        assert_eq!(info.backend, Backend::Symphonia, "{info:?}");
        assert_eq!(info.codec, "aac");
        assert_eq!(info.container, "mp4");
        check_stereo(&p, 0.01);
        check_seek(&p, 3.0, 16_000, 0.05);
    }

    #[test]
    fn alac_in_m4a() {
        let d = Dir::new("alac");
        let (w, p) = (d.path("a.wav"), d.path("a.m4a"));
        stereo_wav(&w, 44_100);
        if !afconvert(&w, &p, &["-f", "m4af", "-d", "alac"]) {
            return;
        }
        let dec = check_stereo(&p, 0.001);
        assert_eq!(dec.info().backend, Backend::Symphonia);
        assert_eq!(dec.info().codec, "alac");
    }

    #[test]
    fn flac_and_aiff() {
        let d = Dir::new("flac");
        let w = d.path("a.wav");
        stereo_wav(&w, 44_100);
        let f = d.path("a.flac");
        if afconvert(&w, &f, &["-f", "flac", "-d", "flac"]) {
            let dec = check_stereo(&f, 0.001);
            assert_eq!(dec.info().backend, Backend::Symphonia);
            check_seek(&f, 3.0, 0, 0.01);
        }
        let a = d.path("a.aiff");
        if afconvert(&w, &a, &["-f", "AIFF", "-d", "BEI16"]) {
            check_stereo(&a, 0.001);
        }
    }

    #[test]
    fn caf_goes_to_avfoundation() {
        let d = Dir::new("caf");
        let (w, p) = (d.path("a.wav"), d.path("a.caf"));
        stereo_wav(&w, 44_100);
        if !afconvert(&w, &p, &["-f", "caff", "-d", "LEI16"]) {
            return;
        }
        let info = probe(&p).unwrap();
        assert_eq!(info.backend, Backend::AvFoundation, "{info:?}");
        assert_eq!((info.channels, info.sample_rate), (2, 44_100));
        assert_eq!(info.container, "caf");
        assert!(info.duration_ms.is_some_and(|d| d.abs_diff(8000) < 50));
        check_stereo(&p, 0.001);
        check_seek(&p, 3.0, 16_000, 0.02);
        check_seek(&p, 0.0, 16_000, 0.02);
    }

    #[test]
    fn avfoundation_can_be_forced() {
        let d = Dir::new("force");
        let p = d.path("a.wav");
        stereo_wav(&p, 48_000);
        let mut dec = Decoder::open_with(&p, Some(Backend::AvFoundation)).unwrap();
        assert_eq!(dec.info().backend, Backend::AvFoundation);
        let ch = join(&read_all(&mut dec), 2);
        assert!((ch[0].len() as i64 - 128_000).abs() <= 160);
        assert!((freq_hz(&ch[0]) - LEFT_HZ).abs() < 15.0);
        assert!((freq_hz(&ch[1]) - RIGHT_HZ).abs() < 30.0);
        // AVFoundation also reads what Symphonia does not: ALAC in CAF is
        // covered above; here a corrupt file stays an error.
        let junk = d.path("junk.caf");
        std::fs::write(&junk, vec![7u8; 2048]).unwrap();
        assert!(Decoder::open_with(&junk, Some(Backend::AvFoundation)).is_err());
    }

    #[test]
    fn video_with_audio_track() {
        let d = Dir::new("video");
        let (w, p) = (d.path("a.wav"), d.path("v.mp4"));
        stereo_wav(&w, 44_100);
        if !convert(
            "ffmpeg",
            &[
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=64x48:rate=5",
                "-i",
                s(&w),
                "-c:v",
                "mpeg4",
                "-c:a",
                "aac",
                "-b:a",
                "128k",
                "-shortest",
                s(&p),
            ],
        ) {
            return;
        }
        let dec = check_stereo(&p, 0.02);
        assert_eq!(dec.info().backend, Backend::Symphonia);
        // The same file through the OS decoder.
        let mut av = Decoder::open_with(&p, Some(Backend::AvFoundation)).unwrap();
        let ch = join(&read_all(&mut av), 2);
        assert!((ch[0].len() as f32 / 16_000.0 - SECS).abs() < 0.1);
        assert!((freq_hz(&ch[0]) - LEFT_HZ).abs() < 15.0);
        assert!((freq_hz(&ch[1]) - RIGHT_HZ).abs() < 30.0);
    }

    #[test]
    fn mp3_and_vorbis_when_ffmpeg_is_present() {
        let d = Dir::new("ffmpeg");
        let w = d.path("a.wav");
        stereo_wav(&w, 44_100);
        let mp3 = d.path("a.mp3");
        if convert(
            "ffmpeg",
            &[
                "-y",
                "-loglevel",
                "error",
                "-i",
                s(&w),
                "-b:a",
                "128k",
                s(&mp3),
            ],
        ) {
            let dec = check_stereo(&mp3, 0.02);
            assert_eq!(dec.info().backend, Backend::Symphonia);
            assert_eq!(dec.info().container, "mp3");
        }
        let ogg = d.path("a.ogg");
        let vorbis = |enc: &[&str]| {
            let mut args = vec!["-y", "-loglevel", "error", "-i", s(&w), "-c:a"];
            args.extend_from_slice(enc);
            args.push(s(&ogg));
            convert("ffmpeg", &args)
        };
        if vorbis(&["libvorbis"]) || vorbis(&["vorbis", "-strict", "-2"]) {
            let dec = check_stereo(&ogg, 0.01);
            assert_eq!(dec.info().backend, Backend::Symphonia);
            assert_eq!(dec.info().codec, "vorbis");
        }
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn unknown_formats_are_unsupported_off_macos() {
    let d = Dir::new("nomac");
    let p = d.path("a.caf");
    std::fs::write(&p, b"caff\0\x01\0\0 not really").unwrap();
    match Decoder::open(&p) {
        Err(DecodeError::Unsupported { format }) => {
            assert!(
                format.contains("not supported on this platform"),
                "{format}"
            )
        }
        other => panic!("{:?}", other.map(|d| d.info().clone())),
    }
}
