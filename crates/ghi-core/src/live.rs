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
//! - A long partial is cut: at its next sentence end after [`SOFT_CUT_S`], or
//!   wherever it is at [`FORCE_FINAL_S`] (the stream is flushed and reopened).
//! - Another speaker taking over inside an utterance ([`turn_change`]) shows
//!   the first speaker's words as their own line at once ([`Prefix`]); the
//!   stream goes on untouched (a reopen there costs accuracy), and its final
//!   drops the words already shown.
//! - After a line the engine asks for ([`SpeechEngines::reset_after`]: NeMo,
//!   a Vietnamese one) the stream is reopened too.
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
/// A partial this long (seconds of audio) ends at its next sentence end.
pub const SOFT_CUT_S: f64 = 8.0;
/// A partial this long (seconds of audio) without a final is forced out.
pub const FORCE_FINAL_S: f64 = 12.0;
/// Another speaker talking this long (seconds) inside one utterance shows the
/// first speaker's words as a line.
pub const TURN_CUT_S: f64 = 1.0;
/// How often (seconds of audio) a running utterance checks for a new speaker.
pub const TURN_CHECK_S: f64 = 0.5;
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

/// Lets a discard whose caller gave up waiting be dropped by the persist
/// thread, so a late store transaction never leaves a pending row that would
/// cut audio recorded after it [RT-1]. The persist thread holds the lock while
/// it runs the transaction; the caller takes it after a timeout and either finds
/// the answer (it ran) or marks the request abandoned (it will not run).
#[derive(Debug, Default)]
pub struct DiscardGate(pub std::sync::Mutex<bool>);

impl DiscardGate {
    /// After a timeout: the late answer if the transaction ran meanwhile, else
    /// `None` and the request is abandoned.
    pub fn settle(&self, rx: &Receiver<Result<i64, String>>) -> Option<Result<i64, String>> {
        let mut abandoned = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match rx.try_recv() {
            Ok(r) => Some(r),
            Err(_) => {
                *abandoned = true;
                None
            }
        }
    }
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
        gate: std::sync::Arc<DiscardGate>,
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

/// Whether a partial that has run `running_s` seconds of audio should end now:
/// at a sentence end once it is long, anywhere once it is too long (continuous
/// speech without the pause that ends an utterance).
pub fn cut_due(partial: &str, running_s: f64) -> bool {
    running_s >= FORCE_FINAL_S || (running_s >= SOFT_CUT_S && ends_sentence(partial))
}

fn ends_sentence(text: &str) -> bool {
    text.trim_end().ends_with(['.', '?', '!', '…'])
}

/// Where another speaker took over inside the utterance whose words began
/// showing at `from` (seconds, the diarizer's clock): the start of the first
/// turn of at least [`TURN_CUT_S`] after `from` by a speaker other than the
/// first such turn's. Shorter turns (a word of agreement, the previous
/// speaker's tail) don't count.
pub fn turn_change(segs: &[SpeakerSegment], from: f64) -> Option<f64> {
    let mut turns: Vec<&SpeakerSegment> = segs
        .iter()
        .filter(|s| s.end - s.start.max(from) >= TURN_CUT_S)
        .collect();
    turns.sort_by(|a, b| a.start.total_cmp(&b.start));
    let first = turns.first()?.speaker;
    turns
        .iter()
        .find(|t| t.speaker != first)
        .map(|t| t.start.max(from))
}

/// A word as matched between a partial and the final: letters and digits
/// only, lowercase (the final's punctuation spacing is cleaned, the partial's
/// raw: "is , fine" vs "is, fine"). Empty for punctuation alone.
fn bare(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// How far a turn may be past the words it shows (seconds): a word shows a
/// little after it ends, so only words that showed by the turn plus this are
/// the first speaker's for sure; later ones wait for the final.
pub const TURN_SHOW_S: f64 = 0.15;

/// The words of the utterance in progress, from its partials, so a part of it
/// can show as a line before its final (a speaker turn inside it). A greedy
/// streaming recognizer only appends to a partial, so the final starts with
/// the words shown, and [`Prefix::strip`] drops them from it.
#[derive(Debug, Default, Clone)]
pub struct Prefix {
    /// Each word of the partial with the stream time (seconds) it showed.
    seen: Vec<(String, f64)>,
    /// Words already shown as a line.
    shown: usize,
}

impl Prefix {
    /// A partial arrived at stream time `at`.
    pub fn partial(&mut self, text: &str, at: f64) {
        let words: Vec<&str> = text.split_whitespace().collect();
        let same = self
            .seen
            .iter()
            .zip(&words)
            .take_while(|(a, b)| a.0 == **b)
            .count();
        self.seen.truncate(same.max(self.shown));
        for w in words.iter().skip(self.seen.len()) {
            self.seen.push(((*w).to_string(), at));
        }
    }

    /// The partial without the words already shown.
    pub fn rest<'a>(&self, text: &'a str) -> &'a str {
        let mut rest = text.trim_start();
        for _ in 0..self.shown {
            rest = rest
                .split_once(char::is_whitespace)
                .map_or("", |(_, r)| r.trim_start());
        }
        rest
    }

