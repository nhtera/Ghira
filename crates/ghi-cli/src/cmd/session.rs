// SPDX-License-Identifier: Apache-2.0
//! `ghi session` and `ghi jobs`: whole meetings headless through `ghi-core`
//! (phase 8), for soak runs, the eval kit and integration tests.
//!
//! `ghi session --dir D (--replay mic.wav [system.wav] | live capture)` records
//! a meeting into the store at D, printing every core event as a
//! `ghi.session-event/1` line and a `ghi.session/1` summary last. With
//! `--process`, the notes and final-pass jobs run after stop. `ghi jobs --dir D`
//! runs whatever is queued (after crash recovery).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use ghi_audio::Track;
use ghi_core::capture::{self, ReplayTrack};
use ghi_core::engines::SpeechEngines;
use ghi_core::events::{EventTx, bus};
use ghi_core::final_pass::FinalPassJob;
use ghi_core::jobs::{JobHandler, JobRunner, Outcome, always_ready};
use ghi_core::live::Mode as LiveMode;
use ghi_core::notes_job::{LlmFactory, NOTES_FINAL_JOB, NotesJob};
use ghi_core::session::{NOTES_LIVE_JOB, Session, SessionConfig};
use ghi_store::store::Store;
use serde_json::json;

use crate::audio::read_wav;
use crate::cmd::notes::ModelArgs;
use crate::cmd::record::{Mode, STOP, install_signal_handlers};
use crate::contract::{ErrorCode, ErrorDoc, LangMode};
use crate::engine::EngineArgs;

#[derive(Debug, Clone, clap::Args)]
pub struct SessionArgs {
    /// The data directory (the encrypted store; created on first use).
    #[arg(long)]
    pub dir: PathBuf,
    #[arg(long, value_enum, default_value_t = Mode::Room)]
    pub mode: Mode,
    /// Replay WAV files instead of capturing: the mic, then (call) the system track.
    #[arg(long, num_args = 1..=2)]
    pub replay: Vec<PathBuf>,
    #[arg(long, value_enum, default_value_t = LangMode::Auto)]
    pub lang: LangMode,
    /// Replay speed: 1 = real time (live-lag figures), 0 = as fast as possible.
    #[arg(long, default_value_t = 1.0)]
    pub speed: f64,
    /// Live capture: stop after this many seconds (default: Ctrl-C).
    #[arg(long)]
    pub duration: Option<f64>,
    /// Discard the last N seconds just before stopping (RT-1 checks).
    #[arg(long)]
    pub discard_last: Option<f64>,
    /// Run the notes and final-pass jobs right after stop.
    #[arg(long)]
    pub process: bool,
    #[arg(long, default_value = "Meeting")]
    pub title: String,
    /// Live ASR chunk in ms (560 Balanced/Max, 1120 Light).
    #[arg(long, default_value_t = 560)]
    pub live_chunk_ms: u32,
    /// Record without speech engines, as the app does while the models are
    /// missing: no live transcript; `ghi jobs` (or `--process`) makes it in a
    /// build with speech engines.
    #[arg(long)]
    pub record_only: bool,
    #[command(flatten)]
    pub engine: EngineArgs,
    #[command(flatten)]
    pub model: ModelArgs,
}

#[derive(Debug, Clone, clap::Args)]
pub struct JobsArgs {
    #[arg(long)]
    pub dir: PathBuf,
    #[command(flatten)]
    pub engine: EngineArgs,
    #[command(flatten)]
    pub model: ModelArgs,
}

fn internal(what: &str, e: impl std::fmt::Display) -> ErrorDoc {
    ErrorDoc::new(ErrorCode::Internal, format!("{what}: {e}"))
}

fn language(lang: LangMode) -> Option<String> {
    crate::cmd::transcribe::language_code(lang).map(str::to_string)
}

