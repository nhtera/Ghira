// SPDX-License-Identifier: Apache-2.0
//! The final pass (doc 02 §E, RT-2): after a meeting (or an import), the
//! stored audio is re-diarized and re-transcribed with the final config
//! (1120 ms chunks, doc 06), and the result replaces the live transcript as
//! version 2, keeping what the user did.
//!
//! Stages: Decoding → RefiningSpeakers (diarization, whole track) →
//! ImprovingTranscript (ASR in ~10-min chunks cut at the quietest moment
//! near each boundary) → MatchingVoices (v1 → v2 carry-over: Hungarian on
//! time overlap; names, Me and "not a person" follow the speaker) → custom
//! vocabulary → transcript v2 → `notes_final` queued.
//!
//! - Lines the user edited are kept as written; v2 lines under them are dropped.
//! - Call mode: the mic track is Me; the far track is diarized. Room mode:
//!   the mic track is diarized.
//! - Voice step (14c, `voice_step`): after the carry-over, unnamed clusters
//!   are matched with Me's profile (room mode) and, with the third-party flag
//!   on, others'; Me learns from the mic. Never blocks the pass.
//! - Preempted by a recording (or the app leaving the foreground, or killed),
//!   the pass resumes later: each finished ASR chunk of a track and the
//!   diarization are saved sealed in the store (`final_pass_ckpt`, keyed by a
//!   stamp of the audio, engine and chunking) and skipped on the next run;
//!   only the cheap global steps (alignment, voice carry-over, voice step) are
//!   redone. Chunks are at most [`CHECKPOINT_CHUNK_S`] long, so a yield loses
//!   seconds of work, not the whole pass. The checkpoints go when the pass
//!   stores its transcript (or fails), with the audio, and with the meeting.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;

use ghi_audio::{SAMPLE_RATE, Track};
use ghi_speech::{SpeakerSegment, Word};
use ghi_store::store::{NewSegment, NewSpeaker, Store, TrackKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::aligner;
use crate::carry::{self, Turn};
use crate::engines::SpeechEngines;
use crate::events::Stage;
use crate::jobs::{JobCtx, JobHandler, Outcome};
use crate::live::line_language;
use crate::notes_job::NOTES_FINAL_JOB;
use crate::recluster;
use crate::session::JOB_PAYLOAD_VERSION;
use crate::speakers::COLOR_ORDER;
use crate::vocab::Vocabulary;
use crate::voice_step::{Input as VoiceInput, VoiceStep};

const RATE: f64 = SAMPLE_RATE as f64;
/// Store setting holding the custom vocabulary (a JSON list of strings).
pub const VOCABULARY_SETTING: &str = crate::vocab::TERMS_SETTING;

/// Loads engines with the final-pass config.
pub type EnginesFactory = Arc<dyn Fn() -> Result<Arc<dyn SpeechEngines>, String> + Send + Sync>;

pub struct FinalPassJob {
    pub engines: EnginesFactory,
    /// The speech models are installed (else the job waits for them).
    pub ready: crate::jobs::Ready,
    /// Target ASR chunk length (seconds).
    pub chunk_s: f64,
    /// Voice matching (phase 14c); `None` leaves speakers as carried over.
    pub voice: Option<VoiceStep>,
}

/// A v2 line before it is stored.
#[derive(Debug, Clone)]
struct Line {
    track: Track,
    /// Diarization label (diarized track) or `None` (Me / unknown).
    label: Option<u32>,
    me: bool,
    t0_ms: i64,
    t1_ms: i64,
    text: String,
    words: Vec<(i64, i64, Option<f32>)>,
    confidence: Option<f32>,
    /// Another speaker talked over this line.
    overlap: bool,
}

fn ms(s: f64) -> i64 {
    (s * 1000.0).round() as i64
}

/// Chunk boundaries (sample indices) near every `chunk` samples, moved to
/// the quietest 100 ms within ±15 s so no word is cut.
pub fn cut_points(pcm: &[f32], chunk_s: f64) -> Vec<usize> {
    let win = (RATE * 0.1) as usize;
    let chunk = (RATE * chunk_s) as usize;
    let search = (RATE * 15.0) as usize;
    let mut cuts = vec![0];
    let mut next = chunk;
    while next + search < pcm.len() {
        let lo = next.saturating_sub(search);
        let hi = (next + search).min(pcm.len() - win);
        let mut best = (f32::INFINITY, next);
        let mut i = lo;
        while i < hi {
            let e: f32 = pcm[i..i + win].iter().map(|x| x * x).sum();
            if e < best.0 {
                best = (e, i + win / 2);
            }
            i += win;
        }
        cuts.push(best.1);
        next = best.1 + chunk;
    }
    cuts.push(pcm.len());
    cuts
}

/// The longest ASR chunk (seconds) the pass uses, whatever the engine or
/// config asks for: a chunk is the unit that is saved and skipped on resume,
/// and the work a yield loses (cuts still land on the quietest moment).
pub const CHECKPOINT_CHUNK_S: f64 = 60.0;

/// Bumped when what a checkpoint holds changes shape.
const CKPT_VERSION: u32 = 1;

/// A final ASR result: its words on the track's timeline.
type Final = Vec<Word>;

/// Progress of the stages on the job's 0..1 scale (the UI shows it as the
/// percent): decoding, diarization, ASR, matching.
const P_DIAR: (f32, f32) = (0.02, 0.12);
const P_ASR: (f32, f32) = (0.12, 0.97);
const P_MATCH: f32 = 0.97;

/// What a job has saved: parts finished (all runs), parts in all, and the
/// progress shown; the job's resume payload (numbers only).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Mark {
    done: u64,
    total: u64,
    p: f32,
    /// Which checkpoints `done` counts (the stamp's first 64 bits): when it
    /// changes (another audio or engine) the runner starts its accounting over.
    stamp_id: u64,
}

