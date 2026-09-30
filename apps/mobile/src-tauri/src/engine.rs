// SPDX-License-Identifier: Apache-2.0
//! The live engine thread: reads the backlog, runs streaming ASR and
//! diarization (Light tier: 1120 ms ASR chunk, diarization `v3-streaming`),
//! and turns finals into speaker-tagged transcript lines.
//!
//! Spike code: phase 8 builds the real live pipeline (final pass, "Me"
//! anchoring, renames, word-level speakers) in the shared core.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ghi_speech::SpeakerSegment;

use crate::backlog::BacklogReader;
use crate::session::Shared;

/// Audio per step while live, and the cap while catching up (a step is what
/// may have to be redone after a lock lands in it).
const LIVE_STEP: usize = 2_560; // 160 ms
const CATCH_UP_STEP: usize = 8_000; // 500 ms
const POLL: Duration = Duration::from_millis(40);
pub const SAMPLE_RATE: u32 = ghi_audio::SAMPLE_RATE;

/// Model files the engine needs, by registry id.
pub const MODELS: [&str; 2] = ["nemotron-3.5-asr", "nemotron-3-diarization"];

/// Path of a registry model in `dir`, if it is there with the pinned size.
pub fn model_file(dir: &Path, id: &str) -> Option<PathBuf> {
    let m = ghi_models::find(id)?;
    let path = ghi_models::path_in(dir, &m);
    let len = std::fs::metadata(&path).ok()?.len();
    (len == m.size).then_some(path)
}

#[cfg_attr(not(feature = "nemo"), allow(dead_code))]
/// The speaker who talks most within `start..end` (seconds), if anyone does.
pub fn majority_speaker(segs: &[SpeakerSegment], start: f64, end: f64) -> Option<u32> {
    let mut best: Option<(u32, f64)> = None;
    let mut totals: Vec<(u32, f64)> = Vec::new();
    for s in segs {
        let overlap = s.end.min(end) - s.start.max(start);
        if overlap <= 0.0 {
            continue;
        }
        match totals.iter_mut().find(|(k, _)| *k == s.speaker) {
            Some((_, t)) => *t += overlap,
            None => totals.push((s.speaker, overlap)),
        }
    }
    for (k, t) in totals {
        if best.is_none_or(|(_, b)| t > b) {
            best = Some((k, t));
        }
    }
    best.map(|(k, _)| k)
}

/// What one engine step produced, committed only if the step was not suspect.
#[cfg_attr(not(any(test, feature = "nemo")), allow(dead_code))]
#[derive(Debug)]
pub enum Update {
    Partial(String),
    /// A final line; times are seconds of the current streams.
    Final {
        start: f64,
        end: f64,
        speaker: Option<u32>,
        text: String,
    },
}

/// Runs the engine until the session stops and the backlog is drained.
pub fn run(shared: Arc<Shared>, mut backlog: BacklogReader, models_dir: PathBuf) {
    // Whatever happens (errors, a panic), the session learns the engine is done.
    struct Done(Arc<Shared>);
    impl Drop for Done {
        fn drop(&mut self) {
            self.0.engine_done();
        }
    }
    let _done = Done(shared.clone());
    #[cfg(feature = "nemo")]
    let result = nemo::run(&shared, &mut backlog, &models_dir);
    // No engine in this build (host, desktop preview): record only.
    #[cfg(not(feature = "nemo"))]
    let result: Result<(), String> = {
        let _ = &models_dir;
        Err("this build has no speech engine (feature `nemo`): recording only".into())
    };
    if let Err(e) = result {
        shared.engine_failed(e);
        // Record only from here: keep reading so the session can finish
        // (the audio is in mic.opus).
        let _ = drain(&shared, &mut backlog);
    }
}

/// Reads and discards the backlog until the session ends.
fn drain(shared: &Shared, backlog: &mut BacklogReader) -> Result<(), String> {
    while pump(shared, backlog, |_| Ok(Vec::new()), || Ok(Vec::new()))? != Flow::Finished {}
    Ok(())
}

