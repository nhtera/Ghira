// SPDX-License-Identifier: Apache-2.0
//! Live transcript latency bench on the real engines: whole sessions replayed
//! in real time (capture → ring → live engine → events), timed as the UI sees
//! them.
//!
//! Per set it prints one `BENCH` line:
//! - `line`: seconds of audio per final line (p50 / max) and lines ≥ 10 s;
//! - `commit`: wall time from a line's last word to its final event (p50/p95/max);
//! - `first`: wall time from a line's first word to the first caption text (p50/p95);
//! - `stall`: longest wait between caption updates inside one utterance (p95/max);
//! - `lag` / `skipped`: the engine's own health figures (max ring lag, audio skipped);
//! - `wer`: word error rate of the live lines against the reference (FLEURS,
//!   ViMedCSS, `kit:` sets with refs, and AMI with `ami-text` fetched);
//! - `rtf`: the same session replayed as fast as possible, lossless (wall / audio).
//!
//! Sets come from `tools/eval/scripts/fetch_public_sets.py` (fleurs-vi,
//! fleurs-en, ami-sdm + ami-text, voxconverse, vimedcss, earnings21, vietmed).
//! Run as-fast-as-possible passes one at a time: concurrent GPU sessions change
//! the results.
//!
//! ```sh
//! cargo test -p ghi-core --features nemo --release --test live_bench -- --ignored --nocapture
//! # knobs: GHI_LIVE_BENCH_SETS=vox,ami,vi,en,med,call,kit:<eval-kit set>  GHI_LIVE_BENCH_SECONDS=180
//! #        GHI_LIVE_BENCH_CHUNK_MS=560  GHI_LIVE_BENCH_EOU_MS=<pause ms>  GHI_LIVE_BENCH_LABEL=<name>
//! #        GHI_LIVE_BENCH_OUT=<file.jsonl>  GHI_LIVE_BENCH_RTF=0  GHI_LIVE_BENCH_SPEED=0 (lines + WER only)
//! #        GHI_LIVE_BENCH_DUMP=<dir> (lines per set)  GHI_LIVE_BENCH_PARTIALS=1 (captions to stderr)
//! ```

#![cfg(feature = "nemo")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ghi_audio::Track;
use ghi_core::capture::{ReplayTrack, replay};
use ghi_core::engines::{NemoEngines, SpeechEngines};
use ghi_core::events::{Event, bus};
use ghi_core::live::Mode;
use ghi_core::session::{Session, SessionConfig};
use ghi_speech::nemo::Device;
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::Store;

const RATE: usize = 16_000;

struct Set {
    name: String,
    mode: Mode,
    /// Mic, then (call) system.
    tracks: Vec<Vec<f32>>,
    /// Reference text of the mic track, for WER.
    reference: Option<String>,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Any audio file the import path reads, as 16 kHz mono.
fn read_wav(path: &Path) -> Vec<f32> {
    let mut d = ghi_audio::decode::Decoder::open(path)
        .unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
    let mut pcm = Vec::new();
    while let Some(b) = d.next_block().unwrap() {
        let n = b.channels.len() as f32;
        pcm.extend((0..b.frames()).map(|i| b.channels.iter().map(|c| c[i]).sum::<f32>() / n));
    }
    pcm
}

/// An eval-kit set's clips (`audio/<id>.*` with `refs/<id>.txt`) back to back
/// with short pauses (a talk with natural breaks), and their joined reference.
fn clips(set: &str, seconds: f64) -> Option<(Vec<f32>, String)> {
    let dir = root().join("tools/eval/data").join(set);
    let mut ids: Vec<String> = std::fs::read_dir(dir.join("refs"))
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".txt").map(str::to_string))
        .collect();
    ids.sort();
    let gap = vec![0.0f32; RATE * 4 / 10];
    let (mut pcm, mut text) = (Vec::new(), String::new());
    for id in ids {
        if pcm.len() as f64 / RATE as f64 >= seconds {
            break;
        }
        let audio = std::fs::read_dir(dir.join("audio"))
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.file_stem().is_some_and(|s| s == id.as_str()))?;
        pcm.extend(read_wav(&audio));
        pcm.extend_from_slice(&gap);
        let r = std::fs::read_to_string(dir.join(format!("refs/{id}.txt"))).ok()?;
        text.push_str(r.trim());
        text.push(' ');
    }
    (!pcm.is_empty()).then_some((pcm, text))
}