impl Mark {
    fn from_payload(v: &serde_json::Value) -> Mark {
        Mark {
            done: v["done"].as_u64().unwrap_or(0),
            total: v["total"].as_u64().unwrap_or(0),
            p: v["p"].as_f64().unwrap_or(0.0) as f32,
            stamp_id: v["stamp"].as_u64().unwrap_or(0),
        }
    }

    fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "done": self.done,
            "total": self.total,
            "p": self.p,
            "stamp": self.stamp_id,
        })
    }
}

#[derive(Serialize, Deserialize)]
struct CkWord {
    t: String,
    /// Start, end (`f64::to_bits`) and confidence (`f32::to_bits`): exact.
    s: u64,
    e: u64,
    c: u32,
    k: Option<u32>,
}

fn encode_finals(finals: &[Final]) -> Vec<u8> {
    let v: Vec<Vec<CkWord>> = finals
        .iter()
        .map(|f| {
            f.iter()
                .map(|w| CkWord {
                    t: w.text.clone(),
                    s: w.start.to_bits(),
                    e: w.end.to_bits(),
                    c: w.confidence.to_bits(),
                    k: w.speaker,
                })
                .collect()
        })
        .collect();
    serde_json::to_vec(&v).unwrap_or_default()
}

fn decode_finals(b: &[u8]) -> Option<Vec<Final>> {
    let v: Vec<Vec<CkWord>> = serde_json::from_slice(b).ok()?;
    Some(
        v.into_iter()
            .map(|f| {
                f.into_iter()
                    .map(|w| Word {
                        text: w.t,
                        start: f64::from_bits(w.s),
                        end: f64::from_bits(w.e),
                        confidence: f32::from_bits(w.c),
                        speaker: w.k,
                    })
                    .collect()
            })
            .collect(),
    )
}

fn encode_segs(segs: &[SpeakerSegment]) -> Vec<u8> {
    let v: Vec<(u64, u64, u32)> = segs
        .iter()
        .map(|s| (s.start.to_bits(), s.end.to_bits(), s.speaker))
        .collect();
    serde_json::to_vec(&v).unwrap_or_default()
}

fn decode_segs(b: &[u8]) -> Option<Vec<SpeakerSegment>> {
    let v: Vec<(u64, u64, u32)> = serde_json::from_slice(b).ok()?;
    Some(
        v.into_iter()
            .map(|(start, end, speaker)| SpeakerSegment {
                start: f64::from_bits(start),
                end: f64::from_bits(end),
                speaker,
            })
            .collect(),
    )
}

/// Names what the checkpoints are computed from: this build's checkpoint
/// shape, the engine and its models, the language, the chunking and the audio
/// itself (so a discard or trim, which change the audio, invalidate them).
fn pass_stamp(
    engine: &str,
    language: Option<&str>,
    chunk_s: f64,
    pcm: &HashMap<Track, Vec<f32>>,
) -> String {
    let mut h = Sha256::new();
    h.update(b"ghi-final-pass-ckpt");
    h.update(CKPT_VERSION.to_le_bytes());
    h.update(engine.as_bytes());
    h.update([0]);
    h.update(language.unwrap_or("").as_bytes());
    h.update([0]);
    h.update(chunk_s.to_bits().to_le_bytes());
    let mut tracks: Vec<&Track> = pcm.keys().collect();
    tracks.sort_by_key(|t| t.index());
    let mut buf = Vec::with_capacity(16 * 1024);
    for t in tracks {
        h.update((t.index() as u64).to_le_bytes());
        h.update((pcm[t].len() as u64).to_le_bytes());
        for block in pcm[t].chunks(4096) {
            buf.clear();
            buf.extend(block.iter().flat_map(|x| x.to_le_bytes()));
            h.update(&buf);
        }
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The checkpoints of one run: what earlier runs saved, and saving what this
/// one finishes. Saving never fails the pass (a part that could not be saved
/// is computed again next time).
struct Ckpt<'a> {
    ctx: &'a JobCtx<'a>,
    meeting: &'a str,
    stamp: String,
    saved: HashMap<String, Vec<u8>>,
    mark: &'a Cell<Mark>,
}

impl Ckpt<'_> {
    fn save(&mut self, part: &str, data: Vec<u8>) {
        match self
            .ctx
            .store
            .put_pass_checkpoint(self.meeting, &self.stamp, part, &data)
        {
            Ok(true) => {
                let mut m = self.mark.get();
                m.done += 1;
                self.mark.set(m);
                self.ctx.made_progress();
                if let Err(e) = self.ctx.checkpoint(f64::from(m.p), &m.json()) {
                    log::warn!("final pass checkpoint note: {e}");
                }
            }
            // Nothing was written (no audio, or a sensitive meeting): not progress.
            Ok(false) => {}
            Err(e) => log::warn!("final pass checkpoint not saved: {e}"),
        }
    }

    /// Shows `p` (never less than shown before, also across restarts).
    fn show(&self, stage: Stage, p: f32) {
        let mut m = self.mark.get();
        m.p = m.p.max(p);
        self.mark.set(m);
        self.ctx.progress(Some(stage), m.p);
    }
}

