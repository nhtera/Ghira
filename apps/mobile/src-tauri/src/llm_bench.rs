// SPDX-License-Identifier: Apache-2.0
//! Notes-model speed check on a phone (test-hooks builds only), launched with
//! `GHI_SELFTEST=llm:<text file in Documents>`: loads the notes model in
//! process and times one prompt read and one answer, plain and with a JSON
//! schema (the notes path), with the thermal state and memory around it. The
//! engine's diagnostic switches (`GHI_LLM_NO_MMAP`, `GHI_LLM_KV_F16`,
//! `GHI_LLM_NO_FA`) and `GHI_METAL_RESIDENCY=1` come from the launch
//! environment. Writes `Documents/selftest-<unix>.json`; numbers only.

use std::path::Path;
use std::time::{Duration, Instant};

use ghi_llm::sidecar::{Body, Op, Sidecar, WireMessage};

/// Characters of the text file used as the prompt (about 3,000 tokens of English).
const PROMPT_CHARS: usize = 12_000;
const ANSWER_TOKENS: u32 = 200;

pub fn run(models: &Path, text: &str) -> serde_json::Value {
    let stats = || {
        let s = crate::platform::device_stats();
        serde_json::json!({"thermal": s.thermal, "memory_mb": s.memory_mb, "battery": s.battery})
    };
    let before = stats();
    let knobs: Vec<String> = [
        "GHI_LLM_NO_MMAP",
        "GHI_LLM_KV_F16",
        "GHI_LLM_NO_FA",
        "GHI_METAL_RESIDENCY",
    ]
    .into_iter()
    .filter(|k| std::env::var_os(k).is_some_and(|v| v == "1"))
    .map(str::to_owned)
    .collect();
    let result = (|| -> Result<serde_json::Value, String> {
        let m = ghi_models::find(ghi_app::core::preset().llm_id)
            .ok_or("no notes model in the registry")?;
        let path = ghi_models::path_in(models, &m);
        let t = Instant::now();
        let mut s = Sidecar::in_process().map_err(|e| e.to_string())?;
        let loaded = s
            .request(
                Op::Load {
                    model_path: path.to_string_lossy().into_owned(),
                    n_ctx: 8192,
                    n_gpu_layers: 999,
                    seed: 42,
                    chat_format: "qwen3".into(),
                },
                Duration::from_secs(180),
            )
            .map_err(|e| e.to_string())?;
        let devices = match loaded.body {
            Body::Loaded { devices, .. } => devices,
            other => return Err(format!("load: {other:?}")),
        };
        let load_s = t.elapsed().as_secs_f64();
        let after_load = stats();
        let prompt: String = text.chars().take(PROMPT_CHARS).collect();
        let mut runs = Vec::new();
        for schema in [
            None,
            Some(
                serde_json::json!({"type": "object", "properties": {"summary": {"type": "array", "items": {"type": "string"}}}, "required": ["summary"]}),
            ),
        ] {
            let with_schema = schema.is_some();
            let reply = s
                .request(
                    Op::Complete {
                        messages: vec![
                            WireMessage {
                                role: "system".into(),
                                content: "Summarize the meeting transcript in short points.".into(),
                            },
                            WireMessage {
                                role: "user".into(),
                                content: prompt.clone(),
                            },
                        ],
                        schema,
                        max_tokens: ANSWER_TOKENS,
                        temperature: 0.7,
                    },
                    Duration::from_secs(1200),
                )
                .map_err(|e| e.to_string())?;
            match reply.body {
                Body::Completed {
                    tokens_in,
                    tokens_out,
                    wall_s,
                    prompt_s,
                    ..
                } => {
                    let prompt_s = prompt_s.unwrap_or(0.0);
                    let gen_s = (wall_s - prompt_s).max(0.001);
                    runs.push(serde_json::json!({
                        "schema": with_schema,
                        "tokens_in": tokens_in,
                        "prompt_s": prompt_s,
                        "prompt_tok_s": f64::from(tokens_in) / prompt_s.max(0.001),
                        "tokens_out": tokens_out,
                        "gen_s": gen_s,
                        "out_tok_s": f64::from(tokens_out) / gen_s,
                        "after": stats(),
                    }));
                }
                other => return Err(format!("complete: {other:?}")),
            }
        }
        Ok(
            serde_json::json!({"devices": devices, "load_s": load_s, "after_load": after_load, "runs": runs}),
        )
    })();
    match result {
        Ok(v) => serde_json::json!({"llm": v, "knobs": knobs, "before": before}),
        Err(e) => serde_json::json!({"error": e, "knobs": knobs, "before": before}),
    }
}
