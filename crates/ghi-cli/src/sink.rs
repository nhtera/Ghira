// SPDX-License-Identifier: Apache-2.0
//! File sinks for `ghi record`: the stand-in for the phase 5 bundle writer.
//!
//! - [`WavSink`]: 16 kHz 16-bit WAV per track plus the call mix, in the eval
//!   kit layout (`<id>.mic.wav`, `<id>.system.wav`, `<id>.wav`).
//! - [`OggPages`]: Ogg Opus per track (`<id>.mic.opus`, ...), fed by
//!   `ghi_audio::encoder::OpusRecorder`, with a barrier per ~1 s page and a
//!   full sync every few pages, so a crash loses at most a few seconds.
//!
//! Both record markers in `<id>.markers.jsonl`; [`Tally`] measures levels on
//! the way through.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use ghi_audio::encoder::PageSink;
use ghi_audio::pipeline::FrameSink;
use ghi_audio::{Marker, MarkerKind, SAMPLE_RATE, Track};

use crate::contract::RecordMarker;

/// Path of a track file: `<dir>/<id>.<track>.<ext>`.
pub fn track_path(dir: &Path, id: &str, track: Track, ext: &str) -> PathBuf {
    dir.join(format!("{id}.{}.{ext}", track.name()))
}

/// Seconds on the 16 kHz timeline.
pub fn seconds(pos: u64) -> f64 {
    pos as f64 / f64::from(SAMPLE_RATE)
}

/// A marker as reported in `ghi.record/1` and `<id>.markers.jsonl`.
pub fn describe_marker(m: &Marker) -> RecordMarker {
    let (kind, detail) = match m.kind {
        MarkerKind::Pause => ("pause", None),
        MarkerKind::Resume => ("resume", None),
        MarkerKind::Discard { last_s } => ("discard", Some(format!("last_s={last_s}"))),
        MarkerKind::Gap { track, to, .. } => (
            "gap",
            Some(format!("track={} to_s={:.3}", track.name(), seconds(to))),
        ),
        MarkerKind::AecOn => ("aec_on", None),
        MarkerKind::AecOff => ("aec_off", None),
        MarkerKind::Sleep => ("sleep", None),
        MarkerKind::Wake { slept_s } => ("wake", Some(format!("slept_s={slept_s:.0}"))),
    };
    RecordMarker {
        t_s: seconds(m.pos),
        kind: kind.to_owned(),
        detail,
    }
}

/// Makes a file's data durable. `full`: `F_FULLFSYNC` (survives power loss);
/// otherwise `F_BARRIERFSYNC` (ordered, cheap) on macOS, `fsync` elsewhere.
pub fn sync_file(file: &File, full: bool) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        let cmd = if full {
            libc::F_FULLFSYNC
        } else {
            libc::F_BARRIERFSYNC
        };
        // SAFETY: valid open descriptor; the command takes no argument.
        if unsafe { libc::fcntl(file.as_raw_fd(), cmd) } == 0 {
            return Ok(());
        }
        // Some file systems (network, FAT) lack these commands.
    }
    let _ = full;
    file.sync_data()
}

/// Makes newly created directory entries durable.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// Appends markers to `<id>.markers.jsonl`, one JSON object per line.
pub struct MarkerLog {
    file: File,
    pub markers: Vec<RecordMarker>,
}

impl MarkerLog {
    pub fn create(dir: &Path, id: &str) -> io::Result<MarkerLog> {
        let file = create_new(&dir.join(format!("{id}.markers.jsonl")))?;
        Ok(MarkerLog {
            file,
            markers: Vec::new(),
        })
    }

    pub fn append(&mut self, m: &Marker) -> io::Result<()> {
        let m = describe_marker(m);
        let line = serde_json::to_string(&m).map_err(io::Error::other)?;
        writeln!(self.file, "{line}")?;
        self.markers.push(m);
        Ok(())
    }

