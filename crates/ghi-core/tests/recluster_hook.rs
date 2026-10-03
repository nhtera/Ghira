// SPDX-License-Identifier: Apache-2.0
//! The re-cluster hook in the real final pass (phase 14d S3): a long import
//! whose diarizer saturates at 8 gets its segments replaced when the speaker
//! model is ready; per-participant tracks and a missing model skip it.
//! The fake diarizer is windowed: it labels what it hears (by pitch, in order
//! of first appearance) and caps a long stream at 8 voices like the real one.
#![cfg(feature = "voice")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ghi_core::engines::{BoxAsr, BoxDiar, FakeEngines, Script, SpeechEngines};
use ghi_core::events::bus;
use ghi_core::final_pass::FinalPassJob;
use ghi_core::import::{ImportOptions, import_file, import_tracks};
use ghi_core::jobs::{JobRunner, Ready, always_ready};
use ghi_core::profiles::{FakeVoice, VoiceEmbed, VoiceFactory};
use ghi_core::voice_step::VoiceStep;
use ghi_speech::{DiarStream, SpeakerSegment};
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::Store;

const RATE: usize = 16_000;
/// Streams longer than this are "the whole recording": the diarizer's cap applies.
const WHOLE_S: usize = 140;

struct HearingDiar {
    pcm: Vec<f32>,
}

impl DiarStream for HearingDiar {
    fn push(&mut self, pcm: &[f32], _rate: u32) -> ghi_speech::Result<()> {
        self.pcm.extend_from_slice(pcm);
        Ok(())
    }
    fn finish(&mut self) -> ghi_speech::Result<()> {
        Ok(())
    }
    fn segments(&self) -> ghi_speech::Result<Vec<SpeakerSegment>> {
        let cap = if self.pcm.len() > WHOLE_S * RATE {
            8
        } else {
            u32::MAX
        };
        let step = RATE / 10;
        let loud: Vec<bool> = self
            .pcm
            .chunks(step)
            .map(|c| c.iter().any(|x| x.abs() > 0.01))
            .collect();
        let mut seen: Vec<u64> = Vec::new();
        let mut out = Vec::new();
        let mut i = 0;
        while i < loud.len() {
            if !loud[i] {
                i += 1;
                continue;
            }
            let start = i;
            while i < loud.len() && loud[i] {
                i += 1;
            }
            let bucket = FakeVoice::bucket(&self.pcm[start * step..(i * step).min(self.pcm.len())]);
            let label = seen.iter().position(|b| *b == bucket).unwrap_or_else(|| {
                seen.push(bucket);
                seen.len() - 1
            });
            out.push(SpeakerSegment {
                start: start as f64 / 10.0,
                end: i as f64 / 10.0,
                speaker: (label as u32 + 1).min(cap),
            });
        }
        Ok(out)
    }
}

struct Engines {
    asr: Arc<FakeEngines>,
    diars: AtomicUsize,
}

impl SpeechEngines for Engines {
    fn asr(&self, language: Option<&str>) -> ghi_speech::Result<BoxAsr> {
        self.asr.asr(language)
    }
    fn diar(&self) -> ghi_speech::Result<BoxDiar> {
        self.diars.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(HearingDiar { pcm: Vec::new() }))
    }
    fn chunk_ms(&self) -> u32 {
        self.asr.chunk_ms()
    }
}

fn open_store() -> (tempfile::TempDir, Arc<Store>) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    (tmp, store)
}

/// Turn starts (s) and identities (pitch buckets): the first window hears
/// 3..=10, the later ones 7..=14, so 12 voices and never more than 8 at once.
fn layout() -> Vec<(f64, u64)> {
    let first = (0..16).map(|k| (k as f64 * 6.5, 3 + (k % 8) as u64));
    let second = (0..17).map(|k| (134.0 + k as f64 * 6.5, 7 + (k % 8) as u64));
    first.chain(second).collect()
}

