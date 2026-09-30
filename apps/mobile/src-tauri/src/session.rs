// SPDX-License-Identifier: Apache-2.0
//! One recording: capture → Ogg Opus file + backlog → live engine.
//!
//! ```text
//! Swift tap ─► ring ─► pump thread: ghi-audio Pipeline ─┬─► mic.opus (crash-safe pages)
//!   (any rate)          (16 kHz, 10 ms frames)          └─► backlog.pcm ─► engine thread
//! ```
//!
//! The pump never waits on the engine, so audio is recorded whatever the
//! engine does (held while locked or hot, loading, or failed). Everything the
//! session learns is in [`Shared`]; the UI polls it with `snapshot`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ghi_audio::encoder::{EncoderConfig, OpusRecorder, PageSink};
use ghi_audio::pipeline::{Pipeline, PipelineConfig};
use ghi_audio::ring::RingProducer;
use ghi_audio::{CaptureEvent, Marker, Route, Track};
use serde::Serialize;
use specta::Type;

use crate::backlog::{self, BacklogWriter};
use crate::engine;
use crate::gate::{Gate, Hold};
use crate::platform;

const STEP: Duration = Duration::from_millis(10);
const METRICS_EVERY: Duration = Duration::from_secs(10);
/// Ring between the Swift tap and the pump: 2 s at 48 kHz.
const RING_SAMPLES: usize = 96_000;

/// A finished transcript line. Times are seconds of processed audio.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub start: f64,
    pub end: f64,
    /// 1-based diarization speaker.
    pub speaker: Option<u32>,
    pub text: String,
}

/// What the session is doing, for the UI and the Live Activity.
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
    /// A phone call or another app took the audio session.
    Interrupted,
    /// Stopped; the engine finishes the backlog.
    Finishing,
    Done,
    /// No engine (not built, models missing or failed): recording only.
    RecordOnly,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// Audio recorded, in seconds.
    pub recorded_s: f64,
    /// Audio the engine has processed, in seconds.
    pub processed_s: f64,
    /// Audio waiting for the engine, in seconds.
    pub backlog_s: f64,
    /// Engine compute time / audio time, over the last 10 s of steps.
    pub rtf: Option<f64>,
    /// Audio seconds processed per wall second during the last catch-up.
    pub catch_up_x: Option<f64>,
    pub model_load_s: Option<f64>,
    pub dropped_samples: f64,
    /// Engine steps (or model loads) that were still running when the app
    /// went to the background; each one rebuilt the models and streams.
    pub gpu_overlaps: u32,
    pub engine_resets: u32,
    /// `ProcessInfo.ThermalState`: 0 nominal, 1 fair, 2 serious, 3 critical.
    pub thermal: Option<i32>,
    pub memory_mb: Option<f64>,
    /// 0..1, or `None` when unknown (simulator).
    pub battery: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub id: String,
    pub phase: Phase,
    /// Capture is running (Stop and Mark apply).
    pub recording: bool,
    pub elapsed_s: f64,
    /// Lines from the requested index on.
    pub lines: Vec<Line>,
    /// Total number of lines (the next `since`).
    pub line_count: u32,
    pub partial: String,
    pub marks: Vec<f64>,
    pub stats: Stats,
    pub error: Option<String>,
}

#[derive(Debug, Default)]
struct Inner {
    lines: Vec<Line>,
    partial: String,
    marks: Vec<f64>,
    error: Option<String>,
    interrupted: bool,
    stopped: bool,
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
    /// (audio samples, compute time) of recent steps, for the RTF.
    window: Vec<(Instant, u64, Duration)>,
    catch_up: Option<CatchUp>,
    catch_up_x: Option<f64>,
    model_load_s: Option<f64>,
    thermal: Option<i32>,
    /// Backlog position where the current ASR/diarization streams started.
    stream_offset_samples: u64,
    resets: u32,
    /// The phase last pushed to the Live Activity.
    pushed: Option<(Phase, usize)>,
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
    started: Instant,
    dir: PathBuf,
}

