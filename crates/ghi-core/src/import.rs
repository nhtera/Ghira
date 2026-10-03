// SPDX-License-Identifier: Apache-2.0
//! File import (doc 02 §H): a recording becomes a meeting like a live one.
//!
//! The file is hashed (SHA-256, duplicate detection), decoded to 16 kHz
//! (`ghi-audio::decode`: Symphonia, AVFoundation on macOS, Ogg Opus) and
//! encoded into the meeting's encrypted bundles as it streams (a 3-hour file
//! is never held in memory); then the final pass transcribes it and the
//! notes follow, through the job runner.
//!
//! - Mixed to mono (one room track) by default; `split_channels` keeps the
//!   first two channels as two tracks (a Zoom/Teams stereo recording: you on
//!   the first, the others on the second) and treats it as a call.
//! - Decoding is not resumable: the meeting stays `importing` until done; one
//!   left `importing` by a crash is deleted at the next start
//!   ([`crate::recover`]), freeing its hash so the file can be imported again.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ghi_audio::decode::Decoder;
use ghi_audio::encoder::{EncoderConfig, OpusRecorder};
use ghi_audio::pipeline::FrameSink;
use ghi_audio::{FRAME_SAMPLES, SAMPLE_RATE, Track};
use ghi_store::organize::TrackSpeaker;
use ghi_store::store::{NewMeeting, NewSpeaker, Store, TrackKind};
use sha2::{Digest, Sha256};

use crate::activity::Meter;
use crate::events::{Event, EventTx, Stage};
use crate::pages::BundlePages;
use crate::presets;
use crate::session::{FINAL_PASS_JOB, JOB_PAYLOAD_VERSION};
use crate::speakers::COLOR_ORDER;

/// Status while an import is being decoded.
pub const IMPORTING: &str = "importing";

#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    pub title: Option<String>,
    pub language: Option<String>,
    pub split_channels: bool,
    /// When the recording was made (unix ms); default: the file's
    /// modification time.
    pub started_at: Option<i64>,
    /// Set to stop the import (the half-made meeting is removed).
    pub cancel: Option<Arc<AtomicBool>>,
    /// Called with the new meeting and 0..1 as decoding advances.
    pub on_progress: Option<OnProgress>,
    /// The file's SHA-256 when already known (staging hashed it).
    pub source_hash: Option<String>,
    /// While this says so, decoding waits (a recording runs).
    pub hold: Option<Hold>,
}

/// See [`ImportOptions::hold`].
#[derive(Clone)]
pub struct Hold(pub Arc<dyn Fn() -> bool + Send + Sync>);

impl std::fmt::Debug for Hold {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Hold")
    }
}

/// Gets the meeting being made and 0..1.
pub type ProgressFn = dyn Fn(&str, f32) + Send + Sync;

/// A progress callback for [`ImportOptions::on_progress`].
#[derive(Clone)]
pub struct OnProgress(pub Arc<ProgressFn>);

impl std::fmt::Debug for OnProgress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OnProgress")
    }
}

/// The error of an import stopped through [`ImportOptions::cancel`].
pub const CANCELLED: &str = "import cancelled";

