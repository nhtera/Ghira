// SPDX-License-Identifier: Apache-2.0
//! `ghi`: headless CLI for the eval harness and tests. Output contract:
//! `tools/eval/docs/formats.md` §2.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use ghi_cli::cmd::{self, bench::Topology};
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
        #[arg(long, value_enum, default_value_t = LangMode::Auto)]
        lang: LangMode,
        /// Print JSON (currently the only output format).
        #[arg(long)]
        json: bool,
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
        Command::Notes { transcript, .. } => {
            ghi_cli::read_transcript(&transcript)?;
            Err(ghi_cli::not_implemented(
                "notes",
                "no LLM engine yet (phase 6)",
            ))
        }
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