/// The ASR chunks of every track: `(track, cut points)`, in track order.
type Plan = Vec<(Track, Vec<usize>)>;

/// Transcribes the chunks of one track; words on the track's timeline
/// (seconds). Finished chunks come from the checkpoints, the others are run
/// and saved as they finish. `None`: preempted (what finished is saved).
fn transcribe(
    engines: &dyn SpeechEngines,
    pcm: &[f32],
    track: Track,
    cuts: &[usize],
    language: Option<&str>,
    ck: &mut Ckpt,
    progress: &Cell<(usize, usize)>,
) -> Result<Option<Vec<Final>>, String> {
    let ctx = ck.ctx;
    let mut out = Vec::new();
    for (idx, w) in cuts.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        let part = format!("asr.{}.{idx}", track.index());
        let advance = |ck: &Ckpt| {
            let (done, total) = progress.get();
            let done = done + (b - a);
            progress.set((done, total));
            ck.show(
                Stage::ImprovingTranscript,
                P_ASR.0 + (P_ASR.1 - P_ASR.0) * done as f32 / total.max(1) as f32,
            );
        };
        if let Some(finals) = ck.saved.get(&part).and_then(|b| decode_finals(b)) {
            out.extend(finals);
            advance(ck);
            continue;
        }
        if ctx.preempted() {
            return Ok(None);
        }
        let mut chunk_out: Vec<Final> = Vec::new();
        let off = a as f64 / RATE;
        let mut asr = engines.asr(language).map_err(|e| e.to_string())?;
        let collect =
            |asr: &mut crate::engines::BoxAsr, out: &mut Vec<Final>| -> Result<(), String> {
                while let Some(r) = asr.next_result().map_err(|e| e.to_string())? {
                    if !r.is_final || r.text.trim().is_empty() {
                        continue;
                    }
                    let mut words: Vec<Word> = r
                        .words
                        .iter()
                        .map(|w| Word {
                            start: w.start + off,
                            end: w.end + off,
                            ..w.clone()
                        })
                        .collect();
                    if words.is_empty() {
                        let end = off + r.audio_processed;
                        words.push(Word {
                            text: r.text.trim().to_string(),
                            start: (end - 1.0).max(off),
                            end,
                            confidence: 1.0,
                            speaker: None,
                        });
                    }
                    out.push(words);
                }
                Ok(())
            };
        for block in pcm[a..b].chunks(SAMPLE_RATE as usize) {
            if ctx.preempted() {
                return Ok(None);
            }
            asr.push(block, SAMPLE_RATE).map_err(|e| e.to_string())?;
            collect(&mut asr, &mut chunk_out)?;
        }
        // Engines that decode at the end (Whisper) stop mid-chunk on preemption.
        if !asr
            .finish_abortable(&|| ctx.preempted())
            .map_err(|e| e.to_string())?
        {
            return Ok(None);
        }
        collect(&mut asr, &mut chunk_out)?;
        ck.save(&part, encode_finals(&chunk_out));
        out.extend(chunk_out);
        advance(ck);
    }
    Ok(Some(out))
}

fn line_from(track: Track, label: Option<u32>, me: bool, l: &aligner::AlignedLine) -> Line {
    let n = l.words.len().max(1) as f32;
    Line {
        track,
        label,
        me,
        t0_ms: ms(l.start),
        t1_ms: ms(l.end),
        text: l.text(),
        words: l
            .words
            .iter()
            .map(|w| (ms(w.word.start), ms(w.word.end), Some(w.word.confidence)))
            .collect(),
        confidence: Some(l.words.iter().map(|w| w.word.confidence).sum::<f32>() / n),
        overlap: l.overlap(),
    }
}

impl JobHandler for FinalPassJob {
    fn kind(&self) -> &'static str {
        crate::session::FINAL_PASS_JOB
    }

    fn ready(&self) -> bool {
        (self.ready)()
    }

    /// The live transcript and its notes stay; the meeting is usable.
    fn failed(&self, ctx: &JobCtx) {
        if let Ok(m) = ctx.meeting() {
            let _ = ctx.store.clear_pass_checkpoints(m);
            let _ = ctx.store.set_meeting_status(m, "ready");
        }
    }

    fn run(&self, ctx: &JobCtx) -> Result<Outcome, String> {
        self.run_with(ctx, true)
    }
}