/// How [`pump`] ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Flow {
    /// The session stopped and the backlog is done.
    Finished,
    /// A step was suspect (the app left the foreground while it ran): its
    /// results were dropped; the caller reopens its streams and rewinds.
    Reset,
}

/// Steps the engine over the backlog; `step` gets each chunk of audio and
/// `finish` runs once the session stopped and everything was read. Their
/// updates are committed to the session unless the step was suspect.
pub fn pump(
    shared: &Shared,
    backlog: &mut BacklogReader,
    mut step: impl FnMut(&[f32]) -> Result<Vec<Update>, String>,
    finish: impl FnOnce() -> Result<Vec<Update>, String>,
) -> Result<Flow, String> {
    let gate = &shared.gate;
    let mut pcm = Vec::new();
    loop {
        if !gate.enter(POLL) {
            continue;
        }
        let behind = backlog.available();
        let max = if behind > LIVE_STEP as u64 * 2 {
            CATCH_UP_STEP
        } else {
            LIVE_STEP
        };
        let n = match backlog.read(max, &mut pcm) {
            Ok(n) => n,
            Err(e) => {
                gate.leave();
                return Err(format!("backlog: {e}"));
            }
        };
        if n == 0 {
            if gate.stopping() && shared.capture_done() {
                let r = finish();
                if gate.leave() {
                    return Ok(Flow::Reset);
                }
                shared.commit(r?, backlog.position(), true);
                return Ok(Flow::Finished);
            }
            gate.leave();
            std::thread::sleep(POLL);
            continue;
        }
        let t = Instant::now();
        let r = step(&pcm);
        if gate.leave() {
            // Redone after the reset: not counted as processed.
            return Ok(Flow::Reset);
        }
        shared.engine_progress(n, t.elapsed(), backlog.position());
        shared.commit(r?, backlog.position(), false);
    }
}

/// Reads a 16 kHz mono 16-bit PCM WAV file (the self-test input).
pub fn read_wav_16k(bytes: &[u8]) -> Result<Vec<f32>, String> {
    let bad = |why: &str| Err(format!("not a 16 kHz mono 16-bit WAV: {why}"));
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return bad("no RIFF/WAVE header");
    }
    let (mut at, mut format_ok) = (12, false);
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &bytes[at + 8..(at + 8 + len).min(bytes.len())];
        if id == b"fmt " && body.len() >= 16 {
            let u16_at = |i: usize| u16::from_le_bytes([body[i], body[i + 1]]);
            let rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
            format_ok = u16_at(0) == 1 && u16_at(2) == 1 && rate == SAMPLE_RATE && u16_at(14) == 16;
        } else if id == b"data" {
            if !format_ok {
                return bad("format");
            }
            return Ok(body
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
                .collect());
        }
        at += 8 + len + (len & 1);
    }
    bad("no data chunk")
}

/// Self-test result (`Documents/selftest-<unix time>.json`).
#[cfg_attr(not(feature = "nemo"), allow(dead_code))]
#[derive(Debug, serde::Serialize)]
pub struct SelfTest {
    pub file: String,
    pub audio_s: f64,
    pub model_load_s: f64,
    pub compute_s: f64,
    pub rtf: f64,
    /// Seconds from the first push to the first final line.
    pub first_final_s: Option<f64>,
    pub lines: Vec<crate::session::Line>,
    pub error: Option<String>,
}

#[cfg(feature = "nemo")]
pub use nemo::selftest;

#[cfg(feature = "nemo")]
mod nemo {
    use std::cell::RefCell;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use ghi_speech::nemo::{Asr, AsrConfig, AsrOptions, Device, DiarConfig, Diarizer};
    use ghi_speech::{AsrStream, DiarStream};

    use super::{Flow, MODELS, POLL, SAMPLE_RATE, Update, majority_speaker, model_file, pump};
    use crate::backlog::BacklogReader;
    use crate::session::Shared;

    struct Models {
        asr: Asr,
        diar: Diarizer,
    }

