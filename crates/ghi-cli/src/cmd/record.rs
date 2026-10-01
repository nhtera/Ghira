// SPDX-License-Identifier: Apache-2.0
//! `ghi record`: capture (macOS) or replay WAV files through the capture
//! pipeline into the file sinks, and `ghi recover`: decode Ogg Opus tracks
//! left by a crash. Tooling for tests, soak runs and the eval kit.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ghi_audio::encoder::{EncoderConfig, OpusRecorder, read_ogg_opus_detailed};
use ghi_audio::pipeline::{FrameSink, Pipeline, PipelineConfig};
use ghi_audio::ring::{RingConsumer, RingProducer, ring};
use ghi_audio::{CaptureEvent, Route, SAMPLE_RATE, Track};

use crate::audio::{Audio, read_wav};
use crate::contract::{
    ErrorCode, ErrorDoc, Perf, RECORD, RECOVER, Record, RecordEvent, RecordTrack, Recover,
    RecoveredFile,
};
use crate::sink::{OggPages, StorePages, Tally, WavSink, seconds, sync_dir, track_path, write_wav};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Mode {
    /// Mic + system audio (the far end of a call).
    Call,
    /// Mic only.
    Room,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    /// 16 kHz 16-bit WAV per track, plus the mix for calls (eval kit layout).
    Wav,
    /// Ogg Opus per track, synced page by page (crash-safe).
    Opus,
    /// A meeting in the encrypted Ghira store at `--out` (a data directory):
    /// Ogg Opus pages sealed into the store's audio bundles.
    Store,
}

pub struct Args<'a> {
    pub mode: Mode,
    pub out: &'a Path,
    pub id: Option<&'a str>,
    /// Meeting title (`--format store`).
    pub title: Option<&'a str>,
    pub duration_s: Option<f64>,
    pub format: Format,
    /// WAV files played through the pipeline at 1x instead of capturing:
    /// `mic` (room) or `mic,system` (call).
    pub replay: &'a [PathBuf],
    /// Tap only these processes.
    pub pids: &'a [i32],
    pub aec: bool,
    /// Seconds of silence on the system track before a warning.
    pub silent_s: f32,
}

/// Disk space below which recording warns, and below which it stops.
const DISK_LOW: u64 = 1 << 30;
const DISK_STOP: u64 = 64 << 20;

/// A capture that delivers nothing for this long (not paused or asleep) is
/// stopped instead of recording silence forever.
const NO_AUDIO: Duration = Duration::from_secs(10);

pub(crate) static STOP: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

/// Ctrl-C / SIGTERM end the recording cleanly instead of killing it.
pub(crate) fn install_signal_handlers() {
    #[cfg(unix)]
    for sig in [libc::SIGINT, libc::SIGTERM] {
        // SAFETY: the handler only stores to an atomic (async-signal-safe).
        unsafe {
            libc::signal(
                sig,
                on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t,
            )
        };
    }
}

fn internal(what: &str, e: impl std::fmt::Display) -> ErrorDoc {
    ErrorDoc::new(ErrorCode::Internal, format!("{what}: {e}"))
}

fn tracks_of(mode: Mode) -> &'static [Track] {
    match mode {
        Mode::Call => &Track::ALL,
        Mode::Room => &[Track::Mic],
    }
}

/// Where the audio comes from.
enum Handle {
    #[cfg(target_os = "macos")]
    Mac(ghi_audio::macos::MacCapture),
    Replay {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    },
}

impl Handle {
    fn ended(&self) -> bool {
        match self {
            #[cfg(target_os = "macos")]
            Handle::Mac(_) => false,
            Handle::Replay { thread, .. } => thread.as_ref().is_none_or(|t| t.is_finished()),
        }
    }

    fn stop(self) {
        match self {
            #[cfg(target_os = "macos")]
            Handle::Mac(capture) => drop(capture),
            Handle::Replay { stop, thread } => {
                stop.store(true, Ordering::Relaxed);
                if let Some(t) = thread {
                    let _ = t.join();
                }
            }
        }
    }
}

