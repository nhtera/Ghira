// SPDX-License-Identifier: Apache-2.0
//! The live engine thread: reads the sealed backlog, runs streaming ASR and
//! diarization (Light tier: 1120 ms ASR chunk, diarization `v3-streaming`) and
//! turns finals into speaker-tagged lines through the `ghi-core` blocks
//! (`aligner`, `SpeakerTracker`, the persist thread and the event bus).
//!
//! Kept mobile-only (phase 16 D3): the lock / catch-up contract of [`Gate`]
//! (suspect steps dropped and redone, models unloaded after 30 s inactive)
//! would add desktop regression risk inside `ghi_core::live`.
//!
//! The streams, the models and the speaker tracker all live on this thread's
//! stack: when the thread ends they are gone, before the session reports it
//! is done (and so before the job runner may load the final pass's models).
//!
//! [`Gate`]: crate::gate::Gate

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use ghi_core::aligner;
use ghi_core::engines::SpeechEngines;
use ghi_core::events::{Event, EventTx, LineInfo, SpeakerInfo, WordInfo};
use ghi_core::live::{LineOut, PersistMsg, SpeakerOut, line_language};
use ghi_core::speakers::{Change, Source, SpeakerId, SpeakerTracker};
use ghi_speech::{AsrResult, SpeakerSegment, Word};

use crate::backlog::BacklogReader;
use crate::session::Shared;

/// Audio per step while live, and the cap while catching up (a step is what
/// may have to be redone after a lock lands in it).
const LIVE_STEP: usize = 2_560; // 160 ms
const CATCH_UP_STEP: usize = 8_000; // 500 ms
const POLL: Duration = Duration::from_millis(40);
pub const SAMPLE_RATE: u32 = ghi_audio::SAMPLE_RATE;
const RATE: f64 = SAMPLE_RATE as f64;
/// The ASR chunk the live and final engines are loaded with.
pub const CHUNK_MS: u32 = 1120;
/// Inactive this long, the engine drops its streams and models (the GPU
/// memory goes back to the system) and reloads on return.
pub const UNLOAD_AFTER: Duration = Duration::from_secs(30);

/// Model files the engine needs, by registry id.
pub const MODELS: [&str; 2] = ["nemotron-3.5-asr", "nemotron-3-diarization"];

/// The speech models are installed (the live engine and the final pass wait
/// for them; "record now, process later").
pub fn models_ready(dir: &Path) -> bool {
    cfg!(feature = "nemo") && ghi_models::installed(dir, &MODELS)
}

/// Path of a registry model in `dir`, if it is there with the pinned size.
pub fn model_file(dir: &Path, id: &str) -> Option<PathBuf> {
    let m = ghi_models::find(id)?;
    let path = ghi_models::path_in(dir, &m);
    let len = std::fs::metadata(&path).ok()?.len();
    (len == m.size).then_some(path)
}

/// Opens the speech engines (loads the models). Called by the engine thread
/// while the app is active, and by the final pass job.
pub type EnginesProvider = Arc<dyn Fn() -> Result<Arc<dyn SpeechEngines>, String> + Send + Sync>;

/// Metal on devices; the simulator runs the CPU backend.
#[cfg(feature = "nemo")]
const DEVICE: ghi_speech::nemo::Device = if cfg!(target_abi = "sim") {
    ghi_speech::nemo::Device::Cpu
} else {
    ghi_speech::nemo::Device::Gpu
};

/// NeMo engines over the registry models in `dir`, verified first.
pub fn nemo_provider(dir: &Path) -> EnginesProvider {
    let dir = dir.to_path_buf();
    Arc::new(move || {
        #[cfg(feature = "nemo")]
        {
            let path = |id: &str| ghi_app::core::checked_model(&dir, id);
            let e = ghi_core::engines::NemoEngines::load(
                &path(MODELS[0])?,
                &path(MODELS[1])?,
                CHUNK_MS,
                DEVICE,
            )
            .map_err(|e| e.to_string())?;
            Ok(Arc::new(e) as Arc<dyn SpeechEngines>)
        }
        #[cfg(not(feature = "nemo"))]
        {
            let _ = &dir;
            Err("this build has no speech engine (feature `nemo`): recording only".into())
        }
    })
}

/// The engines a recording uses: the scripted fakes when a test-hooks build
/// asks (`GHI_FAKE_ENGINES=1`; CI without models), else NeMo.
pub fn provider(models: &Path) -> EnginesProvider {
    #[cfg(feature = "test-hooks")]
    if std::env::var_os("GHI_FAKE_ENGINES").is_some() {
        return Arc::new(|| Ok(ghi_core::engines::FakeEngines::new(fake_script()) as _));
    }
    nemo_provider(models)
}

/// Whether live recording would have engines here (fakes count).
pub fn engines_available(models: &Path) -> bool {
    if cfg!(feature = "test-hooks") && std::env::var_os("GHI_FAKE_ENGINES").is_some() {
        return true;
    }
    models_ready(models)
}

/// Two voices taking turns, a line every 4 s for ten minutes.
#[cfg(feature = "test-hooks")]
pub fn fake_script() -> ghi_core::engines::Script {
    let mut utterances = Vec::new();
    let mut turns = Vec::new();
    for i in 0..150u32 {
        let t = f64::from(i) * 4.0;
        utterances.push((t + 0.5, t + 3.0, format!("câu số {i} hello there")));
        turns.push(SpeakerSegment {
            start: t,
            end: t + 3.5,
            speaker: 1 + i % 2,
        });
    }
    ghi_core::engines::Script { utterances, turns }
}

#[cfg_attr(not(feature = "nemo"), allow(dead_code))]
/// The speaker who talks most within `start..end` (seconds), if anyone does.
pub fn majority_speaker(segs: &[SpeakerSegment], start: f64, end: f64) -> Option<u32> {
    aligner::majority(segs, start, end)
}

/// What one engine step produced, applied only if the step was not suspect.
/// Times are seconds of the current streams.
#[derive(Debug, Clone)]
pub enum Update {
    Partial(String),
    Final {
        start: f64,
        end: f64,
        text: String,
        words: Vec<Word>,
        /// The diarizer's segments when the line was produced.
        segs: Vec<SpeakerSegment>,
        /// Shown at a speaker turn before its utterance ended: the utterance
        /// is still open (a reset redoes it; the words shown are not redone).
        early: bool,
    },
}

/// Meeting spans (start ms, end ms) discarded during the recording. The engine
/// trails the audio, so it can still make text for a span after the discard
/// was applied to the store: the [`Applier`] drops it. The session holds the
/// lock while it registers a span and queues the store's discard, and the
/// applier while it filters and queues lines, so a line is either dropped or
/// queued before the discard (which then removes it).
pub type Cuts = Arc<Mutex<Vec<(i64, i64)>>>;

/// An unchanged partial is sent again after this long, so a screen that missed
/// it (a reload, a snapshot that has none) shows it again within a second.
const PARTIAL_KEEPALIVE: Duration = Duration::from_secs(1);