fn write_wav(path: &Path, turns: &[(f64, u64)], total_s: f64) {
    let mut w = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 1,
            sample_rate: RATE as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for i in 0..(total_s * RATE as f64) as usize {
        let t = i as f64 / RATE as f64;
        let x = turns
            .iter()
            .find(|(a, _)| t >= *a && t < a + 6.0)
            .map_or(0.0, |(_, b)| {
                (i as f32 * (*b as f32 * 100.0) * std::f32::consts::TAU / RATE as f32).sin() * 0.2
            });
        w.write_sample((x * 32_767.0) as i16).unwrap();
    }
    w.finalize().unwrap();
}

fn script(turns: &[(f64, u64)]) -> Script {
    Script {
        utterances: turns
            .iter()
            .map(|(a, _)| (a + 0.5, a + 5.5, "xin chào mọi người".to_string()))
            .collect(),
        turns: vec![],
    }
}

fn runner(store: &Arc<Store>, engines: Arc<Engines>, voice_ready: Ready) -> Arc<JobRunner> {
    let (tx, _rx) = bus();
    let embedder: VoiceFactory = Arc::new(|| Ok(Box::new(FakeVoice) as Box<dyn VoiceEmbed>));
    JobRunner::new(
        store.clone(),
        tx,
        vec![Arc::new(FinalPassJob {
            engines: Arc::new(move || Ok(engines.clone() as Arc<dyn SpeechEngines>)),
            chunk_s: 600.0,
            ready: always_ready(),
            voice: Some(VoiceStep {
                embedder,
                ready: voice_ready,
                third_party: Arc::new(|| None),
            }),
        })],
    )
}

fn engines(turns: &[(f64, u64)]) -> Arc<Engines> {
    Arc::new(Engines {
        asr: FakeEngines::new(script(turns)),
        diars: AtomicUsize::new(0),
    })
}

fn speakers(store: &Store, meeting: &str) -> usize {
    store.speakers(meeting).unwrap().len()
}

fn long_import(store: &Store, dir: &Path) -> String {
    let turns = layout();
    let wav = dir.join("allhands.wav");
    write_wav(&wav, &turns, 250.0);
    let (tx, _rx) = bus();
    import_file(store, &wav, &ImportOptions::default(), &tx)
        .unwrap()
        .meeting
}

#[test]
fn a_saturated_diarizer_is_replaced_by_the_recluster() {
    let (tmp, store) = open_store();
    let meeting = long_import(&store, tmp.path());
    let e = engines(&layout());
    assert_eq!(runner(&store, e.clone(), always_ready()).run_pending(), 1);
    assert_eq!(speakers(&store, &meeting), 12, "the cap of 8 was lifted");
    assert!(e.diars.load(Ordering::SeqCst) >= 4, "whole file + windows");
}

#[test]
fn a_voice_model_that_is_not_ready_skips_it() {
    let (tmp, store) = open_store();
    let meeting = long_import(&store, tmp.path());
    let e = engines(&layout());
    let not_ready: Ready = Arc::new(|| false);
    assert_eq!(runner(&store, e.clone(), not_ready).run_pending(), 1);
    assert_eq!(speakers(&store, &meeting), 8, "still the capped result");
    assert_eq!(
        e.diars.load(Ordering::SeqCst),
        1,
        "no windows were diarized"
    );
}

#[test]
fn per_participant_tracks_skip_it() {
    let (tmp, store) = open_store();
    let dir: PathBuf = tmp
        .path()
        .join("2026-07-03 14.05.02 Sprint 81234567890/Audio Record");
    std::fs::create_dir_all(&dir).unwrap();
    let turns = layout();
    // Nine participants, one track each; each speaks only her own turns.
    let mut files = Vec::new();
    for who in 3u64..12 {
        let mine: Vec<(f64, u64)> = turns.iter().copied().filter(|t| t.1 == who).collect();
        let p = dir.join(format!("audioP{who}{who}{who}{who}.wav"));
        write_wav(&p, &mine, 250.0);
        files.push((p, Some(format!("P{who}"))));
    }
    let (tx, _rx) = bus();
    let r = import_tracks(&store, &files, &ImportOptions::default(), &tx).unwrap();
    let e = engines(&turns);
    assert_eq!(runner(&store, e.clone(), always_ready()).run_pending(), 1);
    assert_eq!(e.diars.load(Ordering::SeqCst), 0, "tracks never diarize");
    assert_eq!(
        speakers(&store, &r.meeting),
        9,
        "the participants, no clusters"
    );
}
