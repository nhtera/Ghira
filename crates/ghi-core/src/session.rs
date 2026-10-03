// SPDX-License-Identifier: Apache-2.0
//! A recording session: capture → pump → bundles, ASR ring → engine → persist.
//!
//! ```text
//!  Capture rings ─► pump thread: ghi-audio Pipeline ─┬─► OpusRecorder ─► BundlePages (store bundles)
//!                    (10 ms step, never waits)       └─► ASR ring ─► engine thread ─► persist thread ─► Store
//!                                                                         └──────────────► event bus
//! ```
//!
//! State machine (brief §7): Idle → Starting → Recording ⇄ Paused → Stopping
//! → Processing (jobs) → Ready | Failed. At stop the meeting gets a
//! `notes_live` job (notes from the live transcript, ≤3 min) and a
//! `final_pass` job [RT-7].
//!
//! Without speech engines (models not installed yet) a session records
//! only: audio, marks and discards work, there is no live transcript, and the
//! jobs queued at stop wait until the models arrive ("record now, process
//! later").

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Sender, bounded, unbounded};
use ghi_audio::encoder::{EncoderConfig, OpusRecorder};
use ghi_audio::pipeline::{Pipeline, PipelineConfig};
use ghi_audio::{SAMPLE_RATE, Track};
use ghi_store::store::{NewMeeting, Store, TrackKind};

use crate::capture::Capture;
use crate::engines::SpeechEngines;
use crate::events::{
    ErrorKind, Event, EventTx, LineInfo, SessionSnapshot, SessionState, SpeakerInfo, WordInfo,
};
use crate::live::{Engine, EngineCmd, LiveConfig, Mode, PersistMsg};
use crate::pages::{BundlePages, Metered, Muted};
use crate::persist::Persist;
use crate::speakers::SpeakerId;

/// Job kinds queued at stop.
pub const NOTES_LIVE_JOB: &str = "notes_live";
pub const FINAL_PASS_JOB: &str = "final_pass";
/// Payload shape version of the jobs this build writes.
pub const JOB_PAYLOAD_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub mode: Mode,
    pub language: Option<String>,
    pub title: String,
    /// Queue the notes and final-pass jobs at stop.
    pub queue_jobs: bool,
    /// Never skip audio for the live transcript: wait for the engine instead
    /// (fast replays for eval and soak runs; live capture keeps real time and
    /// lets the final pass fill what was skipped).
    pub lossless: bool,
}

/// In lossless mode the pump waits while this many ASR frames (5 s) wait.
const LOSSLESS_BACKLOG: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionError(pub String);

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err(what: &str, e: impl std::fmt::Display) -> SessionError {
    SessionError(format!("{what}: {e}"))
}

fn kind_of(t: Track) -> TrackKind {
    match t {
        Track::Mic => TrackKind::Mic,
        Track::System => TrackKind::System,
    }
}

enum PumpCmd {
    Pause,
    Resume,
    /// Flushes the page in progress, picks the pages to keep per track (with
    /// each file's nonce prefix), mutes the audio still in the pipeline and
    /// starts holding new pages. Replies with the keep list and the position
    /// up to which audio counts as discarded.
    Discard {
        t_cut_ms: i64,
        reply: Sender<Discarding>,
    },
    /// Rotates the bundles to `keep` and writes the held pages. An empty
    /// `keep` just stops holding (a discard that failed).
    Release {
        keep: Vec<(Track, u32)>,
        reply: Sender<Result<(), String>>,
    },
    Stop,
}

struct Discarding {
    keep: Vec<(Track, u32, Option<String>)>,
    /// Timeline position (samples) before which audio is discarded.
    mute_until: u64,
}

/// Something that must know when recording starts and stops (the job
/// runner pauses heavy jobs while recording [RT-10]).
pub trait RecordingHooks: Send + Sync {
    fn recording_started(&self);
    fn recording_stopped(&self);
    /// Waits (at most `max`) for the job that was running when recording
    /// started to yield, so its model is gone before the speech engines
    /// load. Jobs yield at their safe points (the final pass between chunks;
    /// the notes job only before it starts, so a run in progress may outlast
    /// `max`). Returns whether nothing is running.
    fn wait_idle(&self, _max: Duration) -> bool {
        true
    }
}

/// How long `start` waits for a running job to yield before loading engines.
const JOB_YIELD_WAIT: Duration = Duration::from_secs(5);
/// How long a second edit (discard, split, snapshot) waits for the first.
const OP_WAIT: Duration = Duration::from_secs(15);
/// The level meter reports at most this often.
const LEVEL_EVERY: Duration = Duration::from_millis(100);