/// [`FinalPassJob`] that never queues `notes_final` (the phone: no local
/// LLM; notes only through the cloud send sheet). With no notes job to settle
/// the meeting, the pass itself sets it `ready` and sends
/// `StateChanged { Ready }` (also from `failed`). The phone registers
/// `VoiceLearnJob` beside it whenever Me is enrolled (the voice step queues
/// `voice_learn`).
pub struct FinalPassNoNotes(pub FinalPassJob);

impl JobHandler for FinalPassNoNotes {
    fn kind(&self) -> &'static str {
        self.0.kind()
    }

    fn ready(&self) -> bool {
        self.0.ready()
    }

    fn failed(&self, ctx: &JobCtx) {
        self.0.failed(ctx);
        if let Ok(m) = ctx.meeting() {
            announce_ready(ctx, m);
        }
    }

    fn run(&self, ctx: &JobCtx) -> Result<Outcome, String> {
        self.0.run_with(ctx, false)
    }
}

impl FinalPassJob {
    /// The pass; `notes` queues `notes_final` once the transcript is stored.
    fn run_with(&self, ctx: &JobCtx, notes: bool) -> Result<Outcome, String> {
        // What the job has saved (from an earlier run's payload to begin
        // with); a yield hands it back so the runner can tell progress from
        // spinning.
        let mark = Cell::new(Mark::from_payload(&ctx.job.payload));
        let restart = || Ok(Outcome::Yield(mark.get().json()));
        let meeting = ctx.meeting()?.to_string();
        let store: &Arc<Store> = ctx.store;
        let err = |e: ghi_store::StoreError| e.to_string();
        let m = store.get_meeting(&meeting).map_err(err)?;
        // A sensitive meeting keeps no audio: there is nothing to re-read. It
        // settles like a meeting without audio, so it never stays "processing".
        if m.sensitive {
            if notes {
                queue_notes(store, &meeting)?;
            } else {
                settle_ready(ctx, &meeting).map_err(err)?;
            }
            return Ok(Outcome::Done);
        }
        let call = m.mode == "call";
        // Speakers an earlier, yielded run added and never used (unnamed, no
        // lines) would pile up with every yield.
        store.remove_orphan_speakers(&meeting).map_err(err)?;

        // Decoding.
        ctx.progress(Some(Stage::Decoding), mark.get().p);
        let mut pcm: HashMap<Track, Vec<f32>> = HashMap::new();
        for (kind, _) in store.tracks(&meeting).map_err(err)? {
            if ctx.preempted() {
                return restart();
            }
            let track = match kind {
                TrackKind::Mic | TrackKind::File => Track::Mic,
                TrackKind::System => Track::System,
            };
            let bundle = store.open_bundle(&meeting, kind).map_err(err)?;
            let ogg = bundle.read_all().map_err(err)?;
            let audio = ghi_audio::encoder::read_ogg_opus(&ogg[..]).map_err(|e| e.to_string())?;
            pcm.insert(track, audio);
        }
        if pcm.is_empty() {
            if notes {
                queue_notes(store, &meeting)?;
            } else {
                settle_ready(ctx, &meeting).map_err(err)?;
            }
            return Ok(Outcome::Done);
        }
        let audio_ms =
            pcm.values().map(Vec::len).max().unwrap_or(0) as i64 * 1000 / i64::from(SAMPLE_RATE);
        log::info!(
            "final pass decoded audio_ms={audio_ms} tracks={}",
            pcm.len()
        );
        if audio_ms > m.duration_ms {
            store
                .extend_meeting_duration(&meeting, audio_ms)
                .map_err(err)?;
        }
        if ctx.preempted() {
            return restart();
        }
        let engines = (self.engines)()?;
        let language = m.lang.as_deref();
        let chunk_s = engines
            .final_chunk_s()
            .unwrap_or(self.chunk_s)
            .min(CHECKPOINT_CHUNK_S);
        let mut tracks: Vec<Track> = pcm.keys().copied().collect();
        tracks.sort_by_key(|t| t.index());
        let plan: Plan = tracks
            .iter()
            .map(|&t| (t, cut_points(&pcm[&t], chunk_s)))
            .collect();

        // RefiningSpeakers: diarize the far track (call) or the room mic.
        let diar_track = if call && pcm.contains_key(&Track::System) {
            Track::System
        } else {
            Track::Mic
        };
        // The stamp ties checkpoints to this audio, engine and chunking.
        let stamp = pass_stamp(&engines.checkpoint_id(), language, chunk_s, &pcm);
        let saved = match store.pass_checkpoints(&meeting, &stamp) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("final pass checkpoints unreadable: {e}");
                HashMap::new()
            }
        };
        let asr_samples: usize = plan
            .iter()
            .map(|(_, c)| c.last().copied().unwrap_or(0))
            .sum();
        let asr_parts: u64 = plan
            .iter()
            .map(|(_, c)| c.len().saturating_sub(1) as u64)
            .sum();
        // A multi-track import knows who spoke when (each participant's own
        // track): those spans stand in for the diarizer, which is not run.
        let known = store.track_speakers(&meeting).map_err(err)?;
        let from_tracks = !known.is_empty();
        let mut m0 = mark.get();
        m0.total = asr_parts + u64::from(!from_tracks);
        m0.done = saved.len() as u64;
        m0.stamp_id = u64::from_str_radix(&stamp[..16], 16).unwrap_or(0);
        if saved.is_empty() {
            // Nothing carried over (another audio or engine, or a first run).
            m0.p = 0.0;
        }
        mark.set(m0);
        let mut ck = Ckpt {
            ctx,
            meeting: &meeting,
            stamp,
            saved,
            mark: &mark,
        };
        ck.show(Stage::RefiningSpeakers, P_DIAR.0);
        let spans: Vec<TrackSpan> = known
            .iter()
            .enumerate()
            .flat_map(|(i, k)| {
                k.spans.iter().map(move |s| TrackSpan {
                    label: i as u32 + 1,
                    t0_ms: s[0],
                    t1_ms: s[1],
                })
            })
            .collect();
        let expected = store
            .expected_speakers(&meeting)
            .ok()
            .flatten()
            .map(|n| n as usize);
        let voice_ready = self.voice.as_ref().is_some_and(|v| (v.ready)());
        // What the diarization depends on besides the audio: the speaker model
        // and the expected count (the re-clustering).
        let diar_part = format!(
            "diar.{}.{}.{}",
            diar_track.index(),
            expected.map_or(-1, |n| n as i64),
            if voice_ready {
                voice_model_key()
            } else {
                "-".to_string()
            }
        );
        let segs: Vec<SpeakerSegment> = if from_tracks {
            known
                .iter()
                .enumerate()
                .flat_map(|(i, k)| {
                    k.spans.iter().map(move |s| SpeakerSegment {
                        start: s[0] as f64 / 1000.0,
                        end: s[1] as f64 / 1000.0,
                        speaker: i as u32 + 1,
                    })
                })
                .collect()
        } else if let Some(segs) = ck.saved.get(&diar_part).and_then(|b| decode_segs(b)) {
            ck.show(Stage::RefiningSpeakers, P_DIAR.1);
            segs
        } else {
            let mut diar = engines.diar().map_err(|e| e.to_string())?;
            let blocks = pcm[&diar_track].chunks(10 * SAMPLE_RATE as usize);
            let n = blocks.len().max(1) as f32;
            for (i, block) in blocks.enumerate() {
                if ctx.preempted() {
                    return restart();
                }
                diar.push(block, SAMPLE_RATE).map_err(|e| e.to_string())?;
                ck.show(
                    Stage::RefiningSpeakers,
                    P_DIAR.0 + (P_DIAR.1 - P_DIAR.0) * 0.9 * (i + 1) as f32 / n,
                );
            }
            diar.finish().map_err(|e| e.to_string())?;
            let mut segs = diar.segments().map_err(|e| e.to_string())?;
            // The diarizer tops out at 8 voices: when it did, tell them apart
            // again with the speaker model (phase 14d, D12). Never blocks the pass.
            if recluster_wanted(&segs)
                && let Some(voice) = self.voice.as_ref().filter(|v| (v.ready)())
            {
                match (voice.embedder)() {
                    Ok(mut embedder) => {
                        match recluster::run(
                            &pcm[&diar_track],
                            engines.as_ref(),
                            embedder.as_mut(),
                            &segs,
                            &|| ctx.preempted(),
                            expected,
                        ) {
                            Ok(Some(better)) => segs = better,
                            Ok(None) => {}
                            Err(e) => log::warn!("recluster skipped: {} chars of error", e.len()),
                        }
                        if ctx.preempted() {
                            return restart();
                        }
                    }
                    Err(_) => log::warn!("recluster skipped: no speaker model"),
                }
            }
            ck.save(&diar_part, encode_segs(&segs));
            ck.show(Stage::RefiningSpeakers, P_DIAR.1);
            segs
        };

        // ImprovingTranscript.
        let mut lines: Vec<Line> = Vec::new();
        let progress = Cell::new((0usize, asr_samples));
        for (t, cuts) in &plan {
            let t = *t;
            let me = call && t == Track::Mic && diar_track != Track::Mic;
            let Some(finals) = transcribe(
                engines.as_ref(),
                &pcm[&t],
                t,
                cuts,
                language,
                &mut ck,
                &progress,
            )?
            else {
                return restart();
            };
            for words in finals {
                if me {
                    let l = aligner::align(&words, &[]);
                    for al in l {
                        lines.push(line_from(t, None, true, &al));
                    }
                } else {
                    for al in aligner::align(&words, &segs) {
                        let mut line = line_from(t, al.speaker, false, &al);
                        if from_tracks {
                            // Who spoke comes from the tracks: a line no span
                            // reaches goes to the nearest one, and "talked
                            // over" means two people really spoke at once
                            // for a while (the spans' trailing hangover and
                            // a quick turn-over don't count).
                            line.label = line
                                .label
                                .or_else(|| nearest_label(&spans, line.t0_ms, line.t1_ms));
                            line.overlap =
                                overlap_ms(&spans, line.t0_ms, line.t1_ms) >= TRACK_OVERLAP_MIN_MS;
                        }
                        lines.push(line);
                    }
                }
            }
        }
        drop(engines); // free the speech models before notes load the LLM
        lines.sort_by_key(|l| (l.t0_ms, l.track.index()));

        // MatchingVoices: carry live speakers over to the final clusters.
        ck.show(Stage::MatchingVoices, P_MATCH);
        let v1 = store.segments(&meeting).map_err(err)?;
        let speakers = store.speakers(&meeting).map_err(err)?;
        let me_gid = speakers
            .iter()
            .find(|s| s.is_me && s.merged_into.is_none())
            .map(|s| s.gid.clone());
        let live_idx: HashMap<String, u32> = speakers
            .iter()
            .enumerate()
            .map(|(i, s)| (s.gid.clone(), i as u32 + 1))
            .collect();
        let live_turns: Vec<Turn> = v1
            .iter()
            .filter_map(|s| {
                let g = s.speaker_gid.as_ref()?;
                if Some(g) == me_gid.as_ref() && call {
                    return None;
                }
                Some(Turn {
                    speaker: live_idx[g],
                    t0_ms: s.t0_ms,
                    t1_ms: s.t1_ms,
                })
            })
            .collect();
        let final_turns: Vec<Turn> = lines
            .iter()
            .filter_map(|l| {
                Some(Turn {
                    speaker: l.label?,
                    t0_ms: l.t0_ms,
                    t1_ms: l.t1_ms,
                })
            })
            .collect();
        let (mut label_gid, unmatched): (HashMap<u32, String>, Vec<u32>) = if from_tracks {
            // The speakers were made at import; a merged one answers for
            // the speaker it was merged into, down the chain (A into B, B
            // into C: C).
            let target = |gid: &str| {
                let mut at = gid.to_string();
                for _ in 0..speakers.len() {
                    match speakers
                        .iter()
                        .find(|s| s.gid == at)
                        .and_then(|s| s.merged_into.clone())
                    {
                        Some(next) => at = next,
                        None => break,
                    }
                }
                at
            };
            (
                known
                    .iter()
                    .enumerate()
                    .map(|(i, k)| (i as u32 + 1, target(&k.speaker_gid)))
                    .collect(),
                Vec::new(),
            )
        } else {
            let c = carry::carry_over(&live_turns, &final_turns);
            let by_idx: HashMap<u32, &String> = live_idx.iter().map(|(g, i)| (*i, g)).collect();
            (
                c.matched
                    .iter()
                    .map(|(f, l)| (*f, by_idx[l].clone()))
                    .collect(),
                c.unmatched,
            )
        };
        if ctx.preempted() {
            return restart();
        }
        // New speakers for clusters live never had ("Name your speakers").
        let first_idx = speakers.iter().map(|s| s.label_idx).max().unwrap_or(-1) + 1;
        let used: Vec<i64> = speakers.iter().map(|s| s.color_slot).collect();
        let mut free = COLOR_ORDER
            .iter()
            .map(|&c| i64::from(c))
            .filter(|c| !used.contains(c));
        for (label, idx) in unmatched.iter().zip(first_idx..) {
            let gid = store
                .add_speaker(
                    &meeting,
                    NewSpeaker {
                        label_idx: idx,
                        color_slot: free.next().unwrap_or(0),
                        ..Default::default()
                    },
                )
                .map_err(err)?;
            label_gid.insert(*label, gid);
        }
        let me_gid = match (call && lines.iter().any(|l| l.me), me_gid) {
            (true, None) => Some(
                store
                    .add_speaker(
                        &meeting,
                        NewSpeaker {
                            label_idx: -1,
                            is_me: true,
                            color_slot: free.next().unwrap_or(0),
                            ..Default::default()
                        },
                    )
                    .map_err(err)?,
            ),
            (_, g) => g,
        };

        // Edited lines stay as the user wrote them; v2 lines under them go.
        let edited: Vec<&ghi_store::store::Segment> = v1.iter().filter(|s| s.edited).collect();
        // Nothing from a discarded span comes back [RT-1] (its audio is
        // silence; this also covers anything the engine still hears there).
        let discarded = store.discarded_spans(&meeting).map_err(err)?;
        // The user's terms and the names they gave speakers (RT-14).
        let vocab = Some(Vocabulary::new(&crate::vocab::meeting_terms(
            store, &meeting,
        )?))
        .filter(|v| !v.is_empty());
        let me_spans: Vec<(i64, i64)> = lines
            .iter()
            .filter(|l| l.me)
            .filter(|l| !discarded.iter().any(|&(a, b)| a < l.t1_ms && l.t0_ms < b))
            .map(|l| (l.t0_ms, l.t1_ms))
            .collect();
        let mut overlaps: Vec<String> = Vec::new();
        let mut v2: Vec<NewSegment> = lines
            .into_iter()
            .filter(|l| {
                !edited
                    .iter()
                    .any(|e| e.t0_ms < l.t1_ms && l.t0_ms < e.t1_ms)
            })
            .filter(|l| !discarded.iter().any(|&(a, b)| a < l.t1_ms && l.t0_ms < b))
            .map(|l| {
                let text = match &vocab {
                    Some(v) => v.correct(&l.text).unwrap_or(l.text),
                    None => l.text,
                };
                let gid = ghi_store::new_gid();
                if l.overlap {
                    overlaps.push(gid.clone());
                }
                NewSegment {
                    gid: Some(gid),
                    speaker_gid: if l.me {
                        me_gid.clone()
                    } else {
                        l.label.and_then(|k| label_gid.get(&k).cloned())
                    },
                    t0_ms: l.t0_ms,
                    t1_ms: l.t1_ms,
                    lang: line_language(&text, language),
                    confidence: l.confidence,
                    words: l
                        .words
                        .into_iter()
                        .map(|(t0_ms, t1_ms, conf)| ghi_store::store::Word { t0_ms, t1_ms, conf })
                        .collect(),
                    text,
                    edited: false,
                }
            })
            .collect();
        for e in edited {
            let gid = ghi_store::new_gid();
            if e.overlap {
                overlaps.push(gid.clone());
            }
            v2.push(NewSegment {
                gid: Some(gid),
                speaker_gid: e.speaker_gid.clone(),
                t0_ms: e.t0_ms,
                t1_ms: e.t1_ms,
                text: e.text.clone(),
                lang: e.lang.clone(),
                confidence: e.confidence,
                words: store.segment_words(&e.gid).map_err(err)?,
                edited: true,
            });
        }
        v2.sort_by_key(|s| s.t0_ms);
        if ctx.preempted() {
            return restart();
        }
        // Names and Me are decided before the transcript is stored, so the
        // notes that follow see them.
        if let Some(voice) = &self.voice {
            let input = VoiceInput {
                meeting: &meeting,
                // Without a far-side track a call is a room: nobody is Me.
                call: call && diar_track != Track::Mic,
                file_source: m.source == "file",
                pcm: &pcm,
                diar_track,
                segs: &segs,
                me_spans: &me_spans,
                label_gid: &label_gid,
                v2: &v2,
            };
            if !voice.run(ctx, &input)? {
                return restart();
            }
        }
        let lines = v2.len();
        // The marks of the lines another speaker talked over go in with them.
        store
            .replace_transcript_marked(&meeting, v2, &overlaps)
            .map_err(err)?;
        log::info!("final pass stored lines={lines}");
        // Done: nothing is left to resume (and the audio-derived rows go).
        if let Err(e) = store.clear_pass_checkpoints(&meeting) {
            log::warn!("final pass checkpoints not cleared: {e}");
        }
        if notes {
            queue_notes(store, &meeting)?;
        } else {
            // No notes job follows to settle the meeting.
            settle_ready(ctx, &meeting).map_err(err)?;
        }
        ctx.progress(Some(Stage::MatchingVoices), 1.0);
        Ok(Outcome::Done)
    }
}

