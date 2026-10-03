// SPDX-License-Identifier: Apache-2.0
//! Re-clustering past the diarizer's cap (phase 14d S3): synthetic tones stand
//! in for voices (`FakeVoice` buckets by pitch) and a windowed fake diarizer
//! labels what it hears, at most 8 per window like the real one.
//! The VoxConverse eval driver is `#[ignore]` (needs NeMo, models and data).
#![cfg(feature = "voice")]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ghi_core::engines::{BoxAsr, BoxDiar, Result, SpeechEngines};
use ghi_core::profiles::{FakeVoice, VoiceEmbed};
use ghi_core::recluster::{self, Params, SATURATED_AT};
use ghi_speech::{DiarStream, SpeakerSegment};

const RATE: usize = 16_000;

/// Diarizes the audio it is fed: runs of sound. With `by_pitch`, labelled by
/// pitch bucket in order of first appearance in this stream (labels restart
/// per window); otherwise 1, 2, 1, 2 whatever the pitch.
struct HearingDiar {
    pcm: Vec<f32>,
    by_pitch: bool,
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
            let run = &self.pcm[start * step..(i * step).min(self.pcm.len())];
            let label = if self.by_pitch {
                let bucket = FakeVoice::bucket(run);
                seen.iter().position(|b| *b == bucket).unwrap_or_else(|| {
                    seen.push(bucket);
                    seen.len() - 1
                })
            } else {
                out.len() % 2
            };
            out.push(SpeakerSegment {
                start: start as f64 / 10.0,
                end: i as f64 / 10.0,
                speaker: label as u32 + 1,
            });
        }
        Ok(out)
    }
}

struct Engines {
    windows: AtomicUsize,
    by_pitch: bool,
}

fn engines() -> Engines {
    Engines {
        windows: AtomicUsize::new(0),
        by_pitch: true,
    }
}

impl SpeechEngines for Engines {
    fn asr(&self, _: Option<&str>) -> Result<BoxAsr> {
        unreachable!("not used")
    }
    fn diar(&self) -> Result<BoxDiar> {
        self.windows.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(HearingDiar {
            pcm: Vec::new(),
            by_pitch: self.by_pitch,
        }))
    }
    fn chunk_ms(&self) -> u32 {
        560
    }
}

/// A tone of `bucket * 100` Hz.
fn tone(bucket: u64, seconds: f64) -> Vec<f32> {
    let hz = bucket as f64 * 100.0;
    (0..(seconds * RATE as f64) as usize)
        .map(|i| (2.0 * std::f64::consts::PI * hz * i as f64 / RATE as f64).sin() as f32 * 0.4)
        .collect()
}

/// 6 s of each bucket in turn, 0.5 s of silence between; returns the audio
/// and the (start s, end s, identity) truth.
fn talk(order: &[u64]) -> (Vec<f32>, Vec<(f64, f64, u64)>) {
    let mut pcm = Vec::new();
    let mut truth = Vec::new();
    for &b in order {
        let t0 = pcm.len() as f64 / RATE as f64;
        pcm.extend(tone(b, 6.0));
        truth.push((t0, t0 + 6.0, b));
        pcm.extend(vec![0.0; RATE / 2]);
    }
    (pcm, truth)
}

/// Two halves split by 30 s of silence from 104 s on, so the first window
/// ends at 105 s and the rest is windows 2 and 3 (the first half is at most
/// 16 turns, the second at least 17).
fn two_halves(first: &[u64], second: &[u64]) -> Vec<f32> {
    assert!(first.len() <= 16 && second.len() >= 17);
    let (mut pcm, _) = talk(first);
    pcm.resize(134 * RATE, 0.0);
    pcm.extend(talk(second).0);
    pcm
}

/// 12 identities over several windows of about 120 s; each window hears 8 at
/// most (3..=10 early, 7..=14 late).
fn twelve() -> (Vec<f32>, Vec<(f64, f64, u64)>) {
    let mut order = Vec::new();
    for round in 0..6 {
        for k in 0..18 {
            let base = if round < 3 { 3 } else { 7 };
            order.push(base + (k % 8) as u64);
        }
    }
    talk(&order)
}