/// What `stop` reports.
#[derive(Debug, Clone, PartialEq)]
pub struct StopReport {
    pub meeting: String,
    pub duration_ms: i64,
    pub jobs: Vec<i64>,
}

pub struct Session {
    meeting: String,
    store: Arc<Store>,
    events: EventTx,
    state: Arc<Mutex<SessionState>>,
    pump: Sender<PumpCmd>,
    engine: Sender<EngineCmd>,
    persist: Option<Sender<PersistMsg>>,
    /// Timeline position (16 kHz samples) the pump has reached.
    position: Arc<AtomicU64>,
    /// The capture source ended by itself (a replay played out).
    source_ended: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    /// A live transcript is being made (engines were given).
    transcribing: bool,
    cfg: SessionConfig,
    hooks: Option<Arc<dyn RecordingHooks>>,
    /// Held by the multi-step operations (discard, split, snapshot) so they
    /// never interleave.
    ops: Mutex<()>,
}

impl Session {
    /// Creates the meeting and starts recording from `capture`; with no
    /// `engines`, records without a live transcript. The engines are already
    /// loaded: prefer [`Session::start_with_loader`], which loads them after
    /// the jobs paused.
    pub fn start(
        store: Arc<Store>,
        engines: Option<Arc<dyn SpeechEngines>>,
        capture: Capture,
        cfg: SessionConfig,
        events: EventTx,
        hooks: Option<Arc<dyn RecordingHooks>>,
    ) -> Result<Session, SessionError> {
        Session::start_with_loader(store, move || Ok(engines), capture, cfg, events, hooks)
    }