/// What reached the UI bus, as numbers (no text), for the live metrics: it
/// tells an engine that stopped producing from a screen that stopped showing.
#[derive(Debug, Default)]
pub struct UiStats(Mutex<UiInner>);

#[derive(Debug, Default)]
struct UiInner {
    partials: u32,
    /// Partials that went to the screen (the unchanged ones are held back).
    partials_sent: u32,
    /// Partials whose text equals the previous one (nothing new decoded).
    partials_same: u32,
    finals: u32,
    /// Lines sent to the UI (a final can split into several).
    lines: u32,
    /// Of those, lines without a speaker yet.
    lines_unattributed: u32,
    last_partial_len: usize,
    last_partial_hash: u64,
    /// Sum of squares and count of the samples the engine was fed.
    audio_sq: f64,
    audio_n: u64,
    /// Results the ASR handed back, partial or final, before any filtering.
    asr_results: u32,
    steps: u32,
    /// Seconds of audio the ASR says it has processed, as of its last result.
    asr_audio_s: f64,
    /// The last change the screen could show (a new partial text or a line).
    last_update: Option<Instant>,
    last_final: Option<Instant>,
    max_gap: Duration,
    max_final_gap: Duration,
}

/// One window of [`UiStats`] (the counters reset after each read).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiWindow {
    pub partials: u32,
    pub partials_sent: u32,
    pub partials_same: u32,
    pub finals: u32,
    pub lines: u32,
    pub lines_unattributed: u32,
    pub partial_chars: usize,
    /// RMS of the audio the engine was fed (0..1): silence vs speech.
    pub audio_rms: f64,
    pub asr_results: u32,
    pub steps: u32,
    pub asr_audio_s: f64,
    pub max_update_gap_s: f64,
    pub max_final_gap_s: f64,
}

impl UiStats {
    fn lock(&self) -> std::sync::MutexGuard<'_, UiInner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The engine is about to be fed `pcm`.
    pub fn audio(&self, pcm: &[f32]) {
        let mut i = self.lock();
        i.audio_sq += pcm
            .iter()
            .map(|&x| f64::from(x) * f64::from(x))
            .sum::<f64>();
        i.audio_n += pcm.len() as u64;
        i.steps += 1;
    }

    /// The ASR handed back a result.
    pub fn asr_result(&self, audio_processed: f64) {
        let mut i = self.lock();
        i.asr_results += 1;
        i.asr_audio_s = audio_processed;
    }

    fn visible(i: &mut UiInner, now: Instant) {
        if let Some(t) = i.last_update {
            i.max_gap = i.max_gap.max(now.duration_since(t));
        }
        i.last_update = Some(now);
    }

    fn partial(&self, text: &str) {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        let hash = h.finish();
        let mut i = self.lock();
        i.partials += 1;
        i.last_partial_len = text.chars().count();
        // The screen only changes when the text does.
        if i.last_partial_hash == hash {
            i.partials_same += 1;
            return;
        }
        i.last_partial_hash = hash;
        Self::visible(&mut i, Instant::now());
    }

    fn partial_sent(&self) {
        self.lock().partials_sent += 1;
    }

    fn final_result(&self) {
        let now = Instant::now();
        let mut i = self.lock();
        i.finals += 1;
        if let Some(t) = i.last_final {
            i.max_final_gap = i.max_final_gap.max(now.duration_since(t));
        }
        i.last_final = Some(now);
    }

    fn line(&self, unattributed: bool) {
        let mut i = self.lock();
        i.lines += 1;
        i.lines_unattributed += u32::from(unattributed);
        i.last_partial_hash = 0;
        i.last_partial_len = 0;
        Self::visible(&mut i, Instant::now());
    }

    /// The window since the last call; a gap still open counts up to `now`.
    pub fn take(&self) -> UiWindow {
        let now = Instant::now();
        let mut i = self.lock();
        let open = i
            .last_update
            .map_or(Duration::ZERO, |t| now.duration_since(t));
        let open_final = i
            .last_final
            .map_or(Duration::ZERO, |t| now.duration_since(t));
        let w = UiWindow {
            partials: i.partials,
            partials_sent: i.partials_sent,
            partials_same: i.partials_same,
            finals: i.finals,
            lines: i.lines,
            lines_unattributed: i.lines_unattributed,
            partial_chars: i.last_partial_len,
            asr_results: i.asr_results,
            steps: i.steps,
            asr_audio_s: i.asr_audio_s,
            audio_rms: if i.audio_n == 0 {
                0.0
            } else {
                (i.audio_sq / i.audio_n as f64).sqrt()
            },
            max_update_gap_s: i.max_gap.max(open).as_secs_f64(),
            max_final_gap_s: i.max_final_gap.max(open_final).as_secs_f64(),
        };
        // The next window starts with the gap that is open now.
        *i = UiInner {
            last_partial_len: i.last_partial_len,
            last_partial_hash: i.last_partial_hash,
            last_update: i.last_update,
            last_final: i.last_final,
            max_gap: open,
            max_final_gap: open_final,
            ..UiInner::default()
        };
        w
    }
}

/// Turns committed updates into lines, speakers, events and store writes.
pub struct Applier {
    stats: Arc<UiStats>,
    /// The partial the screen has now and when it was sent: an unchanged one is
    /// not sent again before `keepalive` (a step is 160 ms; the text changes
    /// about once per chunk).
    last_partial: Option<(String, Instant)>,
    keepalive: Duration,
    cuts: Cuts,
    meeting: String,
    language: Option<String>,
    tracker: SpeakerTracker,
    events: EventTx,
    persist: Sender<PersistMsg>,
    /// Bumped when the diarizer stream is reopened (its labels restart).
    generation: u32,
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

fn ms(s: f64) -> i64 {
    (s * 1000.0).round() as i64
}

impl Applier {
    pub fn new(
        meeting: String,
        language: Option<String>,
        events: EventTx,
        persist: Sender<PersistMsg>,
    ) -> Applier {
        Applier {
            stats: Arc::default(),
            last_partial: None,
            keepalive: PARTIAL_KEEPALIVE,
            cuts: Cuts::default(),
            meeting,
            language,
            tracker: SpeakerTracker::new(),
            events,
            persist,
            generation: 0,
        }
    }

    pub fn stats(&self) -> Arc<UiStats> {
        self.stats.clone()
    }

    /// Counts what reaches the UI into `stats` (the live metrics read it).
    pub fn with_stats(mut self, stats: Arc<UiStats>) -> Applier {
        self.stats = stats;
        self
    }

    /// Drops the text of the spans in `cuts` (discards while recording).
    pub fn with_cuts(mut self, cuts: Cuts) -> Applier {
        self.cuts = cuts;
        self
    }

    /// The streams were reopened: the diarizer's labels start over.
    pub fn streams_reopened(&mut self) {
        self.generation += 1;
    }

