// SPDX-License-Identifier: Apache-2.0
//! The live engine: one thread per session.
//!
//! It reads 10 ms frames from the capture pipeline's ASR ring, pushes 100 ms
//! blocks in a fixed order (mic ASR, system ASR, diarization), drains the
//! results, aligns words to speakers and hands lines to the persist thread
//! and the event bus. It never touches the store (doc 05 review: a job holding
//! the store must not stall the transcript).
//!
//! - Call mode: the mic track is Me (no voice matching yet); the system track
//!   is transcribed and diarized.
//! - Room mode: the mic track is transcribed and diarized.
//! - A partial running 15 s without a final is force-finalized (the stream is
//!   flushed and reopened).
//! - A gap in the frames (ASR fell behind and skipped) reopens the streams at
//!   the new position; the final pass fills the hole.
//! - Discard drops the ASR text in progress (streams reopened) but keeps the
//!   diarizer, so speakers keep their numbers.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use ghi_audio::pipeline::AsrConsumer;
use ghi_audio::{FRAME_SAMPLES, SAMPLE_RATE, Track};
use ghi_speech::{AsrResult, SpeakerSegment, Word};

use crate::aligner;
use crate::engines::{BoxAsr, BoxDiar, SpeechEngines};
use crate::events::{Event, EventTx, LineInfo, SpeakerInfo, WordInfo};
use crate::speakers::{Change, Source, SpeakerId, SpeakerTracker};

const RATE: f64 = SAMPLE_RATE as f64;
/// Frames per pushed block (100 ms).
const BLOCK_FRAMES: usize = 10;
/// A partial this long (seconds of audio) without a final is forced out.
pub const FORCE_FINAL_S: f64 = 15.0;
const HEALTH_EVERY: Duration = Duration::from_secs(1);
/// Skipped audio up to this long (samples) is replaced by silence for the
/// diarizer instead of restarting it.
const MAX_SILENT_GAP: u64 = 120 * SAMPLE_RATE as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Mic (Me) + system audio (the far end).
    Call,
    /// One mic in a room.
    Room,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Call => "call",
            Mode::Room => "room",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LiveConfig {
    pub meeting: String,
    pub mode: Mode,
    /// BCP-47 code, or `None` for automatic.
    pub language: Option<String>,
}

/// A final line on its way to the store.
#[derive(Debug, Clone, PartialEq)]
pub struct LineOut {
    pub gid: String,
    pub speaker: Option<SpeakerId>,
    pub track: Track,
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub text: String,
    pub lang: Option<String>,
    pub confidence: Option<f32>,
    pub words: Vec<(i64, i64, Option<f32>)>,
    pub overlap: bool,
}

/// Speaker state the persist thread needs.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakerOut {
    pub id: SpeakerId,
    /// "Speaker N" (0 while provisional, and for Me).
    pub number: u32,
    pub name: Option<String>,
    pub color_slot: u8,
    pub is_me: bool,
}

/// From the engine to the persist thread.
#[derive(Debug)]
pub enum PersistMsg {
    Lines(Vec<LineOut>),
    Speaker(Change, SpeakerOut),
    /// Moves the given lines to the new speaker of a split.
    Split {
        from: SpeakerId,
        new: SpeakerOut,
        lines: Vec<String>,
    },
    Mark {
        t_ms: i64,
    },
    /// Discard everything from `t_cut_ms`: flush pending lines, then run the
    /// store transaction; replies with the `discards` row id.
    Discard {
        t_cut_ms: i64,
        now_ms: i64,
        keep: Vec<ghi_store::edits::KeepPages>,
        reply: Sender<Result<i64, String>>,
    },
    Flush(Sender<()>),
    /// The store gid of every session speaker made so far (for snapshots).
    SpeakerGids(Sender<Vec<(SpeakerId, String)>>),
}