    /// Like [`Session::start`], but the speech engines come from `load_engines`,
    /// which runs after `hooks.recording_started()` (heavy jobs pause and get
    /// [`JOB_YIELD_WAIT`] to release their models, so the LLM and the ASR are
    /// not resident together on small machines) and after capture is being
    /// pumped (the first seconds are not lost to the load). `Ok(None)` records
    /// without a live transcript; `Err` abandons the start: jobs resume and
    /// the half-made meeting is removed. `recording_started` is called once.
    pub fn start_with_loader(
        store: Arc<Store>,
        load_engines: impl FnOnce() -> Result<Option<Arc<dyn SpeechEngines>>, SessionError>,
        mut capture: Capture,
        mut cfg: SessionConfig,
        events: EventTx,
        hooks: Option<Arc<dyn RecordingHooks>>,
    ) -> Result<Session, SessionError> {
        let tracks: Vec<Track> = [
            capture.mic.is_some().then_some(Track::Mic),
            capture.system.is_some().then_some(Track::System),
        ]
        .into_iter()
        .flatten()
        .collect();
        if tracks.is_empty() {
            return Err(SessionError("the capture has no tracks".into()));
        }
        // A call without the far end's audio is one mic: diarize it.
        if cfg.mode == Mode::Call && !tracks.contains(&Track::System) {
            cfg.mode = Mode::Room;
        }
        if let Some(h) = &hooks {
            h.recording_started();
        }
        // Until start succeeds, a failure resumes the jobs and removes the
        // half-made meeting.
        struct Undo<'a> {
            hooks: Option<&'a Arc<dyn RecordingHooks>>,
            store: &'a Store,
            meeting: Option<String>,
            armed: bool,
        }
        impl Drop for Undo<'_> {
            fn drop(&mut self) {
                if !self.armed {
                    return;
                }
                if let Some(h) = self.hooks {
                    h.recording_stopped();
                }
                if let Some(m) = &self.meeting {
                    let _ = self.store.delete_meeting(m);
                }
            }
        }
        let mut undo = Undo {
            hooks: hooks.as_ref(),
            store: &store,
            meeting: None,
            armed: true,
        };
        let meeting = store
            .create_meeting(NewMeeting {
                title: cfg.title.clone(),
                started_at: 0,
                source: "live".into(),
                mode: cfg.mode.as_str().into(),
                lang: cfg.language.clone(),
                ..Default::default()
            })
            .map_err(|e| err("creating the meeting", e))?
            .gid;
        undo.meeting = Some(meeting.clone());
        let state = Arc::new(Mutex::new(SessionState::Starting));
        events.emit(Event::StateChanged {
            meeting: meeting.clone(),
            state: SessionState::Starting,
        });

        let mut writers: [Option<ghi_store::bundle::BundleWriter>; 2] = [None, None];
        for &t in &tracks {
            writers[t.index()] = Some(
                store
                    .open_track(&meeting, kind_of(t))
                    .map_err(|e| err("opening the audio file", e))?,
            );
        }
        let recorder = Metered::new(Muted::new(
            OpusRecorder::new(BundlePages::new(writers), &tracks, EncoderConfig::default())
                .map_err(|e| err("audio encoder", e))?,
        ));
        let pcfg = PipelineConfig {
            route: capture.route,
            aec_enabled: cfg.mode == Mode::Call,
            call_detected: cfg.mode == Mode::Call,
            ..PipelineConfig::default()
        };
        let (pipeline, mut asr_ring) =
            Pipeline::new(pcfg, capture.mic.take(), capture.system.take(), recorder);
        if cfg.lossless {
            asr_ring.never_skip();
        }

        let (persist_tx, persist_rx) = unbounded();
        let (engine_tx, engine_rx) = unbounded();
        let (pump_tx, pump_rx) = unbounded();
        let capture_done = Arc::new(AtomicBool::new(false));
        let position = Arc::new(AtomicU64::new(0));
        let source_ended = Arc::new(AtomicBool::new(false));
        // Until the engines are known, a lossless replay waits for them.
        let lossless = Arc::new(AtomicBool::new(cfg.lossless));
        let mut threads = Vec::new();

        let persist = Persist::new(store.clone(), meeting.clone(), events.clone());
        threads.push(spawn("ghi-persist", move || persist.run(persist_rx))?);
        {
            let ctx = PumpCtx {
                lossless: lossless.clone(),
                store: store.clone(),
                meeting: meeting.clone(),
                events: events.clone(),
                tracks: tracks.clone(),
                position: position.clone(),
                capture_done: capture_done.clone(),
                source_ended: source_ended.clone(),
            };
            threads.push(spawn("ghi-pump", move || {
                pump(ctx, pipeline, capture, pump_rx)
            })?);
        }

        // Audio is being pumped (and kept) while the engines load.
        // The pump keeps the capture ring drained while a running job gets
        // time to release its model.
        if let Some(h) = &hooks {
            h.wait_idle(JOB_YIELD_WAIT);
        }
        let loaded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            load_engines().and_then(|engines| {
                engines
                    .map(|engines| {
                        Engine::new(
                            LiveConfig {
                                meeting: meeting.clone(),
                                mode: cfg.mode,
                                language: cfg.language.clone(),
                            },
                            engines,
                            &tracks,
                            events.clone(),
                            persist_tx.clone(),
                        )
                        .map_err(|e| err("speech engine", e))
                    })
                    .transpose()
            })
        }));
        let engine = loaded
            .unwrap_or_else(|_| Err(SessionError("loading the speech engines panicked".into())));
        let engine = match engine {
            Ok(e) => e,
            Err(e) => return Err(abort(pump_tx, persist_tx, threads, e)),
        };
        let live = engine.is_some();
        // Nothing reads the ASR ring without engines: never wait on it.
        cfg.lossless &= live;
        lossless.store(cfg.lossless, Ordering::Release);
        if let Some(engine) = engine {
            let done = capture_done;
            match spawn("ghi-engine", move || engine.run(asr_ring, engine_rx, done)) {
                Ok(t) => threads.push(t),
                Err(e) => return Err(abort(pump_tx, persist_tx, threads, e)),
            }
        } else {
            // Speaker edits go nowhere: there are no live speakers.
            drop((asr_ring, engine_rx));
        }

        undo.armed = false;
        drop(undo);
        if !live {
            events.emit(Event::Error {
                meeting: Some(meeting.clone()),
                kind: ErrorKind::ModelsMissing,
                message: "no speech engines (models missing): recording only; \
                          the transcript and notes come once they are ready"
                    .into(),
            });
        }
        *state.lock().unwrap_or_else(|e| e.into_inner()) = SessionState::Recording;
        events.emit(Event::SessionStarted {
            meeting: meeting.clone(),
            mode: cfg.mode.as_str().into(),
            language: cfg.language.clone(),
            title: cfg.title.clone(),
        });
        events.emit(Event::StateChanged {
            meeting: meeting.clone(),
            state: SessionState::Recording,
        });
        Ok(Session {
            meeting,
            store,
            events,
            state,
            pump: pump_tx,
            engine: engine_tx,
            persist: Some(persist_tx),
            position,
            source_ended,
            threads,
            transcribing: live,
            cfg,
            hooks,
            ops: Mutex::new(()),
        })
    }

    pub fn meeting(&self) -> &str {
        &self.meeting
    }

    pub fn state(&self) -> SessionState {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_state(&self, s: SessionState) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = s;
        log::info!("session state {s:?}");
        self.events.emit(Event::StateChanged {
            meeting: self.meeting.clone(),
            state: s,
        });
    }

    /// Meeting time reached by the recording (ms).
    pub fn now_ms(&self) -> i64 {
        (self.position.load(Ordering::Relaxed) * 1000 / u64::from(SAMPLE_RATE)) as i64
    }

    /// A live transcript is being made (false: recording only, models missing).
    pub fn transcribing(&self) -> bool {
        self.transcribing
    }

    /// A replay played out (live capture never ends by itself).
    pub fn source_ended(&self) -> bool {
        self.source_ended.load(Ordering::Acquire)
    }

    pub fn pause(&self) {
        if self.state() == SessionState::Recording {
            let _ = self.pump.send(PumpCmd::Pause);
            self.set_state(SessionState::Paused);
        }
    }

    pub fn resume(&self) {
        if self.state() == SessionState::Paused {
            let _ = self.pump.send(PumpCmd::Resume);
            self.set_state(SessionState::Recording);
        }
    }

    pub fn mark(&self) -> i64 {
        let t_ms = self.now_ms();
        if let Some(p) = &self.persist {
            let _ = p.send(PersistMsg::Mark { t_ms });
        }
        self.events.emit(Event::MarkAdded {
            meeting: self.meeting.clone(),
            t_ms,
        });
        t_ms
    }

    pub fn rename(&self, id: SpeakerId, name: &str) {
        let _ = self.engine.send(EngineCmd::Rename {
            id,
            name: name.to_string(),
        });
    }

    pub fn merge(&self, from: SpeakerId, into: SpeakerId) {
        let _ = self.engine.send(EngineCmd::Merge { from, into });
    }

    pub fn not_a_person(&self, id: SpeakerId) {
        let _ = self.engine.send(EngineCmd::NotAPerson { id });
    }

    /// Takes the lock of the multi-step operations; a second caller waits up
    /// to [`OP_WAIT`] for the first, then gets an error.
    fn op_lock(&self) -> Result<MutexGuard<'_, ()>, SessionError> {
        let end = Instant::now() + OP_WAIT;
        loop {
            match self.ops.try_lock() {
                Ok(g) => return Ok(g),
                Err(TryLockError::Poisoned(p)) => return Ok(p.into_inner()),
                Err(TryLockError::WouldBlock) if Instant::now() < end => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(SessionError("another edit is still running".into()));
                }
            }
        }
    }

    /// What a reloaded webview needs to redraw this session: state, speakers
    /// (from the engine; from the store without one), the final lines and the
    /// marks stored so far. Events with a `seq` above the snapshot's follow.
    pub fn snapshot(&self) -> SessionSnapshot {
        let wait = Duration::from_secs(10);
        // Not interleaved with a discard (which would show half of itself).
        let _op = self.op_lock().ok();
        // Read before the engine is asked: every line announced up to `seq`
        // reached the persist thread before the engine answers.
        let seq = self.events.last_seq();
        let live_speakers = self.transcribing.then(|| {
            let (tx, rx) = bounded(1);
            self.engine.send(EngineCmd::Snapshot { reply: tx }).ok()?;
            rx.recv_timeout(wait).ok()
        });
        let mut gids: Vec<(SpeakerId, String)> = Vec::new();
        if let Some(p) = &self.persist {
            // The lines still waiting to be written, then who is who.
            let (tx, rx) = bounded(1);
            if p.send(PersistMsg::Flush(tx)).is_ok() {
                let _ = rx.recv_timeout(wait);
            }
            let (tx, rx) = bounded(1);
            if p.send(PersistMsg::SpeakerGids(tx)).is_ok()
                && let Ok(v) = rx.recv_timeout(wait)
            {
                gids = v;
            }
        }
        let stored = self.store.speakers(&self.meeting).unwrap_or_default();
        let mut speakers = live_speakers.flatten();
        if speakers.is_none() {
            // No engine (or it did not answer): the store's speakers, numbered
            // in order.
            let own: Vec<_> = stored.iter().filter(|s| s.merged_into.is_none()).collect();
            gids = own
                .iter()
                .zip(1u32..)
                .map(|(s, id)| (id, s.gid.clone()))
                .collect();
            speakers = Some(
                own.iter()
                    .zip(1u32..)
                    .map(|(s, id)| SpeakerInfo {
                        id,
                        label: match (&s.display_name, s.is_me, s.label_idx) {
                            (Some(n), _, _) => n.clone(),
                            (None, true, _) => "Me".into(),
                            (None, false, i) if i < 0 => "Identifying…".into(),
                            (None, false, i) => format!("Speaker {}", i + 1),
                        },
                        color_slot: s.color_slot.clamp(0, 8) as u8,
                        is_me: s.is_me,
                        provisional: !s.is_me && s.label_idx < 0,
                        not_person: s.not_person,
                        others: s.color_slot == 0,
                    })
                    .collect(),
            );
        }
        let mut by_gid = std::collections::HashMap::new();
        for (id, gid) in gids {
            by_gid.entry(gid).or_insert(id);
        }
        let lines = self
            .store
            .segments(&self.meeting)
            .unwrap_or_default()
            .into_iter()
            .map(|seg| {
                let words = self.store.segment_words(&seg.gid).unwrap_or_default();
                LineInfo {
                    speaker: seg
                        .speaker_gid
                        .as_ref()
                        .and_then(|g| by_gid.get(g))
                        .copied(),
                    t0_ms: seg.t0_ms,
                    t1_ms: seg.t1_ms,
                    // Stored per line (`segments.overlap`).
                    overlap: seg.overlap,
                    words: seg
                        .text
                        .split_whitespace()
                        .zip(&words)
                        .map(|(t, w)| WordInfo {
                            text: t.to_string(),
                            t0_ms: w.t0_ms,
                            t1_ms: w.t1_ms,
                            low_confidence: w
                                .conf
                                .is_some_and(|c| c < crate::aligner::LOW_CONFIDENCE),
                        })
                        .collect(),
                    gid: seg.gid,
                    text: seg.text,
                }
            })
            .collect();
        SessionSnapshot {
            seq,
            meeting: self.meeting.clone(),
            state: self.state(),
            now_ms: self.now_ms(),
            transcribing: self.transcribing,
            mode: self.cfg.mode.as_str().into(),
            language: self.cfg.language.clone(),
            title: self.cfg.title.clone(),
            consent_confirmed: self
                .store
                .get_meeting(&self.meeting)
                .map(|m| m.consent_confirmed)
                .unwrap_or(false),
            speakers: speakers.unwrap_or_default(),
            lines,
            marks: self
                .store
                .marks(&self.meeting)
                .unwrap_or_default()
                .into_iter()
                .map(|m| m.t_ms)
                .collect(),
        }
    }

    /// Splits the given lines (segment gids) off `from` into a new speaker.
    /// `Err`: the edit lock timed out or the engine did not answer.
    pub fn split(
        &self,
        from: SpeakerId,
        lines: Vec<String>,
    ) -> Result<Option<SpeakerId>, SessionError> {
        let _op = self.op_lock()?;
        let (tx, rx) = bounded(1);
        self.engine
            .send(EngineCmd::Split {
                from,
                lines,
                reply: tx,
            })
            .map_err(|e| err("split", e))?;
        // `Ok(None)`: the engine refused (an unknown speaker, nothing to split).
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|e| err("split", e))
    }

    /// Discards the last `last_s` seconds [RT-1]: audio, lines, marks, notes
    /// citing them, as one operation (speakers left without lines go at
    /// stop). Returns the cut (ms). The span stays on the timeline as silence.
    pub fn discard(&self, last_s: f64) -> Result<i64, SessionError> {
        if !last_s.is_finite() || last_s <= 0.0 {
            return Err(SessionError("discard: a positive number of seconds".into()));
        }
        self.discard_inner(|now_ms| {
            Ok((now_ms - (last_s.min(24.0 * 3600.0) * 1000.0) as i64).max(0))
        })
    }

    /// Like [`Session::discard`], from an absolute meeting time: everything
    /// from `t_cut_ms` on goes. A UI that previewed a span and confirms later
    /// passes the cut it showed, so what is removed is what was approved.
    /// Rejects a cut before 0 or after the time the recording has reached.
    pub fn discard_from(&self, t_cut_ms: i64) -> Result<i64, SessionError> {
        self.discard_inner(|now_ms| {
            if t_cut_ms < 0 || t_cut_ms > now_ms {
                return Err(SessionError(format!(
                    "discard: {t_cut_ms} ms is outside the recording (0..{now_ms} ms)"
                )));
            }
            Ok(t_cut_ms)
        })
    }

    /// `cut` maps the meeting time reached (read under the edit lock) to the
    /// cut position.
    fn discard_inner(
        &self,
        cut: impl FnOnce(i64) -> Result<i64, SessionError>,
    ) -> Result<i64, SessionError> {
        let _op = self.op_lock()?;
        let now_ms = self.now_ms();
        let t_cut_ms = cut(now_ms)?;
        let wait = Duration::from_secs(30);
        // 1. The pump flushes, picks the pages to keep and holds new ones.
        let (tx, rx) = bounded(1);
        self.pump
            .send(PumpCmd::Discard {
                t_cut_ms,
                reply: tx,
            })
            .map_err(|e| err("discard", e))?;
        let d = rx.recv_timeout(wait).map_err(|e| err("discard", e))?;
        // From here on, any failure must stop the hold (or the rest of the
        // recording would pile up in memory and be lost).
        let release = |keep: Vec<(Track, u32)>| -> Result<(), SessionError> {
            let (tx, rx) = bounded(1);
            self.pump
                .send(PumpCmd::Release { keep, reply: tx })
                .map_err(|e| err("discard", e))?;
            rx.recv_timeout(wait)
                .map_err(|e| err("discard", e))?
                .map_err(|e| err("discard audio", e))
        };
        let text_side = || -> Result<i64, SessionError> {
            // 2. The engine drops the text in progress and the discarded
            //    frames still in the ring.
            if self.transcribing {
                let (tx, rx) = bounded(1);
                self.engine
                    .send(EngineCmd::ResetAsr {
                        mute_until: d.mute_until,
                        reply: tx,
                    })
                    .map_err(|e| err("discard", e))?;
                rx.recv_timeout(wait).map_err(|e| err("discard", e))?;
            }
            // 3. One store transaction for the text side (+ a pending audio row).
            let (tx, rx) = bounded(1);
            let keep = d
                .keep
                .iter()
                .map(|(t, k, p)| ghi_store::edits::KeepPages {
                    kind: kind_of(*t),
                    pages: *k,
                    prefix: p.clone(),
                })
                .collect();
            self.persist
                .as_ref()
                .ok_or_else(|| SessionError("stopped".into()))?
                .send(PersistMsg::Discard {
                    t_cut_ms,
                    now_ms,
                    keep,
                    reply: tx,
                })
                .map_err(|e| err("discard", e))?;
            rx.recv_timeout(wait)
                .map_err(|e| err("discard", e))?
                .map_err(|e| err("discard", e))
        };
        let id = match text_side() {
            Ok(id) => id,
            Err(e) => {
                let _ = release(Vec::new());
                return Err(e);
            }
        };
        // 4. The audio side: rotate the bundles, then write what was held.
        release(d.keep.iter().map(|(t, k, _)| (*t, *k)).collect())?;
        self.store
            .discard_audio_done(id)
            .map_err(|e| err("discard", e))?;
        self.events.emit(Event::DiscardApplied {
            meeting: self.meeting.clone(),
            from_ms: t_cut_ms,
        });
        Ok(t_cut_ms)
    }

    /// Stops recording, waits for the threads, closes the meeting and queues
    /// the notes and final-pass jobs.
    pub fn stop(mut self) -> Result<StopReport, SessionError> {
        self.set_state(SessionState::Stopping);
        let _ = self.pump.send(PumpCmd::Stop);
        // The engine stops once the pump is done and the ring is drained;
        // the persist thread once every sender is gone.
        self.persist.take();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        if let Some(h) = &self.hooks {
            h.recording_stopped();
        }
        let duration_ms = self.now_ms();
        self.store
            .finish_meeting(&self.meeting, duration_ms)
            .map_err(|e| err("closing the meeting", e))?;
        // Speakers left without lines (by a discard, or never confirmed).
        self.store
            .remove_orphan_speakers(&self.meeting)
            .map_err(|e| err("closing the meeting", e))?;
        let mut jobs = Vec::new();
        if self.cfg.queue_jobs {
            self.store
                .set_meeting_status(&self.meeting, "processing")
                .map_err(|e| err("closing the meeting", e))?;
            for kind in [NOTES_LIVE_JOB, FINAL_PASS_JOB] {
                jobs.push(
                    self.store
                        .enqueue_job(
                            Some(&self.meeting),
                            kind,
                            JOB_PAYLOAD_VERSION,
                            &serde_json::json!({}),
                        )
                        .map_err(|e| err("queueing jobs", e))?,
                );
            }
            self.set_state(SessionState::Processing);
        } else {
            self.set_state(SessionState::Ready);
        }
        Ok(StopReport {
            meeting: self.meeting.clone(),
            duration_ms,
            jobs,
        })
    }
}