/// A participant's speech span on the import's timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TrackSpan {
    label: u32,
    t0_ms: i64,
    t1_ms: i64,
}

/// A line is "talked over" with at least this much simultaneous speech (ms).
const TRACK_OVERLAP_MIN_MS: i64 = 500;

/// The label whose span is nearest the middle of `[t0, t1]` (0 distance
/// inside it); `None` without spans.
fn nearest_label(spans: &[TrackSpan], t0_ms: i64, t1_ms: i64) -> Option<u32> {
    let mid = (t0_ms + t1_ms) / 2;
    spans
        .iter()
        .min_by_key(|s| {
            if mid < s.t0_ms {
                s.t0_ms - mid
            } else {
                (mid - s.t1_ms).max(0)
            }
        })
        .map(|s| s.label)
}

/// How long, within `[t0, t1]`, two or more participants speak at once. The
/// spans carry the detector's hangover on their ends, which is not speech:
/// it is taken off first.
fn overlap_ms(spans: &[TrackSpan], t0_ms: i64, t1_ms: i64) -> i64 {
    let mut edges: Vec<(i64, i32)> = Vec::new();
    for s in spans {
        let end = s.t1_ms - crate::activity::HANGOVER_MS;
        let (a, b) = (s.t0_ms.max(t0_ms), end.min(t1_ms));
        if b > a {
            edges.push((a, 1));
            edges.push((b, -1));
        }
    }
    edges.sort_unstable();
    let (mut active, mut from, mut total) = (0, 0, 0);
    for (t, d) in edges {
        if active >= 2 {
            total += t - from;
        }
        active += d;
        from = t;
    }
    total
}