    fn sync(&self) -> io::Result<()> {
        sync_file(&self.file, false)
    }
}

/// Creates a file that must not exist yet: a recording is never overwritten.
pub fn create_new(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

type Wav = hound::WavWriter<BufWriter<File>>;

fn wav(path: &Path) -> io::Result<Wav> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    hound::WavWriter::new(BufWriter::new(create_new(path)?), spec).map_err(io::Error::other)
}

fn write_i16(w: &mut Wav, samples: &[f32]) -> io::Result<()> {
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        w.write_sample(v).map_err(io::Error::other)?;
    }
    Ok(())
}

/// WAV files in the eval kit layout. The mix is `(mic + system) / 2`.
pub struct WavSink {
    tracks: [Option<Wav>; 2],
    mix: Option<Wav>,
    /// Frames waiting for the other track's frame at the same position.
    pending: [VecDeque<(u64, Vec<f32>)>; 2],
    pub log: MarkerLog,
}

impl WavSink {
    pub fn create(dir: &Path, id: &str, tracks: &[Track]) -> io::Result<WavSink> {
        let mut out: [Option<Wav>; 2] = [None, None];
        for &t in tracks {
            out[t.index()] = Some(wav(&track_path(dir, id, t, "wav"))?);
        }
        let mix = if tracks.len() == 2 {
            Some(wav(&dir.join(format!("{id}.wav")))?)
        } else {
            None
        };
        Ok(WavSink {
            tracks: out,
            mix,
            pending: [VecDeque::new(), VecDeque::new()],
            log: MarkerLog::create(dir, id)?,
        })
    }

    /// Writes the WAV headers' final sizes.
    pub fn finish(self) -> io::Result<()> {
        for w in self.tracks.into_iter().flatten().chain(self.mix) {
            w.finalize().map_err(io::Error::other)?;
        }
        self.log.sync()
    }

    fn write_mix(&mut self) -> io::Result<()> {
        let Some(mix) = self.mix.as_mut() else {
            return Ok(());
        };
        let [mic, system] = &mut self.pending;
        while let (Some((pm, _)), Some((ps, _))) = (mic.front(), system.front()) {
            // Both tracks share the timeline; drop a frame the other track
            // never produced (it only happens at the very start or end).
            if pm < ps {
                mic.pop_front();
                continue;
            }
            if ps < pm {
                system.pop_front();
                continue;
            }
            let (_, a) = mic.pop_front().unwrap();
            let (_, b) = system.pop_front().unwrap();
            let mixed: Vec<f32> = a.iter().zip(&b).map(|(x, y)| 0.5 * (x + y)).collect();
            write_i16(mix, &mixed)?;
        }
        Ok(())
    }
}

impl FrameSink for WavSink {
    fn frame(&mut self, track: Track, pos: u64, samples: &[f32]) -> io::Result<()> {
        if let Some(w) = self.tracks[track.index()].as_mut() {
            write_i16(w, samples)?;
        }
        if self.mix.is_some() {
            self.pending[track.index()].push_back((pos, samples.to_vec()));
            self.write_mix()?;
        }
        Ok(())
    }

    fn marker(&mut self, marker: &Marker) -> io::Result<()> {
        self.log.append(marker)
    }
}

/// Ogg Opus page files, one per track.
pub struct OggPages {
    files: [Option<File>; 2],
    pub log: MarkerLog,
}

impl OggPages {
    pub fn create(dir: &Path, id: &str, tracks: &[Track]) -> io::Result<OggPages> {
        let mut files: [Option<File>; 2] = [None, None];
        for &t in tracks {
            files[t.index()] = Some(create_new(&track_path(dir, id, t, "opus"))?);
        }
        let log = MarkerLog::create(dir, id)?;
        sync_dir(dir)?;
        Ok(OggPages { files, log })
    }
}

