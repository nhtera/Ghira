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
//! - Call mode: the mic track is Me (no voice matching yet); the far track is
//!   diarized. Room mode: the mic track is diarized.
//! - Preempted by a recording, the pass yields and starts over later (it
//!   is short next to the meeting: ~4 min for 60 min on an M4 Pro).

use std::collections::HashMap;
use std::sync::Arc;

use ghi_audio::{SAMPLE_RATE, Track};
use ghi_speech::{SpeakerSegment, Word};
use ghi_store::store::{NewSegment, NewSpeaker, Store, TrackKind};

use crate::aligner;
use crate::carry::{self, Turn};
use crate::engines::SpeechEngines;
use crate::events::Stage;
use crate::jobs::{JobCtx, JobHandler, Outcome};
use crate::live::line_language;
use crate::notes_job::NOTES_FINAL_JOB;
use crate::session::JOB_PAYLOAD_VERSION;
use crate::speakers::COLOR_ORDER;
use crate::vocab::Vocabulary;

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

/// A final ASR result: its words on the track's timeline, and its text.
type Final = (Vec<Word>, String);

/// Transcribes `pcm` chunk by chunk; words on the track's timeline (seconds).
fn transcribe(
    engines: &dyn SpeechEngines,
    pcm: &[f32],
    language: Option<&str>,
    chunk_s: f64,
    ctx: &JobCtx,
) -> Result<Option<Vec<Final>>, String> {
    let cuts = cut_points(pcm, chunk_s);
    let mut out = Vec::new();
    for w in cuts.windows(2) {
        if ctx.preempted() {
            return Ok(None);
        }
        let (a, b) = (w[0], w[1]);
        let off = a as f64 / RATE;
        let mut asr = engines.asr(language).map_err(|e| e.to_string())?;
        let collect = |asr: &mut crate::engines::BoxAsr,
                       out: &mut Vec<(Vec<Word>, String)>|
         -> Result<(), String> {
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
                out.push((words, r.text.trim().to_string()));
            }
            Ok(())
        };
        for block in pcm[a..b].chunks(SAMPLE_RATE as usize) {
            if ctx.preempted() {
                return Ok(None);
            }
            asr.push(block, SAMPLE_RATE).map_err(|e| e.to_string())?;
            collect(&mut asr, &mut out)?;
        }
        asr.finish().map_err(|e| e.to_string())?;
        collect(&mut asr, &mut out)?;
        ctx.progress(
            Some(Stage::ImprovingTranscript),
            b as f32 / pcm.len().max(1) as f32,
        );
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
            let _ = ctx.store.set_meeting_status(m, "ready");
        }
    }

    fn run(&self, ctx: &JobCtx) -> Result<Outcome, String> {
        let restart = || Ok(Outcome::Yield(serde_json::json!({})));
        let meeting = ctx.meeting()?.to_string();
        let store: &Arc<Store> = ctx.store;
        let err = |e: ghi_store::StoreError| e.to_string();
        let m = store.get_meeting(&meeting).map_err(err)?;
        let call = m.mode == "call";

        // Decoding.
        ctx.progress(Some(Stage::Decoding), 0.0);
        let mut pcm: HashMap<Track, Vec<f32>> = HashMap::new();
        for (kind, _) in store.tracks(&meeting).map_err(err)? {
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
            queue_notes(store, &meeting)?;
            return Ok(Outcome::Done);
        }
        let audio_ms =
            pcm.values().map(Vec::len).max().unwrap_or(0) as i64 * 1000 / i64::from(SAMPLE_RATE);
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

        // RefiningSpeakers: diarize the far track (call) or the room mic.
        let diar_track = if call && pcm.contains_key(&Track::System) {
            Track::System
        } else {
            Track::Mic
        };
        ctx.progress(Some(Stage::RefiningSpeakers), 0.0);
        let mut diar = engines.diar().map_err(|e| e.to_string())?;
        for block in pcm[&diar_track].chunks(10 * SAMPLE_RATE as usize) {
            if ctx.preempted() {
                return restart();
            }
            diar.push(block, SAMPLE_RATE).map_err(|e| e.to_string())?;
        }
        diar.finish().map_err(|e| e.to_string())?;
        let segs: Vec<SpeakerSegment> = diar.segments().map_err(|e| e.to_string())?;

        // ImprovingTranscript.
        let mut lines: Vec<Line> = Vec::new();
        let mut tracks: Vec<Track> = pcm.keys().copied().collect();
        tracks.sort_by_key(|t| t.index());
        for t in tracks {
            let me = call && t == Track::Mic && diar_track != Track::Mic;
            let Some(finals) = transcribe(engines.as_ref(), &pcm[&t], language, self.chunk_s, ctx)?
            else {
                return restart();
            };
            for (words, _) in finals {
                if me {
                    let l = aligner::align(&words, &[]);
                    for al in l {
                        lines.push(line_from(t, None, true, &al));
                    }
                } else {
                    for al in aligner::align(&words, &segs) {
                        lines.push(line_from(t, al.speaker, false, &al));
                    }
                }
            }
        }
        drop(engines); // free the speech models before notes load the LLM
        lines.sort_by_key(|l| (l.t0_ms, l.track.index()));

        // MatchingVoices: carry live speakers over to the final clusters.
        ctx.progress(Some(Stage::MatchingVoices), 0.0);
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
        let c = carry::carry_over(&live_turns, &final_turns);
        let by_idx: HashMap<u32, &String> = live_idx.iter().map(|(g, i)| (*i, g)).collect();
        let mut label_gid: HashMap<u32, String> = c
            .matched
            .iter()
            .map(|(f, l)| (*f, by_idx[l].clone()))
            .collect();
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
        for (label, idx) in c.unmatched.iter().zip(first_idx..) {
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
        let vocab =
            Some(Vocabulary::new(&crate::vocab::effective_terms(store)?)).filter(|v| !v.is_empty());
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
                NewSegment {
                    gid: None,
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
            v2.push(NewSegment {
                gid: None,
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
        store.replace_transcript(&meeting, v2).map_err(err)?;
        queue_notes(store, &meeting)?;
        ctx.progress(Some(Stage::MatchingVoices), 1.0);
        Ok(Outcome::Done)
    }
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
}