/// Stops a half-started session's threads (the Undo guard in `start` then
/// resumes the jobs and removes the meeting); returns `e`.
fn abort(
    pump: Sender<PumpCmd>,
    persist: Sender<PersistMsg>,
    threads: Vec<JoinHandle<()>>,
    e: SessionError,
) -> SessionError {
    let _ = pump.send(PumpCmd::Stop);
    drop(persist);
    for t in threads {
        let _ = t.join();
    }
    e
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> Result<JoinHandle<()>, SessionError> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .map_err(|e| err("starting a thread", e))
}

struct PumpCtx {
    /// Set once the engines are known (a replay waits for them until then).
    lossless: Arc<AtomicBool>,
    store: Arc<Store>,
    meeting: String,
    events: EventTx,
    tracks: Vec<Track>,
    position: Arc<AtomicU64>,
    capture_done: Arc<AtomicBool>,
    source_ended: Arc<AtomicBool>,
}

/// The UI event for a capture event the user should see (the pipeline already
/// acted on it: sleep and wake stop and restart the timeline, so the gap is
/// not recorded as audio, and the bundles carry the markers [RT-10]).
fn lifecycle_event(meeting: &str, ev: ghi_audio::CaptureEvent) -> Option<Event> {
    use ghi_audio::CaptureEvent as C;
    let meeting = meeting.to_string();
    Some(match ev {
        C::Sleep => Event::Slept { meeting },
        C::Wake => Event::Woke { meeting },
        C::SystemRestarted => Event::SystemAudioRestarted { meeting },
        C::SilentSystemTrack { silent_s } => Event::SilentSystemTrack { meeting, silent_s },
        C::DiskLow { free_bytes } => Event::DiskLow {
            meeting,
            free_bytes,
        },
        C::DiskFull => Event::DiskFull { meeting },
        C::TrackLost { track } => Event::TrackLost {
            meeting,
            track: track.index() as u8,
        },
        C::RouteChanged {
            input_bluetooth_hfp,
            ..
        } => Event::RouteChanged {
            meeting,
            bluetooth_hfp: input_bluetooth_hfp,
        },
        _ => return None,
    })
}