    /// Applies one step's updates; `offset_samples` is where the streams
    /// started on the backlog timeline, `now_samples` the read position.
    pub fn apply(&mut self, updates: Vec<Update>, offset_samples: u64, now_samples: u64) {
        let offset = offset_samples as f64 / RATE;
        let cuts = self.cuts.clone();
        let cuts = cuts.lock().unwrap_or_else(|e| e.into_inner());
        // The engine is still inside a discarded span: its partial is not shown.
        let now_ms = ms(now_samples as f64 / RATE);
        let inside_cut = cuts.iter().any(|&(_, end)| now_ms <= end);
        for u in updates {
            match u {
                Update::Partial(_) if inside_cut => {
                    // The screen drops its partial with the discard.
                    self.last_partial = None;
                }
                Update::Partial(text) => {
                    self.stats.partial(&text);
                    let now = Instant::now();
                    let fresh = self.last_partial.as_ref().is_none_or(|(last, at)| {
                        *last != text || now.duration_since(*at) >= self.keepalive
                    });
                    if fresh {
                        self.last_partial = Some((text.clone(), now));
                        self.stats.partial_sent();
                        self.events.emit(Event::TranscriptPartial {
                            meeting: self.meeting.clone(),
                            track: 0,
                            text,
                        })
                    }
                }
                Update::Final {
                    text, words, segs, ..
                } => {
                    self.stats.final_result();
                    self.final_line(&text, words, segs, offset, &cuts)
                }
            }
        }
        let mut ch = Vec::new();
        self.tracker.tick(now_samples as f64 / RATE, &mut ch);
        self.changes(ch);
    }

