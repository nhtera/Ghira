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
use std::path::Path;

use ghi_audio::decode::Decoder;
use ghi_audio::encoder::{EncoderConfig, OpusRecorder};
use ghi_audio::pipeline::FrameSink;
use ghi_audio::{FRAME_SAMPLES, SAMPLE_RATE, Track};
use ghi_store::store::{NewMeeting, Store, TrackKind};
use sha2::{Digest, Sha256};

use crate::events::{Event, EventTx, Stage};
use crate::pages::BundlePages;
use crate::session::{FINAL_PASS_JOB, JOB_PAYLOAD_VERSION};

/// Status while an import is being decoded.
pub const IMPORTING: &str = "importing";

#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    pub title: Option<String>,
    pub language: Option<String>,
    pub split_channels: bool,
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

/// SHA-256 (hex) of a file, streamed.
pub fn file_sha256(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Imports `path` into `store`. `progress` gets 0..1 as decoding advances.
pub fn import_file(
    store: &Store,
    path: &Path,
    opts: &ImportOptions,
    events: &EventTx,
) -> Result<ImportReport, String> {
    let err = |e: ghi_store::StoreError| e.to_string();
    let hash = file_sha256(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(m) = store.meeting_by_source_hash(&hash).map_err(err)? {
        let meeting = store.get_meeting(&m).map_err(err)?;
        if meeting.status == IMPORTING {
            // Left over by a failed or interrupted import: start again.
            store.delete_meeting(&m).map_err(err)?;
        } else {
            return Ok(ImportReport {
                meeting: m,
                duplicate: true,
                duration_ms: meeting.duration_ms,
                channels: 0,
                tracks: 0,
                jobs: Vec::new(),
            });
        }
    }
    let mut dec = Decoder::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let info = dec.info().clone();
    let split = opts.split_channels && info.channels >= 2;
    let tracks: Vec<Track> = if split {
        vec![Track::Mic, Track::System]
    } else {
        vec![Track::Mic]
    };
    let title = opts.title.clone().unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Imported recording".into())
    });
    let meeting = store
        .create_meeting(NewMeeting {
            title,
            started_at: 0,
            source: "file".into(),
            mode: if split { "call" } else { "room" }.into(),
            lang: opts.language.clone(),
            ..Default::default()
        })
        .map_err(err)?
        .gid;
    store.set_meeting_status(&meeting, IMPORTING).map_err(err)?;
    store.set_source_hash(&meeting, &hash).map_err(err)?;
    // Any failure from here removes the half-made meeting (and frees the hash).
    let progress = |p: f32| {
        events.emit(Event::JobProgress {
            meeting: Some(meeting.clone()),
            job: 0,
            kind: "import".into(),
            stage: Some(Stage::Decoding),
            progress: p,
        })
    };
    let encoded = (|| -> Result<(u64, i64), String> {
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
    use std::sync::Arc;

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
}