/// Commands from the session to the engine thread.
#[derive(Debug)]
pub enum EngineCmd {
    Rename {
        id: SpeakerId,
        name: String,
    },
    Merge {
        from: SpeakerId,
        into: SpeakerId,
    },
    Split {
        from: SpeakerId,
        lines: Vec<String>,
        reply: Sender<Option<SpeakerId>>,
    },
    NotAPerson {
        id: SpeakerId,
    },
    /// The speakers in play (merged ones left out), for a session snapshot.
    Snapshot {
        reply: Sender<Vec<SpeakerInfo>>,
    },
    /// A discard: drop the ASR text in progress, and treat frames before
    /// `mute_until` (timeline samples) as silence — they hold discarded
    /// speech still on its way through the ring [RT-1]. Replies when done.
    ResetAsr {
        mute_until: u64,
        reply: Sender<()>,
    },
}

/// Language of a line when not fixed: Vietnamese writes with diacritics,
/// which the engine always produces for Vietnamese (doc 05 review §3).
pub fn line_language(text: &str, fixed: Option<&str>) -> Option<String> {
    if let Some(l) = fixed {
        return Some(l.split('-').next().unwrap_or(l).to_ascii_lowercase());
    }
    let letters = text.chars().filter(|c| c.is_alphabetic()).count();
    if letters == 0 {
        return None;
    }
    Some(
        if ghi_text::has_diacritics(text) {
            "vi"
        } else {
            "en"
        }
        .into(),
    )
}

fn ms(s: f64) -> i64 {
    (s * 1000.0).round() as i64
}

struct AsrTrack {
    track: Track,
    stream: BoxAsr,
    /// Timeline position (16 kHz samples) where the stream started.
    offset: u64,
    pushed: u64,
    partial: String,
    partial_since: Option<u64>,
}

struct Diar {
    stream: BoxDiar,
    offset: u64,
    generation: u32,
}

pub struct Engine {
    cfg: LiveConfig,
    engines: Arc<dyn SpeechEngines>,
    asr: Vec<AsrTrack>,
    diar: Diar,
    diar_track: Track,
    tracker: SpeakerTracker,
    events: EventTx,
    persist: Sender<PersistMsg>,
    /// Timeline position of the next expected frame.
    next_pos: Option<u64>,
    /// Latest position pushed (the meeting clock as far as the engine knows).
    now_pos: u64,
    /// Frames before this position are silence (a discard).
    mute_until: u64,
    /// Lines of the split being applied (for its event).
    split_lines: Vec<String>,
}

fn speaker_info(t: &SpeakerTracker, id: SpeakerId) -> SpeakerInfo {
    let s = t.get(id).expect("tracked speaker");
    SpeakerInfo {
        id,
        label: t.label(id),
        color_slot: s.color_slot,
        is_me: s.is_me,
        provisional: s.provisional,
        not_person: s.not_person,
        others: s.others,
    }
}

fn speaker_out(t: &SpeakerTracker, id: SpeakerId) -> SpeakerOut {
    let s = t.get(id).expect("tracked speaker");
    SpeakerOut {
        id,
        number: s.number,
        name: s.name.clone(),
        color_slot: s.color_slot,
        is_me: s.is_me,
    }
}