/// Speech engines with a given ASR chunk (live 560/1120, final 1120).
#[cfg(feature = "nemo")]
pub fn engines(args: &EngineArgs, chunk_ms: u32) -> Result<Arc<dyn SpeechEngines>, ErrorDoc> {
    use ghi_core::engines::NemoEngines;
    use ghi_speech::nemo::Device;
    let (asr, _) = crate::engine::model_path(crate::engine::ASR_MODEL, args.asr_model.as_ref())?;
    let (diar, _) = crate::engine::model_path(crate::engine::DIAR_MODEL, args.diar_model.as_ref())?;
    // Pinned registry models are hash-checked before native code parses them.
    for (id, path, explicit) in [
        (crate::engine::ASR_MODEL, &asr, args.asr_model.is_some()),
        (crate::engine::DIAR_MODEL, &diar, args.diar_model.is_some()),
    ] {
        if let (false, Some(m)) = (explicit, ghi_models::find(id)) {
            ghi_models::verify_for_load(path, &m).map_err(|e| {
                ErrorDoc::new(ErrorCode::EngineUnavailable, format!("model {id}: {e}"))
            })?;
        }
    }
    let device = if args.cpu || std::env::var("GHI_DEVICE").as_deref() == Ok("cpu") {
        Device::Cpu
    } else {
        Device::Gpu
    };
    Ok(Arc::new(
        NemoEngines::load(&asr, &diar, chunk_ms, device).map_err(crate::engine::speech_error)?,
    ))
}

#[cfg(not(feature = "nemo"))]
pub fn engines(_args: &EngineArgs, _chunk_ms: u32) -> Result<Arc<dyn SpeechEngines>, ErrorDoc> {
    Err(crate::engine::unavailable("session"))
}

/// The local model, with a context sized to the transcript (phase 6 note:
/// a fixed 32k context costs memory a short meeting doesn't need).
fn llm_factory(model: &ModelArgs) -> LlmFactory {
    let model = model.clone();
    Arc::new(move |bytes: usize| {
        let wanted = (bytes / 3) as u32 * 6 / 5 + 6_144;
        let n_ctx = wanted.clamp(8_192, model.n_ctx);
        ghi_llm::local::LocalLlm::open_registry(&model.model, n_ctx)
            .map(|l| Box::new(l) as Box<dyn ghi_llm::Llm + Send>)
            .map_err(|e| e.to_string())
    })
}

/// The job runner with every handler, in priority order.
pub fn runner(
    store: Arc<Store>,
    events: EventTx,
    engine: &EngineArgs,
    model: &ModelArgs,
) -> Arc<JobRunner> {
    let template = ghi_llm::template::builtin("general").expect("built-in template");
    let llm = llm_factory(model);
    let engine = engine.clone();
    let handlers: Vec<Arc<dyn JobHandler>> = vec![
        Arc::new(NotesJob {
            kind: NOTES_LIVE_JOB,
            version: 1,
            template: template.clone(),
            llm: llm.clone(),
            ready: always_ready(),
        }),
        Arc::new(FinalPassJob {
            engines: Arc::new(move || engines(&engine, 1120).map_err(|e| e.message)),
            chunk_s: 600.0,
            ready: always_ready(),
            voice: None,
        }),
        Arc::new(NotesJob {
            kind: NOTES_FINAL_JOB,
            version: 2,
            template,
            llm,
            ready: always_ready(),
        }),
    ];
    JobRunner::new(store, events, handlers)
}

/// Prints every event as a `ghi.session-event/1` line until the bus closes.
fn print_events(rx: ghi_core::events::EventRx) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        for env in rx {
            let mut v = serde_json::to_value(&env).unwrap_or_default();
            v["schema"] = json!("ghi.session-event/1");
            println!("{v}");
        }
    })
}

fn run_jobs(runner: &JobRunner) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let mut t = Instant::now();
    while let Some((job, r)) = runner.run_one() {
        out.push(json!({
            "id": job.id,
            "kind": job.kind,
            "meeting": job.meeting_gid,
            "result": match &r {
                Ok(Outcome::Done) => json!("done"),
                Ok(Outcome::Yield(_)) => json!("yielded"),
                Err(e) => json!({"error": e}),
            },
            "wall_s": t.elapsed().as_secs_f64(),
        }));
        t = Instant::now();
        if matches!(r, Ok(Outcome::Yield(_))) {
            break;
        }
    }
    out
}