const RATE: f64 = engine::SAMPLE_RATE as f64;
/// Audio the ASR may still hold without a partial: a 1120 ms chunk, its right
/// context, the 800 ms endpoint hold, and margin.
const UNSETTLED_SAMPLES: u64 = 3 * engine::SAMPLE_RATE as u64;

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn capture_done(&self) -> bool {
        self.capture_done.load(Ordering::Acquire)
    }

    #[cfg_attr(not(feature = "nemo"), allow(dead_code))]
    pub fn timed_load<T>(&self, load: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let t = Instant::now();
        let r = load();
        let mut s = self.lock();
        s.model_load_s = Some(t.elapsed().as_secs_f64());
        s.engine_ready = r.is_ok();
        r
    }

    /// Commits what an engine step produced. `position` is the backlog read
    /// position after the step; `finished` means the streams were flushed.
    pub fn commit(&self, updates: Vec<engine::Update>, position: u64, finished: bool) {
        let mut s = self.lock();
        let offset = s.stream_offset_samples;
        for u in updates {
            match u {
                engine::Update::Partial(text) => s.partial = text,
                engine::Update::Final {
                    start,
                    end,
                    speaker,
                    text,
                } => {
                    let line = Line {
                        start: start + offset as f64 / RATE,
                        end: end + offset as f64 / RATE,
                        speaker,
                        text,
                    };
                    let _ = append_json(&self.dir.join("transcript.jsonl"), &line);
                    s.partial.clear();
                    s.lines.push(line);
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
        } else if s.partial.is_empty() {
            let settled = position.saturating_sub(UNSETTLED_SAMPLES);
            s.committed_samples = s.committed_samples.max(settled);
        }
    }

    /// The models are being reloaded (the gate was poisoned).
    #[cfg_attr(not(feature = "nemo"), allow(dead_code))]
    pub fn models_unloaded(&self) {
        self.lock().engine_ready = false;
    }

    /// Where a reset redoes the audio from.
    #[cfg_attr(not(feature = "nemo"), allow(dead_code))]
    pub fn committed_position(&self) -> u64 {
        self.lock().committed_samples
    }

    /// The engine reopened its streams at backlog `position` after dropping
    /// a suspect step (and reloaded the models if the gate was poisoned).
    #[cfg_attr(not(feature = "nemo"), allow(dead_code))]
    pub fn engine_reset(&self, position: u64) {
        let mut s = self.lock();
        s.stream_offset_samples = position;
        s.read_samples = position;
        s.partial.clear();
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
        s.error = Some(e);
    }

    pub fn engine_done(&self) {
        self.lock().engine_done = true;
        // Everything is transcribed (or can't be): the raw backlog can go.
        let _ = fs::remove_file(self.dir.join("backlog.pcm"));
        self.activity_update();
    }

    /// Pushes the phase and mark count to the Live Activity (ends it when
    /// the capture has stopped and the engine is done).
    pub fn activity_update(&self) {
        let phase = self.phase();
        let (marks, finished) = {
            let mut s = self.lock();
            s.pushed = Some((phase, s.marks.len()));
            (s.marks.len() as u32, s.engine_done && self.capture_done())
        };
        platform::activity_update(phase, marks, finished);
    }

    /// Updates the Live Activity if the phase or marks changed since the last push.
    fn activity_refresh(&self) {
        let phase = self.phase();
        let changed = {
            let s = self.lock();
            s.pushed != Some((phase, s.marks.len()))
        };
        if changed {
            self.activity_update();
        }
    }

    pub fn set_thermal(&self, state: i32) {
        self.lock().thermal = Some(state);
        self.gate.set_hot(state >= 2);
    }

    pub fn phase(&self) -> Phase {
        let s = self.lock();
        let behind = s.backlog_samples() as f64 / RATE;
        if s.engine_done {
            return if s.engine_failed {
                Phase::RecordOnly
            } else {
                Phase::Done
            };
        }
        // Stopped or interrupted first: while locked the Live Activity must
        // not claim "Recording" when nothing is.
        if s.stopped {
            return if s.engine_failed {
                Phase::Done
            } else {
                Phase::Finishing
            };
        }
        if s.interrupted {
            return Phase::Interrupted;
        }
        if s.engine_failed {
            return Phase::RecordOnly;
        }
        match self.gate.hold() {
            Hold::Suspended => return Phase::Locked,
            Hold::Hot => return Phase::Hot,
            Hold::None => {}
        }
        if !s.engine_ready {
            Phase::Loading
        } else if behind > 5.0 {
            Phase::CatchingUp
        } else {
            Phase::Live
        }
    }

    fn stats(&self) -> Stats {
        let device = platform::device_stats();
        let s = self.lock();
        let (audio, compute) = s
            .window
            .iter()
            .fold((0u64, Duration::ZERO), |(a, c), (_, n, d)| (a + n, c + *d));
        Stats {
            recorded_s: s.recorded_samples as f64 / RATE,
            processed_s: s.processed_samples as f64 / RATE,
            backlog_s: s.backlog_samples() as f64 / RATE,
            rtf: (audio > 0).then(|| compute.as_secs_f64() / (audio as f64 / RATE)),
            catch_up_x: s.catch_up_x,
            model_load_s: s.model_load_s,
            dropped_samples: s.dropped_samples as f64,
            gpu_overlaps: self.gate.overlaps(),
            engine_resets: s.resets,
            thermal: s.thermal.or(device.thermal),
            memory_mb: device.memory_mb,
            battery: device.battery,
        }
    }
}

enum Cmd {
    Pause,
    Resume,
    Stop,
}

pub struct Session {
    pub id: String,
    pub shared: Arc<Shared>,
    cmd: Sender<Cmd>,
}

/// The audio tap's side of the ring (the C ABI pushes into it).
static PRODUCER: Mutex<Option<RingProducer>> = Mutex::new(None);
/// The recording in progress, or the last one.
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

/// Ogg Opus pages of the mic track, and markers, into the session directory.
struct FilePages {
    mic: File,
    markers: PathBuf,
}

impl PageSink for FilePages {
    fn write_page(&mut self, _track: Track, page: &[u8]) -> io::Result<()> {
        self.mic.write_all(page)
    }

    fn sync(&mut self, _track: Track, durable: bool) -> io::Result<()> {
        if durable {
            self.mic.sync_data()
        } else {
            Ok(())
        }
    }

    fn marker(&mut self, marker: &Marker) -> io::Result<()> {
        let line = serde_json::json!({"pos": marker.pos, "kind": format!("{:?}", marker.kind)});
        append_json(&self.markers, &line)
    }
}

fn append_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    f.write_all(&line)
}