fn saturated(n: u32) -> Vec<SpeakerSegment> {
    (1..=n)
        .map(|s| SpeakerSegment {
            start: f64::from(s),
            end: f64::from(s) + 1.0,
            speaker: s,
        })
        .collect()
}

fn no_stop() -> bool {
    false
}

const P: Params = Params {
    threshold: 0.5,
    min_cluster_s: 8.0,
};

fn labels_of(segs: &[SpeakerSegment]) -> std::collections::BTreeSet<u32> {
    segs.iter().map(|s| s.speaker).collect()
}

fn label_at(segs: &[SpeakerSegment], t: f64) -> u32 {
    segs.iter()
        .find(|s| s.start <= t && t < s.end)
        .unwrap_or_else(|| panic!("no segment at {t}"))
        .speaker
}

#[test]
fn twelve_voices_come_back_as_twelve_clusters() {
    let (pcm, truth) = twelve();
    let e = engines();
    let segs = recluster::run_with(&pcm, &e, &mut FakeVoice, &saturated(8), &no_stop, P)
        .unwrap()
        .expect("more than 8 voices");
    assert!(e.windows.load(Ordering::SeqCst) >= 5, "windowed");
    let labels = labels_of(&segs);
    assert_eq!(labels.len(), 12, "{labels:?}");
    assert_eq!(*labels.iter().max().unwrap(), 12, "labels are 1..=n");
    let mut of = std::collections::BTreeMap::new();
    for (a, b, id) in truth {
        let s = label_at(&segs, (a + b) / 2.0);
        assert_eq!(*of.entry(id).or_insert(s), s, "identity {id}");
    }
    assert_eq!(
        of.values().collect::<std::collections::BTreeSet<_>>().len(),
        12
    );
}

#[test]
fn the_threshold_reaches_the_clustering() {
    let (pcm, _) = twelve();
    let d = recluster::analyse(&pcm, &engines(), &mut FakeVoice, &no_stop)
        .unwrap()
        .unwrap();
    let (_, strict) = d.assign(P, &no_stop).unwrap();
    // Everything that may merge does (labels of one window still never do).
    let (_, loose) = d
        .assign(
            Params {
                threshold: -2.0,
                min_cluster_s: 0.0,
            },
            &no_stop,
        )
        .unwrap();
    assert_eq!(strict, 12);
    assert!(loose < strict, "{loose} vs {strict}");
    assert!(loose >= 8, "one window's 8 voices stay apart: {loose}");
}

#[test]
fn the_users_speaker_count_sets_the_cluster_count() {
    let (pcm, _) = twelve();
    let d = recluster::analyse(&pcm, &engines(), &mut FakeVoice, &no_stop)
        .unwrap()
        .unwrap();
    let keep_small = Params {
        min_cluster_s: 0.0,
        ..P
    };
    let n = |t| d.assign_to(keep_small, t, &no_stop).unwrap().1;
    assert_eq!(n(None), 12);
    // Told ten people spoke: merged past the threshold down to ten.
    assert_eq!(n(Some(10)), 10);
    // A count above what the threshold gives changes nothing (a cap).
    assert_eq!(n(Some(20)), 12);
    assert!(n(Some(0)) >= 8, "one window's voices never merge");
}

#[test]
fn eight_or_fewer_voices_keep_the_capped_result() {
    let (pcm, _) = talk(&(0..8).cycle().take(24).map(|i| 3 + i).collect::<Vec<_>>());
    let e = engines();
    // Not saturated: nothing runs.
    assert!(
        recluster::run_with(&pcm, &e, &mut FakeVoice, &saturated(5), &no_stop, P)
            .unwrap()
            .is_none()
    );
    assert_eq!(e.windows.load(Ordering::SeqCst), 0);
    // Saturated but only 8 real voices: still not better.
    assert!(
        recluster::run_with(&pcm, &e, &mut FakeVoice, &saturated(8), &no_stop, P)
            .unwrap()
            .is_none()
    );
    assert_eq!(SATURATED_AT, 8);
}

