// SPDX-License-Identifier: Apache-2.0
//! The local model: [`crate::Llm`] over the worker process.

use std::path::Path;
use std::time::Duration;

use crate::provider::{Completion, EngineInfo, Llm, Request, Role};
use crate::sidecar::{Body, Op, Sidecar, WireMessage};
use crate::{LlmError, Result};

/// Fixed sampling seed: same input, same output.
const SEED: u32 = 42;
/// Offload every layer (Metal on Apple silicon).
const ALL_LAYERS: u32 = 999;
const LOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// A completion is alive while the worker reports progress (after every
/// prompt batch and every few output tokens): it is killed only after this
/// long without a line, on any machine speed (phase 6 → 8: a fixed budget
/// calibrated on an M4 Pro would kill a base M1's notes run).
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// …or after this long in total.
const HARD_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// Tokenizing a transcript (no progress lines).
const COUNT_TIMEOUT: Duration = Duration::from_secs(20);

/// A model loaded in its own worker process. Dropping it (or [`unload`]) kills
/// the worker, which frees the model memory.
///
/// [`unload`]: LocalLlm::unload
/// Kills every local worker process now, even one busy generating. The
/// desktop calls this at shutdown so no worker outlives the app (dropping a
/// `LocalLlm` still shuts its worker down).
pub fn kill_workers() {
    crate::sidecar::kill_all();
}

pub struct LocalLlm {
    sidecar: Sidecar,
    engine: EngineInfo,
    n_ctx: u32,
    progress: Option<Box<dyn FnMut(u32, u32) + Send>>,
}

impl LocalLlm {
    pub fn open(
        model_path: &Path,
        engine: EngineInfo,
        n_ctx: u32,
        chat_format: &str,
    ) -> Result<LocalLlm> {
        let path = model_path
            .to_str()
            .ok_or_else(|| LlmError::Invalid("model path is not valid UTF-8".into()))?;
        let mut sidecar = crate::sidecar::start()?;
        let reply = sidecar.request(
            Op::Load {
                model_path: path.to_string(),
                n_ctx,
                n_gpu_layers: ALL_LAYERS,
                seed: SEED,
                chat_format: chat_format.to_string(),
            },
            LOAD_TIMEOUT,
        )?;
        match reply.body {
            Body::Loaded {
                n_ctx,
                load_s,
                devices,
            } => {
                log::info!("llm loaded load_s={load_s:.1} n_ctx={n_ctx} devices={devices:?}");
                Ok(LocalLlm {
                    sidecar,
                    engine,
                    n_ctx,
                    progress: None,
                })
            }
            Body::Error { message } => Err(LlmError::Worker(message)),
            _ => Err(LlmError::Worker("unexpected reply to load".into())),
        }
    }

    /// Open a model from the registry (`ghi_models`), looked up in
    /// `ghi_models::models_dir()`.
    pub fn open_registry(id: &str, n_ctx: u32) -> Result<LocalLlm> {
        LocalLlm::open_registry_in(&ghi_models::models_dir(), id, n_ctx)
    }

    /// Open a registry model from `dir`, checking its pinned SHA-256 first.
    pub fn open_registry_in(dir: &Path, id: &str, n_ctx: u32) -> Result<LocalLlm> {
        let model = ghi_models::find(id)
            .ok_or_else(|| LlmError::Invalid(format!("unknown model id {id}")))?;
        let path = ghi_models::path_in(dir, &model);
        // A missing or damaged model file means the engine is unavailable.
        ghi_models::verify_for_load(&path, &model)
            .map_err(|e| LlmError::Worker(format!("model {id}: {e}")))?;
        let engine = EngineInfo {
            name: model.id.clone(),
            version: model.revision.chars().take(8).collect(),
        };
        log::info!("llm model load id={id} n_ctx={n_ctx}");
        let format = model
            .chat_format
            .as_deref()
            .ok_or_else(|| LlmError::Invalid(format!("model {id} is not a chat model")))?;
        LocalLlm::open(&path, engine, n_ctx, format)
    }

    /// Kill the worker (kill = unload).
    pub fn unload(mut self) {
        self.sidecar.kill();
    }
}

impl Llm for LocalLlm {
    fn engine(&self) -> EngineInfo {
        self.engine.clone()
    }

    fn context_tokens(&self) -> u32 {
        self.n_ctx
    }

    fn stopper(&self) -> Option<std::sync::Arc<dyn Fn() + Send + Sync>> {
        self.sidecar.stopper()
    }

    fn set_progress(&mut self, progress: Box<dyn FnMut(u32, u32) + Send>) {
        self.progress = Some(progress);
    }

    fn count_tokens(&mut self, text: &str) -> Result<u32> {
        let reply = self.sidecar.request(
            Op::Count {
                text: text.to_string(),
            },
            COUNT_TIMEOUT,
        )?;
        match reply.body {
            Body::Counted { tokens } => Ok(tokens),
            Body::Error { message } => Err(LlmError::Worker(message)),
            _ => Err(LlmError::Worker("unexpected reply to count".into())),
        }
    }

    fn complete(&mut self, req: &Request) -> Result<Completion> {
        let messages = req
            .messages
            .iter()
            .map(|m| WireMessage {
                role: match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                }
                .to_string(),
                content: m.content.clone(),
            })
            .collect();
        let reply = self.sidecar.request_live(
            Op::Complete {
                messages,
                schema: req.schema.clone(),
                max_tokens: req.max_tokens,
                temperature: req.temperature,
            },
            IDLE_TIMEOUT,
            HARD_TIMEOUT,
            &mut |read, written| {
                if let Some(p) = self.progress.as_mut() {
                    p(read, written);
                }
            },
        )?;
        match reply.body {
            Body::Completed {
                text,
                tokens_in,
                tokens_out,
                truncated,
                wall_s,
                prompt_s,
            } => {
                // Sizes and times only (no content): how fast the model runs here.
                let prompt_s = prompt_s.unwrap_or(0.0);
                let gen_s = (wall_s - prompt_s).max(0.001);
                log::info!(
                    "llm completion tokens_in={tokens_in} prompt_s={prompt_s:.1} prompt_tok_s={:.0} tokens_out={tokens_out} gen_s={gen_s:.1} out_tok_s={:.1} truncated={truncated}",
                    f64::from(tokens_in) / prompt_s.max(0.001),
                    f64::from(tokens_out) / gen_s
                );
                Ok(Completion {
                    text,
                    tokens_in,
                    tokens_out,
                    truncated,
                })
            }
            Body::Error { message } => Err(LlmError::Worker(message)),
            _ => Err(LlmError::Worker("unexpected reply to complete".into())),
        }
    }
}