    /// Whether some words were shown as a line.
    pub fn any_shown(&self) -> bool {
        self.shown > 0
    }

    /// A new stream hears the utterance again (the iPhone redoes audio after a
    /// suspect step): keep only the words shown, so its final drops them too.
    pub fn restart(&mut self) {
        self.seen.truncate(self.shown);
    }

    /// The words not shown yet that showed by `until` (stream seconds), but
    /// never the partial's last word (it may still grow), as a final. Their
    /// times are estimated: spread over `from` (where the utterance, or the
    /// last shown part, began) to `cap` (the turn), so the line starts before
    /// any word the final keeps back and ends no earlier than its words did.
    pub fn take(&mut self, until: f64, from: f64, cap: f64) -> Option<AsrResult> {
        let last = self.seen.len().saturating_sub(1);
        let end = self.shown
            + self.seen[self.shown.min(last)..last]
                .iter()
                .take_while(|w| w.1 <= until)
                .count();
        if end <= self.shown {
            return None;
        }
        let n = end - self.shown;
        let from = from.clamp(0.0, cap.max(0.0));
        let step = (cap.max(from) - from) / n as f64;
        let words: Vec<Word> = self.seen[self.shown..end]
            .iter()
            .enumerate()
            .map(|(k, (text, _))| Word {
                text: text.clone(),
                start: from + k as f64 * step,
                end: from + (k + 1) as f64 * step,
                confidence: 1.0,
                speaker: None,
            })
            .collect();
        self.shown = end;
        Some(AsrResult {
            is_final: true,
            text: words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            words,
            languages: Vec::new(),
            audio_processed: until,
        })
    }