#[test]
fn empty_and_single_window_audio_keep_the_capped_result() {
    let e = engines();
    assert!(
        recluster::run_with(&[], &e, &mut FakeVoice, &saturated(8), &no_stop, P)
            .unwrap()
            .is_none()
    );
    let (one, _) = talk(&(3..15).collect::<Vec<_>>()); // 12 voices in 78 s
    assert!(
        recluster::run_with(&one, &e, &mut FakeVoice, &saturated(8), &no_stop, P)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        e.windows.load(Ordering::SeqCst),
        0,
        "no window was diarized"
    );
}

#[test]
fn stop_abandons_the_work() {
    let (pcm, _) = twelve();
    let e = engines();
    let calls = AtomicUsize::new(0);
    let stop = || calls.fetch_add(1, Ordering::SeqCst) >= 3;
    let out = recluster::run_with(&pcm, &e, &mut FakeVoice, &saturated(8), &stop, P).unwrap();
    assert!(out.is_none());
    assert!(e.windows.load(Ordering::SeqCst) < 6, "stopped early");
}

/// Counts embeddings so a stop can fire between two windows of one speaker.
struct Counting(Rc<Cell<usize>>);

impl VoiceEmbed for Counting {
    fn embed(&mut self, pcm: &[f32]) -> std::result::Result<Option<Vec<f32>>, String> {
        self.0.set(self.0.get() + 1);
        FakeVoice.embed(pcm)
    }
}

#[test]
fn a_stop_while_embedding_wipes_every_vector() {
    let (pcm, _) = twelve();
    let embedded = Rc::new(Cell::new(0));
    let mut voice = Counting(embedded.clone());
    let seen = embedded.clone();
    let stop = move || seen.get() >= 10;
    let before = recluster::wiped_vectors_this_thread();
    let out = recluster::analyse(&pcm, &engines(), &mut voice, &stop).unwrap();
    assert!(out.is_none(), "stopped in the embedding phase");
    assert!(embedded.get() >= 10);
    // Items finished before the stop held vectors; all were wiped on the way out.
    assert!(recluster::wiped_vectors_this_thread() - before >= 2);
}

#[test]
fn a_finished_analysis_wipes_on_drop() {
    let (pcm, _) = twelve();
    let before = recluster::wiped_vectors_this_thread();
    let d = recluster::analyse(&pcm, &engines(), &mut FakeVoice, &no_stop)
        .unwrap()
        .unwrap();
    let (_, voiced) = d.counts();
    assert_eq!(
        recluster::wiped_vectors_this_thread(),
        before,
        "alive until dropped"
    );
    drop(d);
    assert_eq!(recluster::wiped_vectors_this_thread() - before, voiced);
}

#[test]
fn run_is_the_gated_run_with() {
    let (pcm, _) = twelve();
    let e = engines();
    let out = recluster::run(&pcm, &e, &mut FakeVoice, &saturated(8), &no_stop, None).unwrap();
    assert_eq!(out.is_some(), recluster::ENABLED);
    let before = e.windows.load(Ordering::SeqCst);
    assert!(
        recluster::run(&pcm, &e, &mut FakeVoice, &saturated(3), &no_stop, None)
            .unwrap()
            .is_none()
    );
    assert_eq!(e.windows.load(Ordering::SeqCst), before);
}

#[test]
fn two_labels_of_one_window_never_merge() {
    // One pitch throughout, labelled 1, 2, 1, 2 by the diarizer: the voices
    // are identical, yet two people never speak as one within a window.
    let order: Vec<u64> = vec![5; 16];
    let pcm = two_halves(&order, &[5; 17]);
    let e = Engines {
        windows: AtomicUsize::new(0),
        by_pitch: false,
    };
    let d = recluster::analyse(&pcm, &e, &mut FakeVoice, &no_stop)
        .unwrap()
        .unwrap();
    let (segs, voiced) = d.assign(P, &no_stop).unwrap();
    assert_eq!(voiced, 2);
    assert_eq!(labels_of(&segs).len(), 2);
    // Adjacent turns in a window are the two different people.
    assert_ne!(label_at(&segs, 3.0), label_at(&segs, 9.5));
}

