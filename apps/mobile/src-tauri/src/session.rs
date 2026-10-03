// SPDX-License-Identifier: Apache-2.0
//! One recording on `ghi-core` building blocks (phase 16 D3).
//!
//! ```text
//! Swift tap ─► ring ─► pump thread: ghi-audio Pipeline ─┬─► Metered<OpusRecorder<BundlePages>> ─► store bundle
//!   (any rate)          (16 kHz, 10 ms frames)          ├─► sealed backlog ─► engine thread ─► Persist ─► store
//!                                                       └─► level meter, pocket detector
//! ```
//!
//! The pump never waits on the engine, so audio is recorded whatever the
//! engine does (held while locked or hot, loading, or failed). The engine
//! ([`crate::engine`]) is mobile-only: it obeys the iOS GPU [`Gate`](crate::gate::Gate).
//! Everything the session learns is in [`Shared`]; the UI polls it with
//! [`Recorder::snapshot`] (for a reloaded webview: it flushes the lines and
//! reads them back, so it is not for polling) and follows `coreEvent` /
//! `MobileEvent`.
//!
//! The session slot is the [`Recorder`] (managed Tauri state, built by the
//! mobile core). The only statics are the audio tap's ring producer and
//! [`current`], the handle the C ABI callbacks (which carry no context) use.
//!
//! Job runner contract: `recording_started` when a recording begins and
//! `recording_stopped` only after the engine thread has ended (its models are
//! dropped by then), so the runner never claims a job while a session exists
//! and the live engine is never resident together with the final pass's.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ghi_audio::encoder::{EncoderConfig, OpusRecorder};
use ghi_audio::muffle::MuffleDetector;
use ghi_audio::pipeline::{Pipeline, PipelineConfig};
use ghi_audio::ring::RingProducer;
use ghi_audio::{CaptureEvent, Route, Track};
use ghi_core::events::{Event, EventTx, SessionSnapshot, SessionState};
use ghi_core::live::PersistMsg;
use ghi_core::pages::{BundlePages, Metered};
use ghi_core::persist::Persist;
use ghi_core::session::{FINAL_PASS_JOB, RecordingHooks};
use ghi_store::store::{NewMeeting, Store, TrackKind};
use serde::Serialize;
use specta::Type;

use crate::backlog::{self, BacklogWriter};
use crate::cmd::events::{self as mobile_events, InterruptionKind, MobileEvent};
use crate::cmd::lifecycle::{DeviceTier, TierClass};
use crate::cmd::record::{
    ConsentMessage, RecordMode, RecordOnlyReason, RecordStart, RecordState, ResumePrompt,
};
use crate::cmd::types::{ProcessingTarget, RecordPhase};
use crate::engine::{self, Applier, EngineCtx, EnginesProvider, Update};
use crate::gate::{Gate, Hold};
use crate::metrics::{Metrics, Sample};
use crate::platform;

const STEP: Duration = Duration::from_millis(10);
const METRICS_EVERY: Duration = Duration::from_secs(10);
const DISK_EVERY: Duration = Duration::from_secs(60);
/// The recording stops with "disk low" below this much free space (and does
/// not start).
pub const MIN_FREE_BYTES: u64 = 500 * 1024 * 1024;
/// Error codes of `record_start` (camelCase strings the UI maps to copy).
pub const ERR_DISK_LOW: &str = "diskLow";
pub const ERR_CALL_ACTIVE: &str = "callActive";
pub const ERR_MIC_IN_USE: &str = "micInUse";
pub const ERR_MIC_DENIED: &str = "microphoneDenied";
pub const ERR_WAITING: &str = "waitingForTranscription";
pub const ERR_PAIRING: &str = "pairingNotAvailable";
pub const ERR_RUNNING: &str = "alreadyRecording";

/// Ring between the Swift tap and the pump: 2 s at 48 kHz.
const RING_SAMPLES: usize = 96_000;
/// How long a start waits for the previous recording's engine to drain.
const DRAIN_WAIT: Duration = Duration::from_secs(60);

const RATE: f64 = engine::SAMPLE_RATE as f64;
/// Audio the ASR may still hold without a partial: a 1120 ms chunk, its right
/// context, the 800 ms endpoint hold, and margin.
const UNSETTLED_SAMPLES: u64 = 3 * engine::SAMPLE_RATE as u64;

/// A finished transcript line of the self-test. Times are seconds.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub start: f64,
    pub end: f64,
    /// 1-based diarization speaker.
    pub speaker: Option<u32>,
    pub text: String,
}

/// What the session is doing, as the Live Activity's native side knows it
/// (`platform::activity_*`). The UI uses [`RecordPhase`], which has more
/// states; [`From<RecordPhase>`] folds them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// Loading the models (first recording only).
    Loading,
    Live,
    /// The app is in the background: recording only; transcript catches up later.
    Locked,
    /// Processing the audio recorded while locked or hot.
    CatchingUp,
    /// Thermal state `.serious` or worse: recording only.
    Hot,
    /// A phone call or another app took the audio session, or the user paused.
    Interrupted,
    /// Stopped; the engine finishes the backlog.
    Finishing,
    Done,
    /// No engine (not built, models missing or failed): recording only.
    RecordOnly,
}

impl From<RecordPhase> for Phase {
    fn from(p: RecordPhase) -> Phase {
        match p {
            RecordPhase::Idle | RecordPhase::Done => Phase::Done,
            RecordPhase::Loading => Phase::Loading,
            RecordPhase::Live => Phase::Live,
            RecordPhase::Locked => Phase::Locked,
            RecordPhase::CatchingUp => Phase::CatchingUp,
            RecordPhase::Hot => Phase::Hot,
            RecordPhase::Interrupted | RecordPhase::Paused => Phase::Interrupted,
            RecordPhase::Finishing => Phase::Finishing,
            RecordPhase::RecordOnly => Phase::RecordOnly,
        }
    }
}

#[derive(Debug, Default)]
struct Inner {
    marks: u32,
    error: Option<String>,
    /// The audio session was taken (a call, another app); waiting for the user.
    interrupted: bool,
    interrupted_by_call: bool,
    /// The interruption ended and the user was not asked yet.
    resume_pending: bool,
    paused: bool,
    stopped: bool,
    /// A live engine is expected (tier, models); false: recording only.
    engine_expected: bool,
    record_only: Option<RecordOnlyReason>,
    engine_ready: bool,
    engine_failed: bool,
    engine_done: bool,
    recorded_samples: u64,
    processed_samples: u64,
    /// Backlog samples written by the pump and read by the engine.
    written_samples: u64,
    read_samples: u64,
    /// Backlog position up to which the transcript is final: a reset redoes
    /// the audio from here.
    committed_samples: u64,
    dropped_samples: u64,
    /// (when, audio samples, compute time) of recent steps, for the RTF.
    window: Vec<(Instant, u64, Duration)>,
    catch_up: Option<CatchUp>,
    catch_up_x: Option<f64>,
    model_load_s: Option<f64>,
    thermal: Option<i32>,
    /// Backlog position where the current ASR/diarization streams started.
    stream_offset_samples: u64,
    resets: u32,
    /// The phase and mark count last pushed to the Live Activity.
    pushed: Option<(RecordPhase, u32)>,
    pocket: bool,
    level_db: f32,
    /// An utterance is in progress (a partial was the last thing heard).
    partial_open: bool,
}