impl PageSink for OggPages {
    fn write_page(&mut self, track: Track, page: &[u8]) -> io::Result<()> {
        match self.files[track.index()].as_mut() {
            Some(f) => f.write_all(page),
            None => Ok(()),
        }
    }

    fn sync(&mut self, track: Track, durable: bool) -> io::Result<()> {
        if let Some(f) = self.files[track.index()].as_ref() {
            sync_file(f, durable)?;
        }
        if durable {
            self.log.sync()?;
        }
        Ok(())
    }

    fn marker(&mut self, marker: &Marker) -> io::Result<()> {
        self.log.append(marker)
    }
}

/// Level statistics of one track.
#[derive(Debug, Clone, Copy, Default)]
pub struct Level {
    pub samples: u64,
    pub sum_sq: f64,
    pub peak: f32,
}

impl Level {
    pub fn rms_dbfs(&self) -> Option<f64> {
        if self.samples == 0 {
            return None;
        }
        let rms = (self.sum_sq / self.samples as f64).sqrt();
        Some(
            if rms > 0.0 {
                20.0 * rms.log10()
            } else {
                f64::NEG_INFINITY
            }
            .max(-120.0),
        )
    }
}

/// Passes frames through to `inner`, measuring each track's level.
pub struct Tally<S> {
    pub inner: S,
    pub levels: [Level; 2],
}

impl<S> Tally<S> {
    pub fn new(inner: S) -> Tally<S> {
        Tally {
            inner,
            levels: [Level::default(); 2],
        }
    }
}

impl<S: FrameSink> FrameSink for Tally<S> {
    fn frame(&mut self, track: Track, pos: u64, samples: &[f32]) -> io::Result<()> {
        let l = &mut self.levels[track.index()];
        l.samples += samples.len() as u64;
        for &s in samples {
            l.sum_sq += f64::from(s) * f64::from(s);
            l.peak = l.peak.max(s.abs());
        }
        self.inner.frame(track, pos, samples)
    }

    fn marker(&mut self, marker: &Marker) -> io::Result<()> {
        self.inner.marker(marker)
    }
}

/// Writes one 16 kHz mono track as a 16-bit WAV file.
/// Replaces `path` (a derived file, e.g. a recovered track).
pub fn write_wav(path: &Path, samples: &[f32]) -> io::Result<()> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let mut w = wav(path)?;
    write_i16(&mut w, samples)?;
    w.finalize().map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_sink_writes_tracks_and_mix() {
        let dir = std::env::temp_dir().join(format!("ghi-sink-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut sink = WavSink::create(&dir, "t", &Track::ALL).unwrap();
        let a = [0.5f32; 160];
        let b = [-0.25f32; 160];
        for pos in 0..10u64 {
            sink.frame(Track::Mic, pos * 160, &a).unwrap();
            sink.frame(Track::System, pos * 160, &b).unwrap();
        }
        sink.marker(&Marker {
            pos: 800,
            kind: MarkerKind::Pause,
        })
        .unwrap();
        sink.finish().unwrap();
        let read = |name: &str| {
            let mut r = hound::WavReader::open(dir.join(name)).unwrap();
            r.samples::<i16>().map(|s| s.unwrap()).collect::<Vec<_>>()
        };
        assert_eq!(read("t.mic.wav").len(), 1600);
        let mix = read("t.wav");
        assert_eq!(mix.len(), 1600);
        assert!((i32::from(mix[0]) - 4096).abs() <= 1, "{}", mix[0]);
        let markers = std::fs::read_to_string(dir.join("t.markers.jsonl")).unwrap();
        assert!(markers.contains("\"pause\"") && markers.contains("0.05"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn levels() {
        let mut l = Level::default();
        assert_eq!(l.rms_dbfs(), None);
        l.samples = 4;
        l.sum_sq = 4.0 * 0.25;
        assert!((l.rms_dbfs().unwrap() + 6.02).abs() < 0.01);
    }
}