/// Waits while a recording runs ([`ImportOptions::hold`]); stops early with
/// [`CANCELLED`] if the import is cancelled meanwhile.
fn wait_hold(opts: &ImportOptions) -> Result<(), String> {
    let Some(hold) = &opts.hold else {
        return Ok(());
    };
    while (hold.0)() {
        if opts
            .cancel
            .as_ref()
            .is_some_and(|c| c.load(Ordering::Relaxed))
        {
            return Err(CANCELLED.into());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(())
}

/// Sums track samples without the harsh clip of a hard clamp: linear up to
/// 0.9, then a smooth approach to 1.
fn soft_limit(x: f32) -> f32 {
    const KNEE: f32 = 0.9;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        x.signum() * (KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportReport {
    pub meeting: String,
    /// The same file was imported before: `meeting` is that one.
    pub duplicate: bool,
    pub duration_ms: i64,
    pub channels: u16,
    pub tracks: usize,
    /// Jobs queued (the final pass; notes follow it).
    pub jobs: Vec<i64>,
}

/// A file's modification time (unix ms).
fn modified_ms(path: &Path) -> Option<i64> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    let d = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(d.as_millis()).ok()
}

/// SHA-256 (hex) of a file, streamed.
pub fn file_sha256(path: &Path) -> std::io::Result<String> {
    file_sha256_until(path, None)
}

/// [`file_sha256`] that stops with `ErrorKind::Interrupted` once `cancel` is
/// set (checked every megabyte).
pub fn file_sha256_until(path: &Path, cancel: Option<&AtomicBool>) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Most participant tracks of one import; a bigger set is refused
/// (`tooManyTracks`) before anything is decoded.
pub const MAX_TRACKS: usize = 49;
/// Consecutive speech spans of one track start at least this far apart (ms):
/// the shortest span plus the gap that would have merged it with the next.
const MIN_SPAN_PERIOD_MS: i64 = 650;
/// Longest a participant's name is kept.
const MAX_NAME_CHARS: usize = 100;
/// Samples a track's decoder is read ahead (1 s).
const READ_AHEAD: usize = SAMPLE_RATE as usize;

/// The source hash of a multi-track import: SHA-256 over the sorted per-file
/// hashes, so the same set of files is a duplicate in any order.
pub fn tracks_hash(paths: &[PathBuf], cancel: Option<&AtomicBool>) -> std::io::Result<String> {
    let mut each: Vec<String> = paths
        .iter()
        .map(|p| file_sha256_until(p, cancel))
        .collect::<std::io::Result<_>>()?;
    each.sort();
    let mut h = Sha256::new();
    for x in &each {
        h.update(x.as_bytes());
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// An earlier import of the same source, to report instead of importing again
/// (one left `importing` by a failure is deleted so it can start over).
fn earlier(store: &Store, hash: &str) -> Result<Option<ImportReport>, String> {
    let err = |e: ghi_store::StoreError| e.to_string();
    let Some(m) = store.meeting_by_source_hash(hash).map_err(err)? else {
        return Ok(None);
    };
    let meeting = store.get_meeting(&m).map_err(err)?;
    if meeting.status == IMPORTING {
        store.delete_meeting(&m).map_err(err)?;
        return Ok(None);
    }
    Ok(Some(ImportReport {
        meeting: m,
        duplicate: true,
        duration_ms: meeting.duration_ms,
        channels: 0,
        tracks: 0,
        jobs: Vec::new(),
    }))
}

/// One participant's decoder while the tracks are mixed in lockstep.
struct Lane {
    dec: Decoder,
    /// Decoded, not yet mixed (mono, 16 kHz).
    pending: Vec<f32>,
    done: bool,
    meter: Meter,
}

impl Lane {
    /// Reads ahead until a second is waiting or the file ends.
    fn refill(&mut self) -> Result<(), String> {
        while !self.done && self.pending.len() < READ_AHEAD {
            match self.dec.next_block().map_err(|e| e.to_string())? {
                Some(block) => {
                    let n = block.frames();
                    let ch = block.channels.len().max(1) as f32;
                    let from = self.pending.len();
                    self.pending.extend(
                        (0..n).map(|k| block.channels.iter().map(|c| c[k]).sum::<f32>() / ch),
                    );
                    self.meter.push(&self.pending[from..]);
                }
                None => self.done = true,
            }
        }
        Ok(())
    }
}

/// Imports one meeting from several participants' tracks (Zoom "separate
/// audio file for each participant", phase 14d D9): `files` are `(path,
/// participant name)` pairs. The tracks are mixed (summed, soft-limited; shorter
/// ones padded with silence) into one stored `file` track; each participant's
/// speech spans become a named speaker and are stored (`track_speakers`) for
/// the final pass, which then skips the diarizer and attributes words by
/// those spans. A participant without a name is an unnamed speaker.
pub fn import_tracks(
    store: &Store,
    files: &[(PathBuf, Option<String>)],
    opts: &ImportOptions,
    events: &EventTx,
) -> Result<ImportReport, String> {
    let err = |e: ghi_store::StoreError| e.to_string();
    if files.is_empty() {
        return Err("noTracks".into());
    }
    if files.len() > MAX_TRACKS {
        return Err("tooManyTracks".into());
    }
    // Everything that can be refused is refused before a meeting exists, with
    // a code that names no path: a missing file, one that cannot be read, a
    // recording too long to keep its participants' spans.
    if files.iter().any(|(p, _)| !p.is_file()) {
        return Err("trackMissing".into());
    }
    let paths: Vec<PathBuf> = files.iter().map(|(p, _)| p.clone()).collect();
    let hash = match &opts.source_hash {
        Some(h) => h.clone(),
        None => tracks_hash(&paths, opts.cancel.as_deref()).map_err(|e| match e.kind() {
            std::io::ErrorKind::Interrupted => CANCELLED.to_string(),
            std::io::ErrorKind::NotFound => "trackMissing".to_string(),
            _ => "trackUnreadable".to_string(),
        })?,
    };
    if let Some(dup) = earlier(store, &hash)? {
        return Ok(dup);
    }
    let mut lanes = Vec::with_capacity(files.len());
    let mut longest_ms = 0u64;
    for (path, _) in files {
        let dec = Decoder::open(path).map_err(|_| "trackUnreadable".to_string())?;
        longest_ms = longest_ms.max(dec.info().duration_ms.unwrap_or(0));
        lanes.push(Lane {
            dec,
            pending: Vec::new(),
            done: false,
            meter: Meter::default(),
        });
    }
    if ghi_store::organize::track_speakers_bound(files.len(), longest_ms as i64, MIN_SPAN_PERIOD_MS)
        > ghi_store::organize::MAX_TRACK_SPEAKERS_BYTES
    {
        return Err("tooLong".into());
    }
    let first = &files[0].0;
    let tags = ghi_audio::decode::tags(first);
    let found = presets::title_date(first, tags.title.as_deref(), tags.date_ms);
    let meeting = store
        .create_meeting(NewMeeting {
            title: opts
                .title
                .clone()
                .or(found.title)
                .unwrap_or_else(|| "Imported recording".into()),
            started_at: opts
                .started_at
                .or(found.started_at_ms)
                .or_else(|| modified_ms(first))
                .unwrap_or(0),
            source: "file".into(),
            mode: "room".into(),
            lang: opts.language.clone(),
            ..Default::default()
        })
        .map_err(err)?
        .gid;
    // Reported per whole percent: the loop below runs many times a second,
    // and every event makes the UI re-read the meeting list.
    let reported = std::cell::Cell::new(-1);
    let progress = |p: f32| {
        let pct = (p.clamp(0.0, 1.0) * 100.0) as i32;
        if reported.replace(pct) == pct {
            return;
        }
        if let Some(f) = &opts.on_progress {
            (f.0)(&meeting, p);
        }
        events.emit(Event::JobProgress {
            meeting: Some(meeting.clone()),
            job: 0,
            kind: "import".into(),
            stage: Some(Stage::Decoding),
            progress: p,
        })
    };
    let total = longest_ms as f64 * f64::from(SAMPLE_RATE) / 1000.0;
    // Any failure from here removes the half-made meeting (and frees the hash).
    let encoded = (|| -> Result<(u64, i64), String> {
        store.set_meeting_status(&meeting, IMPORTING).map_err(err)?;
        store.set_source_hash(&meeting, &hash).map_err(err)?;
        let writer = store.open_track(&meeting, TrackKind::File).map_err(err)?;
        let mut rec = OpusRecorder::new(
            BundlePages::new([Some(writer), None]),
            &[Track::Mic],
            EncoderConfig::default(),
        )
        .map_err(|e| e.to_string())?;
        let mut pos: u64 = 0;
        loop {
            if opts
                .cancel
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Relaxed))
            {
                return Err(CANCELLED.into());
            }
            wait_hold(opts)?;
            for l in &mut lanes {
                l.refill()?;
            }
            // Whole frames every live track has; once all ended, the rest.
            let alive_min = lanes
                .iter()
                .filter(|l| !l.done)
                .map(|l| l.pending.len())
                .min();
            let frames = match alive_min {
                Some(n) => n / FRAME_SAMPLES,
                None => lanes
                    .iter()
                    .map(|l| l.pending.len())
                    .max()
                    .unwrap_or(0)
                    .div_ceil(FRAME_SAMPLES),
            };
            if frames == 0 {
                break;
            }
            let mut frame = vec![0.0f32; FRAME_SAMPLES];
            for k in 0..frames {
                frame.fill(0.0);
                for l in &lanes {
                    let a = k * FRAME_SAMPLES;
                    if a < l.pending.len() {
                        let b = ((k + 1) * FRAME_SAMPLES).min(l.pending.len());
                        for (o, x) in frame.iter_mut().zip(&l.pending[a..b]) {
                            *o += x;
                        }
                    }
                }
                frame.iter_mut().for_each(|x| *x = soft_limit(*x));
                rec.frame(Track::Mic, pos, &frame)
                    .map_err(|e| e.to_string())?;
                pos += FRAME_SAMPLES as u64;
            }
            for l in &mut lanes {
                let n = (frames * FRAME_SAMPLES).min(l.pending.len());
                l.pending.drain(..n);
            }
            if total > 0.0 {
                progress((pos as f64 / total).min(1.0) as f32);
            }
        }
        let pages = rec.finish().map_err(|e| e.to_string())?;
        if let Some(w) = pages.writers.into_iter().next().flatten() {
            store
                .finish_track(&meeting, TrackKind::File, w)
                .map_err(err)?;
        }
        let duration_ms = (pos * 1000 / u64::from(SAMPLE_RATE)) as i64;
        // The speakers: named as in the files, with their speech spans.
        let mut speakers = Vec::with_capacity(lanes.len());
        for (i, ((_, name), lane)) in files.iter().zip(lanes.drain(..)).enumerate() {
            let name: Option<String> = name
                .as_deref()
                .map(|n| n.trim().chars().take(MAX_NAME_CHARS).collect::<String>())
                .filter(|n| !n.is_empty());
            let gid = store
                .add_speaker(
                    &meeting,
                    NewSpeaker {
                        label_idx: i as i64,
                        color_slot: COLOR_ORDER.get(i).map_or(0, |&c| i64::from(c)),
                        ..Default::default()
                    },
                )
                .map_err(err)?;
            if let Some(n) = &name {
                store.rename_speaker(&gid, Some(n)).map_err(err)?;
            }
            speakers.push(TrackSpeaker {
                label: name.unwrap_or_default(),
                speaker_gid: gid,
                spans: lane.meter.finish(),
            });
        }
        store.set_track_speakers(&meeting, &speakers).map_err(err)?;
        // Only a recording that looks like Zoom's is labelled as one.
        if files
            .iter()
            .any(|(p, _)| presets::detect_source(p) == Some("zoom"))
        {
            store.set_source_app(&meeting, Some("zoom")).map_err(err)?;
        }
        store.finish_meeting(&meeting, duration_ms).map_err(err)?;
        store
            .set_meeting_status(&meeting, "processing")
            .map_err(err)?;
        let job = store
            .enqueue_job(
                Some(&meeting),
                FINAL_PASS_JOB,
                JOB_PAYLOAD_VERSION,
                &serde_json::json!({}),
            )
            .map_err(err)?;
        Ok((pos, job))
    })();
    let (pos, job) = match encoded {
        Ok(v) => v,
        Err(e) => {
            let _ = store.delete_meeting(&meeting);
            return Err(e);
        }
    };
    progress(1.0);
    Ok(ImportReport {
        meeting,
        duplicate: false,
        duration_ms: (pos * 1000 / u64::from(SAMPLE_RATE)) as i64,
        channels: 1,
        tracks: files.len(),
        jobs: vec![job],
    })
}

/// Imports `path` into `store`. `progress` gets 0..1 as decoding advances.
pub fn import_file(
    store: &Store,
    path: &Path,
    opts: &ImportOptions,
    events: &EventTx,
) -> Result<ImportReport, String> {
    let err = |e: ghi_store::StoreError| e.to_string();
    let hash = match &opts.source_hash {
        Some(h) => h.clone(),
        None => file_sha256(path).map_err(|e| format!("{}: {e}", path.display()))?,
    };
    if let Some(dup) = earlier(store, &hash)? {
        return Ok(dup);
    }
    let mut dec = Decoder::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let info = dec.info().clone();
    let split = opts.split_channels && info.channels >= 2;
    let tracks: Vec<Track> = if split {
        vec![Track::Mic, Track::System]
    } else {
        vec![Track::Mic]
    };
    // A title and date from the name or the container's tags (phase 14d); the
    // file stem and its modification time are the fallback.
    let tags = ghi_audio::decode::tags(path);
    let found = presets::title_date(path, tags.title.as_deref(), tags.date_ms);
    let title = opts.title.clone().or(found.title).unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Imported recording".into())
    });
    let meeting = store
        .create_meeting(NewMeeting {
            title,
            // When the recording was made: the file's own date by default.
            started_at: opts
                .started_at
                .or(found.started_at_ms)
                .or_else(|| modified_ms(path))
                .unwrap_or(0),
            source: "file".into(),
            mode: if split { "call" } else { "room" }.into(),
            lang: opts.language.clone(),
            ..Default::default()
        })
        .map_err(err)?
        .gid;
    // Any failure from here removes the half-made meeting (and frees the hash).
    // Reported per whole percent: the loop below runs many times a second,
    // and every event makes the UI re-read the meeting list.
    let reported = std::cell::Cell::new(-1);
    let progress = |p: f32| {
        let pct = (p.clamp(0.0, 1.0) * 100.0) as i32;
        if reported.replace(pct) == pct {
            return;
        }
        if let Some(f) = &opts.on_progress {
            (f.0)(&meeting, p);
        }
        events.emit(Event::JobProgress {
            meeting: Some(meeting.clone()),
            job: 0,
            kind: "import".into(),
            stage: Some(Stage::Decoding),
            progress: p,
        })
    };
    let encoded = (|| -> Result<(u64, i64), String> {
        store.set_meeting_status(&meeting, IMPORTING).map_err(err)?;
        store.set_source_hash(&meeting, &hash).map_err(err)?;
        if let Some(app) = presets::detect_source(path) {
            store.set_source_app(&meeting, Some(app)).map_err(err)?;
        }
        let mut writers: [Option<ghi_store::bundle::BundleWriter>; 2] = [None, None];
        for &t in &tracks {
            let kind = match t {
                Track::Mic if !split => TrackKind::File,
                Track::Mic => TrackKind::Mic,
                Track::System => TrackKind::System,
            };
            writers[t.index()] = Some(store.open_track(&meeting, kind).map_err(err)?);
        }
        let mut rec =
            OpusRecorder::new(BundlePages::new(writers), &tracks, EncoderConfig::default())
                .map_err(|e| e.to_string())?;
        // Frames are cut on a 10 ms grid; leftovers wait for the next block.
        let mut pending: Vec<Vec<f32>> = vec![Vec::new(); tracks.len()];
        let mut pos: u64 = 0;
        let total = info
            .duration_ms
            .map(|d| d as f64 * f64::from(SAMPLE_RATE) / 1000.0);
        while let Some(block) = dec.next_block().map_err(|e| e.to_string())? {
            if opts
                .cancel
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Relaxed))
            {
                return Err(CANCELLED.into());
            }
            wait_hold(opts)?;
            let n = block.frames();
            if split {
                for (i, p) in pending.iter_mut().enumerate() {
                    p.extend_from_slice(&block.channels[i][..n]);
                }
            } else {
                let ch = block.channels.len().max(1) as f32;
                pending[0]
                    .extend((0..n).map(|k| block.channels.iter().map(|c| c[k]).sum::<f32>() / ch));
            }
            while pending[0].len() >= FRAME_SAMPLES {
                for (i, &t) in tracks.iter().enumerate() {
                    let frame: Vec<f32> = pending[i].drain(..FRAME_SAMPLES).collect();
                    rec.frame(t, pos, &frame).map_err(|e| e.to_string())?;
                }
                pos += FRAME_SAMPLES as u64;
            }
            if let Some(t) = total {
                progress((pos as f64 / t).min(1.0) as f32);
            }
        }
        if !pending[0].is_empty() {
            for (i, &t) in tracks.iter().enumerate() {
                let mut frame = std::mem::take(&mut pending[i]);
                frame.resize(FRAME_SAMPLES, 0.0);
                rec.frame(t, pos, &frame).map_err(|e| e.to_string())?;
            }
            pos += FRAME_SAMPLES as u64;
        }
        let pages = rec.finish().map_err(|e| e.to_string())?;
        for (i, w) in pages.writers.into_iter().enumerate() {
            if let Some(w) = w {
                let kind = match Track::from_index(i as u32).expect("track index") {
                    Track::Mic if !split => TrackKind::File,
                    Track::Mic => TrackKind::Mic,
                    Track::System => TrackKind::System,
                };
                store.finish_track(&meeting, kind, w).map_err(err)?;
            }
        }
        let duration_ms = (pos * 1000 / u64::from(SAMPLE_RATE)) as i64;
        store.finish_meeting(&meeting, duration_ms).map_err(err)?;
        store
            .set_meeting_status(&meeting, "processing")
            .map_err(err)?;
        let job = store
            .enqueue_job(
                Some(&meeting),
                FINAL_PASS_JOB,
                JOB_PAYLOAD_VERSION,
                &serde_json::json!({}),
            )
            .map_err(err)?;
        Ok((pos, job))
    })();
    let (pos, job) = match encoded {
        Ok(v) => v,
        Err(e) => {
            let _ = store.delete_meeting(&meeting);
            return Err(e);
        }
    };
    let duration_ms = (pos * 1000 / u64::from(SAMPLE_RATE)) as i64;
    progress(1.0);
    Ok(ImportReport {
        meeting,
        duplicate: false,
        duration_ms,
        channels: info.channels,
        tracks: tracks.len(),
        jobs: vec![job],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::bus;
    use ghi_store::keys::{MemoryKeyStore, Protection};

    fn wav(path: &Path, rate: u32, channels: &[Vec<f32>]) {
        let n = channels[0].len();
        let data_len = (n * channels.len() * 2) as u32;
        let mut b = Vec::new();
        b.extend(b"RIFF");
        b.extend((36 + data_len).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend((channels.len() as u16).to_le_bytes());
        b.extend(rate.to_le_bytes());
        b.extend((rate * 2 * channels.len() as u32).to_le_bytes());
        b.extend(((2 * channels.len()) as u16).to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend(data_len.to_le_bytes());
        for k in 0..n {
            for c in channels {
                b.extend(((c[k] * 32_000.0) as i16).to_le_bytes());
            }
        }
        std::fs::write(path, b).unwrap();
    }

    fn tone(hz: f32, rate: u32, secs: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize)
            .map(|i| (i as f32 * hz * std::f32::consts::TAU / rate as f32).sin() * 0.3)
            .collect()
    }

    #[test]
    fn stereo_split_into_two_tracks_and_duplicates_are_found() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::open(
                &tmp.path().join("s"),
                Arc::new(MemoryKeyStore::default()),
                Protection::default(),
            )
            .unwrap(),
        );
        let path = tmp.path().join("zoom.wav");
        wav(
            &path,
            44_100,
            &[tone(300.0, 44_100, 3.0), tone(900.0, 44_100, 3.0)],
        );
        let (tx, _rx) = bus();
        let r = import_file(
            &store,
            &path,
            &ImportOptions {
                split_channels: true,
                ..Default::default()
            },
            &tx,
        )
        .unwrap();
        assert!(!r.duplicate);
        assert_eq!((r.channels, r.tracks), (2, 2));
        assert!(
            (2_950..=3_050).contains(&r.duration_ms),
            "{}",
            r.duration_ms
        );
        let m = store.get_meeting(&r.meeting).unwrap();
        assert_eq!(
            (m.mode.as_str(), m.status.as_str(), m.source.as_str()),
            ("call", "processing", "file")
        );
        let mic = store.open_bundle(&r.meeting, TrackKind::Mic).unwrap();
        let sys = store.open_bundle(&r.meeting, TrackKind::System).unwrap();
        let decode = |b: &ghi_store::bundle::BundleReader| {
            ghi_audio::encoder::read_ogg_opus(&b.read_all().unwrap()[..]).unwrap()
        };
        let (a, b) = (decode(&mic), decode(&sys));
        // Zero crossings ≈ 2 × frequency × seconds: L stays 300 Hz, R 900 Hz.
        let zc = |x: &[f32]| {
            x.windows(2)
                .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
                .count()
        };
        assert!((1_500..2_100).contains(&zc(&a)), "mic {}", zc(&a));
        assert!((4_800..6_000).contains(&zc(&b)), "system {}", zc(&b));
        assert_eq!(
            store.jobs_for_meeting(&r.meeting).unwrap()[0].kind,
            FINAL_PASS_JOB
        );

        let again = import_file(&store, &path, &ImportOptions::default(), &tx).unwrap();
        assert!(again.duplicate && again.meeting == r.meeting);
    }

    #[test]
    fn mono_mixdown_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(
            &tmp.path().join("s"),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        let path = tmp.path().join("room.wav");
        wav(
            &path,
            16_000,
            &[tone(440.0, 16_000, 1.0), tone(440.0, 16_000, 1.0)],
        );
        let (tx, _rx) = bus();
        let r = import_file(&store, &path, &ImportOptions::default(), &tx).unwrap();
        assert_eq!(r.tracks, 1);
        assert_eq!(store.get_meeting(&r.meeting).unwrap().mode, "room");
        assert!(
            store
                .open_bundle(&r.meeting, TrackKind::File)
                .unwrap()
                .page_count()
                >= 2
        );
        assert!(
            import_file(
                &store,
                &tmp.path().join("missing.wav"),
                &ImportOptions::default(),
                &tx
            )
            .is_err()
        );
    }

    #[test]
    fn progress_date_and_cancel() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(
            &tmp.path().join("s"),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        let path = tmp.path().join("memo.wav");
        wav(&path, 16_000, &[tone(440.0, 16_000, 2.0)]);
        let (tx, _rx) = bus();
        let seen = Arc::new(std::sync::Mutex::new(Vec::<(String, f32)>::new()));
        let log = seen.clone();
        let r = import_file(
            &store,
            &path,
            &ImportOptions {
                started_at: Some(1_700_000_000_000),
                on_progress: Some(OnProgress(Arc::new(move |m, p| {
                    log.lock().unwrap().push((m.to_string(), p))
                }))),
                ..Default::default()
            },
            &tx,
        )
        .unwrap();
        let m = store.get_meeting(&r.meeting).unwrap();
        assert_eq!(m.started_at, 1_700_000_000_000, "the file's own date");
        let seen = seen.lock().unwrap();
        assert!(seen.iter().all(|(g, _)| *g == r.meeting));
        assert_eq!(seen.last().map(|(_, p)| *p), Some(1.0));

        // Cancelled: an error, and no meeting is left behind.
        let other = tmp.path().join("other.wav");
        wav(&other, 16_000, &[tone(220.0, 16_000, 2.0)]);
        let cancel = Arc::new(AtomicBool::new(true));
        let e = import_file(
            &store,
            &other,
            &ImportOptions {
                cancel: Some(cancel),
                ..Default::default()
            },
            &tx,
        )
        .unwrap_err();
        assert_eq!(e, CANCELLED);
        assert_eq!(store.list_meetings(10, 0).unwrap().len(), 1);
    }
}