#[test]
fn small_clusters_join_the_nearest_one_that_is_never_heard_with_them() {
    // 5 is heard in the first window only, 9 once (6 s) in the later ones.
    let first: Vec<u64> = (0..16).map(|i| 3 + (i % 3) as u64).collect(); // 3, 4, 5
    let mut second: Vec<u64> = (0..17).map(|i| 3 + (i % 2) as u64).collect(); // 3, 4
    second[8] = 9;
    let pcm = two_halves(&first, &second);
    let d = recluster::analyse(&pcm, &engines(), &mut FakeVoice, &no_stop)
        .unwrap()
        .unwrap();
    let t9 = 134.0 + 8.0 * 6.5 + 3.0;
    let t5 = 2.0 * 6.5 + 3.0;
    let (segs, n) = d.assign(P, &no_stop).unwrap();
    assert_eq!(n, 3, "9 had too little speech to stand alone");
    assert_eq!(label_at(&segs, t9), label_at(&segs, t5), "it joined 5");
    let (segs, n) = d
        .assign(
            Params {
                min_cluster_s: 0.0,
                ..P
            },
            &no_stop,
        )
        .unwrap();
    assert_eq!(n, 4);
    assert_ne!(label_at(&segs, t9), label_at(&segs, t5));
}

#[test]
fn a_speaker_without_a_voice_never_takes_a_neighbours_label() {
    // A 1.5 s blip of its own pitch among ten voices of the first window.
    let blip_at = 70.0; // in the silence after the tenth turn
    let blip = |second: &[u64]| {
        let first: Vec<u64> = (3..13).collect();
        let mut pcm = two_halves(&first, second);
        let at = (blip_at * RATE as f64) as usize;
        for (k, x) in tone(20, 1.5)[..].iter().enumerate() {
            pcm[at + k] = *x;
        }
        pcm
    };
    // Window 2 has two voices of its own: the blip may follow one of them.
    // Window 2 has the same voices as window 1: nothing to follow, so it
    // keeps a label of its own.
    for second in [
        [13, 14].repeat(9),
        (3..13).cycle().take(17).collect::<Vec<u64>>(),
    ] {
        let pcm = blip(&second);
        let d = recluster::analyse(&pcm, &engines(), &mut FakeVoice, &no_stop)
            .unwrap()
            .unwrap();
        let (items, voiced_items) = d.counts();
        assert_eq!(items - voiced_items, 1, "exactly the blip has no voice");
        let (segs, _) = d.assign(P, &no_stop).unwrap();
        let blip_label = label_at(&segs, blip_at + 0.75);
        let window_one: Vec<u32> = (0..10)
            .map(|k| label_at(&segs, k as f64 * 6.5 + 3.0))
            .collect();
        assert!(
            !window_one.contains(&blip_label),
            "{blip_label} in {window_one:?}"
        );
    }
}

