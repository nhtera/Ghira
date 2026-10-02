// SPDX-License-Identifier: Apache-2.0
//! `ghi voice ...`: voice profiles from the command line (phase 14c). Dev and
//! test tooling: `score` compares two recordings with the speaker model (for
//! tuning the match thresholds), `enroll` and `delete` manage Me's profile in
//! a store. Audio must be 16 kHz mono-mixable WAV (`ghi decode --wav` makes
//! one).

use std::path::{Path, PathBuf};

use ghi_core::profiles::{
    ClusterVoice, Span, VOICE_MODEL, VoiceEmbed, cosine, embed_windows, open_tract, pick_windows,
};
use ghi_core::voice_job::EnrollError;
use serde_json::json;

use crate::audio::{Audio, read_wav};
use crate::contract::{ErrorCode, ErrorDoc};
use crate::keystore::{open_store, store_error};

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Action {
    /// Make (or replace) Me's voice profile from a spoken passage
    /// (`ghi.voice-enroll/1`). --consent says the speaker agreed.
    Enroll {
        /// The data directory.
        #[arg(long)]
        dir: PathBuf,
        /// 16 kHz WAV of the speaker alone, ideally 10-25 s.
        #[arg(long)]
        wav: PathBuf,
        /// The speaker agreed to a voice profile being kept (required).
        #[arg(long)]
        consent: bool,
        #[command(flatten)]
        model: ModelArg,
    },
    /// Delete Me's voice profile; the key is destroyed first (`ghi.voice-delete/1`).
    Delete {
        #[arg(long)]
        dir: PathBuf,
    },
    /// Cosine similarity of two recordings' voices (`ghi.voice-score/1`).
    Score {
        a: PathBuf,
        b: PathBuf,
        #[command(flatten)]
        model: ModelArg,
    },
}

#[derive(Debug, Clone, clap::Args)]
pub struct ModelArg {
    /// The speaker model file (default: the registry's, in the models directory).
    #[arg(long)]
    pub model: Option<PathBuf>,
}

fn bad(msg: impl Into<String>) -> ErrorDoc {
    ErrorDoc::new(ErrorCode::BadInput, msg)
}

fn open_model(arg: &ModelArg) -> Result<Box<dyn VoiceEmbed>, ErrorDoc> {
    let path = match &arg.model {
        Some(p) => p.clone(),
        None => {
            let m = ghi_models::find(VOICE_MODEL).ok_or_else(|| {
                ErrorDoc::new(ErrorCode::Internal, "no speaker model in the registry")
            })?;
            ghi_models::path_in(&ghi_models::models_dir(), &m)
        }
    };
    open_tract(&path).map_err(|e| {
        ErrorDoc::new(
            ErrorCode::EngineUnavailable,
            format!("speaker model {}: {e}", path.display()),
        )
    })
}

fn wav16(path: &Path) -> Result<Vec<f32>, ErrorDoc> {
    let Audio {
        samples,
        sample_rate,
    } = read_wav(path)?;
    if sample_rate != 16_000 {
        return Err(bad(format!(
            "{}: {sample_rate} Hz, need 16 kHz (try `ghi decode --wav`)",
            path.display()
        )));
    }
    Ok(samples)
}

/// A recording's voice: all of it, as one speaker.
fn voice_of(embedder: &mut dyn VoiceEmbed, pcm: &[f32]) -> Result<ClusterVoice, ErrorDoc> {
    let whole = [Span {
        who: 0,
        t0_ms: 0,
        t1_ms: (pcm.len() * 1000 / 16_000) as i64,
    }];
    let windows = pick_windows(&whole, 0, pcm.len());
    embed_windows(embedder, pcm, &windows, &|| false)
        .map_err(|e| ErrorDoc::new(ErrorCode::Internal, e))?
        .ok_or_else(|| bad("too little speech: need at least 2 s of voice"))
}

pub fn run(action: &Action) -> Result<(), ErrorDoc> {
    match action {
        Action::Score { a, b, model } => {
            let mut embedder = open_model(model)?;
            let va = voice_of(embedder.as_mut(), &wav16(a)?)?;
            let vb = voice_of(embedder.as_mut(), &wav16(b)?)?;
            crate::emit(&json!({
                "schema": "ghi.voice-score/1",
                "cosine": cosine(&va.vec, &vb.vec),
                "windows_a": va.windows,
                "windows_b": vb.windows,
            }))
        }
        Action::Enroll {
            dir,
            wav,
            consent,
            model,
        } => {
            if !*consent {
                return Err(bad(
                    "--consent is required: it records that the speaker agreed to a voice profile",
                ));
            }
            let pcm = wav16(wav)?;
            let store = open_store(dir)?;
            let mut embedder = open_model(model)?;
            let key = if cfg!(target_os = "windows") {
                "onboarding.voice.consent_win"
            } else {
                "onboarding.voice.consent_mac"
            };
            let gid = ghi_core::voice_job::enroll_from_pcm(
                &store,
                embedder.as_mut(),
                &pcm,
                &ghi_core::voice_job::self_consent(key),
            )
            .map_err(|e| match e {
                EnrollError::NoConsent | EnrollError::TooLittleSpeech => bad(e.to_string()),
                EnrollError::Model(_) => ErrorDoc::new(ErrorCode::EngineUnavailable, e.to_string()),
                EnrollError::Store(e) => store_error(e),
            })?;
            crate::emit(&json!({"schema": "ghi.voice-enroll/1", "profile": gid}))
        }
        Action::Delete { dir } => {
            let store = open_store(dir)?;
            let me = store.me_person().map_err(store_error)?;
            let deleted = match store.voice_profile(&me).map_err(store_error)? {
                Some(p) => {
                    store.delete_voice_profile(&p.gid).map_err(store_error)?;
                    true
                }
                None => false,
            };
            crate::emit(&json!({"schema": "ghi.voice-delete/1", "deleted": deleted}))
        }
    }
}