/// Names the speaker model the re-clustering uses (its registry id and the
/// start of its pinned hash), so a replaced model re-runs the diarization.
fn voice_model_key() -> String {
    #[cfg(feature = "voice")]
    if let Some(m) = ghi_models::find(crate::profiles::VOICE_MODEL) {
        return format!("{}.{}", m.id, &m.sha256[..12]);
    }
    crate::profiles::VOICE_MODEL.to_string()
}

/// The diarizer returned as many voices as it can tell apart.
fn recluster_wanted(segs: &[SpeakerSegment]) -> bool {
    let mut labels: Vec<u32> = segs.iter().map(|s| s.speaker).collect();
    labels.sort_unstable();
    labels.dedup();
    labels.len() >= recluster::SATURATED_AT
}

/// Marks the meeting `ready` and tells the UI (no notes job follows).
fn settle_ready(ctx: &JobCtx, meeting: &str) -> Result<(), ghi_store::StoreError> {
    ctx.store.set_meeting_status(meeting, "ready")?;
    announce_ready(ctx, meeting);
    Ok(())
}

fn announce_ready(ctx: &JobCtx, meeting: &str) {
    ctx.events.emit(crate::events::Event::StateChanged {
        meeting: meeting.to_string(),
        state: crate::events::SessionState::Ready,
    });
}