struct Source {
    mic: Option<RingConsumer>,
    system: Option<RingConsumer>,
    events: mpsc::Receiver<CaptureEvent>,
    handle: Handle,
    route: Route,
    kind: &'static str,
}

fn monotonic_ns(base: Instant) -> u64 {
    base.elapsed().as_nanos() as u64
}

/// Plays WAV files into the rings at 1x, in 10 ms blocks, stamped with an
/// ideal clock. The shorter file is padded with silence.
fn replay_source(mode: Mode, files: &[PathBuf]) -> Result<Source, ErrorDoc> {
    let tracks = tracks_of(mode);
    if files.len() != tracks.len() {
        return Err(ErrorDoc::new(
            ErrorCode::BadInput,
            format!(
                "--replay needs {} WAV file(s) for --mode {mode:?}",
                tracks.len()
            ),
        ));
    }
    let mut audio: Vec<Audio> = files
        .iter()
        .map(|f| read_wav(f))
        .collect::<Result<_, _>>()?;
    let longest = audio.iter().map(Audio::duration_s).fold(0.0, f64::max);
    for a in &mut audio {
        let n = (longest * f64::from(a.sample_rate)).ceil() as usize;
        a.samples.resize(n.max(a.samples.len()), 0.0);
    }
    let mut consumers: [Option<RingConsumer>; 2] = [None, None];
    let mut feeds: Vec<(RingProducer, Audio)> = Vec::new();
    for (&t, a) in tracks.iter().zip(audio) {
        let (tx, rx) = ring(4 * a.sample_rate as usize);
        consumers[t.index()] = Some(rx);
        feeds.push((tx, a));
    }
    let (events_tx, events) = mpsc::channel();
    for &track in tracks {
        let _ = events_tx.send(CaptureEvent::TrackStarted {
            track,
            device: "replay".into(),
        });
    }
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("replay".into())
            .spawn(move || replay(feeds, &stop))
            .map_err(|e| internal("replay thread", e))?
    };
    let [mic, system] = consumers;
    Ok(Source {
        mic,
        system,
        events,
        handle: Handle::Replay {
            stop,
            thread: Some(thread),
        },
        route: Route::Headphones,
        kind: "replay",
    })
}

fn replay(mut feeds: Vec<(RingProducer, Audio)>, stop: &AtomicBool) {
    let base = Instant::now();
    let start_ns = monotonic_ns(base);
    let mut block = 0u64;
    loop {
        let mut more = false;
        for (tx, a) in &mut feeds {
            let n = (a.sample_rate / 100) as usize;
            let from = block as usize * n;
            if from >= a.samples.len() {
                continue;
            }
            more = true;
            let to = (from + n).min(a.samples.len());
            let at = start_ns + from as u64 * 1_000_000_000 / u64::from(a.sample_rate);
            tx.push(&a.samples[from..to], f64::from(a.sample_rate), at);
        }
        if !more || stop.load(Ordering::Relaxed) {
            return;
        }
        block += 1;
        let due = base + Duration::from_millis(10 * block);
        std::thread::sleep(due.saturating_duration_since(Instant::now()));
    }
}

#[cfg(target_os = "macos")]
fn capture_source(mode: Mode, pids: &[i32]) -> Result<Source, ErrorDoc> {
    use ghi_audio::macos::{self, CaptureConfig, MacCapture, MacError, MicPermission};
    let unavailable = |m: String| ErrorDoc::new(ErrorCode::CaptureUnavailable, m);
    // The prompt blocks until answered; keep it off the main thread.
    let perm = std::thread::spawn(|| match macos::mic_permission() {
        MicPermission::Undetermined => macos::request_mic_permission(),
        p => p,
    })
    .join()
    .map_err(|_| internal("mic permission", "thread panicked"))?;
    if perm != MicPermission::Authorized {
        return Err(unavailable(format!(
            "microphone access is {perm:?}: allow it in System Settings > Privacy & Security > Microphone"
        )));
    }
    let (route, _) = macos::route().unwrap_or((Route::Unknown, false));
    let cfg = CaptureConfig {
        mic: true,
        system: mode == Mode::Call,
        tap_pids: pids.to_vec(),
    };
    let started = MacCapture::start(&cfg).map_err(|e| match e.code {
        MacError::UNSUPPORTED | MacError::MIC_PERMISSION | MacError::SYSTEM_PERMISSION => {
            unavailable(e.to_string())
        }
        _ => internal("capture", e),
    })?;
    Ok(Source {
        mic: started.mic,
        system: started.system,
        events: started.events,
        handle: Handle::Mac(started.capture),
        route,
        kind: "capture",
    })
}

