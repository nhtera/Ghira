// SPDX-License-Identifier: Apache-2.0
//! Loads the speech engines for `transcribe`, `diarize` and `bench`.
//!
//! Models come from the pinned registry (`ghi-models`) in `$GHI_MODELS_DIR`
//! (default `./models`), unless a path is given. Without the `nemo` feature
//! every engine command fails with `engine_unavailable` (exit 3).

use std::path::PathBuf;

use crate::contract::{Engine, ErrorCode, ErrorDoc, Pass};

/// Engine flags shared by the engine commands (not part of the harness contract).
#[derive(Debug, Clone, Default, clap::Args)]
pub struct EngineArgs {
    /// ASR model GGUF (default: the registry's nemotron-3.5-asr).
    #[arg(long)]
    pub asr_model: Option<PathBuf>,
    /// Diarization model GGUF (default: the registry's nemotron-3-diarization).
    #[arg(long)]
    pub diar_model: Option<PathBuf>,
    /// Run on the CPU instead of the GPU (Metal on macOS).
    #[arg(long)]
    pub cpu: bool,
    /// transcribe: decode the whole file in one offline call instead of
    /// streaming (spike S2 comparison; reports languages).
    #[arg(long)]
    pub offline: bool,
}

pub const ASR_MODEL: &str = "nemotron-3.5-asr";
pub const DIAR_MODEL: &str = "nemotron-3-diarization";

/// Streaming chunk per pass (doc 05 §1.1): 560 ms live, 1120 ms final.
pub fn asr_chunk_ms(pass: Pass) -> u32 {
    match pass {
        Pass::Live => 560,
        Pass::Final => 1120,
    }
}

/// Nemotron 3 Diarization presets: low-latency live, long-form final.
pub fn diar_preset(pass: Pass) -> &'static str {
    match pass {
        Pass::Live => "v3-streaming",
        Pass::Final => "v3-offline",
    }
}

/// Resolves a model file and the engine name/version reported in results.
pub fn model_path(id: &str, explicit: Option<&PathBuf>) -> Result<(PathBuf, Engine), ErrorDoc> {
    if let Some(path) = explicit {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        return Ok((
            path.clone(),
            Engine {
                name,
                version: "local".into(),
            },
        ));
    }
    let model = ghi_models::find(id).ok_or_else(|| {
        ErrorDoc::new(
            ErrorCode::Internal,
            format!("model {id} is not in the registry"),
        )
    })?;
    let path = ghi_models::path_in(&ghi_models::models_dir(), &model);
    if !path.is_file() {
        return Err(ErrorDoc::new(
            ErrorCode::EngineUnavailable,
            format!(
                "model {id} not found at {}: run tools/scripts/fetch-models.sh or set GHI_MODELS_DIR",
                path.display()
            ),
        ));
    }
    Ok((
        path,
        Engine {
            name: model.id,
            version: model.revision[..8].to_owned(),
        },
    ))
}

/// Engines listed by `ghi version --json`.
pub fn engines() -> Vec<Engine> {
    #[cfg(feature = "nemo")]
    {
        vec![Engine {
            name: "nemo-speech".into(),
            version: ghi_speech::nemo::engine_version(),
        }]
    }
    #[cfg(not(feature = "nemo"))]
    {
        Vec::new()
    }
}

#[cfg(not(feature = "nemo"))]
pub fn unavailable(command: &str) -> ErrorDoc {
    ErrorDoc::new(
        ErrorCode::EngineUnavailable,
        format!("{command}: ghi was built without speech engines (cargo feature `nemo`)"),
    )
}

pub fn speech_error(e: ghi_speech::SpeechError) -> ErrorDoc {
    ErrorDoc::new(ErrorCode::Internal, e.to_string())
}

/// Peak resident memory so far, in MB: the larger of this process and its
/// finished children (the LLM worker, once it has exited).
pub fn peak_rss_mb() -> Option<f64> {
    #[cfg(unix)]
    {
        let own = max_rss_mb(libc::RUSAGE_SELF)?;
        Some(max_rss_mb(libc::RUSAGE_CHILDREN).map_or(own, |c| c.max(own)))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(unix)]
fn max_rss_mb(who: libc::c_int) -> Option<f64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: getrusage fills the struct we pass.
    if unsafe { libc::getrusage(who, usage.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: initialized by the successful call above.
    let max = unsafe { usage.assume_init() }.ru_maxrss as f64;
    // Bytes on macOS, KiB on Linux.
    let bytes = if cfg!(target_os = "macos") {
        max
    } else {
        max * 1024.0
    };
    Some(bytes / 1_048_576.0)
}

#[cfg(feature = "nemo")]
pub use loaded::*;

#[cfg(feature = "nemo")]
mod loaded {
    use super::*;
    use ghi_speech::nemo::{Asr, AsrConfig, Device, DiarConfig, Diarizer};

    /// GPU (Metal) on macOS unless `--cpu` or `GHI_DEVICE=cpu` (CI runners
    /// without Metal); CPU elsewhere until Vulkan (phase 13).
    fn device(args: &EngineArgs) -> Device {
        let forced_cpu = std::env::var("GHI_DEVICE").is_ok_and(|d| d.eq_ignore_ascii_case("cpu"));
        if args.cpu || forced_cpu || !cfg!(target_os = "macos") {
            Device::Cpu
        } else {
            Device::Gpu
        }
    }

    pub fn load_asr(args: &EngineArgs, pass: Pass) -> Result<(Asr, Engine), ErrorDoc> {
        let (model, engine) = model_path(ASR_MODEL, args.asr_model.as_ref())?;
        let asr = Asr::new(&AsrConfig {
            model,
            device: device(args),
            chunk_ms: Some(asr_chunk_ms(pass)),
            endpointing: true,
        })
        .map_err(speech_error)?;
        Ok((asr, engine))
    }

    pub fn load_diar(args: &EngineArgs, pass: Pass) -> Result<(Diarizer, Engine), ErrorDoc> {
        let (model, engine) = model_path(DIAR_MODEL, args.diar_model.as_ref())?;
        // Presets are model-specific; an explicit model uses its own default.
        let preset = args
            .diar_model
            .is_none()
            .then(|| diar_preset(pass).to_owned());
        let diar = Diarizer::new(&DiarConfig {
            model,
            device: device(args),
            preset,
        })
        .map_err(speech_error)?;
        Ok((diar, engine))
    }
}