fn queue_notes(store: &Store, meeting: &str) -> Result<(), String> {
    if store
        .active_job(meeting, NOTES_FINAL_JOB)
        .map_err(|e| e.to_string())?
        .is_none()
    {
        store
            .enqueue_job(
                Some(meeting),
                NOTES_FINAL_JOB,
                JOB_PAYLOAD_VERSION,
                &serde_json::json!({}),
            )
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_land_in_the_quietest_spot_near_each_boundary() {
        // 40 s of tone with a silent 0.5 s at 22 s; 20 s chunks.
        let mut pcm: Vec<f32> = (0..16_000 * 40)
            .map(|i| (i as f32 * 0.05).sin() * 0.3)
            .collect();
        for x in &mut pcm[16_000 * 22..16_000 * 22 + 8_000] {
            *x = 0.0;
        }
        let cuts = cut_points(&pcm, 20.0);
        assert_eq!(cuts.first(), Some(&0));
        assert_eq!(cuts.last(), Some(&pcm.len()));
        assert_eq!(cuts.len(), 3, "{cuts:?}");
        let c = cuts[1] as f64 / 16_000.0;
        assert!((22.0..22.5).contains(&c), "cut at {c}");
        assert_eq!(cut_points(&pcm[..16_000 * 10], 20.0), vec![0, 160_000]);
    }

    fn pcm_of(n: usize, f: f32) -> HashMap<Track, Vec<f32>> {
        HashMap::from([(Track::Mic, (0..n).map(|i| i as f32 * f).collect())])
    }

    #[test]
    fn the_stamp_follows_audio_engine_language_and_chunking() {
        let base = pass_stamp("e1", Some("vi"), 60.0, &pcm_of(1000, 0.5));
        assert_eq!(base, pass_stamp("e1", Some("vi"), 60.0, &pcm_of(1000, 0.5)));
        assert_ne!(base, pass_stamp("e2", Some("vi"), 60.0, &pcm_of(1000, 0.5)));
        assert_ne!(base, pass_stamp("e1", Some("en"), 60.0, &pcm_of(1000, 0.5)));
        assert_ne!(base, pass_stamp("e1", None, 60.0, &pcm_of(1000, 0.5)));
        assert_ne!(base, pass_stamp("e1", Some("vi"), 30.0, &pcm_of(1000, 0.5)));
        // One sample differs (a discard or trim changed the audio) / one more.
        let mut p = pcm_of(1000, 0.5);
        p.get_mut(&Track::Mic).unwrap()[400] = 0.0;
        assert_ne!(base, pass_stamp("e1", Some("vi"), 60.0, &p));
        assert_ne!(base, pass_stamp("e1", Some("vi"), 60.0, &pcm_of(1001, 0.5)));
        // The same samples on another track are another audio.
        let sys = HashMap::from([(Track::System, pcm_of(1000, 0.5)[&Track::Mic].clone())]);
        assert_ne!(base, pass_stamp("e1", Some("vi"), 60.0, &sys));
        // Fits the store's token rule.
        assert!(base.len() <= 96 && base.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn checkpoint_parts_round_trip_exactly() {
        let w = |t: &str, s: f64, e: f64, c: f32, k| Word {
            text: t.into(),
            start: s,
            end: e,
            confidence: c,
            speaker: k,
        };
        let finals = vec![
            vec![
                w("xin", 0.1, 0.30000000000000004, 0.9, None),
                w("chào", 0.4, 1.0 / 3.0, 0.123_456_79, Some(2)),
            ],
            vec![w("tạm biệt", 61.25, 62.0, 1.0, None)],
            vec![w(
                "x",
                f64::MIN_POSITIVE / 3.0,
                1.0 + f64::EPSILON,
                f32::MIN_POSITIVE,
                None,
            )],
        ];
        assert_eq!(decode_finals(&encode_finals(&finals)), Some(finals));
        assert_eq!(decode_finals(b"not json"), None);
        let segs = vec![
            SpeakerSegment {
                start: 0.0,
                end: 1.5,
                speaker: 1,
            },
            SpeakerSegment {
                start: 1.5,
                end: 2.0 / 3.0,
                speaker: 2,
            },
        ];
        assert_eq!(decode_segs(&encode_segs(&segs)), Some(segs));
        let m = Mark {
            done: 3,
            total: 9,
            p: 0.375,
            stamp_id: u64::MAX - 5,
        };
        assert_eq!(Mark::from_payload(&m.json()), m);
        assert_eq!(Mark::from_payload(&serde_json::json!({})), Mark::default());
    }
}
