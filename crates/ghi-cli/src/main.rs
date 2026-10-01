// SPDX-License-Identifier: Apache-2.0
//! `ghi`: headless CLI for the eval harness and tests. Output contract:
//! `tools/eval/docs/formats.md` §2.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use ghi_cli::cmd::{
    self,
    bench::Topology,
    record::{Format, Mode},
};
use ghi_cli::contract::{ErrorDoc, LangMode, Pass};
use ghi_cli::engine::EngineArgs;

#[derive(Parser)]
#[command(
    name = "ghi",
    version,
    about = "Headless Ghira CLI for the eval harness and tests",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the versions of ghi, the core and the speech engines.
    Version {
        /// Print a `ghi.version/1` document instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Transcribe an audio file (`ghi.transcript/1`, or `ghi.event/1` lines with --stream).
    Transcribe {
        audio: PathBuf,
        #[arg(long, value_enum, default_value_t = LangMode::Auto)]
        lang: LangMode,
        #[arg(long, value_enum, default_value_t = Pass::Final)]
        pass: Pass,
        /// Print caption events as NDJSON while transcribing.
        #[arg(long)]
        stream: bool,
        /// Feed audio at 1x speed, as the live app does, so caption lag can be measured.
        #[arg(long, requires = "stream")]
        realtime: bool,
        /// Print JSON (currently the only output format).
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        engine: EngineArgs,
    },
    /// Find who spoke when (`ghi.diarization/1`).
    Diarize {
        audio: PathBuf,
        #[arg(long, value_enum, default_value_t = Pass::Final)]
        pass: Pass,
        #[arg(long, value_parser = clap::value_parser!(u8).range(1..))]
        max_speakers: Option<u8>,
        /// Print JSON (currently the only output format).
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        engine: EngineArgs,
    },
    /// Write meeting notes from a `ghi.transcript/1` file (`ghi.notes/1`).
    Notes {
        transcript: PathBuf,
        /// Language of the notes (auto = the meeting's language).
        #[arg(long, value_enum, default_value_t = LangMode::Auto)]
        lang: LangMode,
        #[command(flatten)]
        notes: cmd::notes::NotesArgs,
        #[command(flatten)]
        model: cmd::notes::ModelArgs,
        #[command(flatten)]
        cloud: cmd::cloud::CloudArgs,
        /// Print JSON (currently the only output format).
        #[arg(long)]
        json: bool,
    },
    /// Answer a question about a `ghi.transcript/1` file, with citations (`ghi.ask/1`).
    Ask {
        transcript: PathBuf,
        question: String,
        /// Language of the answer (auto = the meeting's language).
        #[arg(long, value_enum, default_value_t = LangMode::Auto)]
        lang: LangMode,
        #[command(flatten)]
        model: cmd::notes::ModelArgs,
        #[command(flatten)]
        cloud: cmd::cloud::CloudArgs,
        /// Print JSON (currently the only output format).
        #[arg(long)]
        json: bool,
    },
    /// Cloud AI provider API keys in the OS keystore (`ghi.keys/1`).
    Keys {
        #[command(subcommand)]
        action: KeysAction,
    },
    /// Run the pipeline on an audio file and report timings (`ghi.bench/1`).
    Bench {
        audio: PathBuf,
        #[arg(long, value_enum, default_value_t = Pass::Live)]
        pass: Pass,
        #[arg(long, value_enum, default_value_t = Topology::Single)]
        topology: Topology,
        /// Print JSON (currently the only output format).
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        engine: EngineArgs,
    },
    /// Record the mic (and system audio) to files (`ghi.record/1`). Capture
    /// tooling for tests and soak runs; stop with Ctrl-C or --duration.
    Record {
        #[arg(long, value_enum, default_value_t = Mode::Call)]
        mode: Mode,
        /// Output directory.
        #[arg(long)]
        out: PathBuf,
        /// File name stem (default `rec-<unix time>`).
        #[arg(long, value_parser = cmd::record::parse_id)]
        id: Option<String>,
        /// Meeting title for `--format store`.
        #[arg(long)]
        title: Option<String>,
        /// Stop after this many seconds of recording.
        #[arg(long, value_parser = cmd::record::parse_duration)]
        duration: Option<f64>,
        #[arg(long, value_enum, default_value_t = Format::Wav)]
        format: Format,
        /// Play WAV files through the pipeline at 1x instead of capturing:
        /// MIC (room) or MIC,SYSTEM (call).
        #[arg(long, value_delimiter = ',', num_args = 1..)]
        replay: Vec<PathBuf>,
        /// Capture system audio from these processes only (repeatable).
        #[arg(long = "pid")]
        pids: Vec<i32>,
        /// Turn echo cancellation off (it only runs on speakers anyway).
        #[arg(long)]
        no_aec: bool,
        /// Seconds of silent system audio before a warning.
        #[arg(long, default_value_t = 5.0)]
        silent_warn: f32,
        /// Print JSON (currently the only output format).
        #[arg(long)]
        json: bool,
    },
    /// Decode the Ogg Opus tracks in a recording directory, including a torn
    /// last page after a crash, to `*.recovered.wav` (`ghi.recover/1`).
    Recover { dir: PathBuf },
    /// Inspect and manage an encrypted Ghira store (a data directory).
    Store {
        /// The data directory (created on first use).
        #[arg(long)]
        dir: PathBuf,
        #[command(subcommand)]
        action: StoreAction,
    },
    /// Show processes using audio and what meeting auto-detect would do (`ghi.detect/1`).
    Detect {
        /// Poll every second for this many seconds; print one line per prompt.
        #[arg(long)]
        watch: Option<u64>,
    },
    /// Model manager: hardware tier, installed models, verify, fetch, import (`ghi.models/1`).
    Models {
        #[command(subcommand)]
        action: cmd::models::Action,
    },
    /// Probe or decode an audio file with the import decoders (`ghi.decode/1`).
    Decode(cmd::decode::Args),
    /// Record a meeting through the core pipeline into the store (replay or
    /// live), printing `ghi.session-event/1` lines and a `ghi.session/1` summary.
    Session(cmd::session::SessionArgs),
    /// Run the queued jobs (notes, final pass, import) after crash recovery (`ghi.jobs/1`).
    Jobs(cmd::session::JobsArgs),
    /// Import an audio/video file as a meeting (`ghi.import/1`); --process
    /// transcribes it and writes notes right away.
    Import(cmd::session::ImportArgs),
}