    /// Loaded on the first recording (in the foreground) and kept, unless a
    /// load or step overlapped a move to the background.
    static CACHE: Mutex<Option<Arc<Models>>> = Mutex::new(None);

    /// Metal on devices; the simulator runs the CPU backend.
    const DEVICE: Device = if cfg!(target_abi = "sim") {
        Device::Cpu
    } else {
        Device::Gpu
    };

    fn models(dir: &Path) -> Result<Arc<Models>, String> {
        let mut slot = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = slot.as_ref() {
            return Ok(m.clone());
        }
        let path = |id: &str| {
            model_file(dir, id).ok_or_else(|| {
                format!("model {id} is missing: push it with apps/mobile/scripts/push-models.sh")
            })
        };
        let asr = Asr::new(&AsrConfig {
            model: path(MODELS[0])?,
            device: DEVICE,
            chunk_ms: Some(1120),
            endpointing: true,
        })
        .map_err(|e| e.to_string())?;
        let diar = Diarizer::new(&DiarConfig {
            model: path(MODELS[1])?,
            device: DEVICE,
            preset: Some("v3-streaming".into()),
        })
        .map_err(|e| e.to_string())?;
        let m = Arc::new(Models { asr, diar });
        *slot = Some(m.clone());
        Ok(m)
    }

    fn forget_models() {
        CACHE.lock().unwrap_or_else(|e| e.into_inner()).take();
    }

    pub fn run(shared: &Shared, backlog: &mut BacklogReader, dir: &Path) -> Result<(), String> {
        loop {
            // Loading uploads weights to the GPU: only while the app is active.
            while !shared.gate.enter(POLL) {}
            if shared.gate.take_poison() {
                // A load or step overlapped a move to the background: some of
                // its GPU work (possibly weight uploads) may have been refused.
                forget_models();
                shared.models_unloaded();
            }
            let loaded = shared.timed_load(|| models(dir));
            if shared.gate.leave() {
                // The load itself may be incomplete: load again when active.
                continue;
            }
            let m = loaded?;
            match live(shared, backlog, &m)? {
                Flow::Finished => return Ok(()),
                Flow::Reset => {
                    // Redo from the end of the last committed line; the new
                    // streams start there (and renumber speakers).
                    let from = shared.committed_position();
                    backlog.rewind(from);
                    shared.engine_reset(from);
                }
            }
        }
    }

    fn live(shared: &Shared, backlog: &mut BacklogReader, m: &Models) -> Result<Flow, String> {
        let err = |e: ghi_speech::SpeechError| e.to_string();
        // Opening streams may touch the GPU too.
        while !shared.gate.enter(POLL) {}
        let opened = (|| {
            Ok::<_, String>((
                m.asr.stream(&AsrOptions::default()).map_err(err)?,
                m.diar.stream().map_err(err)?,
            ))
        })();
        if shared.gate.leave() {
            return Ok(Flow::Reset);
        }
        let (asr, diar) = opened?;
        let (asr, diar) = (RefCell::new(asr), RefCell::new(diar));
        let collect = || -> Result<Vec<Update>, String> {
            let mut out = Vec::new();
            let mut asr = asr.borrow_mut();
            while let Some(r) = asr.next_result().map_err(err)? {
                if !r.is_final {
                    out.push(Update::Partial(r.text));
                    continue;
                }
                let text = r.text.trim().to_string();
                if text.is_empty() {
                    continue;
                }
                let (start, end) = match (r.words.first(), r.words.last()) {
                    (Some(a), Some(b)) => (a.start, b.end),
                    _ => (r.audio_processed, r.audio_processed),
                };
                let segs = diar.borrow().segments().map_err(err)?;
                let speaker = majority_speaker(&segs, start, end);
                out.push(Update::Final {
                    start,
                    end,
                    speaker,
                    text,
                });
            }
            Ok(out)
        };
        pump(
            shared,
            backlog,
            |pcm| {
                asr.borrow_mut().push(pcm, SAMPLE_RATE).map_err(err)?;
                diar.borrow_mut().push(pcm, SAMPLE_RATE).map_err(err)?;
                collect()
            },
            || {
                asr.borrow_mut().finish().map_err(err)?;
                diar.borrow_mut().finish().map_err(err)?;
                collect()
            },
        )
    }