#[cfg(not(target_os = "macos"))]
fn capture_source(_: Mode, _: &[i32]) -> Result<Source, ErrorDoc> {
    Err(ErrorDoc::new(
        ErrorCode::CaptureUnavailable,
        "record: live capture is macOS-only for now (Windows in phase 13); use --replay",
    ))
}

fn default_id() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("rec-{secs}")
}

/// clap parser for `--id`: it becomes a file name, so a bad one is a usage
/// error (exit 2).
pub fn parse_id(s: &str) -> Result<String, String> {
    if valid_id(s) {
        Ok(s.to_owned())
    } else {
        Err("use letters, digits, '-' and '_'".into())
    }
}

/// clap parser for `--duration`: seconds, more than zero.
pub fn parse_duration(s: &str) -> Result<f64, String> {
    match s.parse::<f64>() {
        Ok(d) if d > 0.0 && d.is_finite() => Ok(d),
        _ => Err("expected a number of seconds greater than 0".into()),
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Free bytes on the volume holding `dir`.
fn free_bytes(dir: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
        // SAFETY: zeroed statvfs is a valid out-parameter; path is NUL-terminated.
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(path.as_ptr(), &mut st) } != 0 {
            return None;
        }
        #[allow(clippy::unnecessary_cast, clippy::useless_conversion)]
        Some(u64::from(st.f_bavail) * st.f_frsize as u64)
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        None
    }
}

fn describe_event(ev: &CaptureEvent) -> (&'static str, Option<String>) {
    match ev {
        CaptureEvent::TrackStarted { track, device } => (
            "track_started",
            Some(format!("{} on {device}", track.name())),
        ),
        CaptureEvent::RouteChanged {
            route,
            input_bluetooth_hfp,
        } => (
            "route_changed",
            Some(format!("{route:?} hfp={input_bluetooth_hfp}")),
        ),
        CaptureEvent::MicDeviceChanged { device } => ("mic_device_changed", Some(device.clone())),
        CaptureEvent::SystemRestarted => ("system_restarted", None),
        CaptureEvent::TrackLost { track } => ("track_lost", Some(track.name().into())),
        CaptureEvent::Sleep => ("sleep", None),
        CaptureEvent::Wake => ("wake", None),
        CaptureEvent::SilentSystemTrack { silent_s } => (
            "silent_system_track",
            Some(format!(
                "no call audio for {silent_s:.0} s: check System Audio Recording access \
                 (System Settings > Privacy & Security)"
            )),
        ),
        CaptureEvent::DiskLow { free_bytes } => {
            ("disk_low", Some(format!("{free_bytes} bytes free")))
        }
        CaptureEvent::DiskFull => ("disk_full", None),
        CaptureEvent::Overrun { track, dropped } => (
            "overrun",
            Some(format!("{} dropped {dropped} samples", track.name())),
        ),
        CaptureEvent::AsrSkipped { frames } => ("asr_skipped", Some(format!("{frames} frames"))),
        CaptureEvent::Error { code, detail } => ("error", Some(format!("{code}: {detail}"))),
    }
}

fn route_name(route: Route) -> &'static str {
    match route {
        Route::Unknown => "unknown",
        Route::Speakers => "speakers",
        Route::Headphones => "headphones",
        Route::Bluetooth => "bluetooth",
        Route::External => "external",
    }
}