impl Engine {
    pub fn new(
        cfg: LiveConfig,
        engines: Arc<dyn SpeechEngines>,
        tracks: &[Track],
        events: EventTx,
        persist: Sender<PersistMsg>,
    ) -> Result<Engine, String> {
        let lang = cfg.language.clone();
        let asr = tracks
            .iter()
            .map(|&track| {
                Ok(AsrTrack {
                    track,
                    stream: engines.asr(lang.as_deref()).map_err(|e| e.to_string())?,
                    offset: 0,
                    pushed: 0,
                    partial: String::new(),
                    partial_since: None,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let diar_track = match cfg.mode {
            Mode::Call if tracks.contains(&Track::System) => Track::System,
            _ => Track::Mic,
        };
        let diar = Diar {
            stream: engines.diar().map_err(|e| e.to_string())?,
            offset: 0,
            generation: 0,
        };
        Ok(Engine {
            cfg,
            engines,
            asr,
            diar,
            diar_track,
            tracker: SpeakerTracker::new(),
            events,
            persist,
            next_pos: None,
            now_pos: 0,
            mute_until: 0,
            split_lines: Vec::new(),
        })
    }

    fn meeting(&self) -> String {
        self.cfg.meeting.clone()
    }

    /// Runs until `capture_done` is set and the ring is drained, then
    /// flushes the streams.
    pub fn run(
        mut self,
        mut ring: AsrConsumer,
        cmds: Receiver<EngineCmd>,
        capture_done: Arc<AtomicBool>,
    ) {
        let mut mic = Vec::with_capacity(BLOCK_FRAMES * FRAME_SAMPLES);
        let mut sys = Vec::with_capacity(BLOCK_FRAMES * FRAME_SAMPLES);
        let mut block_pos: Option<u64> = None;
        let mut last_health = Instant::now();
        loop {
            while let Ok(cmd) = cmds.try_recv() {
                if let EngineCmd::ResetAsr { mute_until, .. } = &cmd {
                    // The block being assembled holds discarded audio too.
                    self.mute_until = self.mute_until.max(*mute_until);
                    mic.fill(0.0);
                    sys.fill(0.0);
                }
                self.command(cmd);
            }
            let mut got = false;
            while let Some(f) = ring.pop() {
                got = true;
                if let Some(expected) = self.next_pos
                    && f.pos != expected
                {
                    // Frames were skipped: push what we have, then go on at
                    // the new position.
                    if let Some(p) = block_pos.take() {
                        self.push_block(p, &mic, &sys);
                        mic.clear();
                        sys.clear();
                    }
                    self.skip_to(expected, f.pos);
                }
                if block_pos.is_none() {
                    block_pos = Some(f.pos);
                }
                if f.pos < self.mute_until {
                    mic.extend_from_slice(&[0.0; FRAME_SAMPLES]);
                    sys.extend_from_slice(&[0.0; FRAME_SAMPLES]);
                } else {
                    mic.extend_from_slice(&f.mic);
                    sys.extend_from_slice(&f.system);
                }
                self.next_pos = Some(f.pos + FRAME_SAMPLES as u64);
                if mic.len() >= BLOCK_FRAMES * FRAME_SAMPLES {
                    self.push_block(block_pos.take().unwrap(), &mic, &sys);
                    mic.clear();
                    sys.clear();
                }
            }
            if last_health.elapsed() >= HEALTH_EVERY {
                last_health = Instant::now();
                self.events.emit(Event::Health {
                    meeting: self.meeting(),
                    asr_lag_s: ring.len() as f32 * 0.01,
                    asr_skipped_s: ring.skipped_frames() as f32 * 0.01,
                    aec: false,
                });
            }
            if !got {
                if capture_done.load(Ordering::Acquire) && ring.is_empty() {
                    if let Some(p) = block_pos.take() {
                        self.push_block(p, &mic, &sys);
                    }
                    self.finish();
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    fn push_block(&mut self, pos: u64, mic: &[f32], sys: &[f32]) {
        self.now_pos = pos + mic.len() as u64;
        for i in 0..self.asr.len() {
            let pcm = match self.asr[i].track {
                Track::Mic => mic,
                Track::System => sys,
            };
            if self.asr[i].pushed == 0 {
                self.asr[i].offset = pos;
            }
            if let Err(e) = self.asr[i].stream.push(pcm, SAMPLE_RATE) {
                self.error(format!("asr: {e}"));
            }
            self.asr[i].pushed += pcm.len() as u64;
        }
        let dpcm = match self.diar_track {
            Track::Mic => mic,
            Track::System => sys,
        };
        if let Err(e) = self.diar.stream.push(dpcm, SAMPLE_RATE) {
            self.error(format!("diarization: {e}"));
        }
        for i in 0..self.asr.len() {
            self.drain(i);
            self.force_final_if_stuck(i);
        }
        let mut ch = Vec::new();
        self.tracker.tick(self.now_pos as f64 / RATE, &mut ch);
        self.apply_changes(ch);
    }

    fn error(&self, message: String) {
        self.events.emit(Event::Error {
            meeting: Some(self.meeting()),
            kind: crate::events::ErrorKind::Engine,
            message,
        });
    }

    fn drain(&mut self, i: usize) {
        loop {
            let r = match self.asr[i].stream.next_result() {
                Ok(Some(r)) => r,
                Ok(None) => break,
                Err(e) => {
                    self.error(format!("asr: {e}"));
                    break;
                }
            };
            if r.is_final {
                self.asr[i].partial.clear();
                self.asr[i].partial_since = None;
                self.final_result(i, r);
            } else if r.text != self.asr[i].partial {
                if self.asr[i].partial_since.is_none() {
                    self.asr[i].partial_since = Some(self.now_pos);
                }
                self.asr[i].partial = r.text.clone();
                self.events.emit(Event::TranscriptPartial {
                    meeting: self.meeting(),
                    track: self.asr[i].track.index() as u8,
                    text: r.text,
                });
            }
        }
    }

    fn force_final_if_stuck(&mut self, i: usize) {
        let Some(since) = self.asr[i].partial_since else {
            return;
        };
        if (self.now_pos.saturating_sub(since)) as f64 / RATE < FORCE_FINAL_S {
            return;
        }
        if let Err(e) = self.asr[i].stream.finish() {
            self.error(format!("asr: {e}"));
        }
        self.drain(i);
        self.reopen_asr(i, self.now_pos);
    }

    fn reopen_asr(&mut self, i: usize, pos: u64) {
        match self.engines.asr(self.cfg.language.as_deref()) {
            Ok(s) => {
                let t = &mut self.asr[i];
                t.stream = s;
                t.offset = pos;
                t.pushed = 0;
                t.partial.clear();
                t.partial_since = None;
            }
            Err(e) => self.error(format!("asr: {e}")),
        }
    }

    /// Frames `from..to` were skipped (the engine fell behind). ASR streams
    /// reopen at `to` (the final pass fills the hole). The diarizer hears
    /// silence for a short gap so speakers keep their numbers (it is cheap:
    /// RTF ~0.02); after a long one it starts over (new speakers).
    fn skip_to(&mut self, from: u64, to: u64) {
        for i in 0..self.asr.len() {
            self.reopen_asr(i, to);
        }
        let gap = to.saturating_sub(from);
        if gap <= MAX_SILENT_GAP {
            let second = vec![0.0f32; SAMPLE_RATE as usize];
            let mut left = gap as usize;
            while left > 0 {
                let n = left.min(second.len());
                if let Err(e) = self.diar.stream.push(&second[..n], SAMPLE_RATE) {
                    self.error(format!("diarization: {e}"));
                    break;
                }
                left -= n;
            }
            return;
        }
        {
            match self.engines.diar() {
                Ok(s) => {
                    self.diar = Diar {
                        stream: s,
                        offset: to,
                        generation: self.diar.generation + 1,
                    };
                }
                Err(e) => self.error(format!("diarization: {e}")),
            }
        }
    }

    fn final_result(&mut self, i: usize, r: AsrResult) {
        let text = r.text.trim().to_string();
        if text.is_empty() {
            return;
        }
        let track = self.asr[i].track;
        let off = self.asr[i].offset as f64 / RATE;
        // Words on the meeting timeline (seconds).
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
            // No word times: one word spanning what the engine had consumed.
            let end = off + r.audio_processed;
            words.push(Word {
                text: text.clone(),
                start: (end - 1.0).max(off),
                end,
                confidence: 1.0,
                speaker: None,
            });
        }
        let mut ch = Vec::new();
        let lines: Vec<(Option<SpeakerId>, aligner::AlignedLine)> =
            if self.cfg.mode == Mode::Call && track == Track::Mic {
                let me = self.tracker.me(words[0].start, &mut ch);
                vec![(
                    Some(me),
                    aligner::AlignedLine {
                        speaker: None,
                        start: words[0].start,
                        end: words.last().map_or(0.0, |w| w.end),
                        words: words
                            .into_iter()
                            .map(|w| aligner::AlignedWord {
                                low_confidence: w.confidence < aligner::LOW_CONFIDENCE,
                                overlap: false,
                                speaker: None,
                                word: w,
                            })
                            .collect(),
                    },
                )]
            } else {
                let doff = self.diar.offset as f64 / RATE;
                let segs: Vec<SpeakerSegment> = match self.diar.stream.segments() {
                    Ok(s) => s
                        .into_iter()
                        .map(|s| SpeakerSegment {
                            start: s.start + doff,
                            end: s.end + doff,
                            speaker: s.speaker,
                        })
                        .collect(),
                    Err(e) => {
                        self.error(format!("diarization: {e}"));
                        Vec::new()
                    }
                };
                aligner::align(&words, &segs)
                    .into_iter()
                    .map(|l| {
                        let id = l.speaker.map(|label| {
                            self.tracker.resolve(
                                Source {
                                    track: track.index() as u8,
                                    generation: self.diar.generation,
                                    label,
                                },
                                l.start,
                                &mut ch,
                            )
                        });
                        (id, l)
                    })
                    .collect()
            };
        self.apply_changes(ch);
        let out: Vec<LineOut> = lines
            .into_iter()
            .map(|(speaker, l)| {
                let text = l.text();
                let conf = {
                    let n = l.words.len().max(1) as f32;
                    Some(l.words.iter().map(|w| w.word.confidence).sum::<f32>() / n)
                };
                LineOut {
                    gid: ghi_store::new_gid(),
                    speaker,
                    track,
                    t0_ms: ms(l.start),
                    t1_ms: ms(l.end),
                    lang: line_language(&text, self.cfg.language.as_deref()),
                    confidence: conf,
                    overlap: l.overlap(),
                    words: l
                        .words
                        .iter()
                        .map(|w| (ms(w.word.start), ms(w.word.end), Some(w.word.confidence)))
                        .collect(),
                    text,
                }
            })
            .collect();
        for l in &out {
            self.events.emit(Event::TranscriptFinal {
                meeting: self.meeting(),
                line: LineInfo {
                    gid: l.gid.clone(),
                    speaker: l.speaker,
                    t0_ms: l.t0_ms,
                    t1_ms: l.t1_ms,
                    text: l.text.clone(),
                    overlap: l.overlap,
                    words: l
                        .text
                        .split_whitespace()
                        .zip(&l.words)
                        .map(|(t, (a, b, c))| WordInfo {
                            text: t.to_string(),
                            t0_ms: *a,
                            t1_ms: *b,
                            low_confidence: c.is_some_and(|c| c < aligner::LOW_CONFIDENCE),
                        })
                        .collect(),
                },
            });
        }
        if !out.is_empty() {
            let _ = self.persist.send(PersistMsg::Lines(out));
        }
    }

    fn apply_changes(&mut self, changes: Vec<Change>) {
        let meeting = self.meeting();
        for c in changes {
            let ev = match &c {
                Change::Arrived(id) => Event::SpeakerArrived {
                    meeting: meeting.clone(),
                    speaker: speaker_info(&self.tracker, *id),
                },
                Change::Confirmed(id) => Event::SpeakerConfirmed {
                    meeting: meeting.clone(),
                    speaker: speaker_info(&self.tracker, *id),
                },
                Change::Renamed(id) => Event::SpeakerRenamed {
                    meeting: meeting.clone(),
                    speaker: speaker_info(&self.tracker, *id),
                },
                Change::Merged { from, into } => Event::SpeakersMerged {
                    meeting: meeting.clone(),
                    from: *from,
                    into: *into,
                },
                Change::Split { from, new } => Event::SpeakerSplit {
                    meeting: meeting.clone(),
                    from: *from,
                    speaker: speaker_info(&self.tracker, *new),
                    lines: std::mem::take(&mut self.split_lines),
                },
                Change::NotAPerson(id) => Event::SpeakerNotAPerson {
                    meeting: meeting.clone(),
                    id: *id,
                },
            };
            self.events.emit(ev);
            let id = match c {
                Change::Arrived(id)
                | Change::Confirmed(id)
                | Change::Renamed(id)
                | Change::NotAPerson(id) => id,
                Change::Merged { from, .. } => from,
                // Splits are persisted with their lines (EngineCmd::Split).
                Change::Split { .. } => continue,
            };
            let out = speaker_out(&self.tracker, id);
            let _ = self.persist.send(PersistMsg::Speaker(c, out));
        }
    }

    fn command(&mut self, cmd: EngineCmd) {
        let mut ch = Vec::new();
        match cmd {
            EngineCmd::Rename { id, name } => {
                self.tracker.rename(id, &name, &mut ch);
            }
            EngineCmd::Merge { from, into } => {
                self.tracker.merge(from, into, &mut ch);
            }
            EngineCmd::NotAPerson { id } => {
                self.tracker.set_not_person(id, &mut ch);
            }
            EngineCmd::Split { from, lines, reply } => {
                let new = self
                    .tracker
                    .split(from, self.now_pos as f64 / RATE, &mut ch);
                if let Some(new) = new {
                    let from = self.tracker.canonical(from);
                    // Each line once, as the store moves them.
                    let mut moved = Vec::new();
                    for l in lines {
                        if !moved.contains(&l) {
                            moved.push(l);
                        }
                    }
                    self.split_lines = moved.clone();
                    let _ = self.persist.send(PersistMsg::Split {
                        from,
                        new: speaker_out(&self.tracker, new),
                        lines: moved,
                    });
                }
                let _ = reply.send(new);
            }
            EngineCmd::Snapshot { reply } => {
                let _ = reply.send(
                    self.tracker
                        .speakers()
                        .iter()
                        .filter(|s| s.merged_into.is_none())
                        .map(|s| speaker_info(&self.tracker, s.id))
                        .collect(),
                );
            }
            EngineCmd::ResetAsr { reply, .. } => {
                for i in 0..self.asr.len() {
                    self.reopen_asr(i, self.now_pos);
                }
                let _ = reply.send(());
            }
        }
        self.apply_changes(ch);
    }

    fn finish(&mut self) {
        for i in 0..self.asr.len() {
            if let Err(e) = self.asr[i].stream.finish() {
                self.error(format!("asr: {e}"));
            }
            self.drain(i);
        }
        let _ = self.diar.stream.finish();
        let mut ch = Vec::new();
        self.tracker.tick(f64::INFINITY, &mut ch);
        self.apply_changes(ch);
    }

    /// The tracker's speakers (for snapshots and tests).
    pub fn speakers(&self) -> Vec<SpeakerInfo> {
        self.tracker
            .speakers()
            .iter()
            .map(|s| speaker_info(&self.tracker, s.id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_language_from_diacritics_or_the_fixed_choice() {
        assert_eq!(line_language("chốt lịch beta", None).as_deref(), Some("vi"));
        assert_eq!(
            line_language("ship it on friday", None).as_deref(),
            Some("en")
        );
        assert_eq!(line_language("123", None), None);
        assert_eq!(
            line_language("anything", Some("vi-VN")).as_deref(),
            Some("vi")
        );
    }
}