/// The VoxConverse eval driver: for every file under `$GHI_RECLUSTER_DATA`
/// (default `tools/eval/data/many/voxconverse`) writes the capped diarization
/// and the re-clustered one at several thresholds as RTTMs under `hyp/`, plus
/// `hyp/timing.json`; `ghi-eval recluster-report --dataset <dir>` scores them.
/// `NEMO_SPEECH_DIR=... cargo test -p ghi-core --features nemo,voice --release --test recluster -- --ignored vox_eval --nocapture`
#[cfg(feature = "nemo")]
#[test]
#[ignore = "needs NeMo, the speech and speaker models, and the VoxConverse subset (tools/eval/scripts/fetch_public_sets.py --sets voxconverse --subset many --out tools/eval/data/many)"]
fn vox_eval() {
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    use ghi_core::engines::NemoEngines;
    use ghi_core::profiles::open_tract;
    use ghi_speech::nemo::Device;

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let data = std::env::var_os("GHI_RECLUSTER_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("tools/eval/data/many/voxconverse"));
    let models = root.join("models");
    let (asr, diar, voice) = (
        models.join("nemotron-3.5-asr-streaming-0.6b.q8_0.gguf"),
        models.join("Nemotron-3-Diarization.q8_0.gguf"),
        models.join("3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx"),
    );
    if !data.join("labels").exists() || !asr.exists() || !diar.exists() || !voice.exists() {
        eprintln!("skipped: missing data or models");
        return;
    }
    let engines = NemoEngines::load(&asr, &diar, 560, Device::Gpu).unwrap();
    let mut embedder = open_tract(&voice).unwrap();
    let mut ids: Vec<String> = std::fs::read_dir(data.join("audio"))
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".wav").map(str::to_string))
        .collect();
    ids.sort();
    let thresholds = [0.3f32, 0.4, 0.5, 0.6, 0.7];
    let mut timing = serde_json::Map::new();
    let write = |variant: &str, id: &str, segs: &[SpeakerSegment]| {
        let dir = data.join("hyp").join(variant);
        std::fs::create_dir_all(&dir).unwrap();
        let text: String = segs
            .iter()
            .filter(|s| s.end > s.start)
            .map(|s| {
                format!(
                    "SPEAKER {id} 1 {:.3} {:.3} <NA> <NA> S{} <NA> <NA>\n",
                    s.start,
                    s.end - s.start,
                    s.speaker
                )
            })
            .collect();
        std::fs::write(dir.join(format!("{id}.rttm")), text).unwrap();
    };
    for id in ids {
        let mut r = hound::WavReader::open(data.join(format!("audio/{id}.wav"))).unwrap();
        let spec = r.spec();
        assert_eq!((spec.sample_rate, spec.channels), (16_000, 1), "{id}");
        let pcm: Vec<f32> = r
            .samples::<i16>()
            .map(|s| f32::from(s.unwrap()) / 32_768.0)
            .collect();
        let dur = pcm.len() as f64 / 16_000.0;

        let t = Instant::now();
        let mut d = engines.diar().unwrap();
        for block in pcm.chunks(10 * 16_000) {
            d.push(block, 16_000).unwrap();
        }
        d.finish().unwrap();
        let capped = d.segments().unwrap();
        let capped_s = t.elapsed().as_secs_f64();
        write("capped", &id, &capped);

        let t = Instant::now();
        let analysed = recluster::analyse(&pcm, &engines, embedder.as_mut(), &no_stop).unwrap();
        let analyse_s = t.elapsed().as_secs_f64();
        let (items, voiced) = analysed.as_ref().map_or((0, 0), |a| a.counts());
        for thr in thresholds {
            let p = Params {
                threshold: thr,
                min_cluster_s: recluster::PARAMS.min_cluster_s,
            };
            // What the final pass would keep: the capped result unless more
            // than 8 clusters came out (or the audio is a single window).
            let out = analysed
                .as_ref()
                .and_then(|a| a.assign(p, &no_stop))
                .filter(|(_, n)| *n > SATURATED_AT)
                .map(|(s, _)| s)
                .unwrap_or_else(|| capped.clone());
            write(&format!("recluster-t{thr}"), &id, &out);
        }
        eprintln!(
            "{id}: {dur:.0}s capped {capped_s:.1}s analyse {analyse_s:.1}s items {items}/{voiced}"
        );
        timing.insert(
            id,
            serde_json::json!({"dur_s": dur, "capped_s": capped_s, "analyse_s": analyse_s, "items": items, "voiced": voiced}),
        );
    }
    std::fs::write(
        data.join("hyp/timing.json"),
        serde_json::to_string_pretty(&timing).unwrap(),
    )
    .unwrap();
}