/// What the recording loop reports back.
struct Outcome {
    stopped: &'static str,
    events: Vec<RecordEvent>,
    duration_s: f64,
    route: Route,
    aec: bool,
    erle_db: Option<f64>,
    overruns: [u64; 2],
}

/// Runs the pipeline until the duration, a signal, the end of a replay, a
/// full disk or a writer failure.
fn run_loop<W: FrameSink>(
    pipeline: &mut Pipeline<W>,
    asr: &mut ghi_audio::pipeline::AsrConsumer,
    source: &mut Source,
    out: &Path,
    duration_s: Option<f64>,
) -> Outcome {
    let limit = duration_s.map(|d| (d * f64::from(SAMPLE_RATE)) as u64);
    let mut events = Vec::new();
    // Wall clock: Instant and the capture clock both stop during sleep.
    let mut slept_at: Option<SystemTime> = None;
    let mut last_audio = Instant::now();
    let mut disk_checked = Instant::now() - Duration::from_secs(10);
    let mut disk_warned = false;
    let mut route = source.route;
    let mut overruns = [0u64; 2];
    let mut note = |pipeline: &Pipeline<W>, ev: &CaptureEvent, events: &mut Vec<RecordEvent>| {
        let (kind, detail) = describe_event(ev);
        let t_s = seconds(pipeline.position());
        if !matches!(ev, CaptureEvent::TrackStarted { .. }) {
            crate::warn(&format!(
                "record: {kind}{}",
                detail
                    .as_deref()
                    .map(|d| format!(": {d}"))
                    .unwrap_or_default()
            ));
        }
        if let CaptureEvent::Overrun { track, dropped } = ev {
            overruns[track.index()] += dropped;
        }
        events.push(RecordEvent {
            t_s,
            kind: kind.into(),
            detail,
        });
    };
    let stopped = loop {
        while let Ok(ev) = source.events.try_recv() {
            match ev {
                CaptureEvent::Sleep => slept_at = Some(SystemTime::now()),
                CaptureEvent::Wake => {
                    let slept = slept_at.take().and_then(|t| t.elapsed().ok());
                    pipeline.wake(slept);
                    continue;
                }
                CaptureEvent::RouteChanged { route: r, .. } => route = r,
                _ => {}
            }
            pipeline.handle_event(ev);
        }
        let ended = source.handle.ended();
        let report = pipeline.step();
        if report.frames > 0 || slept_at.is_some() || pipeline.is_paused() {
            last_audio = Instant::now();
        }
        for ev in &report.events {
            note(pipeline, ev, &mut events);
        }
        while asr.pop().is_some() {}
        if pipeline.writer_failed() {
            break "writer_error";
        }
        // Losing the system track leaves a mic-only recording; losing the mic ends it.
        if report
            .events
            .iter()
            .any(|e| matches!(e, CaptureEvent::TrackLost { track: Track::Mic }))
        {
            break "track_lost";
        }
        if last_audio.elapsed() >= NO_AUDIO {
            let ev = CaptureEvent::Error {
                code: 0,
                detail: format!(
                    "no audio from the capture device for {} s",
                    NO_AUDIO.as_secs()
                ),
            };
            note(pipeline, &ev, &mut events);
            break "no_audio";
        }
        if limit.is_some_and(|l| pipeline.position() >= l) {
            break "duration";
        }
        if STOP.load(Ordering::Relaxed) {
            break "signal";
        }
        if ended && report.frames == 0 {
            break "source_ended";
        }
        if disk_checked.elapsed() >= Duration::from_secs(1) {
            disk_checked = Instant::now();
            match free_bytes(out) {
                Some(free) if free < DISK_STOP => {
                    note(pipeline, &CaptureEvent::DiskFull, &mut events);
                    break "disk_full";
                }
                Some(free) if free < DISK_LOW && !disk_warned => {
                    disk_warned = true;
                    note(
                        pipeline,
                        &CaptureEvent::DiskLow { free_bytes: free },
                        &mut events,
                    );
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Outcome {
        stopped,
        events,
        duration_s: seconds(pipeline.position()),
        route,
        aec: pipeline.aec_active(),
        erle_db: pipeline.erle_db(),
        overruns,
    }
}

pub fn run(args: &Args) -> Result<(), ErrorDoc> {
    let started = Instant::now();
    let id = args.id.map(str::to_owned).unwrap_or_else(default_id);
    if !valid_id(&id) {
        return Err(ErrorDoc::new(
            ErrorCode::BadInput,
            format!("--id {id:?}: use letters, digits, '-' and '_'"),
        ));
    }
    std::fs::create_dir_all(args.out).map_err(|e| {
        ErrorDoc::new(
            ErrorCode::BadInput,
            format!("--out {}: {e}", args.out.display()),
        )
    })?;
    if let Some(parent) = args.out.parent().filter(|p| !p.as_os_str().is_empty()) {
        sync_dir(parent).map_err(|e| internal("sync directory", e))?;
    }
    let tracks = tracks_of(args.mode);
    // The store is opened first: a key or database problem fails before capture.
    let store = match args.format {
        Format::Store => Some(crate::keystore::open_store(args.out)?),
        _ => None,
    };
    let session = args.out.join(format!("{id}.session.json"));
    if store.is_none() && session.exists() {
        return Err(ErrorDoc::new(
            ErrorCode::BadInput,
            format!(
                "recording {id:?} already exists in {}: pick another --id",
                args.out.display()
            ),
        ));
    }

    let mut source = if args.replay.is_empty() {
        capture_source(args.mode, args.pids)?
    } else {
        replay_source(args.mode, args.replay)?
    };
    // After the source is up (no orphan file on a permission error), before
    // any audio is written. The rings buffer the first blocks meanwhile.
    // Store recordings are a meeting row instead; their id is the meeting gid.
    let meeting = match &store {
        Some(store) => {
            let new = ghi_store::store::NewMeeting {
                title: args.title.unwrap_or("Recording").to_owned(),
                source: "live".into(),
                mode: format!("{:?}", args.mode).to_lowercase(),
                ..Default::default()
            };
            match store.create_meeting(new) {
                Ok(m) => Some(m.gid),
                Err(e) => {
                    source.handle.stop();
                    return Err(crate::keystore::store_error(e));
                }
            }
        }
        None => {
            if let Err(e) = write_session(args, &id, tracks) {
                source.handle.stop();
                return Err(internal("write session file", e));
            }
            None
        }
    };
    let id = meeting.clone().unwrap_or(id);
    install_signal_handlers();
    let cfg = PipelineConfig {
        route: source.route,
        aec_enabled: args.aec,
        silent_system_secs: args.silent_s,
        // A call recording is a call: silence on the system track is suspect.
        call_detected: args.mode == Mode::Call,
        ..PipelineConfig::default()
    };
    let (mic, system) = (source.mic.take(), source.system.take());
    let ext = match args.format {
        Format::Wav => "wav",
        Format::Opus => "opus",
        Format::Store => "ghb",
    };
    let (outcome, levels, markers) = match args.format {
        Format::Wav => {
            let sink = WavSink::create(args.out, &id, tracks)
                .map_err(|e| internal("create WAV files", e))?;
            let (mut pipeline, mut asr) = Pipeline::new(cfg, mic, system, Tally::new(sink));
            let outcome = run_loop(
                &mut pipeline,
                &mut asr,
                &mut source,
                args.out,
                args.duration_s,
            );
            source.handle.stop();
            let tally = pipeline.finish();
            let markers = tally.inner.log.markers.clone();
            tally
                .inner
                .finish()
                .map_err(|e| internal("finish WAV files", e))?;
            (outcome, tally.levels, markers)
        }
        Format::Opus => {
            let pages = OggPages::create(args.out, &id, tracks)
                .map_err(|e| internal("create Opus files", e))?;
            let recorder = OpusRecorder::new(pages, tracks, EncoderConfig::default())
                .map_err(|e| internal("start Opus encoder", e))?;
            let (mut pipeline, mut asr) = Pipeline::new(cfg, mic, system, Tally::new(recorder));
            let outcome = run_loop(
                &mut pipeline,
                &mut asr,
                &mut source,
                args.out,
                args.duration_s,
            );
            source.handle.stop();
            let tally = pipeline.finish();
            let pages = tally
                .inner
                .finish()
                .map_err(|e| internal("finish Opus files", e))?;
            sync_dir(args.out).map_err(|e| internal("sync directory", e))?;
            (outcome, tally.levels, pages.log.markers)
        }
        Format::Store => {
            let (store, gid) = (store.as_ref().unwrap(), meeting.as_deref().unwrap());
            let mut pages = StorePages {
                writers: [None, None],
                markers: Vec::new(),
            };
            let mut opened = Ok(());
            for &t in tracks {
                match store.open_track(gid, track_kind(t)) {
                    Ok(w) => pages.writers[t.index()] = Some(w),
                    Err(e) => {
                        opened = Err(crate::keystore::store_error(e));
                        break;
                    }
                }
            }
            let recorder = match opened {
                Ok(()) => OpusRecorder::new(pages, tracks, EncoderConfig::default())
                    .map_err(|e| internal("start Opus encoder", e)),
                Err(e) => {
                    let _ = close_store_meeting(store, gid, tracks, &mut pages, 0.0);
                    Err(e)
                }
            };
            let recorder = match recorder {
                Ok(r) => r,
                Err(e) => {
                    source.handle.stop();
                    // The pages moved into the failed encoder were dropped; the
                    // store recovers unfinished tracks when it next opens.
                    let _ = store.finish_meeting(gid, 0);
                    return Err(e);
                }
            };
            let (mut pipeline, mut asr) = Pipeline::new(cfg, mic, system, Tally::new(recorder));
            let outcome = run_loop(
                &mut pipeline,
                &mut asr,
                &mut source,
                args.out,
                args.duration_s,
            );
            source.handle.stop();
            let tally = pipeline.finish();
            // Every stop path closes the tracks and the meeting; the first
            // error is reported after that.
            let finished = tally.inner.finish();
            let (markers, closed) = match finished {
                Ok(mut pages) => {
                    let closed =
                        close_store_meeting(store, gid, tracks, &mut pages, outcome.duration_s);
                    (pages.markers, closed)
                }
                Err(e) => {
                    let _ = store.finish_meeting(gid, (outcome.duration_s * 1000.0) as i64);
                    (Vec::new(), Err(internal("finish Opus pages", e)))
                }
            };
            closed?;
            (outcome, tally.levels, markers)
        }
    };
    if outcome.stopped == "writer_error" {
        return Err(internal("record", "writing the recording failed"));
    }
    let doc = Record {
        schema: RECORD.into(),
        id: id.clone(),
        mode: format!("{:?}", args.mode).to_lowercase(),
        format: ext.into(),
        source: source.kind.into(),
        duration_s: outcome.duration_s,
        tracks: tracks
            .iter()
            .map(|&t| {
                let l = levels[t.index()];
                RecordTrack {
                    track: t.name().into(),
                    file: match (&store, &meeting) {
                        (Some(s), Some(gid)) => match s.bundle_path(gid, track_kind(t)) {
                            Ok(p) => p.strip_prefix(args.out).unwrap_or(&p).display().to_string(),
                            Err(_) => String::new(),
                        },
                        _ => track_path(Path::new(""), &id, t, ext).display().to_string(),
                    },
                    duration_s: seconds(l.samples),
                    rms_dbfs: l.rms_dbfs(),
                    peak: f64::from(l.peak),
                    overrun_samples: outcome.overruns[t.index()],
                }
            })
            .collect(),
        mix: (args.format == Format::Wav && args.mode == Mode::Call).then(|| format!("{id}.wav")),
        route: route_name(outcome.route).into(),
        aec: outcome.aec,
        erle_db: outcome.erle_db,
        markers,
        events: outcome.events,
        stopped: outcome.stopped.into(),
        perf: Perf {
            wall_s: started.elapsed().as_secs_f64(),
            rtf: None,
            peak_rss_mb: crate::engine::peak_rss_mb(),
        },
    };
    crate::emit(&doc)
}

/// Finishes every open track writer, then the meeting, even after an error;
/// returns the first error.
fn close_store_meeting(
    store: &ghi_store::store::Store,
    gid: &str,
    tracks: &[Track],
    pages: &mut StorePages,
    duration_s: f64,
) -> Result<(), ErrorDoc> {
    let mut first = Ok(());
    for &t in tracks {
        if let Some(w) = pages.writers[t.index()].take()
            && let Err(e) = store.finish_track(gid, track_kind(t), w)
            && first.is_ok()
        {
            first = Err(crate::keystore::store_error(e));
        }
    }
    if let Err(e) = store.finish_meeting(gid, (duration_s * 1000.0) as i64)
        && first.is_ok()
    {
        first = Err(crate::keystore::store_error(e));
    }
    first
}

fn track_kind(t: Track) -> ghi_store::store::TrackKind {
    match t {
        Track::Mic => ghi_store::store::TrackKind::Mic,
        Track::System => ghi_store::store::TrackKind::System,
    }
}

/// Writes `<id>.session.json` before any audio, so a recording that crashes
/// can still be placed in time.
fn write_session(args: &Args, id: &str, tracks: &[Track]) -> std::io::Result<()> {
    let started_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let doc = serde_json::json!({
        "schema": "ghi.record-session/1",
        "id": id,
        "mode": format!("{:?}", args.mode).to_lowercase(),
        "format": format!("{:?}", args.format).to_lowercase(),
        "sample_rate": SAMPLE_RATE,
        "tracks": tracks.iter().map(|t| t.name()).collect::<Vec<_>>(),
        "started_unix_ms": started_unix_ms,
    });
    let path = args.out.join(format!("{id}.session.json"));
    let file = crate::sink::create_new(&path)?;
    serde_json::to_writer_pretty(&file, &doc).map_err(std::io::Error::other)?;
    crate::sink::sync_file(&file, true)?;
    sync_dir(args.out)
}

/// `ghi recover <dir>`: decodes every `*.opus` track in `dir` to WAV next to
/// it, tolerating a torn last page, and reports what was recovered.
pub fn recover(dir: &Path) -> Result<(), ErrorDoc> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| ErrorDoc::new(ErrorCode::BadInput, format!("{}: {e}", dir.display())))?;
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "opus"))
        .collect();
    paths.sort();
    let mut files = Vec::new();
    for path in paths {
        let wav = path.with_extension("recovered.wav");
        let decoded = std::fs::File::open(&path)
            .and_then(|f| read_ogg_opus_detailed(std::io::BufReader::new(f)))
            .and_then(|d| write_wav(&wav, &d.samples).map(|()| d));
        // One unreadable file must not cost the others.
        files.push(match decoded {
            Ok(d) => RecoveredFile {
                file: file_name(&path),
                wav: file_name(&wav),
                duration_s: seconds(d.samples.len() as u64),
                complete: d.complete,
                bad_pages: d.bad_pages as u64,
                truncated: d.truncated,
                error: None,
            },
            Err(e) => RecoveredFile {
                file: file_name(&path),
                wav: String::new(),
                duration_s: 0.0,
                complete: false,
                bad_pages: 0,
                truncated: false,
                error: Some(e.to_string()),
            },
        });
    }
    crate::emit(&Recover {
        schema: RECOVER.into(),
        files,
    })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert!(valid_id("rec-1_a"));
        assert!(!valid_id(""));
        assert!(!valid_id("../x"));
        assert!(!valid_id("a.b"));
        assert!(default_id().starts_with("rec-"));
    }

    #[test]
    fn replay_needs_one_file_per_track() {
        let err = replay_source(Mode::Call, &[PathBuf::from("a.wav")])
            .err()
            .unwrap();
        assert_eq!(err.code, ErrorCode::BadInput);
    }

    #[test]
    fn free_space_is_known_here() {
        assert!(free_bytes(&std::env::temp_dir()).unwrap_or(1) > 0);
    }
}