/// The AMI manual transcript of a meeting's first `seconds` (`ami-text` in the
/// eval kit): segments by start time, each counted where its middle falls.
fn ami_reference(meeting: &str, seconds: f64) -> Option<String> {
    let tsv = root().join(format!(
        "tools/eval/data/ami-sdm/refs/{meeting}.segments.tsv"
    ));
    let mut segs: Vec<(f64, String)> = std::fs::read_to_string(tsv)
        .ok()?
        .lines()
        .filter_map(|l| {
            let mut f = l.splitn(4, '\t');
            let (b, e) = (
                f.next()?.parse::<f64>().ok()?,
                f.next()?.parse::<f64>().ok()?,
            );
            let text = f.nth(1)?.to_string();
            ((b + e) / 2.0 < seconds).then_some((b, text))
        })
        .collect();
    segs.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(segs.into_iter().map(|s| s.1).collect::<Vec<_>>().join(" "))
}

fn sets(seconds: f64) -> Vec<Set> {
    let want =
        std::env::var("GHI_LIVE_BENCH_SETS").unwrap_or_else(|_| "vox,ami,vi,en,med,call".into());
    let cap = |mut v: Vec<f32>| {
        v.truncate((seconds * RATE as f64) as usize);
        v
    };
    let data = root().join("tools/eval/data");
    let mut out = Vec::new();
    for name in want.split(',').map(str::trim) {
        let set = match name {
            "vox" => Set {
                name: "vox-aepyx".into(),
                mode: Mode::Room,
                tracks: vec![cap(read_wav(&data.join("voxconverse/audio/aepyx.wav")))],
                reference: None,
            },
            "ami" => Set {
                name: "ami-is1009a".into(),
                mode: Mode::Room,
                tracks: vec![cap(read_wav(&data.join("ami-sdm/audio/IS1009a.wav")))],
                reference: ami_reference("IS1009a", seconds),
            },
            "vi" | "en" | "med" => {
                let set = match name {
                    "vi" => "fleurs-vi",
                    "en" => "fleurs-en",
                    _ => "vimedcss",
                };
                let Some((pcm, text)) = clips(set, seconds) else {
                    eprintln!("skipped {name}: no {set} data");
                    continue;
                };
                Set {
                    name: set.to_string(),
                    mode: Mode::Room,
                    tracks: vec![pcm],
                    reference: Some(text),
                }
            }
            // Any other eval-kit set with refs: `kit:<set>`.
            kit if kit.starts_with("kit:") => {
                let set = &kit[4..];
                let Some((pcm, text)) = clips(set, seconds) else {
                    eprintln!("skipped {set}: no data with refs");
                    continue;
                };
                Set {
                    name: set.to_string(),
                    mode: Mode::Room,
                    tracks: vec![pcm],
                    reference: Some(text),
                }
            }
            "call" => {
                let Some((mic, text)) = clips("fleurs-en", seconds) else {
                    continue;
                };
                Set {
                    name: "call-en+vox".into(),
                    mode: Mode::Call,
                    tracks: vec![
                        mic,
                        cap(read_wav(&data.join("voxconverse/audio/aepyx.wav"))),
                    ],
                    reference: Some(text),
                }
            }
            other => panic!("unknown set {other}"),
        };
        out.push(set);
    }
    out
}

/// Lowercase words without punctuation.
fn words(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

fn wer(reference: &str, hyp: &str) -> f64 {
    let (r, h) = (words(reference), words(hyp));
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for (i, rw) in r.iter().enumerate() {
        let mut cur = vec![i + 1; h.len() + 1];
        for (j, hw) in h.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(rw != hw))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[h.len()] as f64 / r.len().max(1) as f64
}

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    s[((s.len() - 1) as f64 * p).round() as usize]
}

#[derive(Default)]
struct Stats {
    line_s: Vec<f64>,
    commit: Vec<f64>,
    first: Vec<f64>,
    stall: Vec<f64>,
    lag_max: f64,
    skipped: f64,
    /// Mic-track text in line order (WER).
    text: String,
    lines: usize,
}

fn store() -> (tempfile::TempDir, Arc<Store>) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(
        tmp.path(),
        Arc::new(MemoryKeyStore::default()),
        Protection::default(),
    )
    .unwrap();
    (tmp, Arc::new(store))
}