    /// Runs `pcm` through fresh ASR + diarization streams as fast as possible
    /// (throughput on this device), with the models in `dir`.
    pub fn selftest(dir: &Path, file: String, pcm: &[f32]) -> super::SelfTest {
        use std::time::Instant;
        let mut out = super::SelfTest {
            file,
            audio_s: pcm.len() as f64 / SAMPLE_RATE as f64,
            model_load_s: 0.0,
            compute_s: 0.0,
            rtf: 0.0,
            first_final_s: None,
            lines: Vec::new(),
            error: None,
        };
        let t = Instant::now();
        let m = match models(dir) {
            Ok(m) => m,
            Err(e) => {
                out.error = Some(e);
                return out;
            }
        };
        out.model_load_s = t.elapsed().as_secs_f64();
        let err = |e: ghi_speech::SpeechError| e.to_string();
        let run = |out: &mut super::SelfTest| -> Result<(), String> {
            let mut asr = m.asr.stream(&AsrOptions::default()).map_err(err)?;
            let mut diar = m.diar.stream().map_err(err)?;
            let t = Instant::now();
            let mut results = Vec::new();
            let collect = |asr: &mut dyn AsrStream, results: &mut Vec<_>| -> Result<(), String> {
                while let Some(r) = asr.next_result().map_err(err)? {
                    if r.is_final && !r.text.trim().is_empty() {
                        results.push((t.elapsed().as_secs_f64(), r));
                    }
                }
                Ok(())
            };
            for chunk in pcm.chunks(8_000) {
                asr.push(chunk, SAMPLE_RATE).map_err(err)?;
                diar.push(chunk, SAMPLE_RATE).map_err(err)?;
                collect(&mut asr, &mut results)?;
            }
            asr.finish().map_err(err)?;
            diar.finish().map_err(err)?;
            collect(&mut asr, &mut results)?;
            out.compute_s = t.elapsed().as_secs_f64();
            out.rtf = out.compute_s / out.audio_s.max(1e-9);
            out.first_final_s = results.first().map(|(s, _)| *s);
            let segs = diar.segments().map_err(err)?;
            for (_, r) in results {
                let (start, end) = match (r.words.first(), r.words.last()) {
                    (Some(a), Some(b)) => (a.start, b.end),
                    _ => (r.audio_processed, r.audio_processed),
                };
                out.lines.push(crate::session::Line {
                    start,
                    end,
                    speaker: majority_speaker(&segs, start, end),
                    text: r.text.trim().to_string(),
                });
            }
            Ok(())
        };
        if let Err(e) = run(&mut out) {
            out.error = Some(e);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(speaker: u32, start: f64, end: f64) -> SpeakerSegment {
        SpeakerSegment {
            start,
            end,
            speaker,
        }
    }

    fn final_line(start: f64, end: f64, text: &str) -> Update {
        Update::Final {
            start,
            end,
            speaker: Some(1),
            text: text.into(),
        }
    }

    fn setup(name: &str, seconds: usize) -> (Arc<Shared>, BacklogReader, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ghi-engine-{name}-{}", std::process::id()));
        let shared = crate::session::test_shared(&dir);
        let (mut w, r, _) = crate::backlog::create(&dir.join("backlog.pcm")).unwrap();
        w.push(&vec![0.1; seconds * SAMPLE_RATE as usize]).unwrap();
        w.publish().unwrap();
        shared.finish_capture_for_test();
        (shared, r, dir)
    }

    #[test]
    fn pump_commits_step_results_and_finishes() {
        let (shared, mut backlog, dir) = setup("commit", 1);
        let mut steps = 0;
        let flow = pump(
            &shared,
            &mut backlog,
            |_| {
                steps += 1;
                Ok(if steps == 1 {
                    vec![Update::Partial("hel".into()), final_line(0.0, 0.1, "hello")]
                } else {
                    Vec::new()
                })
            },
            || Ok(vec![final_line(0.5, 0.9, "tail")]),
        )
        .unwrap();
        assert_eq!(flow, Flow::Finished);
        let texts: Vec<_> = shared
            .lines_for_test()
            .into_iter()
            .map(|l| l.text)
            .collect();
        assert_eq!(texts, ["hello", "tail"]);
        assert_eq!(
            shared.committed_position(),
            SAMPLE_RATE as u64,
            "nothing pending"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_step_that_ends_while_inactive_is_dropped_and_redone() {
        let (shared, mut backlog, dir) = setup("reset", 2);
        // First step: an utterance in progress after a committed line at 0.05 s.
        let mut steps = 0;
        let flow = pump(
            &shared,
            &mut backlog,
            |_| {
                steps += 1;
                match steps {
                    1 => Ok(vec![
                        final_line(0.0, 0.05, "one"),
                        Update::Partial("tw".into()),
                    ]),
                    // The app resigns active while this step runs.
                    _ => {
                        shared.gate.suspend();
                        Ok(vec![final_line(0.1, 0.3, "two (suspect)")])
                    }
                }
            },
            || Ok(Vec::new()),
        )
        .unwrap();
        assert_eq!(flow, Flow::Reset);
        let texts: Vec<_> = shared
            .lines_for_test()
            .into_iter()
            .map(|l| l.text)
            .collect();
        assert_eq!(texts, ["one"], "the suspect step's line was dropped");
        let from = shared.committed_position();
        assert_eq!(
            from,
            (0.05 * SAMPLE_RATE as f64) as u64,
            "redo from the end of `one`"
        );
        backlog.rewind(from);
        shared.engine_reset(from);
        shared.gate.resume();
        let flow = pump(
            &shared,
            &mut backlog,
            |_| Ok(Vec::new()),
            || Ok(vec![final_line(0.05, 0.25, "two")]),
        )
        .unwrap();
        assert_eq!(flow, Flow::Finished);
        let lines = shared.lines_for_test();
        assert_eq!(lines[1].text, "two");
        assert!(
            (lines[1].start - 0.1).abs() < 1e-6,
            "offset by the rewind position"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_16k_mono_wav_only() {
        let mut wav = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        wav.extend(16u32.to_le_bytes());
        wav.extend([1, 0, 1, 0]); // PCM, mono
        wav.extend(16_000u32.to_le_bytes());
        wav.extend(32_000u32.to_le_bytes());
        wav.extend([2, 0, 16, 0]);
        wav.extend(b"data");
        wav.extend(4u32.to_le_bytes());
        wav.extend([0x00, 0x40, 0x00, 0xc0]);
        let pcm = read_wav_16k(&wav).unwrap();
        assert_eq!(pcm, vec![0.5, -0.5]);
        let mut stereo = wav.clone();
        stereo[22] = 2;
        assert!(read_wav_16k(&stereo).is_err());
        assert!(read_wav_16k(b"nope").is_err());
    }

    #[test]
    fn majority_speaker_by_overlap() {
        let segs = [seg(1, 0.0, 2.0), seg(2, 2.0, 5.0), seg(1, 5.0, 6.0)];
        assert_eq!(majority_speaker(&segs, 1.0, 4.0), Some(2));
        assert_eq!(majority_speaker(&segs, 0.0, 2.5), Some(1));
        assert_eq!(majority_speaker(&segs, 7.0, 8.0), None);
        assert_eq!(majority_speaker(&[], 0.0, 1.0), None);
    }

    #[test]
    fn model_file_checks_the_pinned_size() {
        let dir = std::env::temp_dir().join(format!("ghi-mobile-models-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let m = ghi_models::find(MODELS[1]).unwrap();
        std::fs::write(ghi_models::path_in(&dir, &m), b"short").unwrap();
        assert_eq!(model_file(&dir, MODELS[1]), None);
        assert_eq!(model_file(&dir, "no-such-model"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