pub fn run(args: &SessionArgs) -> Result<(), ErrorDoc> {
    let started = Instant::now();
    let store = Arc::new(crate::keystore::open_store(&args.dir)?);
    ghi_core::recover::recover(&store).map_err(|e| internal("recovery", e))?;
    let mode = match args.mode {
        Mode::Call => LiveMode::Call,
        Mode::Room => LiveMode::Room,
    };
    let capture = if args.replay.is_empty() {
        install_signal_handlers();
        capture::live(mode == LiveMode::Call, &[])
            .map_err(|e| ErrorDoc::new(ErrorCode::CaptureUnavailable, e.to_string()))?
    } else {
        let tracks: &[Track] = match (mode, args.replay.len()) {
            (LiveMode::Call, 2) => &[Track::Mic, Track::System],
            (LiveMode::Call, _) => {
                return Err(ErrorDoc::new(
                    ErrorCode::BadInput,
                    "a call replay needs two files: mic, system",
                ));
            }
            (LiveMode::Room, 1) => &[Track::Mic],
            (LiveMode::Room, _) => {
                return Err(ErrorDoc::new(
                    ErrorCode::BadInput,
                    "a room replay is one file",
                ));
            }
        };
        let replay = tracks
            .iter()
            .zip(&args.replay)
            .map(|(&track, path)| {
                let a = read_wav(path)?;
                Ok(ReplayTrack {
                    track,
                    samples: a.samples,
                    sample_rate: a.sample_rate,
                })
            })
            .collect::<Result<Vec<_>, ErrorDoc>>()?;
        let speed = (args.speed > 0.0).then_some(args.speed);
        capture::replay(replay, speed).map_err(|e| internal("replay", e))?
    };
    let (tx, rx) = bus();
    let printer = print_events(rx);
    let runner = runner(store.clone(), tx.clone(), &args.engine, &args.model);
    // The engines load once the jobs paused; their error keeps its code.
    let mut engine_error = None;
    let session = Session::start_with_loader(
        store.clone(),
        || {
            if args.record_only {
                return Ok(None);
            }
            engines(&args.engine, args.live_chunk_ms)
                .map(Some)
                .map_err(|e| {
                    let msg = ghi_core::session::SessionError(e.message.clone());
                    engine_error = Some(e);
                    msg
                })
        },
        capture,
        SessionConfig {
            mode,
            language: language(args.lang),
            title: args.title.clone(),
            queue_jobs: true,
            lossless: !args.replay.is_empty() && args.speed <= 0.0,
        },
        tx.clone(),
        Some(runner.clone()),
    )
    .map_err(|e| {
        engine_error
            .take()
            .unwrap_or_else(|| internal("session", e))
    })?;
    let meeting = session.meeting().to_string();
    let deadline = args
        .duration
        .map(|d| Instant::now() + Duration::from_secs_f64(d));
    loop {
        if session.source_ended()
            || STOP.load(Ordering::Relaxed)
            || deadline.is_some_and(|d| Instant::now() >= d)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if let Some(s) = args.discard_last {
        session.discard(s).map_err(|e| internal("discard", e))?;
    }
    let report = session.stop().map_err(|e| internal("stop", e))?;
    let jobs = if args.process {
        run_jobs(&runner)
    } else {
        Vec::new()
    };
    drop((runner, tx));
    let _ = printer.join();

    let segs = store
        .segments(&meeting)
        .map_err(crate::keystore::store_error)?;
    let speakers = store
        .speakers(&meeting)
        .map_err(crate::keystore::store_error)?;
    let m = store
        .get_meeting(&meeting)
        .map_err(crate::keystore::store_error)?;
    crate::emit(&json!({
        "schema": "ghi.session/1",
        "meeting": meeting,
        "status": m.status,
        "transcript_version": m.transcript_version,
        "duration_s": report.duration_ms as f64 / 1000.0,
        "lines": segs.len(),
        "speakers": speakers.iter().map(|s| json!({
            "label": ghi_core::notes_job::speaker_label(s),
            "color_slot": s.color_slot,
            "is_me": s.is_me,
        })).collect::<Vec<_>>(),
        "jobs": jobs,
        "perf": {
            "wall_s": started.elapsed().as_secs_f64(),
            "peak_rss_mb": crate::engine::peak_rss_mb(),
        },
    }))
}

pub fn jobs(args: &JobsArgs) -> Result<(), ErrorDoc> {
    let store = Arc::new(crate::keystore::open_store(&args.dir)?);
    let recovered = ghi_core::recover::recover(&store).map_err(|e| internal("recovery", e))?;
    let (tx, rx) = bus();
    let printer = print_events(rx);
    let runner = runner(store, tx.clone(), &args.engine, &args.model);
    let jobs = run_jobs(&runner);
    drop((runner, tx));
    let _ = printer.join();
    crate::emit(&json!({
        "schema": "ghi.jobs/1",
        "recovered": {
            "discards": recovered.discards_completed,
            "meetings": recovered.meetings,
        },
        "jobs": jobs,
    }))
}

#[derive(Debug, Clone, clap::Args)]
pub struct ImportArgs {
    /// An audio or video file (WAV, MP3, M4A/AAC, FLAC, Ogg, Opus, MP4, ...).
    #[arg(required_unless_present = "tracks", conflicts_with = "tracks")]
    pub file: Option<PathBuf>,
    /// One recording from several participants' own tracks (Zoom "record a
    /// separate audio file for each participant"): the files, or a Zoom
    /// meeting folder (its `Audio Record` folder is used). Speakers are named
    /// from the file names and the diarizer is skipped.
    #[arg(long, num_args = 1.., value_name = "FILE_OR_DIR")]
    pub tracks: Vec<PathBuf>,
    #[arg(long)]
    pub dir: PathBuf,
    /// Keep the first two channels as separate tracks (a stereo call
    /// recording: you on the first channel, the others on the second).
    #[arg(long)]
    pub split_channels: bool,
    #[arg(long, value_enum, default_value_t = LangMode::Auto)]
    pub lang: LangMode,
    #[arg(long)]
    pub title: Option<String>,
    /// Run the final pass and notes right away.
    #[arg(long)]
    pub process: bool,
    #[command(flatten)]
    pub engine: EngineArgs,
    #[command(flatten)]
    pub model: ModelArgs,
}

/// The participant files named on the command line: files as given, a folder
/// as its audio files (a Zoom meeting folder: its `Audio Record` folder). The
/// mixed `audio_only` recording is never a participant, paths are listed once,
/// and each file carries the name its file name has.
fn track_files(paths: &[PathBuf]) -> Result<Vec<(PathBuf, Option<String>)>, ErrorDoc> {
    const AUDIO: [&str; 8] = ["m4a", "wav", "mp3", "aac", "flac", "ogg", "opus", "mp4"];
    let bad = |m: String| ErrorDoc::new(ErrorCode::BadInput, m);
    let mut files: Vec<PathBuf> = Vec::new();
    for p in paths {
        if p.is_dir() {
            let record = p.join("Audio Record");
            let dir = if record.is_dir() { record } else { p.clone() };
            let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
                .map_err(|e| bad(format!("{}: {e}", dir.display())))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|f| f.is_file())
                .filter(|f| {
                    f.extension()
                        .and_then(|x| x.to_str())
                        .is_some_and(|x| AUDIO.contains(&x.to_lowercase().as_str()))
                })
                .collect();
            found.sort();
            files.extend(found);
        } else {
            files.push(p.clone());
        }
    }
    files.retain(|f| {
        !f.file_name()
            .is_some_and(|n| n.to_string_lossy().to_lowercase().starts_with("audio_only"))
    });
    let mut seen = std::collections::HashSet::new();
    files.retain(|f| seen.insert(f.canonicalize().unwrap_or_else(|_| f.clone())));
    Ok(files
        .into_iter()
        .map(|f| {
            let name = f
                .file_name()
                .and_then(|n| ghi_core::presets::zoom_participant(&n.to_string_lossy()));
            (f, name)
        })
        .collect())
}