    fn final_line(
        &mut self,
        text: &str,
        words: Vec<Word>,
        segs: Vec<SpeakerSegment>,
        off: f64,
        cuts: &[(i64, i64)],
    ) {
        let words: Vec<Word> = words
            .into_iter()
            .map(|w| Word {
                start: w.start + off,
                end: w.end + off,
                ..w
            })
            // Words inside a discarded span never reach the transcript.
            .filter(|w| {
                !cuts
                    .iter()
                    .any(|&(from, to)| ms(w.end) > from && ms(w.start) < to)
            })
            .collect();
        if words.is_empty() {
            return;
        }
        let segs: Vec<SpeakerSegment> = segs
            .into_iter()
            .map(|s| SpeakerSegment {
                start: s.start + off,
                end: s.end + off,
                speaker: s.speaker,
            })
            .collect();
        let _ = text;
        let mut ch = Vec::new();
        let lines: Vec<(Option<SpeakerId>, aligner::AlignedLine)> = aligner::align(&words, &segs)
            .into_iter()
            .map(|l| {
                let id = l.speaker.map(|label| {
                    self.tracker.resolve(
                        Source {
                            track: 0,
                            generation: self.generation,
                            label,
                        },
                        l.start,
                        &mut ch,
                    )
                });
                (id, l)
            })
            .collect();
        self.changes(ch);
        let out: Vec<LineOut> = lines
            .into_iter()
            .map(|(speaker, l)| {
                let text = l.text();
                let n = l.words.len().max(1) as f32;
                let conf = Some(l.words.iter().map(|w| w.word.confidence).sum::<f32>() / n);
                LineOut {
                    gid: ghi_store::new_gid(),
                    speaker,
                    track: ghi_audio::Track::Mic,
                    t0_ms: ms(l.start),
                    t1_ms: ms(l.end),
                    lang: line_language(&text, self.language.as_deref()),
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
        // The screen clears its partial with a line.
        self.last_partial = None;
        let events: Vec<Event> = out
            .iter()
            .map(|l| {
                self.stats.line(l.speaker.is_none());
                Event::TranscriptFinal {
                    meeting: self.meeting.clone(),
                    track: 0,
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
                }
            })
            .collect();
        // Queued for the store before the screen hears of them: a snapshot
        // that notes the bus position and then flushes the store holds every
        // line the bus had announced by then (the screen drops events at or
        // before the snapshot's position).
        if !out.is_empty() {
            let _ = self.persist.send(PersistMsg::Lines(out));
        }
        for e in events {
            self.events.emit(e);
        }
    }

    fn changes(&mut self, changes: Vec<Change>) {
        for c in changes {
            let (ev, id) = match &c {
                Change::Arrived(id) => (
                    Event::SpeakerArrived {
                        meeting: self.meeting.clone(),
                        speaker: speaker_info(&self.tracker, *id),
                    },
                    *id,
                ),
                Change::Confirmed(id) => (
                    Event::SpeakerConfirmed {
                        meeting: self.meeting.clone(),
                        speaker: speaker_info(&self.tracker, *id),
                    },
                    *id,
                ),
                // The phone has no live renames, merges or splits.
                _ => continue,
            };
            // Queued for the store before the screen hears of it (see `final_line`).
            let out = speaker_out(&self.tracker, id);
            let _ = self.persist.send(PersistMsg::Speaker(c, out));
            self.events.emit(ev);
        }
    }
}

/// Everything the engine thread needs.
pub struct EngineCtx {
    pub shared: Arc<Shared>,
    pub backlog: BacklogReader,
    pub provider: EnginesProvider,
    pub applier: Applier,
    pub language: Option<String>,
    /// True once no job is running (a final pass keeps its own models
    /// resident); `None`: nothing to wait for. The live models never load
    /// while this says no.
    pub runner_idle: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
}

/// How often the engine looks again while it waits for the runner.
const IDLE_POLL: Duration = Duration::from_millis(100);

/// Waits until the gate lets the engine run, dropping the models once the
/// app has been inactive for [`UNLOAD_AFTER`] (so a long lock never keeps them
/// resident, whichever wait it is in).
fn wait_active(shared: &Shared, engines: &mut Option<Arc<dyn SpeechEngines>>) {
    while !shared.gate.enter(POLL) {
        if shared.gate.unload_due(UNLOAD_AFTER) && engines.take().is_some() {
            shared.models_unloaded();
        }
    }
}

/// Runs the engine until the session stops and the backlog is drained.
pub fn run(mut ctx: EngineCtx) {
    // Whatever happens (errors, a panic), the session learns the engine is done.
    struct Done(Arc<Shared>);
    impl Drop for Done {
        fn drop(&mut self) {
            self.0.engine_done();
        }
    }
    let _done = Done(ctx.shared.clone());
    if let Err(e) = live_loop(&mut ctx) {
        ctx.shared.engine_failed(e);
        // Record only from here: keep reading so the session can finish
        // (the audio is in the store).
        let _ = drain(&ctx.shared, &mut ctx.backlog);
    }
}

fn live_loop(ctx: &mut EngineCtx) -> Result<(), String> {
    let shared = ctx.shared.clone();
    let gate = &shared.gate;
    let mut engines: Option<Arc<dyn SpeechEngines>> = None;
    // The words of the open utterance shown at a speaker turn: they outlive a
    // reset (the redone utterance's final drops them).
    let prefix = std::cell::RefCell::new(ghi_core::live::Prefix::default());
    loop {
        // Loading uploads weights to the GPU: only while the app is active.
        wait_active(&shared, &mut engines);
        if gate.take_poison() {
            // A load or step overlapped a move to the background: some of
            // its GPU work (possibly weight uploads) may have been refused.
            engines = None;
            shared.models_unloaded();
        }
        if engines.is_none() {
            if ctx.runner_idle.as_ref().is_some_and(|idle| !idle()) {
                // A job (the final pass) still has its models loaded: the live
                // ones wait, the audio keeps filling the backlog.
                gate.leave();
                std::thread::sleep(IDLE_POLL);
                continue;
            }
            let loaded = shared.timed_load(|| (ctx.provider)());
            if gate.leave() {
                // The load itself may be incomplete: load again when active.
                continue;
            }
            engines = Some(loaded?);
        } else if gate.leave() {
            continue;
        }
        let e = engines.as_ref().expect("loaded").clone();
        match live(ctx, &e, &prefix)? {
            Flow::Finished => return Ok(()),
            flow @ (Flow::Reset | Flow::Unload) => {
                if flow == Flow::Unload {
                    // Streams went with `live`; the models go now.
                    engines = None;
                    shared.models_unloaded();
                }
                // Redo from the end of the last committed line; the new
                // streams start there (and renumber speakers).
                let from = shared.committed_position();
                ctx.backlog.rewind(from);
                shared.engine_reset(from);
                ctx.applier.streams_reopened();
            }
        }
    }
}

/// At the stop, more unread audio than this (10 s) is dropped rather than
/// caught up, when a final pass follows ([`Shared::skip_catch_up`]).
const SKIP_BEHIND: u64 = 10 * SAMPLE_RATE as u64;

/// Reads and discards the backlog until the session ends.
fn drain(shared: &Shared, backlog: &mut BacklogReader) -> Result<(), String> {
    while pump(
        shared,
        backlog,
        |_| Ok(Vec::new()),
        || Ok(Vec::new()),
        |_, _, _| {},
    )? != Flow::Finished
    {}
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
    /// Held inactive for [`UNLOAD_AFTER`]: the caller drops streams and models
    /// (the same rewind as a reset) and waits for the app to return.
    Unload,
}

fn live(
    ctx: &mut EngineCtx,
    engines: &Arc<dyn SpeechEngines>,
    prefix: &std::cell::RefCell<ghi_core::live::Prefix>,
) -> Result<Flow, String> {
    use std::cell::{Cell, RefCell};
    let shared = ctx.shared.clone();
    let err = |e: ghi_speech::SpeechError| e.to_string();
    // Opening streams may touch the GPU too.
    while !shared.gate.enter(POLL) {
        if shared.gate.unload_due(UNLOAD_AFTER) {
            return Ok(Flow::Unload);
        }
    }
    let opened = (|| {
        Ok::<_, String>((
            engines.asr(ctx.language.as_deref()).map_err(err)?,
            engines.diar().map_err(err)?,
        ))
    })();
    if shared.gate.leave() {
        return Ok(Flow::Reset);
    }
    let (asr, diar) = opened?;
    let (asr, diar) = (RefCell::new(asr), RefCell::new(diar));
    let stats = ctx.applier.stats();
    // Stream seconds pushed (the diarizer's clock); where the ASR stream in
    // use opened on it; when the utterance in progress showed its first words
    // (and those words, some maybe shown as a turn's line); where to look for
    // the next speaker turn; whether the last line asks for a fresh stream.
    let pushed = Cell::new(0.0f64);
    let asr_base = Cell::new(0.0f64);
    let partial_from = Cell::new(None::<f64>);
    let partial = RefCell::new(String::new());
    // New streams hear the open utterance again from its start.
    prefix.borrow_mut().restart();
    let turn_checked = Cell::new(0.0f64);
    let turn_from = Cell::new(None::<f64>);
    let reset = Cell::new(false);
    let language = ctx.language.clone();
    // A word shows about a chunk after it ends.
    let lag = f64::from(engines.chunk_ms()) / 1000.0 + 0.1;
    // A long utterance (no pause) is cut, and a line the engine asks for starts
    // a fresh stream (as ghi-core live does): checked every step.
    let cut_due = || match partial_from.get() {
        Some(from) => ghi_core::live::cut_due(&partial.borrow(), pushed.get() - from),
        None => reset.get(),
    };
    // A final on the diarizer's clock.
    let line = |r: AsrResult, early: bool| -> Result<Update, String> {
        let base = asr_base.get();
        let words: Vec<Word> = r
            .words
            .into_iter()
            .map(|w| Word {
                start: w.start + base,
                end: w.end + base,
                ..w
            })
            .collect();
        let (start, end) = match (words.first(), words.last()) {
            (Some(a), Some(b)) => (a.start, b.end),
            _ => (r.audio_processed + base, r.audio_processed + base),
        };
        Ok(Update::Final {
            start,
            end,
            text: r.text.trim().to_string(),
            words,
            segs: diar.borrow().segments().map_err(err)?,
            early,
        })
    };
    // Another speaker took over the utterance: the first speaker's words show
    // as their own line now; the stream goes on (as ghi-core live does).
    let show_turn = || -> Result<Option<Update>, String> {
        let (Some(from), now) = (partial_from.get(), pushed.get()) else {
            return Ok(None);
        };
        if now - from < 2.0 * ghi_core::live::TURN_CUT_S
            || now - turn_checked.get() < ghi_core::live::TURN_CHECK_S
        {
            return Ok(None);
        }
        turn_checked.set(now);
        let segs = diar.borrow().segments().map_err(err)?;
        let from = turn_from.get().unwrap_or(from);
        let Some(turn) = ghi_core::live::turn_change(&segs, from) else {
            return Ok(None);
        };
        // The same turn again (the first speaker's segment still runs past it).
        if turn_from.get().is_some_and(|f| turn <= f + 0.01) {
            return Ok(None);
        }
        let turn_s = turn - asr_base.get();
        // The utterance (or the part after the last turn shown) began about a
        // chunk before its first words showed.
        let start = turn_from.get().unwrap_or(from - lag) - asr_base.get();
        let shown = prefix
            .borrow_mut()
            .take(turn_s + ghi_core::live::TURN_SHOW_S, start, turn_s);
        match shown {
            Some(r) => {
                turn_from.set(Some(turn));
                line(r, true).map(Some)
            }
            None => Ok(None),
        }
    };
    // `cut`: false once the session stopped (the stream is already finished;
    // finishing it again would repeat its last final).
    let collect = |cut: bool| -> Result<Vec<Update>, String> {
        let mut out = Vec::new();
        let mut asr = asr.borrow_mut();
        let mut flushed = false;
        loop {
            let Some(r) = asr.next_result().map_err(err)? else {
                if flushed {
                    // The cut's final is out: go on with a fresh stream.
                    *asr = engines.asr(language.as_deref()).map_err(err)?;
                    asr_base.set(pushed.get());
                    reset.set(false);
                    *prefix.borrow_mut() = ghi_core::live::Prefix::default();
                    turn_from.set(None);
                } else if cut && cut_due() {
                    asr.finish().map_err(err)?;
                    flushed = true;
                    partial_from.set(None);
                    continue;
                } else if cut && let Some(early) = show_turn()? {
                    out.push(early);
                    // The screen dropped the words in progress with that line.
                    let rest = prefix.borrow().rest(&partial.borrow()).to_string();
                    if !rest.is_empty() {
                        out.push(Update::Partial(rest));
                    }
                }
                break;
            };
            stats.asr_result(r.audio_processed + asr_base.get());
            if !r.is_final {
                if partial_from.get().is_none() {
                    partial_from.set(Some(pushed.get()));
                }
                let mut p = prefix.borrow_mut();
                p.partial(&r.text, pushed.get() - asr_base.get());
                out.push(Update::Partial(p.rest(&r.text).to_string()));
                partial.replace(r.text);
                continue;
            }
            partial_from.set(None);
            partial.borrow_mut().clear();
            turn_from.set(None);
            if !r.text.trim().is_empty() {
                reset.set(engines.reset_after(r.text.trim()));
            }
            // Without the words a speaker turn already showed.
            let r = prefix.borrow_mut().strip(r);
            if r.text.trim().is_empty() {
                continue;
            }
            out.push(line(r, false)?);
        }
        Ok(out)
    };
    let applier = RefCell::new(&mut ctx.applier);
    pump(
        &shared,
        &mut ctx.backlog,
        |pcm| {
            stats.audio(pcm);
            pushed.set(pushed.get() + pcm.len() as f64 / RATE);
            asr.borrow_mut().push(pcm, SAMPLE_RATE).map_err(err)?;
            diar.borrow_mut().push(pcm, SAMPLE_RATE).map_err(err)?;
            collect(true)
        },
        || {
            asr.borrow_mut().finish().map_err(err)?;
            diar.borrow_mut().finish().map_err(err)?;
            collect(false)
        },
        |updates, offset, now| applier.borrow_mut().apply(updates, offset, now),
    )
}

/// Steps the engine over the backlog; `step` gets each chunk of audio and
/// `finish` runs once the session stopped and everything was read. Their
/// updates are committed to the session and handed to `apply` (with the
/// streams' start and the read position, in samples) unless the step was
/// suspect.
pub fn pump(
    shared: &Shared,
    backlog: &mut BacklogReader,
    mut step: impl FnMut(&[f32]) -> Result<Vec<Update>, String>,
    finish: impl FnOnce() -> Result<Vec<Update>, String>,
    mut apply: impl FnMut(Vec<Update>, u64, u64),
) -> Result<Flow, String> {
    let gate = &shared.gate;
    let mut pcm = Vec::new();
    loop {
        // Stopped far behind with a final pass to follow: the rest is dropped
        // now, with no GPU needed (the pass writes the transcript from the
        // audio; what was live stays until then).
        if gate.stopping()
            && shared.capture_done()
            && shared.skip_catch_up()
            && backlog.available() > SKIP_BEHIND
        {
            log::info!(
                "live engine stopped behind: {:.0} s not caught up (the final pass reads them)",
                backlog.available() as f64 / f64::from(SAMPLE_RATE)
            );
            return Ok(Flow::Finished);
        }
        if !gate.enter(POLL) {
            if gate.unload_due(UNLOAD_AFTER) {
                return Ok(Flow::Unload);
            }
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
                let updates = r?;
                shared.commit(&updates, backlog.position(), true);
                apply(updates, shared.stream_offset(), backlog.position());
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
        let updates = r?;
        shared.commit(&updates, backlog.position(), false);
        apply(updates, shared.stream_offset(), backlog.position());
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

/// Runs `pcm` through fresh ASR + diarization streams as fast as possible
/// (throughput on this device), with the models in `dir`.
#[cfg(feature = "nemo")]
pub fn selftest(dir: &Path, file: String, pcm: &[f32]) -> SelfTest {
    let mut out = SelfTest {
        file,
        audio_s: pcm.len() as f64 / RATE,
        model_load_s: 0.0,
        compute_s: 0.0,
        rtf: 0.0,
        first_final_s: None,
        lines: Vec::new(),
        error: None,
    };
    let t = Instant::now();
    let engines = match nemo_provider(dir)() {
        Ok(e) => e,
        Err(e) => {
            out.error = Some(e);
            return out;
        }
    };
    out.model_load_s = t.elapsed().as_secs_f64();
    let err = |e: ghi_speech::SpeechError| e.to_string();
    let run = |out: &mut SelfTest| -> Result<(), String> {
        let mut asr = engines.asr(None).map_err(err)?;
        let mut diar = engines.diar().map_err(err)?;
        let t = Instant::now();
        let mut results = Vec::new();
        let collect =
            |asr: &mut dyn ghi_speech::AsrStream, results: &mut Vec<_>| -> Result<(), String> {
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
            collect(&mut *asr, &mut results)?;
        }
        asr.finish().map_err(err)?;
        diar.finish().map_err(err)?;
        collect(&mut *asr, &mut results)?;
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

/// The job handlers the phone registers on its [`JobRunner`] (16-G calls this
/// from the mobile core): the final pass without `notes_final` (the phone has
/// no local notes; D5) and, because the voice step queues `voice_learn` when
/// Me is enrolled, the job that learns the voice. Both wait while the models
/// are missing or the device is below the live tier; the runner itself never
/// claims while a session exists or the app is inactive.
///
/// [`JobRunner`]: ghi_core::jobs::JobRunner
pub fn job_handlers(
    models: &Path,
    store: &Arc<ghi_store::store::Store>,
    tier_class: crate::cmd::lifecycle::TierClass,
) -> Vec<Arc<dyn ghi_core::jobs::JobHandler>> {
    use ghi_core::final_pass::{FinalPassJob, FinalPassNoNotes};
    use ghi_core::voice_job::VoiceLearnJob;
    use ghi_core::voice_step::VoiceStep;
    let tier_ok = tier_class == crate::cmd::lifecycle::TierClass::Live;
    let voice_ready: ghi_core::jobs::Ready = {
        let m = models.to_path_buf();
        Arc::new(move || ghi_app::core::voice_ready(&m))
    };
    let third_party = {
        let store = store.clone();
        move || -> Option<ghi_store::voice::ThirdPartyApproved> {
            ghi_app::system::third_party_token(&store)
        }
    };
    let provider = provider(models);
    let m = models.to_path_buf();
    vec![
        Arc::new(FinalPassNoNotes(FinalPassJob {
            engines: Arc::new(move || provider()),
            ready: Arc::new(move || tier_ok && engines_available(&m)),
            // Shorter than the desktop's 10 minutes: a lower peak on the phone.
            chunk_s: 300.0,
            voice: Some(VoiceStep {
                embedder: ghi_app::core::voice_factory(models),
                ready: voice_ready.clone(),
                third_party: Arc::new(third_party.clone()),
            }),
        })),
        Arc::new(VoiceLearnJob {
            embedder: ghi_app::core::voice_factory(models),
            ready: voice_ready,
            third_party: Arc::new(third_party),
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

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
            text: text.into(),
            words: Vec::new(),
            segs: Vec::new(),
            early: false,
        }
    }

    fn setup(name: &str, seconds: usize) -> (Arc<Shared>, BacklogReader, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ghi-engine-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let shared = crate::session::test_shared(&dir);
        let (mut w, r) = crate::backlog::create(&dir.join("t.backlog")).unwrap();
        w.push(&vec![0.1; seconds * SAMPLE_RATE as usize]).unwrap();
        w.publish().unwrap();
        shared.finish_capture_for_test();
        (shared, r, dir)
    }

    fn texts(seen: &Mutex<Vec<(String, u64)>>) -> Vec<String> {
        seen.lock()
            .unwrap()
            .iter()
            .map(|(t, _)| t.clone())
            .collect()
    }

    fn collector(seen: &Mutex<Vec<(String, u64)>>) -> impl FnMut(Vec<Update>, u64, u64) + '_ {
        move |updates, offset, _| {
            for u in updates {
                if let Update::Final { text, .. } = u {
                    seen.lock().unwrap().push((text, offset));
                }
            }
        }
    }

    #[test]
    fn pump_commits_step_results_and_finishes() {
        let (shared, mut backlog, dir) = setup("commit", 1);
        let seen = Mutex::new(Vec::new());
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
            collector(&seen),
        )
        .unwrap();
        assert_eq!(flow, Flow::Finished);
        assert_eq!(texts(&seen), ["hello", "tail"]);
        assert_eq!(
            shared.committed_position(),
            SAMPLE_RATE as u64,
            "nothing pending"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Stopped ten minutes behind with a final pass to follow: done at once,
    /// nothing stepped (no GPU, open or hot phone not needed); a sensitive
    /// meeting (no skip) or a short tail is still read to the end.
    #[test]
    fn a_stop_far_behind_drops_the_backlog_when_a_final_pass_follows() {
        let (shared, mut backlog, dir) = setup("skip", 60);
        shared.set_skip_catch_up(true);
        let mut steps = 0;
        let flow = pump(
            &shared,
            &mut backlog,
            |_| {
                steps += 1;
                Ok(Vec::new())
            },
            || Ok(Vec::new()),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(flow, Flow::Finished);
        assert_eq!(steps, 0, "nothing caught up");
        std::fs::remove_dir_all(dir).unwrap();

        let (shared, mut backlog, dir) = setup("noskip", 60);
        let mut steps = 0;
        let flow = pump(
            &shared,
            &mut backlog,
            |_| {
                steps += 1;
                Ok(Vec::new())
            },
            || Ok(Vec::new()),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(flow, Flow::Finished);
        assert!(steps > 0, "a sensitive meeting is read to the end");
        std::fs::remove_dir_all(dir).unwrap();

        let (shared, mut backlog, dir) = setup("tail", 5);
        shared.set_skip_catch_up(true);
        let mut steps = 0;
        pump(
            &shared,
            &mut backlog,
            |_| {
                steps += 1;
                Ok(Vec::new())
            },
            || Ok(Vec::new()),
            |_, _, _| {},
        )
        .unwrap();
        assert!(steps > 0, "a short tail is still read");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_step_that_ends_while_inactive_is_dropped_and_redone() {
        let (shared, mut backlog, dir) = setup("reset", 2);
        let seen = Mutex::new(Vec::new());
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
            collector(&seen),
        )
        .unwrap();
        assert_eq!(flow, Flow::Reset);
        assert_eq!(texts(&seen), ["one"], "the suspect step's line was dropped");
        assert_eq!(shared.gate.steps_while_inactive(), 1);
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
            collector(&seen),
        )
        .unwrap();
        assert_eq!(flow, Flow::Finished);
        let seen = seen.lock().unwrap();
        assert_eq!(seen[1].0, "two");
        assert_eq!(
            seen[1].1, from,
            "offset by the rewind position, for the applier to add"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_suspect_step_is_never_persisted_through_the_applier() {
        use ghi_core::events::bus;
        let (shared, mut backlog, dir) = setup("applier", 2);
        let (tx, _rx) = bus();
        let (ptx, prx) = crossbeam_channel::unbounded();
        let mut applier = Applier::new("m".into(), None, tx, ptx);
        let word = |t: &str, a: f64, b: f64| Word {
            text: t.into(),
            start: a,
            end: b,
            confidence: 0.9,
            speaker: None,
        };
        let line = |t: &str, a: f64, b: f64| Update::Final {
            start: a,
            end: b,
            text: t.into(),
            words: vec![word(t, a, b)],
            segs: vec![seg(1, 0.0, 1.0)],
            early: false,
        };
        let mut steps = 0;
        let flow = pump(
            &shared,
            &mut backlog,
            |_| {
                steps += 1;
                if steps == 1 {
                    Ok(vec![line("one", 0.0, 0.05)])
                } else {
                    shared.gate.suspend();
                    Ok(vec![line("suspect", 0.1, 0.3)])
                }
            },
            || Ok(Vec::new()),
            |u, off, now| applier.apply(u, off, now),
        )
        .unwrap();
        assert_eq!(flow, Flow::Reset);
        let lines: Vec<String> = prx
            .try_iter()
            .filter_map(|m| match m {
                PersistMsg::Lines(l) => Some(l.into_iter().map(|l| l.text).collect::<Vec<_>>()),
                _ => None,
            })
            .flatten()
            .collect();
        assert_eq!(lines, ["one"], "only the validated step reached persist");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Stop while the app is in the background: nothing runs until it
    /// returns, then the backlog drains and the engine finishes.
    #[test]
    fn stop_while_suspended_drains_and_finishes_after_resume() {
        let (shared, backlog, dir) = setup("suspended", 3);
        shared.gate.suspend();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let t = {
            let (shared, seen) = (shared.clone(), seen.clone());
            let mut backlog = backlog;
            std::thread::spawn(move || {
                pump(
                    &shared,
                    &mut backlog,
                    |_| Ok(Vec::new()),
                    || Ok(vec![final_line(0.0, 0.5, "tail")]),
                    collector(&seen),
                )
            })
        };
        std::thread::sleep(Duration::from_millis(150));
        assert!(!t.is_finished(), "held while inactive");
        assert!(seen.lock().unwrap().is_empty());
        shared.gate.resume();
        assert_eq!(t.join().unwrap().unwrap(), Flow::Finished);
        assert_eq!(texts(&seen), ["tail"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn inactive_for_thirty_seconds_unloads() {
        let (shared, mut backlog, dir) = setup("unload", 1);
        shared.gate.suspend();
        shared
            .gate
            .pretend_suspended_for(UNLOAD_AFTER + Duration::from_secs(1));
        let flow = pump(
            &shared,
            &mut backlog,
            |_| Ok(Vec::new()),
            || Ok(Vec::new()),
            |_, _, _| {},
        )
        .unwrap();
        assert_eq!(flow, Flow::Unload);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Models that count themselves, and a recognizer that takes the app
    /// into the background in the middle of its first step.
    struct Lockable {
        alive: Arc<std::sync::atomic::AtomicUsize>,
        shared: Arc<Shared>,
        /// Only the first step of the first stream takes the app away.
        locked_once: Arc<std::sync::atomic::AtomicBool>,
    }
    struct LockingAsr(Arc<Shared>, Arc<std::sync::atomic::AtomicBool>);
    impl ghi_speech::AsrStream for LockingAsr {
        fn push(&mut self, _: &[f32], _: u32) -> ghi_speech::Result<()> {
            if !self.1.swap(true, std::sync::atomic::Ordering::SeqCst) {
                self.0.gate.suspend();
                self.0
                    .gate
                    .pretend_suspended_for(UNLOAD_AFTER + Duration::from_secs(1));
            }
            Ok(())
        }
        fn finish(&mut self) -> ghi_speech::Result<()> {
            Ok(())
        }
        fn next_result(&mut self) -> ghi_speech::Result<Option<ghi_speech::AsrResult>> {
            Ok(None)
        }
    }
    impl SpeechEngines for Lockable {
        fn asr(&self, _: Option<&str>) -> ghi_core::engines::Result<ghi_core::engines::BoxAsr> {
            Ok(Box::new(LockingAsr(
                self.shared.clone(),
                self.locked_once.clone(),
            )))
        }
        fn diar(&self) -> ghi_core::engines::Result<ghi_core::engines::BoxDiar> {
            ghi_core::engines::FakeEngines::new(ghi_core::engines::Script::default()).diar()
        }
        fn chunk_ms(&self) -> u32 {
            1120
        }
    }
    impl Drop for Lockable {
        fn drop(&mut self) {
            self.alive.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// A lock that lands in a step must not leave the models resident for the
    /// whole lock: the step is dropped, and the wait that follows unloads them
    /// after 30 s inactive.
    #[test]
    fn a_lock_that_lands_in_a_step_unloads_the_models_while_waiting() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (shared, backlog, dir) = setup("lockunload", 2);
        // Capture is still running: the engine must wait, not finish.
        shared.unfinish_capture_for_test();
        let alive = Arc::new(AtomicUsize::new(0));
        let once = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let provider: EnginesProvider = {
            let (alive, shared) = (alive.clone(), shared.clone());
            Arc::new(move || {
                alive.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(Lockable {
                    alive: alive.clone(),
                    shared: shared.clone(),
                    locked_once: once.clone(),
                }) as Arc<dyn SpeechEngines>)
            })
        };
        let (tx, _rx) = ghi_core::events::bus();
        let (ptx, _prx) = crossbeam_channel::unbounded();
        let ctx = EngineCtx {
            shared: shared.clone(),
            backlog,
            provider,
            applier: Applier::new("m".into(), None, tx, ptx),
            language: None,
            runner_idle: None,
        };
        let t = std::thread::spawn(move || run(ctx));
        let start = Instant::now();
        while alive.load(Ordering::SeqCst) != 0 || start.elapsed() < Duration::from_millis(100) {
            assert!(start.elapsed() < Duration::from_secs(10), "never unloaded");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(shared.gate.suspended_for().is_some(), "still locked");
        // Back in the foreground the models load again.
        shared.gate.resume();
        let start = Instant::now();
        while alive.load(Ordering::SeqCst) != 1 {
            assert!(start.elapsed() < Duration::from_secs(10), "never reloaded");
            std::thread::sleep(Duration::from_millis(10));
        }
        shared.finish_capture_for_test();
        t.join().unwrap();
        assert_eq!(alive.load(Ordering::SeqCst), 0, "dropped with the thread");
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

    fn partials_sent(rx: &ghi_core::events::EventRx) -> Vec<String> {
        rx.try_iter()
            .filter_map(|e| match e.event {
                Event::TranscriptPartial { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_unchanged_partial_is_not_sent_again_until_the_keepalive() {
        let (tx, rx) = ghi_core::events::bus();
        let (ptx, _prx) = crossbeam_channel::unbounded();
        let mut applier = Applier::new("m".into(), None, tx, ptx);
        applier.keepalive = Duration::from_millis(60);
        let step = |a: &mut Applier, text: &str| a.apply(vec![Update::Partial(text.into())], 0, 0);
        // Ten steps of one hypothesis, then it grows.
        for _ in 0..10 {
            step(&mut applier, "chốt scope");
        }
        step(&mut applier, "chốt scope cho");
        assert_eq!(partials_sent(&rx), ["chốt scope", "chốt scope cho"]);
        // Still unchanged after the keepalive: a screen that lost it gets it back.
        std::thread::sleep(Duration::from_millis(80));
        step(&mut applier, "chốt scope cho");
        step(&mut applier, "chốt scope cho");
        assert_eq!(partials_sent(&rx), ["chốt scope cho"]);
    }

    #[test]
    fn the_same_partial_after_a_line_or_a_discard_is_sent_again() {
        let (tx, rx) = ghi_core::events::bus();
        let (ptx, _prx) = crossbeam_channel::unbounded();
        let cuts = Cuts::default();
        let mut applier = Applier::new("m".into(), None, tx, ptx).with_cuts(cuts.clone());
        let partial =
            |a: &mut Applier, now: u64| a.apply(vec![Update::Partial("and so".into())], 0, now);
        partial(&mut applier, 0);
        partial(&mut applier, 0);
        assert_eq!(partials_sent(&rx).len(), 1);
        // A line clears the screen's partial: the same words are news again.
        let word = Word {
            text: "hi".into(),
            start: 0.0,
            end: 0.5,
            confidence: 0.9,
            speaker: None,
        };
        applier.apply(
            vec![Update::Final {
                start: 0.0,
                end: 0.5,
                text: "hi".into(),
                words: vec![word],
                segs: vec![seg(1, 0.0, 1.0)],
                early: false,
            }],
            0,
            0,
        );
        partial(&mut applier, 0);
        assert_eq!(partials_sent(&rx), ["and so"]);
        // A discard drops the partial on the screen; the engine is inside the cut,
        // then its partial (the same words) comes back after it.
        cuts.lock().unwrap().push((0, 1000));
        partial(&mut applier, 16_000 / 2);
        assert!(partials_sent(&rx).is_empty(), "inside the cut");
        partial(&mut applier, 16_000 * 2);
        assert_eq!(partials_sent(&rx), ["and so"]);
    }

    /// Runs `talk` through the engine; the lines (text, start ms) and the ASR
    /// streams opened.
    fn talk_lines(
        name: &str,
        talk: ghi_core::engines::Talk,
        seconds: usize,
    ) -> (Vec<(String, i64)>, u32) {
        let (events, opened) = talk_events(name, talk, seconds);
        let lines = events
            .into_iter()
            .filter_map(|e| match e {
                Event::TranscriptFinal { line, .. } => Some((line.text, line.t0_ms)),
                _ => None,
            })
            .collect();
        (lines, opened)
    }

    /// Runs `talk` through the engine; its events and the ASR streams opened.
    fn talk_events(name: &str, talk: ghi_core::engines::Talk, seconds: usize) -> (Vec<Event>, u32) {
        let (shared, backlog, dir) = setup(name, seconds);
        let engines = ghi_core::engines::TalkEngines::new(talk);
        let provider: EnginesProvider = {
            let e = engines.clone();
            Arc::new(move || Ok(e.clone() as Arc<dyn SpeechEngines>))
        };
        let (tx, rx) = ghi_core::events::bus();
        let (ptx, _prx) = crossbeam_channel::unbounded();
        run(EngineCtx {
            shared,
            backlog,
            provider,
            applier: Applier::new("m".into(), None, tx, ptx),
            language: None,
            runner_idle: None,
        });
        std::fs::remove_dir_all(dir).unwrap();
        (rx.try_iter().map(|e| e.event).collect(), engines.opened())
    }

    /// Another speaker taking over shows the first speaker's words while the
    /// utterance goes on (the stream is not reopened); every word once.
    #[test]
    fn another_speakers_turn_shows_the_first_speakers_words_early() {
        let talk = ghi_core::engines::Talk {
            words: words(20, "w"),
            start: 0.5,
            step: 0.5,
            pauses: Vec::new(),
            turns: vec![seg(1, 0.0, 5.0), seg(2, 5.0, 11.0)],
            show_lag: 0.5,
            diar_lag: 1.0,
        };
        let (events, opened) = talk_events("turn", talk.clone(), 12);
        assert_eq!(opened, 1);
        let lines: Vec<&LineInfo> = events
            .iter()
            .filter_map(|e| match e {
                Event::TranscriptFinal { line, .. } => Some(line),
                _ => None,
            })
            .collect();
        assert!(
            lines[0].t1_ms <= 5_000 && lines[0].text.starts_with("w1 w2"),
            "{lines:?}"
        );
        // Every word with its own speaker: w1..w9 the first's, w10.. the second's.
        for l in &lines {
            for w in l.text.split_whitespace() {
                let n: usize = w[1..].parse().unwrap();
                assert_eq!(l.speaker == lines[0].speaker, n <= 9, "{w} in {lines:?}");
            }
        }
        let heard: Vec<&str> = lines
            .iter()
            .flat_map(|l| l.text.split_whitespace())
            .collect();
        assert_eq!(heard, talk.words);
        let at = events
            .iter()
            .position(|e| matches!(e, Event::TranscriptFinal { .. }))
            .unwrap();
        let later: Vec<&str> = events[at..]
            .iter()
            .filter_map(|e| match e {
                Event::TranscriptPartial { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(!later.is_empty(), "shown before the utterance ended");
        assert!(later.iter().all(|t| !t.starts_with("w1 ")), "{later:?}");
    }

    fn words(n: usize, w: &str) -> Vec<String> {
        (1..=n).map(|i| format!("{w}{i}")).collect()
    }

    /// Continuous speech without a pause is cut into lines (the stream is
    /// reopened): every word once, in order.
    #[test]
    fn a_long_utterance_is_cut_without_losing_or_repeating_words() {
        let talk = ghi_core::engines::Talk {
            words: words(60, "w"),
            start: 0.5,
            step: 0.5,
            pauses: Vec::new(),
            turns: Vec::new(),
            show_lag: 0.0,
            diar_lag: 0.0,
        };
        let (lines, opened) = talk_lines("longcut", talk.clone(), 32);
        assert!(lines.len() >= 2 && opened >= 2, "cut: {lines:?}");
        let heard: Vec<&str> = lines.iter().flat_map(|l| l.0.split_whitespace()).collect();
        assert_eq!(heard, talk.words);
    }

    /// A Vietnamese line starts a fresh stream; its words stay on the clock;
    /// one the stop produced is not finished twice (a repeated line).
    #[test]
    fn a_vietnamese_line_starts_a_fresh_stream() {
        let talk = ghi_core::engines::Talk {
            words: words(12, "đ"),
            start: 0.5,
            step: 0.5,
            // The last line comes out of the stop itself (no pause before it).
            pauses: vec![4, 8],
            turns: Vec::new(),
            show_lag: 0.0,
            diar_lag: 0.0,
        };
        let (lines, opened) = talk_lines("vireset", talk.clone(), 10);
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(opened >= 3, "{opened}");
        let heard: Vec<&str> = lines.iter().flat_map(|l| l.0.split_whitespace()).collect();
        assert_eq!(heard, talk.words);
        // The third line (from word 9, after two 1 s pauses) on the meeting clock.
        assert_eq!(lines[2].1, 6_500, "{lines:?}");
    }

    #[test]
    fn a_line_is_queued_for_the_store_before_the_screen_hears_of_it() {
        let (tx, rx) = ghi_core::events::bus();
        let (ptx, prx) = crossbeam_channel::unbounded();
        let mut applier = Applier::new("m".into(), None, tx, ptx);
        // The screen's side: when the line event arrives, the store must have it.
        let seen = std::thread::spawn(move || {
            for e in rx {
                if matches!(e.event, Event::TranscriptFinal { .. }) {
                    return prx.len();
                }
            }
            0
        });
        let word = Word {
            text: "hi".into(),
            start: 0.0,
            end: 0.5,
            confidence: 0.9,
            speaker: None,
        };
        applier.apply(
            vec![Update::Final {
                start: 0.0,
                end: 0.5,
                text: "hi".into(),
                words: vec![word],
                segs: vec![seg(1, 0.0, 1.0)],
                early: false,
            }],
            0,
            0,
        );
        drop(applier);
        assert!(seen.join().unwrap() >= 1, "the persist message came first");
    }

    #[test]
    fn ui_stats_count_a_window_and_the_gap_still_open() {
        let stats = UiStats::default();
        stats.audio(&[0.5; 4]);
        stats.asr_result(1.0);
        stats.partial("a");
        stats.partial("a");
        stats.partial("ab");
        stats.final_result();
        stats.line(true);
        std::thread::sleep(Duration::from_millis(30));
        let w = stats.take();
        assert_eq!((w.partials, w.partials_same), (3, 1));
        assert_eq!((w.finals, w.lines, w.lines_unattributed), (1, 1, 1));
        assert_eq!((w.steps, w.asr_results), (1, 1));
        assert!((w.audio_rms - 0.5).abs() < 1e-9);
        assert!(
            w.max_update_gap_s >= 0.03,
            "the gap since the last change counts"
        );
        // The next window starts empty, but the open gap carries on.
        std::thread::sleep(Duration::from_millis(30));
        let w = stats.take();
        assert_eq!((w.partials, w.finals, w.lines), (0, 0, 0));
        assert!(w.max_update_gap_s >= 0.06);
    }

    #[test]
    fn a_speaker_is_queued_for_the_store_before_the_screen_hears_of_it() {
        let (tx, rx) = ghi_core::events::bus();
        let (ptx, prx) = crossbeam_channel::unbounded();
        let mut applier = Applier::new("m".into(), None, tx, ptx);
        let seen = std::thread::spawn(move || {
            for e in rx {
                if matches!(e.event, Event::SpeakerArrived { .. }) {
                    return prx.len();
                }
            }
            0
        });
        let word = Word {
            text: "hi".into(),
            start: 0.0,
            end: 0.5,
            confidence: 0.9,
            speaker: None,
        };
        applier.apply(
            vec![Update::Final {
                start: 0.0,
                end: 0.5,
                text: "hi".into(),
                words: vec![word],
                segs: vec![seg(1, 0.0, 1.0)],
                early: false,
            }],
            0,
            0,
        );
        drop(applier);
        assert!(seen.join().unwrap() >= 1, "the persist message came first");
    }
}