/// Steps the pipeline every 10 ms; owns the capture and the bundle writers.
fn pump(
    ctx: PumpCtx,
    mut pipeline: Pipeline<Metered<Muted<OpusRecorder<BundlePages>>>>,
    mut capture: Capture,
    cmds: crossbeam_channel::Receiver<PumpCmd>,
) {
    // However the pump ends (a panic included, in debug builds), the engine
    // must learn that no more audio comes, or `stop` waits forever.
    struct Done(Arc<AtomicBool>);
    impl Drop for Done {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let _done = Done(ctx.capture_done.clone());
    let error = |message: String| {
        ctx.events.emit(Event::Error {
            meeting: Some(ctx.meeting.clone()),
            kind: ErrorKind::Capture,
            message,
        })
    };
    let mut last_level = Instant::now();
    let mut level_shown = false;
    loop {
        let mut stop = false;
        loop {
            let cmd = match cmds.try_recv() {
                Ok(cmd) => cmd,
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                // The session is gone without a stop: wind down.
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    stop = true;
                    break;
                }
            };
            match cmd {
                PumpCmd::Pause => pipeline.pause(),
                PumpCmd::Resume => pipeline.resume(),
                PumpCmd::Discard { t_cut_ms, reply } => {
                    let pos = pipeline.position();
                    // The marker makes the encoder end its page now, so the
                    // audio up to here is in the bundles (and the rotation
                    // removes what is after the cut).
                    let now_ms = (pos * 1000 / u64::from(SAMPLE_RATE)) as i64;
                    pipeline.discard((now_ms - t_cut_ms).max(0) as f32 / 1000.0);
                    let rec = &mut pipeline.writer_mut().inner;
                    // Audio captured before the discard but not yet emitted
                    // (up to the pipeline's 0.25 s stall window) is muted.
                    let mute_until = pos + u64::from(SAMPLE_RATE) / 2;
                    rec.mute_until(mute_until);
                    let pages = rec.inner.sink_mut();
                    let keep = ctx
                        .tracks
                        .iter()
                        .map(|&t| (t, pages.keep_before(t, t_cut_ms), pages.prefix_hex(t)))
                        .collect();
                    pages.hold();
                    let _ = reply.send(Discarding { keep, mute_until });
                }
                PumpCmd::Release { keep, reply } => {
                    let r = pipeline
                        .writer_mut()
                        .inner
                        .inner
                        .sink_mut()
                        .release(&keep)
                        .map_err(|e| e.to_string());
                    let _ = reply.send(r);
                }
                PumpCmd::Stop => stop = true,
            }
        }
        while let Ok(ev) = capture.events.try_recv() {
            pipeline.handle_event(ev);
        }
        if ctx.lossless.load(Ordering::Acquire)
            && pipeline.asr_backlog() > LOSSLESS_BACKLOG
            && !stop
        {
            // The engine is behind: let it catch up (the replay waits on the
            // full capture ring in turn).
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        let report = pipeline.step();
        for ev in report.events {
            match ev {
                ghi_audio::CaptureEvent::Error { detail, .. } => error(detail),
                ev => {
                    if let Some(ev) = lifecycle_event(&ctx.meeting, ev) {
                        ctx.events.emit(ev);
                    }
                }
            }
        }
        if last_level.elapsed() >= LEVEL_EVERY {
            last_level = Instant::now();
            let [mic_dbfs, system_dbfs] = pipeline.writer_mut().take_dbfs();
            // Silent while no audio flows, apart from one "nothing" to clear
            // the meters.
            let any = mic_dbfs.is_some() || system_dbfs.is_some();
            if any || level_shown {
                ctx.events.emit(Event::LevelMeter {
                    meeting: ctx.meeting.clone(),
                    mic_dbfs,
                    system_dbfs,
                });
            }
            level_shown = any;
        }
        ctx.position.store(pipeline.position(), Ordering::Relaxed);
        if pipeline.writer_failed() {
            error("writing the audio failed".into());
        }
        if capture.ended() {
            ctx.source_ended.store(true, Ordering::Release);
        }
        if stop {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    capture.stop();
    let _ = pipeline.step();
    ctx.position.store(pipeline.position(), Ordering::Relaxed);
    // Pages still held by a discard that never completed are written out.
    if let Err(e) = pipeline.writer_mut().inner.inner.sink_mut().release(&[]) {
        error(format!("closing the audio: {e}"));
    }
    let pages = match pipeline.finish().inner.inner.finish() {
        Ok(p) => p,
        Err(e) => {
            error(format!("closing the audio: {e}"));
            return;
        }
    };
    for (i, w) in pages.writers.into_iter().enumerate() {
        if let Some(w) = w {
            let t = Track::from_index(i as u32).expect("track index");
            if let Err(e) = ctx.store.finish_track(&ctx.meeting, kind_of(t), w) {
                error(format!("closing the audio: {e}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::{ReplayTrack, replay};
    use crate::events::bus;
    use ghi_store::keys::{MemoryKeyStore, Protection};

    #[test]
    fn edits_wait_for_the_edit_in_progress() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::open(
                tmp.path(),
                Arc::new(MemoryKeyStore::default()),
                Protection::default(),
            )
            .unwrap(),
        );
        let capture = replay(
            vec![ReplayTrack {
                track: Track::Mic,
                samples: vec![0.1; 16_000 * 4],
                sample_rate: 16_000,
            }],
            Some(1.0),
        )
        .unwrap();
        let (tx, _rx) = bus();
        let s = Session::start_with_loader(
            store,
            || Ok(None),
            capture,
            SessionConfig {
                mode: Mode::Room,
                language: None,
                title: "t".into(),
                queue_jobs: false,
                lossless: false,
            },
            tx,
            None,
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        let held = s.ops.lock().unwrap();
        let done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let d = scope.spawn(|| {
                let r = s.discard(0.5);
                done.store(true, Ordering::SeqCst);
                r
            });
            let sp = scope.spawn(|| s.split(1, vec![]));
            std::thread::sleep(Duration::from_millis(300));
            assert!(!done.load(Ordering::SeqCst), "discard waits for the lock");
            drop(held);
            assert!(d.join().unwrap().is_ok());
            // No engine: the split is a clean "nothing to split", not a hang.
            assert!(sp.join().unwrap().is_err());
        });
        s.stop().unwrap();
    }
}