/// `ghi import`: a recording into the store as a meeting (`ghi.import/1`).
pub fn import(args: &ImportArgs) -> Result<(), ErrorDoc> {
    let started = Instant::now();
    let store = Arc::new(crate::keystore::open_store(&args.dir)?);
    ghi_core::recover::recover(&store).map_err(|e| internal("recovery", e))?;
    let (tx, rx) = bus();
    let printer = print_events(rx);
    let opts = ghi_core::import::ImportOptions {
        title: args.title.clone(),
        language: language(args.lang),
        split_channels: args.split_channels,
        ..Default::default()
    };
    let report = match (&args.file, args.tracks.is_empty()) {
        (Some(file), true) => ghi_core::import::import_file(&store, file, &opts, &tx),
        _ => ghi_core::import::import_tracks(&store, &track_files(&args.tracks)?, &opts, &tx),
    }
    .map_err(|e| ErrorDoc::new(ErrorCode::BadInput, e))?;
    let decode_s = started.elapsed().as_secs_f64();
    let jobs = if args.process && !report.duplicate {
        run_jobs(&runner(
            store.clone(),
            tx.clone(),
            &args.engine,
            &args.model,
        ))
    } else {
        Vec::new()
    };
    drop(tx);
    let _ = printer.join();
    let m = store
        .get_meeting(&report.meeting)
        .map_err(crate::keystore::store_error)?;
    crate::emit(&json!({
        "schema": "ghi.import/1",
        "meeting": report.meeting,
        "duplicate": report.duplicate,
        "status": m.status,
        "duration_s": report.duration_ms as f64 / 1000.0,
        "channels": report.channels,
        "tracks": report.tracks,
        "jobs": jobs,
        "perf": {
            "decode_s": decode_s,
            "wall_s": started.elapsed().as_secs_f64(),
        },
    }))
}