impl Session {
    /// Starts a recording in `root/recordings/<id>/`, with models from `models`.
    pub fn start(root: &Path, models: PathBuf) -> Result<Arc<Session>, String> {
        let mut current = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = current.as_ref() {
            if !s.shared.capture_done() {
                return Err("a recording is already running".into());
            }
            // One engine at a time: it owns the lifecycle events, the Live
            // Activity and ~1.3 GB of scheduler memory.
            if !s.shared.lock().engine_done {
                return Err(
                    "the last recording is still being transcribed; try again in a moment".into(),
                );
            }
        }
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis()
            .to_string();
        let io = |e: io::Error| e.to_string();
        let dir = root.join("recordings").join(&id);
        fs::create_dir_all(root.join("recordings")).map_err(io)?;
        // A new directory: never reuse (and truncate) an earlier recording's files.
        fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let sink = FilePages {
            mic: File::create(dir.join("mic.opus")).map_err(io)?,
            markers: dir.join("markers.jsonl"),
        };
        let recorder =
            OpusRecorder::new(sink, &[Track::Mic], EncoderConfig::default()).map_err(io)?;
        let (producer, consumer) = ghi_audio::ring::ring(RING_SAMPLES);
        let cfg = PipelineConfig {
            route: Route::Unknown,
            // Room recording: mic only, no echo to cancel.
            aec_enabled: false,
            ..PipelineConfig::default()
        };
        let (pipeline, asr) = Pipeline::new(cfg, Some(consumer), None, recorder);
        let (backlog_w, backlog_r, _) = backlog::create(&dir.join("backlog.pcm")).map_err(io)?;

        let shared = Arc::new(Shared {
            gate: Gate::default(),
            inner: Mutex::new(Inner::default()),
            capture_done: AtomicBool::new(false),
            started: Instant::now(),
            dir: dir.clone(),
        });
        if let Some(t) = platform::device_stats().thermal {
            shared.set_thermal(t);
        }
        let (cmd, rx) = mpsc::channel();
        {
            let shared = shared.clone();
            thread::Builder::new()
                .name("ghi-pump".into())
                .spawn(move || pump(shared, pipeline, asr, backlog_w, rx))
                .map_err(io)?;
        }
        *PRODUCER.lock().unwrap_or_else(|e| e.into_inner()) = Some(producer);
        if let Err(e) = platform::audio_start() {
            PRODUCER.lock().unwrap_or_else(|e| e.into_inner()).take();
            let _ = cmd.send(Cmd::Stop);
            return Err(e);
        }
        // Only now: no model loading for a session that never recorded.
        {
            let shared = shared.clone();
            if let Err(e) = thread::Builder::new()
                .name("ghi-engine".into())
                .spawn(move || engine::run(shared, backlog_r, models))
            {
                platform::audio_stop();
                PRODUCER.lock().unwrap_or_else(|e| e.into_inner()).take();
                let _ = cmd.send(Cmd::Stop);
                return Err(e.to_string());
            }
        }
        platform::activity_start(shared.phase());
        let session = Arc::new(Session { id, shared, cmd });
        *current = Some(session.clone());
        Ok(session)
    }