    /// A final of the utterance: without the words already shown (matched
    /// without punctuation and case), and the prefix starts over. A final that
    /// does not start with them is kept whole.
    pub fn strip(&mut self, mut r: AsrResult) -> AsrResult {
        let shown = std::mem::take(&mut self.shown);
        let seen = std::mem::take(&mut self.seen);
        let want: Vec<String> = seen[..shown]
            .iter()
            .map(|w| bare(&w.0))
            .filter(|w| !w.is_empty())
            .collect();
        if want.is_empty() {
            return r;
        }
        // How many of `items` cover the shown words (punctuation-only items
        // right after them included); None when they don't start with them.
        let cover = |items: &[&str]| -> Option<usize> {
            let mut matched = 0;
            let mut n = 0;
            for item in items {
                let b = bare(item);
                if matched == want.len() {
                    if !b.is_empty() {
                        break;
                    }
                } else if !b.is_empty() {
                    if b != want[matched] {
                        return None;
                    }
                    matched += 1;
                }
                n += 1;
            }
            (matched == want.len()).then_some(n)
        };
        let tokens: Vec<&str> = r.text.split_whitespace().collect();
        let Some(n_text) = cover(&tokens) else {
            return r;
        };
        let words: Vec<&str> = r.words.iter().map(|w| w.text.as_str()).collect();
        match cover(&words) {
            Some(n_words) => {
                r.words.drain(..n_words);
            }
            // Word list and text disagree: drop as many words as text tokens.
            None => {
                r.words.drain(..n_text.min(r.words.len()));
            }
        }
        r.text = tokens[n_text..].join(" ");
        r
    }
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
    /// The engine asked for a fresh stream after the last final.
    reset: bool,
    /// Position of the last check for a new speaker.
    turn_checked: u64,
    /// The words of the utterance in progress, some maybe shown as a line.
    prefix: Prefix,
    /// Meeting seconds from which to look for the next speaker turn (after
    /// one was shown).
    turn_from: Option<f64>,
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
                    reset: false,
                    turn_checked: 0,
                    prefix: Prefix::default(),
                    turn_from: None,
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
            self.cut_if_due(i);
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
                self.asr[i].turn_from = None;
                self.asr[i].reset = self.engines.reset_after(&r.text);
                // Without the words a speaker turn already showed.
                let r = self.asr[i].prefix.strip(r);
                self.final_result(i, r);
            } else if r.text != self.asr[i].partial {
                if self.asr[i].partial_since.is_none() {
                    self.asr[i].partial_since = Some(self.now_pos);
                }
                let at = self.now_pos.saturating_sub(self.asr[i].offset) as f64 / RATE;
                self.asr[i].prefix.partial(&r.text, at);
                let text = self.asr[i].prefix.rest(&r.text).to_string();
                self.asr[i].partial = r.text;
                self.events.emit(Event::TranscriptPartial {
                    meeting: self.meeting(),
                    track: self.asr[i].track.index() as u8,
                    text,
                });
            }
        }
    }

    fn cut_if_due(&mut self, i: usize) {
        let due = match self.asr[i].partial_since {
            Some(since) => {
                let running = self.now_pos.saturating_sub(since) as f64 / RATE;
                self.show_turn(i, since, running);
                cut_due(&self.asr[i].partial, running)
            }
            // Nothing in progress: the fresh stream the last line asked for.
            None => self.asr[i].reset,
        };
        if !due {
            return;
        }
        // Once only, even if the reopen fails: a finished stream finished
        // again repeats its last final (NeMo).
        self.asr[i].reset = false;
        self.asr[i].partial_since = None;
        if let Err(e) = self.asr[i].stream.finish() {
            self.error(format!("asr: {e}"));
        }
        self.drain(i);
        self.reopen_asr(i, self.now_pos);
    }

    /// A diarized track's utterance another speaker took over: the words
    /// before the turn show as the first speaker's line now.
    fn show_turn(&mut self, i: usize, since: u64, running: f64) {
        let track = self.asr[i].track;
        if track != self.diar_track || (self.cfg.mode == Mode::Call && track == Track::Mic) {
            return;
        }
        let now = self.now_pos;
        let checked = now.saturating_sub(self.asr[i].turn_checked) as f64 / RATE;
        if running < 2.0 * TURN_CUT_S || checked < TURN_CHECK_S {
            return;
        }
        self.asr[i].turn_checked = now;
        let doff = self.diar.offset as f64 / RATE;
        let from = self.asr[i].turn_from.unwrap_or(since as f64 / RATE) - doff;
        let turn = match self.diar.stream.segments() {
            Ok(segs) => turn_change(&segs, from),
            Err(e) => {
                self.error(format!("diarization: {e}"));
                None
            }
        };
        let Some(turn) = turn.map(|t| t + doff) else {
            return;
        };
        // The same turn again (the first speaker's segment still runs past it).
        if self.asr[i].turn_from.is_some_and(|f| turn <= f + 0.01) {
            return;
        }
        let off = self.asr[i].offset as f64 / RATE;
        let turn_s = turn - off;
        // The utterance (or the part after the last turn shown) began about a
        // chunk before its first words showed.
        let lag = f64::from(self.engines.chunk_ms()) / 1000.0 + 0.1;
        let from_s = match self.asr[i].turn_from {
            Some(t) => t - off,
            None => since as f64 / RATE - off - lag,
        };
        if let Some(r) = self.asr[i]
            .prefix
            .take(turn_s + TURN_SHOW_S, from_s, turn_s)
        {
            self.asr[i].turn_from = Some(turn);
            self.final_result(i, r);
            // The screen dropped the words in progress with that line.
            let rest = self.asr[i].prefix.rest(&self.asr[i].partial).to_string();
            if !rest.is_empty() {
                self.events.emit(Event::TranscriptPartial {
                    meeting: self.meeting(),
                    track: track.index() as u8,
                    text: rest,
                });
            }
        }
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
                t.reset = false;
                t.prefix = Prefix::default();
                t.turn_from = None;
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
                track: track.index() as u8,
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
