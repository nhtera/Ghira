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
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crossbeam_channel::{Sender, bounded, unbounded};
use ghi_audio::encoder::{EncoderConfig, OpusRecorder};
use ghi_audio::pipeline::{Pipeline, PipelineConfig};
use ghi_audio::{SAMPLE_RATE, Track};
use ghi_store::store::{NewMeeting, Store, TrackKind};

use crate::capture::Capture;
use crate::engines::SpeechEngines;
use crate::events::{ErrorKind, Event, EventTx, SessionState};
use crate::live::{Engine, EngineCmd, LiveConfig, Mode, PersistMsg};
use crate::pages::{BundlePages, Muted};
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
}

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
}

impl Session {
    /// Creates the meeting and starts recording from `capture`; with no
    /// `engines`, records without a live transcript.
    pub fn start(
        store: Arc<Store>,
        engines: Option<Arc<dyn SpeechEngines>>,
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
        // Nothing reads the ASR ring without engines: never wait on it.
        let live = engines.is_some();
        cfg.lossless &= live;
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
        let recorder = Muted::new(
            OpusRecorder::new(BundlePages::new(writers), &tracks, EncoderConfig::default())
                .map_err(|e| err("audio encoder", e))?,
        );
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
        let mut threads = Vec::new();

        let persist = Persist::new(store.clone(), meeting.clone(), events.clone());
        threads.push(spawn("ghi-persist", move || persist.run(persist_rx))?);

        if let Some(engines) = engines {
            let engine = Engine::new(
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
            .map_err(|e| err("speech engine", e))?;
            let done = capture_done.clone();
            threads.push(spawn("ghi-engine", move || {
                engine.run(asr_ring, engine_rx, done)
            })?);
        } else {
            // Speaker edits go nowhere: there are no live speakers.
            drop((asr_ring, engine_rx));
        }
        {
            let ctx = PumpCtx {
                lossless: cfg.lossless,
                store: store.clone(),
                meeting: meeting.clone(),
                events: events.clone(),
                tracks: tracks.clone(),
                position: position.clone(),
                capture_done,
                source_ended: source_ended.clone(),
            };
            threads.push(spawn("ghi-pump", move || {
                pump(ctx, pipeline, capture, pump_rx)
            })?);
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

    /// Splits the given lines (segment gids) off `from` into a new speaker.
    pub fn split(&self, from: SpeakerId, lines: Vec<String>) -> Option<SpeakerId> {
        let (tx, rx) = bounded(1);
        self.engine
            .send(EngineCmd::Split {
                from,
                lines,
                reply: tx,
            })
            .ok()?;
        rx.recv_timeout(Duration::from_secs(10)).ok().flatten()
    }

    /// Discards the last `last_s` seconds [RT-1]: audio, lines, marks, notes
    /// citing them, as one operation (speakers left without lines go at
    /// stop). Returns the cut (ms). The span stays on the timeline as silence.
    pub fn discard(&self, last_s: f64) -> Result<i64, SessionError> {
        if !last_s.is_finite() || last_s <= 0.0 {
            return Err(SessionError("discard: a positive number of seconds".into()));
        }
        let now_ms = self.now_ms();
        let t_cut_ms = (now_ms - (last_s.min(24.0 * 3600.0) * 1000.0) as i64).max(0);
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

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> Result<JoinHandle<()>, SessionError> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .map_err(|e| err("starting a thread", e))
}

struct PumpCtx {
    lossless: bool,
    store: Arc<Store>,
    meeting: String,
    events: EventTx,
    tracks: Vec<Track>,
    position: Arc<AtomicU64>,
    capture_done: Arc<AtomicBool>,
    source_ended: Arc<AtomicBool>,
}

/// Steps the pipeline every 10 ms; owns the capture and the bundle writers.
fn pump(
    ctx: PumpCtx,
    mut pipeline: Pipeline<Muted<OpusRecorder<BundlePages>>>,
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
    loop {
        let mut stop = false;
        while let Ok(cmd) = cmds.try_recv() {
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
                    let rec = pipeline.writer_mut();
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
        if ctx.lossless && pipeline.asr_backlog() > LOSSLESS_BACKLOG && !stop {
            // The engine is behind: let it catch up (the replay waits on the
            // full capture ring in turn).
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        let report = pipeline.step();
        for ev in report.events {
            if let ghi_audio::CaptureEvent::Error { detail, .. } = &ev {
                error(detail.clone());
            }
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
    if let Err(e) = pipeline.writer_mut().inner.sink_mut().release(&[]) {
        error(format!("closing the audio: {e}"));
    }
    let pages = match pipeline.finish().inner.finish() {
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