    /// Stops recording; the engine keeps going until the backlog is done.
    pub fn stop(&self) {
        platform::audio_stop();
        PRODUCER.lock().unwrap_or_else(|e| e.into_inner()).take();
        self.shared.lock().stopped = true;
        let _ = self.cmd.send(Cmd::Stop);
        self.shared.gate.stop();
        self.activity_update();
    }

    pub fn mark(&self) {
        let t = self.shared.started.elapsed().as_secs_f64();
        self.shared.lock().marks.push(t);
        let _ = append_json(
            &self.shared.dir.join("markers.jsonl"),
            &serde_json::json!({"mark_s": t}),
        );
        self.activity_update();
    }

    pub fn interrupted(&self, began: bool) {
        self.shared.lock().interrupted = began;
        let _ = self.cmd.send(if began { Cmd::Pause } else { Cmd::Resume });
        self.activity_update();
    }

    pub fn activity_update(&self) {
        self.shared.activity_update();
    }

    pub fn snapshot(&self, since: u32) -> Snapshot {
        let phase = self.shared.phase();
        let stats = self.shared.stats();
        let s = self.shared.lock();
        let from = (since as usize).min(s.lines.len());
        Snapshot {
            id: self.id.clone(),
            phase,
            recording: !s.stopped && !self.shared.capture_done(),
            elapsed_s: self.shared.started.elapsed().as_secs_f64(),
            lines: s.lines[from..].to_vec(),
            line_count: s.lines.len() as u32,
            partial: s.partial.clone(),
            marks: s.marks.clone(),
            stats,
            error: s.error.clone(),
        }
    }

    /// Waits until capture and engine are both done (tests).
    #[cfg(test)]
    pub fn join(&self) {
        let t = Instant::now();
        while !(self.shared.capture_done() && self.shared.lock().engine_done) {
            assert!(
                t.elapsed() < Duration::from_secs(10),
                "session did not finish"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}

fn pump(
    shared: Arc<Shared>,
    mut pipeline: Pipeline<OpusRecorder<FilePages>>,
    mut asr: ghi_audio::pipeline::AsrConsumer,
    mut backlog: BacklogWriter,
    rx: Receiver<Cmd>,
) {
    // However the pump ends (a panic included), the capture counts as done.
    struct CaptureDone(Arc<Shared>);
    impl Drop for CaptureDone {
        fn drop(&mut self) {
            self.0.capture_done.store(true, Ordering::Release);
            self.0.activity_update();
        }
    }
    let _done = CaptureDone(shared.clone());
    let metrics = shared.dir.join("metrics.jsonl");
    let mut last_metrics = Instant::now();
    let mut stopping = false;
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
            let _ = append_json(
                &shared.dir.join("events.jsonl"),
                &serde_json::json!({"event": format!("{ev:?}")}),
            );
        }
        while let Some(frame) = asr.pop() {
            if let Err(e) = backlog.push(&frame.mic) {
                fail(&shared, format!("backlog: {e}"));
            }
        }
        if let Err(e) = backlog.publish() {
            fail(&shared, format!("backlog: {e}"));
        }
        {
            let mut s = shared.lock();
            s.recorded_samples = pipeline.position();
            s.written_samples = backlog.written();
            s.dropped_samples += TAP_DROPS.swap(0, Ordering::Relaxed);
        }
        shared.activity_refresh();
        if pipeline.writer_failed() {
            fail(&shared, "writing the audio file failed".into());
        }
        if last_metrics.elapsed() >= METRICS_EVERY {
            last_metrics = Instant::now();
            let _ = append_json(
                &metrics,
                &serde_json::json!({
                    "t_s": shared.started.elapsed().as_secs_f64(),
                    "phase": shared.phase(),
                    "stats": shared.stats(),
                }),
            );
        }
        if stopping {
            break;
        }
        thread::sleep(STEP);
    }
    // Everything left in the ring and the pipeline goes to the file and backlog.
    let _ = pipeline.step();
    while let Some(frame) = asr.pop() {
        let _ = backlog.push(&frame.mic);
    }
    let _ = backlog.publish();
    if let Err(e) = pipeline.finish().finish() {
        fail(&shared, format!("closing the audio file: {e}"));
    }
    {
        let mut s = shared.lock();
        s.recorded_samples = backlog.written();
        s.written_samples = backlog.written();
    }
    let _ = append_json(
        &metrics,
        &serde_json::json!({
            "t_s": shared.started.elapsed().as_secs_f64(),
            "phase": "capture_done",
            "stats": shared.stats(),
        }),
    );
}

/// A session's shared state without threads, for engine tests.
#[cfg(test)]
pub fn test_shared(dir: &Path) -> Arc<Shared> {
    fs::create_dir_all(dir).unwrap();
    Arc::new(Shared {
        gate: Gate::default(),
        inner: Mutex::new(Inner::default()),
        capture_done: AtomicBool::new(false),
        started: Instant::now(),
        dir: dir.to_path_buf(),
    })
}

#[cfg(test)]
impl Shared {
    pub fn finish_capture_for_test(&self) {
        self.lock().stopped = true;
        self.gate.stop();
        self.capture_done.store(true, Ordering::Release);
    }