fn start(
    set: &Set,
    engines: &Arc<dyn SpeechEngines>,
    speed: Option<f64>,
) -> (Session, ghi_core::events::EventRx, tempfile::TempDir) {
    let (tmp, store) = store();
    let (tx, rx) = bus();
    let tracks = set
        .tracks
        .iter()
        .enumerate()
        .map(|(i, pcm)| ReplayTrack {
            track: if i == 0 { Track::Mic } else { Track::System },
            samples: pcm.clone(),
            sample_rate: RATE as u32,
        })
        .collect();
    let capture = replay(tracks, speed).unwrap();
    let s = Session::start(
        store,
        Some(engines.clone()),
        capture,
        SessionConfig {
            sensitive: false,
            mode: set.mode,
            language: None,
            title: set.name.clone(),
            queue_jobs: false,
            // As fast as possible: wait for the engine instead of skipping audio.
            lossless: speed.is_none(),
            echo_cancellation: set.mode == Mode::Call,
        },
        tx,
        None,
    )
    .unwrap();
    (s, rx, tmp)
}

/// One real-time session: every caption update timed against the audio clock.
/// One session at `speed` (`None`: as fast as the engine reads, where the
/// timing figures mean nothing but lines and WER are the same as live).
fn run_live(set: &Set, engines: &Arc<dyn SpeechEngines>, speed: Option<f64>) -> Stats {
    let t0 = Instant::now();
    let (s, rx, _tmp) = start(set, engines, speed);
    let now = || t0.elapsed().as_secs_f64();
    let mut st = Stats::default();
    let mut me = None;
    // Per track: when the utterance's first caption came, and its last update.
    let mut first_at: [Option<f64>; 2] = [None, None];
    let mut last_at: [Option<f64>; 2] = [None, None];
    let mut stall: [f64; 2] = [0.0, 0.0];
    let mut finals: Vec<(i64, u32, String)> = Vec::new();
    let mut ended_at = None;
    loop {
        if s.source_ended() && ended_at.is_none() {
            ended_at = Some(Instant::now());
        }
        if ended_at.is_some_and(|e| e.elapsed() > Duration::from_secs(4)) {
            break;
        }
        let Ok(env) = rx.recv_timeout(Duration::from_millis(10)) else {
            continue;
        };
        let at = now();
        match env.event {
            Event::SpeakerArrived { speaker, .. } if speaker.is_me => me = Some(speaker.id),
            Event::TranscriptPartial { track, text, .. } => {
                if std::env::var_os("GHI_LIVE_BENCH_PARTIALS").is_some() {
                    eprintln!("PARTIAL {text}");
                }
                let t = usize::from(track).min(1);
                if first_at[t].is_none() {
                    first_at[t] = Some(at);
                }
                if let Some(l) = last_at[t] {
                    stall[t] = stall[t].max(at - l);
                }
                last_at[t] = Some(at);
            }
            Event::TranscriptFinal { line, .. } => {
                if std::env::var_os("GHI_LIVE_BENCH_PARTIALS").is_some() {
                    eprintln!("FINAL {:.2} {}", line.t0_ms as f64 / 1000.0, line.text);
                }
                let t = usize::from(
                    set.mode == Mode::Call && line.speaker.is_some() && line.speaker != me,
                );
                let (a, b) = (line.t0_ms as f64 / 1000.0, line.t1_ms as f64 / 1000.0);
                st.line_s.push(b - a);
                st.commit.push(at - b);
                if let Some(f) = first_at[t].take() {
                    st.first.push(f - a);
                    if let Some(l) = last_at[t] {
                        stall[t] = stall[t].max(at - l);
                    }
                    st.stall.push(std::mem::take(&mut stall[t]));
                }
                last_at[t] = None;
                finals.push((line.t0_ms, t as u32, line.text));
            }
            Event::Health {
                asr_lag_s,
                asr_skipped_s,
                ..
            } => {
                st.lag_max = st.lag_max.max(f64::from(asr_lag_s));
                st.skipped = f64::from(asr_skipped_s);
            }
            _ => {}
        }
    }
    s.stop().unwrap();
    while let Ok(env) = rx.try_recv() {
        if let Event::TranscriptFinal { line, .. } = env.event {
            let t =
                u32::from(set.mode == Mode::Call && line.speaker.is_some() && line.speaker != me);
            finals.push((line.t0_ms, t, line.text));
        }
    }
    finals.sort_by_key(|f| f.0);
    st.lines = finals.len();
    st.text = finals
        .iter()
        .filter(|f| f.1 == 0)
        .map(|f| f.2.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    // GHI_LIVE_BENCH_DUMP=<dir>: the lines of each set, one per row (diffing runs).
    if let Some(dir) = std::env::var_os("GHI_LIVE_BENCH_DUMP") {
        let rows: Vec<String> = finals
            .iter()
            .map(|f| format!("{:>9.2}\t{}\t{}", f.0 as f64 / 1000.0, f.1, f.2))
            .collect();
        let _ = std::fs::write(
            Path::new(&dir).join(format!("{}.tsv", set.name)),
            rows.join("\n"),
        );
    }
    st
}

/// The same session as fast as the engine reads: wall seconds per audio second.
fn rtf(set: &Set, engines: &Arc<dyn SpeechEngines>) -> f64 {
    let t = Instant::now();
    let (s, _rx, _tmp) = start(set, engines, None);
    while !s.source_ended() {
        std::thread::sleep(Duration::from_millis(20));
    }
    s.stop().unwrap();
    t.elapsed().as_secs_f64() / (set.tracks[0].len() as f64 / RATE as f64)
}

#[test]
#[ignore = "needs NeMo (tools/scripts/build-nemo.sh), the speech models and the eval sets; minutes of real-time replay"]
fn live_bench() {
    let models = std::env::var_os("GHI_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("models"));
    let (asr, diar) = (
        models.join("nemotron-3.5-asr-streaming-0.6b.q8_0.gguf"),
        models.join("Nemotron-3-Diarization.q8_0.gguf"),
    );
    if !asr.exists() || !diar.exists() {
        eprintln!("skipped: models missing in {}", models.display());
        return;
    }
    let chunk_ms: u32 = std::env::var("GHI_LIVE_BENCH_CHUNK_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(560);
    let eou_ms: Option<u32> = std::env::var("GHI_LIVE_BENCH_EOU_MS")
        .ok()
        .and_then(|v| v.parse().ok());
    let seconds: f64 = std::env::var("GHI_LIVE_BENCH_SECONDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(180.0);
    let label = std::env::var("GHI_LIVE_BENCH_LABEL").unwrap_or_else(|_| "run".into());
    let engines: Arc<dyn SpeechEngines> =
        Arc::new(NemoEngines::load_tuned(&asr, &diar, chunk_ms, eou_ms, Device::Gpu).unwrap());
    // GHI_LIVE_BENCH_SPEED=0: lines and WER only, as fast as possible (long sets).
    let speed = match std::env::var("GHI_LIVE_BENCH_SPEED")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
    {
        Some(x) if x <= 0.0 => None,
        Some(x) => Some(x),
        None => Some(1.0),
    };
    let with_rtf = speed.is_some() && std::env::var("GHI_LIVE_BENCH_RTF").as_deref() != Ok("0");
    for set in sets(seconds) {
        let st = run_live(&set, &engines, speed);
        let w = set.reference.as_deref().map(|r| wer(r, &st.text));
        let r = with_rtf.then(|| rtf(&set, &engines));
        let long = st.line_s.iter().filter(|d| **d >= 10.0).count();
        println!(
            "BENCH {label} {:<12} lines={:<3} line p50={:.1}s max={:.1}s long={long} | commit p50={:.2} p95={:.2} max={:.2} | first p50={:.2} p95={:.2} | stall p95={:.2} max={:.2} | lag={:.2} skipped={:.1} | wer={} rtf={}",
            set.name,
            st.lines,
            pct(&st.line_s, 0.5),
            pct(&st.line_s, 1.0),
            pct(&st.commit, 0.5),
            pct(&st.commit, 0.95),
            pct(&st.commit, 1.0),
            pct(&st.first, 0.5),
            pct(&st.first, 0.95),
            pct(&st.stall, 0.95),
            pct(&st.stall, 1.0),
            st.lag_max,
            st.skipped,
            w.map_or("-".into(), |w| format!("{:.1}%", w * 100.0)),
            r.map_or("-".into(), |r| format!("{r:.3}")),
        );
        if let Some(path) = std::env::var_os("GHI_LIVE_BENCH_OUT") {
            use std::io::Write;
            let row = serde_json::json!({
                "label": label, "set": set.name, "chunk_ms": chunk_ms, "eou_ms": eou_ms, "lines": st.lines,
                "line_p50": pct(&st.line_s, 0.5), "line_max": pct(&st.line_s, 1.0), "long": long,
                "commit_p50": pct(&st.commit, 0.5), "commit_p95": pct(&st.commit, 0.95), "commit_max": pct(&st.commit, 1.0),
                "first_p50": pct(&st.first, 0.5), "first_p95": pct(&st.first, 0.95),
                "stall_p95": pct(&st.stall, 0.95), "stall_max": pct(&st.stall, 1.0),
                "lag_max": st.lag_max, "skipped": st.skipped, "wer": w, "rtf": r,
            });
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap();
            writeln!(f, "{row}").unwrap();
        }
    }
}

#[test]
fn wer_counts_edits_over_reference_words() {
    assert_eq!(wer("xin chào mọi người", "Xin chào, mọi người!"), 0.0);
    assert!((wer("a b c d", "a x c") - 0.5).abs() < 1e-9);
}