impl Inner {
    fn backlog_samples(&self) -> u64 {
        self.written_samples.saturating_sub(self.read_samples)
    }
}

#[derive(Debug, Clone, Copy)]
struct CatchUp {
    since: Instant,
    processed_at_start: u64,
}

/// State shared by the pump, the engine, the C ABI and the commands.
pub struct Shared {
    pub gate: Gate,
    inner: Mutex<Inner>,
    capture_done: AtomicBool,
    /// The recording timeline (16 kHz samples), written by the pump.
    position: AtomicU64,
    meeting: String,
    events: EventTx,
}

impl Shared {
    fn new(meeting: String, events: EventTx) -> Shared {
        Shared {
            gate: Gate::default(),
            inner: Mutex::new(Inner::default()),
            capture_done: AtomicBool::new(false),
            position: AtomicU64::new(0),
            meeting,
            events,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn capture_done(&self) -> bool {
        self.capture_done.load(Ordering::Acquire)
    }

    /// Recorded time, ms.
    pub fn now_ms(&self) -> i64 {
        (self.position.load(Ordering::Relaxed) as f64 * 1000.0 / RATE) as i64
    }

    pub fn timed_load<T>(&self, load: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let t = Instant::now();
        let r = load();
        let mut s = self.lock();
        s.model_load_s = Some(t.elapsed().as_secs_f64());
        s.engine_ready = r.is_ok();
        r
    }

    /// Commits the bookkeeping of what an engine step produced. `position` is
    /// the backlog read position after the step; `finished` means the streams
    /// were flushed.
    pub fn commit(&self, updates: &[Update], position: u64, finished: bool) {
        let mut s = self.lock();
        let offset = s.stream_offset_samples;
        for u in updates {
            match u {
                Update::Partial(_) => s.partial_open = true,
                Update::Final { end, .. } => {
                    s.partial_open = false;
                    let end_samples = offset + (end.max(0.0) * RATE) as u64;
                    s.committed_samples = s.committed_samples.max(end_samples.min(position));
                }
            }
        }
        // After a flush everything read is final. While live, "no partial"
        // doesn't mean the last chunk + endpoint hold has been decoded yet, so
        // only audio older than that margin counts as final (a reset redoes
        // at most the margin of silence, and loses nothing).
        if finished {
            s.committed_samples = position;
        } else if !s.partial_open {
            let settled = position.saturating_sub(UNSETTLED_SAMPLES);
            s.committed_samples = s.committed_samples.max(settled);
        }
    }

    /// The models are being dropped or reloaded.
    pub fn models_unloaded(&self) {
        self.lock().engine_ready = false;
    }

    /// Where a reset redoes the audio from.
    pub fn committed_position(&self) -> u64 {
        self.lock().committed_samples
    }

    /// Where the current streams started on the backlog timeline.
    pub fn stream_offset(&self) -> u64 {
        self.lock().stream_offset_samples
    }

    /// The engine reopened its streams at backlog `position` after dropping
    /// a suspect step (and reloaded the models if the gate was poisoned).
    pub fn engine_reset(&self, position: u64) {
        let mut s = self.lock();
        s.stream_offset_samples = position;
        s.read_samples = position;
        s.partial_open = false;
        s.resets += 1;
    }

    /// One engine step read `samples` in `compute`; `position` is the backlog
    /// read position after it.
    pub fn engine_progress(&self, samples: usize, compute: Duration, position: u64) {
        let now = Instant::now();
        let mut s = self.lock();
        s.processed_samples += samples as u64;
        s.read_samples = position;
        let backlog = s.backlog_samples();
        s.window.push((now, samples as u64, compute));
        s.window
            .retain(|(t, _, _)| now.duration_since(*t) <= METRICS_EVERY);
        // Catch-up: from when the backlog exceeds 5 s until it is under 1 s.
        let behind = backlog as f64 / RATE;
        match s.catch_up {
            None if behind > 5.0 => {
                s.catch_up = Some(CatchUp {
                    since: now,
                    processed_at_start: s.processed_samples,
                })
            }
            Some(c) if behind < 1.0 => {
                let wall = now.duration_since(c.since).as_secs_f64();
                let audio = (s.processed_samples - c.processed_at_start) as f64 / RATE;
                if wall > 1.0 {
                    s.catch_up_x = Some(audio / wall);
                }
                s.catch_up = None;
            }
            _ => {}
        }
    }

    pub fn engine_failed(&self, e: String) {
        let mut s = self.lock();
        s.engine_failed = true;
        s.record_only = Some(RecordOnlyReason::EngineFailed);
        // The message may quote a path or a model id, never content.
        s.error = Some(e);
        drop(s);
        self.events.emit(Event::Error {
            meeting: Some(self.meeting.clone()),
            kind: ghi_core::events::ErrorKind::Engine,
            message: "the live transcript stopped: recording continues".into(),
        });
    }

    pub fn engine_done(&self) {
        self.lock().engine_done = true;
        self.activity_update();
    }

    /// Pushes the phase and mark count to the Live Activity and the webview
    /// (ends the activity when the capture has stopped and the engine is done).
    pub fn activity_update(&self) {
        let phase = self.phase();
        let (marks, finished) = {
            let mut s = self.lock();
            s.pushed = Some((phase, s.marks));
            (
                s.marks,
                (s.engine_done || !s.engine_expected) && self.capture_done(),
            )
        };
        mobile_events::emit(MobileEvent::Phase { phase });
        platform::activity_update(phase.into(), marks, finished);
    }

    /// Updates the Live Activity if the phase or marks changed since the last push.
    fn activity_refresh(&self) {
        let phase = self.phase();
        let changed = {
            let s = self.lock();
            s.pushed != Some((phase, s.marks))
        };
        if changed {
            self.activity_update();
        }
    }

    pub fn set_thermal(&self, state: i32) {
        self.lock().thermal = Some(state);
        self.gate.set_hot(state >= 2);
        mobile_events::emit(MobileEvent::Thermal {
            level: state.clamp(0, 3) as u8,
        });
    }

    pub fn phase(&self) -> RecordPhase {
        let s = self.lock();
        let behind = s.backlog_samples() as f64 / RATE;
        // A stopped recording is done whatever the engine did: record-only
        // sessions and failed engines never fall back to `RecordOnly` here.
        if s.engine_done || (s.stopped && !s.engine_expected) {
            return RecordPhase::Done;
        }
        // Stopped or interrupted first: while locked the Live Activity must
        // not claim "Recording" when nothing is.
        if s.stopped {
            return if s.engine_failed {
                RecordPhase::Done
            } else {
                RecordPhase::Finishing
            };
        }
        if s.interrupted {
            return RecordPhase::Interrupted;
        }
        if s.paused {
            return RecordPhase::Paused;
        }
        if !s.engine_expected || s.engine_failed {
            return RecordPhase::RecordOnly;
        }
        match self.gate.hold() {
            Hold::Suspended => return RecordPhase::Locked,
            Hold::Hot => return RecordPhase::Hot,
            Hold::None => {}
        }
        if !s.engine_ready {
            RecordPhase::Loading
        } else if behind > 5.0 {
            RecordPhase::CatchingUp
        } else {
            RecordPhase::Live
        }
    }

    fn sample(&self) -> Sample {
        let device = platform::device_stats();
        let phase = self.phase();
        let s = self.lock();
        let (audio, compute) = s
            .window
            .iter()
            .fold((0u64, Duration::ZERO), |(a, c), (_, n, d)| (a + n, c + *d));
        Sample {
            t_s: s.recorded_samples as f64 / RATE,
            phase: format!("{phase:?}"),
            recorded_s: s.recorded_samples as f64 / RATE,
            processed_s: s.processed_samples as f64 / RATE,
            backlog_s: s.backlog_samples() as f64 / RATE,
            catch_up_x: s.catch_up_x,
            rtf: (audio > 0).then(|| compute.as_secs_f64() / (audio as f64 / RATE)),
            steps_while_inactive: self.gate.steps_while_inactive(),
            gpu_overlaps: self.gate.overlaps(),
            engine_resets: s.resets,
            thermal: s.thermal.or(device.thermal),
            footprint_mb: device.memory_mb,
            battery: device.battery,
        }
    }
}

enum Cmd {
    Pause,
    Resume,
    Stop,
}

/// What a start needs from the app.
pub struct RecorderDeps {
    /// The store, usable while the app is locked (a recording keeps going).
    pub store: Arc<dyn Fn() -> Result<Arc<Store>, String> + Send + Sync>,
    pub events: EventTx,
    /// The job runner, once the store is open.
    pub runner: Arc<dyn Fn() -> Option<Arc<ghi_core::jobs::JobRunner>> + Send + Sync>,
    pub models: PathBuf,
    /// Where sealed backlogs live (cleaned at launch with [`backlog::sweep`]).
    pub backlog_dir: PathBuf,
    pub metrics_dir: PathBuf,
    pub tier: DeviceTier,
    /// Test seam: the engines a live transcript uses (default: [`engine::provider`]).
    pub provider: Option<EnginesProvider>,
    /// Feed the synthetic microphone instead of starting the audio session
    /// ([`fake_mic_from_env`]; tests).
    pub fake_mic: bool,
}

/// The recording slot: at most one session, plus the one still draining.
pub struct Recorder {
    deps: RecorderDeps,
    slot: Mutex<Option<Arc<Session>>>,
    /// A start is between the slot check and the session being stored.
    starting: AtomicBool,
    freed: Condvar,
    metrics: Metrics,
}

/// The audio tap's side of the ring (the C ABI pushes into it).
static PRODUCER: Mutex<Option<RingProducer>> = Mutex::new(None);
/// The tap is held by something other than a recording (voice enrollment).
static TAP_EXTERNAL: AtomicBool = AtomicBool::new(false);

/// Lets something other than a recording (voice enrollment) receive the
/// audio tap's PCM through `producer`. The caller starts and stops the audio
/// session itself (`platform::audio_start` / `audio_stop`). `Err` while a
/// recording or another tap holds the ring; a recording refuses to start
/// while this is attached.
pub fn attach_tap(producer: RingProducer) -> Result<(), String> {
    let mut slot = PRODUCER.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_some() {
        return Err(ERR_MIC_IN_USE.into());
    }
    *slot = Some(producer);
    TAP_EXTERNAL.store(true, Ordering::Release);
    Ok(())
}

/// Releases a tap from [`attach_tap`]; does nothing if a recording owns the ring.
pub fn detach_tap() {
    let mut slot = PRODUCER.lock().unwrap_or_else(|e| e.into_inner());
    if TAP_EXTERNAL.swap(false, Ordering::AcqRel) {
        slot.take();
    }
}

/// The recording the C ABI callbacks (no context of their own) reach.
static CURRENT: Mutex<Option<Arc<Session>>> = Mutex::new(None);

pub fn current() -> Option<Arc<Session>> {
    CURRENT.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Samples the tap dropped because the ring was being swapped (start/stop).
static TAP_DROPS: AtomicU64 = AtomicU64::new(0);

/// Called by the audio tap with one block of mono PCM at `rate` Hz. Drops the
/// block if no recording is running; a full ring counts its own drops.
pub fn push_pcm(samples: &[f32], rate: f64, host_ns: u64) {
    match PRODUCER.try_lock() {
        Ok(mut p) => {
            if let Some(p) = p.as_mut() {
                p.push(samples, rate, host_ns);
            }
        }
        Err(_) => {
            TAP_DROPS.fetch_add(samples.len() as u64, Ordering::Relaxed);
        }
    }
}

/// Test seam: pretend a phone call is active.
#[cfg(test)]
static FORCE_CALL: AtomicBool = AtomicBool::new(false);

fn call_active() -> bool {
    #[cfg(test)]
    if FORCE_CALL.load(Ordering::SeqCst) {
        return true;
    }
    platform::call_active()
}

/// Free bytes for the recording: iOS's "available for important usage"
/// (which counts purgeable space) when Swift knows it, else what `statvfs`
/// reports for the volume holding `path`.
fn free_bytes(path: &Path) -> Option<u64> {
    platform::available_capacity().or_else(|| statvfs_free(path))
}

#[cfg(unix)]
fn statvfs_free(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c` is a valid NUL-terminated path and `st` is writable.
    if unsafe { libc::statvfs(c.as_ptr(), st.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: statvfs succeeded, so the struct is initialised.
    let st = unsafe { st.assume_init() };
    // The field widths differ by platform.
    #[allow(clippy::unnecessary_cast)]
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

#[cfg(not(unix))]
fn statvfs_free(_: &Path) -> Option<u64> {
    None
}

/// A synthetic microphone for tests: a quiet tone pushed at real time as if
/// the audio tap delivered it (`GHI_FAKE_MIC`, test-hooks builds).
struct FakeMic {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeMic {
    fn start() -> FakeMic {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = thread::Builder::new()
            .name("ghi-fake-mic".into())
            .spawn(move || {
                // 100 ms blocks of 48 kHz audio, a 220 Hz tone with a slow
                // beat so the level moves.
                let mut n = 0u64;
                let mut host = 1_000_000_000u64;
                while !flag.load(Ordering::Acquire) {
                    let block: Vec<f32> = (0..4800u64)
                        .map(|i| {
                            let t = (n + i) as f32 / 48_000.0;
                            (t * 220.0 * std::f32::consts::TAU).sin()
                                * 0.1
                                * (1.0 + (t * 0.5).sin())
                        })
                        .collect();
                    push_pcm(&block, 48_000.0, host);
                    n += 4800;
                    host += 100_000_000;
                    thread::sleep(Duration::from_millis(100));
                }
            })
            .ok();
        FakeMic { stop, thread }
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Whether a test-hooks build was asked for the synthetic microphone
/// (`GHI_FAKE_MIC`); never true otherwise.
pub fn fake_mic_from_env() -> bool {
    cfg!(feature = "test-hooks") && std::env::var_os("GHI_FAKE_MIC").is_some()
}

/// The recording in progress (or draining).
pub struct Session {
    pub id: String,
    pub shared: Arc<Shared>,
    store: Arc<Store>,
    recorder: Arc<Recorder>,
    hooks: Option<Arc<dyn RecordingHooks>>,
    cmd: Sender<Cmd>,
    persist: Mutex<Option<crossbeam_channel::Sender<PersistMsg>>>,
    threads: Mutex<Threads>,
    stopped: AtomicBool,
    /// Job kinds queued once the engine has drained.
    job_kinds: Vec<&'static str>,
    language: Option<String>,
    title: String,
    fake_mic: Mutex<Option<FakeMic>>,
}

#[derive(Default)]
struct Threads {
    pump: Option<JoinHandle<()>>,
    engine: Option<JoinHandle<()>>,
    persist: Option<JoinHandle<()>>,
}

fn io_err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Recorder {
    pub fn new(deps: RecorderDeps) -> Arc<Recorder> {
        let metrics = Metrics::new(&deps.metrics_dir);
        let _ = fs::create_dir_all(&deps.backlog_dir);
        // A crash leaves sealed files nobody can read: they go at launch.
        let swept = backlog::sweep(&deps.backlog_dir);
        if swept > 0 {
            log::info!("removed {swept} stray backlog file(s)");
        }
        Arc::new(Recorder {
            deps,
            slot: Mutex::new(None),
            starting: AtomicBool::new(false),
            freed: Condvar::new(),
            metrics,
        })
    }

    pub fn tier(&self) -> &DeviceTier {
        &self.deps.tier
    }

    fn hooks(&self) -> Option<Arc<dyn RecordingHooks>> {
        (self.deps.runner)().map(|r| r as Arc<dyn RecordingHooks>)
    }

    fn slot(&self) -> MutexGuard<'_, Option<Arc<Session>>> {
        self.slot.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The running session, if capture has not stopped.
    pub fn active(&self) -> Option<Arc<Session>> {
        self.slot()
            .clone()
            .filter(|s| !s.stopped.load(Ordering::Acquire))
    }

    /// The last session, running or draining.
    pub fn latest(&self) -> Option<Arc<Session>> {
        self.slot().clone()
    }

    /// Frees the slot once a session has fully drained.
    fn release(&self, id: &str) {
        let mut slot = self.slot();
        if slot.as_ref().is_some_and(|s| s.id == id) {
            *slot = None;
        }
        let mut cur = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
        if cur.as_ref().is_some_and(|s| s.id == id) {
            *cur = None;
        }
        drop(cur);
        drop(slot);
        self.freed.notify_all();
    }

    /// Starts a recording; returns the meeting id.
    ///
    /// Refused: another recording running, a call active without the M6
    /// acknowledgement, the microphone denied, less than 500 MB free, the
    /// `Phone` target on a device below the live tier, the `Desktop` target.
    /// A previous recording still draining is waited for (up to a minute).
    pub fn start(self: &Arc<Self>, req: &RecordStart) -> Result<String, String> {
        let RecordMode::Room = req.mode;
        match req.target {
            // Pairing arrives in phase 15.
            ProcessingTarget::Desktop => return Err(ERR_PAIRING.into()),
            // Below the live tier a recording is always allowed: it records
            // only, queues no jobs and is processed later.
            ProcessingTarget::Phone | ProcessingTarget::Cloud => {}
        }
        if call_active() && !req.call_acknowledged {
            return Err(ERR_CALL_ACTIVE.into());
        }
        if platform::mic_permission() == crate::cmd::onboarding::MicPermission::Denied {
            return Err(ERR_MIC_DENIED.into());
        }
        if free_bytes(&self.deps.backlog_dir).is_some_and(|f| f < MIN_FREE_BYTES) {
            return Err(ERR_DISK_LOW.into());
        }
        // One at a time; a draining engine owns the lifecycle events, the Live
        // Activity and the GPU.
        {
            let mut slot = self.slot();
            let until = Instant::now() + DRAIN_WAIT;
            while let Some(s) = slot.as_ref() {
                if !s.stopped.load(Ordering::Acquire) {
                    return Err(ERR_RUNNING.into());
                }
                let left = until.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(ERR_WAITING.into());
                }
                slot = self
                    .freed
                    .wait_timeout(slot, left)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            if self.starting.swap(true, Ordering::AcqRel) {
                return Err(ERR_RUNNING.into());
            }
        }
        // Cleared however this start ends (the slot holds the session by then).
        struct Starting<'a>(&'a AtomicBool);
        impl Drop for Starting<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _starting = Starting(&self.starting);
        let store = (self.deps.store)()?;
        let hooks = self.hooks();
        let session = self.begin(&store, hooks.clone(), req);
        let session = match session {
            Ok(s) => s,
            Err(e) => {
                if let Some(h) = &hooks {
                    h.recording_stopped();
                }
                return Err(e);
            }
        };
        let id = session.id.clone();
        *self.slot() = Some(session.clone());
        *CURRENT.lock().unwrap_or_else(|e| e.into_inner()) = Some(session);
        Ok(id)
    }

    fn begin(
        self: &Arc<Self>,
        store: &Arc<Store>,
        hooks: Option<Arc<dyn RecordingHooks>>,
        req: &RecordStart,
    ) -> Result<Arc<Session>, String> {
        if let Some(h) = &hooks {
            // Heavy jobs are asked to yield. Capture starts at once; the
            // engine thread waits for the runner to be idle before loading
            // the live models, so they are never resident with a final pass's.
            h.recording_started();
        }
        let events = self.deps.events.clone();
        let language = req.language.hint();
        let title = req.title.clone().unwrap_or_default();
        let meeting = store
            .create_meeting(NewMeeting {
                title: title.clone(),
                source: "mobile".into(),
                mode: "room".into(),
                lang: language.clone(),
                ..Default::default()
            })
            .map_err(|e| format!("creating the meeting: {e}"))?
            .gid;
        // Everything below undoes the meeting if it fails.
        let result = (|| -> Result<Arc<Session>, String> {
            if req.consent_acknowledged {
                store
                    .set_consent_confirmed(&meeting, true)
                    .map_err(io_err)?;
            }
            events.emit(Event::StateChanged {
                meeting: meeting.clone(),
                state: SessionState::Starting,
            });
            let writer = store.open_track(&meeting, TrackKind::Mic).map_err(io_err)?;
            let recorder = Metered::new(
                OpusRecorder::new(
                    BundlePages::new([Some(writer), None]),
                    &[Track::Mic],
                    EncoderConfig::default(),
                )
                .map_err(io_err)?,
            );
            let (producer, consumer) = ghi_audio::ring::ring(RING_SAMPLES);
            let cfg = PipelineConfig {
                route: Route::Unknown,
                // Room recording: mic only, no echo to cancel.
                aec_enabled: false,
                ..PipelineConfig::default()
            };
            let (pipeline, mut asr) = Pipeline::new(cfg, Some(consumer), None, recorder);
            // The backlog is on disk: every frame goes in, the engine's pace
            // decides when it is read (never skipped to stay real time).
            asr.never_skip();

            // Is there a live transcript? Tier, then models.
            let provider = self
                .deps
                .provider
                .clone()
                .unwrap_or_else(|| engine::provider(&self.deps.models));
            let (live, reason) = if self.deps.tier.tier != TierClass::Live {
                (false, Some(RecordOnlyReason::DeviceTier))
            } else if self.deps.provider.is_none() && !engine::engines_available(&self.deps.models)
            {
                (false, Some(RecordOnlyReason::ModelsMissing))
            } else {
                (true, None)
            };
            // Tier live: the final pass is queued even without models (it
            // waits for them). Below tier: nothing is queued.
            let job_kinds: Vec<&'static str> = if self.deps.tier.tier == TierClass::Live {
                vec![FINAL_PASS_JOB]
            } else {
                Vec::new()
            };

            let shared = Arc::new(Shared::new(meeting.clone(), events.clone()));
            {
                let mut s = shared.lock();
                s.engine_expected = live;
                s.record_only = reason;
            }
            if let Some(t) = platform::device_stats().thermal {
                shared.set_thermal(t);
            }
            // Started from the background (an intent, a widget): no GPU work
            // until the app is active.
            if !crate::lifecycle::app_active() {
                shared.gate.suspend();
            }
            let (backlog_w, backlog_r) = if live {
                let (w, r) = backlog::create(&backlog::path_for(&self.deps.backlog_dir, &meeting))
                    .map_err(io_err)?;
                (Some(w), Some(r))
            } else {
                (None, None)
            };

            let (persist_tx, persist_rx) = crossbeam_channel::unbounded();
            let persist = Persist::new(store.clone(), meeting.clone(), events.clone());
            let persist_thread = thread::Builder::new()
                .name("ghi-persist".into())
                .spawn(move || persist.run(persist_rx))
                .map_err(io_err)?;
            let (cmd, rx) = mpsc::channel();
            let pump_thread = {
                let ctx = PumpCtx {
                    shared: shared.clone(),
                    recorder: self.clone(),
                    data_dir: self.deps.backlog_dir.clone(),
                };
                thread::Builder::new()
                    .name("ghi-pump".into())
                    .spawn(move || pump(ctx, pipeline, asr, backlog_w, rx))
                    .map_err(io_err)?
            };
            {
                let mut slot = PRODUCER.lock().unwrap_or_else(|e| e.into_inner());
                if slot.is_some() {
                    drop(slot);
                    let _ = cmd.send(Cmd::Stop);
                    drop(persist_tx);
                    let _ = pump_thread.join();
                    let _ = persist_thread.join();
                    return Err(ERR_MIC_IN_USE.into());
                }
                *slot = Some(producer);
            }
            let stop_pump = |cmd: &Sender<Cmd>| {
                PRODUCER.lock().unwrap_or_else(|e| e.into_inner()).take();
                let _ = cmd.send(Cmd::Stop);
            };
            let fake_mic = self.deps.fake_mic.then(FakeMic::start);
            let fake_active = fake_mic.is_some();
            if !fake_active && let Err(e) = platform::audio_start() {
                stop_pump(&cmd);
                drop(persist_tx);
                let _ = pump_thread.join();
                let _ = persist_thread.join();
                return Err(e);
            }
            // Only now: no model loading for a session that never recorded.
            let engine_thread = if let Some(backlog) = backlog_r {
                let ctx = EngineCtx {
                    shared: shared.clone(),
                    backlog,
                    provider,
                    applier: Applier::new(
                        meeting.clone(),
                        language.clone(),
                        events.clone(),
                        persist_tx.clone(),
                    ),
                    language: language.clone(),
                    runner_idle: hooks.clone().map(|h| {
                        Arc::new(move || h.wait_idle(Duration::from_millis(1)))
                            as Arc<dyn Fn() -> bool + Send + Sync>
                    }),
                };
                match thread::Builder::new()
                    .name("ghi-engine".into())
                    .spawn(move || engine::run(ctx))
                {
                    Ok(t) => Some(t),
                    Err(e) => {
                        if !fake_active {
                            platform::audio_stop();
                        }
                        stop_pump(&cmd);
                        drop(persist_tx);
                        let _ = pump_thread.join();
                        let _ = persist_thread.join();
                        return Err(e.to_string());
                    }
                }
            } else {
                None
            };
            if reason == Some(RecordOnlyReason::ModelsMissing) {
                events.emit(Event::Error {
                    meeting: Some(meeting.clone()),
                    kind: ghi_core::events::ErrorKind::ModelsMissing,
                    message: "no live transcript: recording only; the transcript comes later"
                        .into(),
                });
            }
            let session = Arc::new(Session {
                id: meeting.clone(),
                shared: shared.clone(),
                store: store.clone(),
                recorder: self.clone(),
                hooks: hooks.clone(),
                cmd,
                persist: Mutex::new(Some(persist_tx)),
                threads: Mutex::new(Threads {
                    pump: Some(pump_thread),
                    engine: engine_thread,
                    persist: Some(persist_thread),
                }),
                stopped: AtomicBool::new(false),
                job_kinds,
                language: language.clone(),
                title: title.clone(),
                fake_mic: Mutex::new(fake_mic),
            });
            events.emit(Event::SessionStarted {
                meeting: meeting.clone(),
                mode: "room".into(),
                language: language.clone(),
                title: title.clone(),
            });
            events.emit(Event::StateChanged {
                meeting: meeting.clone(),
                state: SessionState::Recording,
            });
            platform::activity_start(shared.phase().into());
            shared.activity_update();
            Ok(session)
        })();
        match result {
            Ok(s) => Ok(s),
            Err(e) => {
                let _ = store.delete_meeting(&meeting);
                let _ = fs::remove_file(backlog::path_for(&self.deps.backlog_dir, &meeting));
                Err(e)
            }
        }
    }

    pub fn stop(&self) -> Result<(), String> {
        match self.active() {
            Some(s) => s.stop(),
            None => Err("not recording".into()),
        }
    }

    pub fn pause(&self) -> Result<(), String> {
        self.active().ok_or("not recording")?.pause()
    }

    /// Resumes after a user pause or an interruption (restarts the audio engine).
    pub fn resume(&self) -> Result<(), String> {
        self.active().ok_or("not recording")?.resume()
    }

    pub fn mark(&self) -> Result<(), String> {
        let s = self.active().ok_or("not recording")?;
        s.mark();
        Ok(())
    }

    /// The recording as it stands now.
    pub fn snapshot(&self) -> RecordState {
        match self.latest() {
            Some(s) => s.state(),
            None => RecordState {
                phase: RecordPhase::Idle,
                recording: false,
                elapsed_s: 0.0,
                marks: 0,
                level_db: -100.0,
                backlog_s: 0.0,
                catch_up_x: 0.0,
                pocket: false,
                live: false,
                record_only_reason: None,
                session: None,
            },
        }
    }

    /// The pending "Resume or stop and save?" question after an interruption.
    pub fn resume_prompt(&self) -> ResumePrompt {
        let none = ResumePrompt {
            pending: false,
            meeting: String::new(),
            recorded_s: 0.0,
            call: false,
        };
        let Some(s) = self.active() else {
            return none;
        };
        let st = s.shared.lock();
        if !st.resume_pending {
            return none;
        }
        ResumePrompt {
            pending: true,
            meeting: s.id.clone(),
            recorded_s: st.recorded_samples as f64 / RATE,
            call: st.interrupted_by_call,
        }
    }

    /// The text a user copies to tell the room they are being recorded: the
    /// user's own wording when set (settings), else the defaults. `language`
    /// is the meeting's (start) language, not the UI's; `Auto` gets both.
    pub fn consent_message(
        language: ghi_app::system::MeetingLanguage,
        custom_en: &str,
        custom_vi: &str,
    ) -> ConsentMessage {
        use ghi_app::system::MeetingLanguage as L;
        let pick = |custom: &str, default: &str| {
            if custom.trim().is_empty() {
                default.to_owned()
            } else {
                custom.trim().to_owned()
            }
        };
        let en = pick(
            custom_en,
            "Heads up: I'm recording this meeting on my phone to take notes. \
             The recording stays on my device. Tell me if you'd rather I didn't.",
        );
        let vi = pick(
            custom_vi,
            "Lưu ý: mình đang ghi âm cuộc họp này trên điện thoại để ghi chú. \
             Bản ghi âm chỉ nằm trên máy của mình. Nếu ai không muốn được ghi âm, xin cho mình biết.",
        );
        let text = match language {
            L::En => en.clone(),
            L::Vi => vi.clone(),
            L::Auto => format!("{en}\n\n{vi}"),
        };
        ConsentMessage { en, vi, text }
    }
}

impl Session {
    /// Stops recording and saves: the audio file is closed before this
    /// returns; the engine keeps going until the backlog is done, then the
    /// jobs are queued and the job runner released (all on a finisher thread).
    pub fn stop(self: &Arc<Self>) -> Result<(), String> {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        if !self.fake_mic_stop() {
            platform::audio_stop();
        }
        PRODUCER.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.shared.lock().stopped = true;
        let _ = self.cmd.send(Cmd::Stop);
        self.shared.gate.stop();
        self.set_state(SessionState::Stopping);
        let pump = self
            .threads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pump
            .take();
        if let Some(t) = pump {
            let _ = t.join();
        }
        let duration_ms = self.shared.now_ms();
        let closed = self
            .store
            .finish_meeting(&self.id, duration_ms)
            .map_err(|e| format!("closing the meeting: {e}"));
        self.shared.activity_update();
        let me = self.clone();
        thread::Builder::new()
            .name("ghi-finish".into())
            .spawn(move || me.finish())
            .map_err(io_err)?;
        closed
    }

    fn fake_mic_stop(&self) -> bool {
        match self
            .fake_mic
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            Some(m) => {
                m.stop();
                true
            }
            None => false,
        }
    }

    /// After capture: waits for the engine (its models are dropped when its
    /// thread ends), flushes the lines, queues the jobs and frees the runner.
    fn finish(self: Arc<Self>) {
        let engine = self
            .threads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .engine
            .take();
        if let Some(t) = engine {
            let _ = t.join();
        }
        // The engine's sender went with it; ours closes the persist thread.
        self.persist
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        let persist = self
            .threads
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .persist
            .take();
        if let Some(t) = persist {
            let _ = t.join();
        }
        let _ = fs::remove_file(backlog::path_for(&self.recorder.deps.backlog_dir, &self.id));
        if let Err(e) = self.store.remove_orphan_speakers(&self.id) {
            log::warn!("closing the meeting's speakers: {e}");
        }
        let queued = (|| -> Result<(), String> {
            if self.job_kinds.is_empty() {
                return Ok(());
            }
            self.store
                .set_meeting_status(&self.id, "processing")
                .map_err(io_err)?;
            for kind in &self.job_kinds {
                self.store
                    .enqueue_job(
                        Some(&self.id),
                        kind,
                        ghi_core::session::JOB_PAYLOAD_VERSION,
                        &serde_json::json!({}),
                    )
                    .map_err(io_err)?;
            }
            Ok(())
        })();
        if let Err(e) = queued {
            self.recorder.deps.events.emit(Event::Error {
                meeting: Some(self.id.clone()),
                kind: ghi_core::events::ErrorKind::Storage,
                message: format!("queueing the final pass: {e}"),
            });
        }
        self.recorder.metrics.log(&self.shared.sample());
        // Below the live tier nothing is queued: the meeting stays `done`
        // (recorded), which is not "ready" (processed), so the session simply
        // goes idle.
        self.set_state(if self.job_kinds.is_empty() {
            SessionState::Idle
        } else {
            SessionState::Processing
        });
        // Only now may a job run: nothing of this session is left in memory.
        if let Some(h) = &self.hooks {
            h.recording_stopped();
        }
        self.shared.activity_update();
        self.recorder.release(&self.id);
    }

    fn set_state(&self, state: SessionState) {
        self.recorder.deps.events.emit(Event::StateChanged {
            meeting: self.id.clone(),
            state,
        });
    }

    /// User pause: the timeline stops, the audio engine keeps running.
    pub fn pause(&self) -> Result<(), String> {
        {
            let mut s = self.shared.lock();
            if s.stopped || s.interrupted || s.paused {
                return Err("cannot pause now".into());
            }
            s.paused = true;
        }
        let _ = self.cmd.send(Cmd::Pause);
        self.set_state(SessionState::Paused);
        self.shared.activity_update();
        Ok(())
    }

    /// Resumes after a pause or an interruption. After an interruption the
    /// audio engine is restarted first (never silently: the user asked).
    pub fn resume(&self) -> Result<(), String> {
        let was_interrupted = {
            let s = self.shared.lock();
            if s.stopped || !(s.paused || s.interrupted) {
                return Err("nothing to resume".into());
            }
            s.interrupted
        };
        if was_interrupted && !self.fake_mic_active() {
            platform::audio_start()?;
        }
        {
            let mut s = self.shared.lock();
            s.paused = false;
            s.interrupted = false;
            s.resume_pending = false;
        }
        let _ = self.cmd.send(Cmd::Resume);
        self.set_state(SessionState::Recording);
        self.shared.activity_update();
        Ok(())
    }

    fn fake_mic_active(&self) -> bool {
        self.fake_mic
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    /// Marks the current moment.
    pub fn mark(&self) {
        let t_ms = self.shared.now_ms();
        self.shared.lock().marks += 1;
        if let Some(p) = self
            .persist
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            let _ = p.send(PersistMsg::Mark { t_ms });
        }
        self.recorder.deps.events.emit(Event::MarkAdded {
            meeting: self.id.clone(),
            t_ms,
        });
        self.shared.activity_update();
    }

    /// An audio interruption began (a call, another app) or ended. Recording
    /// pauses on `began` and never resumes by itself: when it ends the user is
    /// asked (`resume_prompt`).
    pub fn interrupted(&self, began: bool) {
        let call = call_active();
        {
            let mut s = self.shared.lock();
            if s.stopped {
                return;
            }
            if began {
                s.interrupted = true;
                s.interrupted_by_call = call;
                s.resume_pending = false;
            } else if s.interrupted {
                s.resume_pending = true;
            } else {
                return;
            }
        }
        if began {
            let _ = self.cmd.send(Cmd::Pause);
            self.set_state(SessionState::Paused);
        }
        mobile_events::emit(MobileEvent::Interruption {
            began,
            kind: if call {
                InterruptionKind::Call
            } else {
                InterruptionKind::Other
            },
        });
        self.shared.activity_update();
    }

    /// The phone call that interrupted the recording ended: ask the user, even
    /// if the audio session never reported the interruption's end.
    pub fn call_ended(&self) {
        {
            let mut s = self.shared.lock();
            if s.stopped || !s.interrupted || !s.interrupted_by_call || s.resume_pending {
                return;
            }
            s.resume_pending = true;
        }
        mobile_events::emit(MobileEvent::Interruption {
            began: false,
            kind: InterruptionKind::Call,
        });
        self.shared.activity_update();
    }

    pub fn activity_update(&self) {
        self.shared.activity_update();
    }

    /// The state for a (re)loaded webview.
    pub fn state(&self) -> RecordState {
        let phase = self.shared.phase();
        let snapshot = self.snapshot();
        let s = self.shared.lock();
        RecordState {
            phase,
            recording: !s.stopped && !s.interrupted && !s.paused,
            elapsed_s: self.shared.now_ms() as f64 / 1000.0,
            marks: s.marks,
            level_db: s.level_db,
            backlog_s: s.backlog_samples() as f64 / RATE,
            catch_up_x: s.catch_up_x.unwrap_or(0.0),
            pocket: s.pocket,
            live: s.engine_expected && !s.engine_failed,
            record_only_reason: s
                .record_only
                .or((phase == RecordPhase::Hot).then_some(RecordOnlyReason::Thermal)),
            session: Some(snapshot),
        }
    }

    /// The core session snapshot (lines so far, speakers, marks).
    fn snapshot(&self) -> SessionSnapshot {
        use ghi_core::events::{LineInfo, SpeakerInfo, WordInfo};
        let wait = Duration::from_secs(5);
        let seq = self.recorder.deps.events.last_seq();
        let mut gids: Vec<(u32, String)> = Vec::new();
        // Clone the sender: the waits below must not hold up `mark` or `finish`.
        let persist = self
            .persist
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(p) = persist {
            // The lines still waiting to be written, then who is who.
            let (tx, rx) = crossbeam_channel::bounded(1);
            if p.send(PersistMsg::Flush(tx)).is_ok() {
                let _ = rx.recv_timeout(wait);
            }
            let (tx, rx) = crossbeam_channel::bounded(1);
            if p.send(PersistMsg::SpeakerGids(tx)).is_ok()
                && let Ok(v) = rx.recv_timeout(wait)
            {
                gids = v;
            }
        }
        let stored = self.store.speakers(&self.id).unwrap_or_default();
        let own: Vec<_> = stored.iter().filter(|s| s.merged_into.is_none()).collect();
        if gids.is_empty() {
            gids = own
                .iter()
                .zip(1u32..)
                .map(|(s, id)| (id, s.gid.clone()))
                .collect();
        }
        let id_of = |gid: &str| gids.iter().find(|(_, g)| g == gid).map(|(i, _)| *i);
        let speakers = own
            .iter()
            .filter_map(|s| {
                Some(SpeakerInfo {
                    id: id_of(&s.gid)?,
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
            })
            .collect();
        let lines = self
            .store
            .segments(&self.id)
            .unwrap_or_default()
            .into_iter()
            .map(|seg| {
                let words = self.store.segment_words(&seg.gid).unwrap_or_default();
                LineInfo {
                    speaker: seg.speaker_gid.as_deref().and_then(id_of),
                    t0_ms: seg.t0_ms,
                    t1_ms: seg.t1_ms,
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
                                .is_some_and(|c| c < ghi_core::aligner::LOW_CONFIDENCE),
                        })
                        .collect(),
                    gid: seg.gid,
                    text: seg.text,
                }
            })
            .collect();
        let state = if self.stopped.load(Ordering::Acquire) {
            SessionState::Stopping
        } else {
            let s = self.shared.lock();
            if s.paused || s.interrupted {
                SessionState::Paused
            } else {
                SessionState::Recording
            }
        };
        SessionSnapshot {
            seq,
            meeting: self.id.clone(),
            state,
            now_ms: self.shared.now_ms(),
            transcribing: self.shared.lock().engine_expected,
            mode: "room".into(),
            language: self.language.clone(),
            title: self.title.clone(),
            consent_confirmed: self
                .store
                .get_meeting(&self.id)
                .map(|m| m.consent_confirmed)
                .unwrap_or(false),
            speakers,
            lines,
            marks: self
                .store
                .marks(&self.id)
                .unwrap_or_default()
                .into_iter()
                .map(|m| m.t_ms)
                .collect(),
        }
    }
}

/// Stops and saves what there is, from another thread (stopping joins the pump).
fn stop_soon(recorder: &Recorder) {
    if let Some(s) = recorder.active() {
        let _ = thread::Builder::new()
            .name("ghi-auto-stop".into())
            .spawn(move || {
                let _ = s.stop();
            });
    }
}

struct PumpCtx {
    shared: Arc<Shared>,
    recorder: Arc<Recorder>,
    data_dir: PathBuf,
}

fn pump(
    ctx: PumpCtx,
    mut pipeline: Pipeline<Metered<OpusRecorder<BundlePages>>>,
    mut asr: ghi_audio::pipeline::AsrConsumer,
    mut backlog: Option<BacklogWriter>,
    rx: Receiver<Cmd>,
) {
    let PumpCtx {
        shared,
        recorder,
        data_dir,
    } = ctx;
    // However the pump ends (a panic included), the capture counts as done.
    struct CaptureDone(Arc<Shared>);
    impl Drop for CaptureDone {
        fn drop(&mut self) {
            self.0.capture_done.store(true, Ordering::Release);
            self.0.activity_update();
        }
    }
    let _done = CaptureDone(shared.clone());
    let meeting = shared.meeting.clone();
    let events = shared.events.clone();
    let mut last_metrics = Instant::now();
    let mut last_disk = Instant::now();
    let mut last_level = Instant::now();
    let mut last_backlog = Instant::now();
    let mut muffle = MuffleDetector::new();
    let mut stopping = false;
    let mut writer_reported = false;
    let mut capture_reported = false;
    let fail = |shared: &Shared, e: String| {
        let mut s = shared.lock();
        if s.error.is_none() {
            s.error = Some(e);
        }
    };
    loop {
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                Cmd::Pause => pipeline.pause(),
                Cmd::Resume => pipeline.resume(),
                Cmd::Stop => stopping = true,
            }
        }
        let report = pipeline.step();
        for ev in report.events {
            let lost = match ev {
                CaptureEvent::Overrun { dropped, .. } => dropped,
                CaptureEvent::AsrSkipped { frames } => frames * ghi_audio::FRAME_SAMPLES as u64,
                _ => 0,
            };
            shared.lock().dropped_samples += lost;
            if let CaptureEvent::Error { .. } | CaptureEvent::TrackLost { .. } = ev
                && !capture_reported
            {
                capture_reported = true;
                events.emit(Event::Error {
                    meeting: Some(meeting.clone()),
                    kind: ghi_core::events::ErrorKind::Capture,
                    message: "the microphone stopped delivering audio".into(),
                });
            }
        }
        while let Some(frame) = asr.pop() {
            if let Some(w) = backlog.as_mut()
                && let Err(e) = w.push(&frame.mic)
            {
                fail(&shared, format!("backlog: {e}"));
            }
            if let Some(on) = muffle.push(&frame.mic) {
                shared.lock().pocket = on;
                mobile_events::emit(MobileEvent::Pocket { muffled: on });
            }
        }
        if let Some(w) = backlog.as_mut()
            && (w.pending() >= backlog::MIN_BATCH || stopping)
            && let Err(e) = w.publish()
        {
            fail(&shared, format!("backlog: {e}"));
        }
        {
            let mut s = shared.lock();
            s.recorded_samples = pipeline.position();
            s.written_samples = backlog.as_ref().map_or(0, |w| w.written());
            s.dropped_samples += TAP_DROPS.swap(0, Ordering::Relaxed);
        }
        shared
            .position
            .store(pipeline.position(), Ordering::Relaxed);
        if last_level.elapsed() >= Duration::from_millis(100) {
            last_level = Instant::now();
            let [mic, _] = pipeline.writer_mut().take_dbfs();
            if let Some(db) = mic {
                shared.lock().level_db = db;
            }
            events.emit(Event::LevelMeter {
                meeting: meeting.clone(),
                mic_dbfs: mic,
                system_dbfs: None,
            });
        }
        shared.activity_refresh();
        if pipeline.writer_failed() && !writer_reported {
            writer_reported = true;
            fail(&shared, "writing the audio file failed".into());
            events.emit(Event::Error {
                meeting: Some(meeting.clone()),
                kind: ghi_core::events::ErrorKind::Storage,
                message: "writing the audio failed: the recording was stopped and saved".into(),
            });
            stop_soon(&recorder);
        }
        if last_backlog.elapsed() >= Duration::from_secs(2) {
            last_backlog = Instant::now();
            let (backlog_s, catch_up_x) = {
                let s = shared.lock();
                (
                    s.backlog_samples() as f64 / RATE,
                    s.catch_up_x.unwrap_or(0.0),
                )
            };
            if backlog_s > 0.0 {
                mobile_events::emit(MobileEvent::Backlog {
                    backlog_s,
                    catch_up_x,
                });
            }
        }
        if last_metrics.elapsed() >= METRICS_EVERY {
            last_metrics = Instant::now();
            recorder.metrics.log(&shared.sample());
        }
        if last_disk.elapsed() >= DISK_EVERY {
            last_disk = Instant::now();
            if let Some(free) = free_bytes(&data_dir)
                && free < MIN_FREE_BYTES
                && !stopping
            {
                events.emit(Event::DiskLow {
                    meeting: meeting.clone(),
                    free_bytes: free,
                });
                stop_soon(&recorder);
            }
        }
        if stopping {
            break;
        }
        thread::sleep(STEP);
    }
    // Everything left in the ring and the pipeline goes to the bundle and backlog.
    let _ = pipeline.step();
    while let Some(frame) = asr.pop() {
        if let Some(w) = backlog.as_mut() {
            let _ = w.push(&frame.mic);
        }
    }
    if let Some(w) = backlog.as_mut() {
        let _ = w.publish();
    }
    let position = pipeline.position();
    let written = backlog.as_ref().map_or(0, |w| w.written());
    if let Err(e) = pipeline.finish().inner.finish() {
        fail(&shared, format!("closing the audio file: {e}"));
    }
    {
        let mut s = shared.lock();
        s.recorded_samples = position;
        s.written_samples = written;
    }
    shared.position.store(position, Ordering::Relaxed);
}

/// A session's shared state without threads, for engine tests.
#[cfg(test)]
pub fn test_shared(dir: &Path) -> Arc<Shared> {
    fs::create_dir_all(dir).unwrap();
    let (events, _rx) = ghi_core::events::bus();
    Arc::new(Shared::new("test".into(), events))
}

#[cfg(test)]
impl Shared {
    /// A capture that is still going (the engine waits for more audio).
    pub fn unfinish_capture_for_test(&self) {
        self.lock().stopped = false;
        self.capture_done.store(false, Ordering::Release);
    }

    pub fn finish_capture_for_test(&self) {
        self.lock().stopped = true;
        self.gate.stop();
        self.capture_done.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests;