    pub fn lines_for_test(&self) -> Vec<Line> {
        self.lock().lines.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// While live, only audio older than the unsettled margin is final; a
    /// flush makes everything final.
    #[test]
    fn live_commits_keep_a_margin_for_undecoded_audio() {
        let dir = std::env::temp_dir().join(format!("ghi-commit-{}", std::process::id()));
        let shared = test_shared(&dir);
        let sec = engine::SAMPLE_RATE as u64;
        shared.commit(Vec::new(), 2 * sec, false);
        assert_eq!(shared.committed_position(), 0, "within the margin");
        shared.commit(Vec::new(), 10 * sec, false);
        assert_eq!(shared.committed_position(), 7 * sec);
        shared.commit(
            vec![engine::Update::Partial("đang nói".into())],
            12 * sec,
            false,
        );
        assert_eq!(
            shared.committed_position(),
            7 * sec,
            "an utterance is in progress"
        );
        let fin = engine::Update::Final {
            start: 9.0,
            end: 11.5,
            speaker: None,
            text: "đang nói đây".into(),
        };
        shared.commit(vec![fin], 13 * sec, false);
        assert_eq!(
            shared.committed_position(),
            sec * 23 / 2,
            "the final's end (later than 13 s minus the margin)"
        );
        shared.commit(Vec::new(), 14 * sec, true);
        assert_eq!(shared.committed_position(), 14 * sec, "flushed");
        fs::remove_dir_all(&dir).unwrap();
    }

    fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
        let t = Instant::now();
        while !f() {
            assert!(
                t.elapsed() < Duration::from_secs(10),
                "timed out waiting for {what}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Host build without the engine: audio still reaches the file and the
    /// backlog, stop finishes the capture, and the engine reports record-only.
    #[test]
    fn records_without_an_engine() {
        let root = std::env::temp_dir().join(format!("ghi-mobile-session-{}", std::process::id()));
        let session = Session::start(&root, root.join("models")).unwrap();
        let tone: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.05).sin() * 0.1).collect();
        let mut t_ns = 1_000_000_000u64;
        for _ in 0..20 {
            push_pcm(&tone, 48_000.0, t_ns);
            t_ns += 100_000_000;
            thread::sleep(Duration::from_millis(5));
        }
        wait_for("recorded audio", || {
            session.snapshot(0).stats.recorded_s > 1.0
        });
        session.mark();
        session.stop();
        session.join();
        let snap = session.snapshot(0);
        assert!(session.shared.capture_done());
        assert!(snap.stats.recorded_s >= 1.5, "{:?}", snap.stats);
        assert_eq!(snap.marks.len(), 1);
        if cfg!(feature = "nemo") {
            assert!(snap.error.is_some(), "models are missing");
        } else {
            assert_eq!(snap.phase, Phase::RecordOnly);
        }
        let dir = root.join("recordings").join(&session.id);
        let opus = fs::read(dir.join("mic.opus")).unwrap();
        assert!(opus.starts_with(b"OggS") && opus.len() > 1000);
        assert!(
            Session::start(&root, root.join("models")).is_ok(),
            "a new session may start"
        );
        current().unwrap().stop();
        current().unwrap().join();
        fs::remove_dir_all(&root).unwrap();
    }
}