#[derive(Subcommand)]
enum KeysAction {
    /// Store a provider's API key, read from the first line of stdin.
    Set { provider: String },
    /// Remove a provider's API key.
    Delete { provider: String },
    /// Which providers have a key stored (never the key itself).
    Status,
}

// Parsed once per run; the size difference between variants doesn't matter.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
enum StoreAction {
    /// List meetings and their audio tracks (`ghi.store-list/1`).
    List,
    /// Accent-insensitive search over transcripts and notes (`ghi.store-search/1`).
    Search {
        query: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Decrypt one audio track of a meeting to a WAV file (`ghi.store-audio/1`).
    Audio {
        meeting: String,
        #[arg(long, value_enum, default_value_t = cmd::store::TrackArg::Mic)]
        track: cmd::store::TrackArg,
        #[arg(long)]
        out: PathBuf,
    },
    /// Store a `ghi.transcript/1` file as a meeting's transcript (`ghi.store-transcript/1`).
    AddTranscript {
        transcript: PathBuf,
        /// Existing meeting gid; default: a new meeting.
        #[arg(long)]
        meeting: Option<String>,
    },
    /// Write (or regenerate) a meeting's notes from its transcript; pinned,
    /// user-written, edited and done items are kept (`ghi.store-notes/1`).
    Notes {
        meeting: String,
        /// Language of the notes (auto = the meeting's language).
        #[arg(long, value_enum, default_value_t = LangMode::Auto)]
        lang: LangMode,
        #[command(flatten)]
        notes: cmd::notes::NotesArgs,
        #[command(flatten)]
        model: cmd::notes::ModelArgs,
        #[command(flatten)]
        cloud: cmd::cloud::CloudArgs,
    },
    /// Delete a meeting: its key is destroyed first, so nothing stays readable.
    Delete { meeting: String },
    /// Export everything into one archive encrypted with a password read from stdin.
    Export {
        #[arg(long)]
        out: PathBuf,
    },
    /// Restore an export into the (empty) --dir; password from stdin.
    Import { archive: PathBuf },
}

fn run(command: Command) -> Result<(), ErrorDoc> {
    match command {
        Command::Version { json: true } => ghi_cli::emit(&ghi_cli::version_doc()),
        Command::Version { json: false } => {
            use std::io::Write;
            writeln!(std::io::stdout().lock(), "ghi {}", ghi_cli::version())
                .map_err(|e| ErrorDoc::new(ghi_cli::contract::ErrorCode::Internal, e.to_string()))
        }
        Command::Transcribe {
            audio,
            lang,
            pass,
            stream,
            realtime,
            engine,
            ..
        } => cmd::transcribe::run(&cmd::transcribe::Args {
            audio: &audio,
            lang,
            pass,
            stream,
            realtime,
            engine: &engine,
        }),
        Command::Diarize {
            audio,
            pass,
            max_speakers,
            engine,
            ..
        } => cmd::diarize::run(&cmd::diarize::Args {
            audio: &audio,
            pass,
            max_speakers,
            engine: &engine,
        }),
        Command::Notes {
            transcript,
            lang,
            notes,
            model,
            cloud,
            ..
        } => cmd::notes::run(&transcript, lang, &notes, &model, &cloud),
        Command::Ask {
            transcript,
            question,
            lang,
            model,
            cloud,
            ..
        } => cmd::notes::ask(&transcript, &question, lang, &model, &cloud),
        Command::Keys { action } => match action {
            KeysAction::Set { provider } => cmd::cloud::keys_set(&provider),
            KeysAction::Delete { provider } => cmd::cloud::keys_delete(&provider),
            KeysAction::Status => cmd::cloud::keys_status(),
        },
        Command::Bench {
            audio,
            pass,
            topology,
            engine,
            ..
        } => cmd::bench::run(&cmd::bench::Args {
            audio: &audio,
            pass,
            topology,
            engine: &engine,
        }),
        Command::Record {
            mode,
            out,
            id,
            title,
            duration,
            format,
            replay,
            pids,
            no_aec,
            silent_warn,
            ..
        } => cmd::record::run(&cmd::record::Args {
            mode,
            out: &out,
            id: id.as_deref(),
            title: title.as_deref(),
            duration_s: duration,
            format,
            replay: &replay,
            pids: &pids,
            aec: !no_aec,
            silent_s: silent_warn,
        }),
        Command::Recover { dir } => cmd::record::recover(&dir),
        Command::Detect { watch } => cmd::detect::run(watch),
        Command::Models { action } => cmd::models::run(&action),
        Command::Decode(args) => cmd::decode::run(&args),
        Command::Session(args) => cmd::session::run(&args),
        Command::Jobs(args) => cmd::session::jobs(&args),
        Command::Import(args) => cmd::session::import(&args),
        Command::Store { dir, action } => match action {
            StoreAction::List => cmd::store::list(&dir),
            StoreAction::Search { query, limit } => cmd::store::search(&dir, &query, limit),
            StoreAction::Audio {
                meeting,
                track,
                out,
            } => cmd::store::audio(&dir, &meeting, track, &out),
            StoreAction::AddTranscript {
                transcript,
                meeting,
            } => cmd::store::add_transcript(&dir, &transcript, meeting.as_deref()),
            StoreAction::Notes {
                meeting,
                lang,
                notes,
                model,
                cloud,
            } => cmd::notes::store_notes(&dir, &meeting, lang, &notes, &model, &cloud),
            StoreAction::Delete { meeting } => cmd::store::delete(&dir, &meeting),
            StoreAction::Export { out } => cmd::store::export(&dir, &out),
            StoreAction::Import { archive } => cmd::store::import(&archive, &dir),
        },
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // The error document is always the last line of stderr.
            ghi_cli::warn(&serde_json::to_string(&err).expect("error document serializes"));
            ExitCode::from(err.code.exit_code())
        }
    }
}
