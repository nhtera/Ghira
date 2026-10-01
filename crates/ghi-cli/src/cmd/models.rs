// SPDX-License-Identifier: Apache-2.0
//! `ghi models`: the model manager from the command line (phase 8): hardware
//! tier, install status, hash verification, pinned downloads and offline
//! import. Downloads are the only network use, through `ghi-net`, and happen
//! only when the user runs `fetch`.

use std::io::Write;
use std::path::{Path, PathBuf};

use ghi_models::{Model, Preset, Tier};
use ghi_net::NetPolicy;
use ghi_net::fetch::UreqTransport;
use serde_json::json;

use crate::contract::{ErrorCode, ErrorDoc};

#[derive(Debug, Clone, clap::Subcommand)]
pub enum Action {
    /// Hardware tier and which pinned models are installed and verified
    /// (`ghi.models/1`). Size-only unless `--verify`.
    Status {
        /// Models directory (default: $GHI_MODELS_DIR, else ./models).
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Also check every installed file's SHA-256.
        #[arg(long)]
        verify: bool,
    },
    /// Check an installed model's SHA-256 (`ghi.models-verify/1`); `all` for every one.
    Verify {
        id: String,
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Download a pinned model through ghi-net (`ghi.models-fetch/1`).
    Fetch {
        id: String,
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Refuse all internet traffic (fails before any connection).
        #[arg(long)]
        strict_offline: bool,
    },
    /// Install a model from a local file; its SHA-256 must be a registry entry's
    /// (`ghi.models-import/1`).
    Import {
        file: PathBuf,
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}

pub fn run(action: &Action) -> Result<(), ErrorDoc> {
    match action {
        Action::Status { dir, verify } => status(&dir_or_default(dir), *verify),
        Action::Verify { id, dir } => verify(&dir_or_default(dir), id),
        Action::Fetch {
            id,
            dir,
            strict_offline,
        } => fetch(&dir_or_default(dir), id, *strict_offline),
        Action::Import { file, dir } => import(&dir_or_default(dir), file),
    }
}

fn dir_or_default(dir: &Option<PathBuf>) -> PathBuf {
    dir.clone().unwrap_or_else(ghi_models::models_dir)
}

fn bad(message: impl Into<String>) -> ErrorDoc {
    ErrorDoc::new(ErrorCode::BadInput, message)
}

fn find(id: &str) -> Result<Model, ErrorDoc> {
    ghi_models::find(id).ok_or_else(|| bad(format!("unknown model id: {id}")))
}

fn preset_json(p: &Preset) -> serde_json::Value {
    json!({
        "tier": p.tier,
        "asr_chunk_ms": p.asr_chunk_ms,
        "final_asr_chunk_ms": p.final_asr_chunk_ms,
        "llm": p.llm_id,
        "ram_budget_bytes": p.ram_budget_bytes,
        "speech_models": p.speech_models,
        "llm_while_recording": ghi_models::llm_allowed_while_recording(p.tier),
        "unload_speech_before_llm": ghi_models::unload_speech_before_llm(p.tier),
    })
}

fn status(dir: &Path, verify: bool) -> Result<(), ErrorDoc> {
    let hw = ghi_models::detect();
    let tier = ghi_models::tier_for(&hw);
    let models = if verify {
        ghi_models::status_verified(dir)
    } else {
        ghi_models::status(dir)
    };
    let presets: Vec<_> = [Tier::Light, Tier::Balanced, Tier::Max]
        .iter()
        .map(|t| preset_json(&ghi_models::preset(*t)))
        .collect();
    crate::emit(&json!({
        "schema": "ghi.models/1",
        "dir": dir,
        "tier": tier,
        "hw": hw,
        "preset": preset_json(&ghi_models::preset(tier)),
        "presets": presets,
        "models": models,
    }))
}

fn verify(dir: &Path, id: &str) -> Result<(), ErrorDoc> {
    let targets: Vec<Model> = if id == "all" {
        ghi_models::registry()
    } else {
        vec![find(id)?]
    };
    let mut results = Vec::new();
    let mut all_ok = true;
    for m in &targets {
        let outcome = ghi_models::verify_file(&ghi_models::path_in(dir, m), m);
        all_ok &= outcome.is_ok();
        results.push(json!({
            "id": m.id,
            "ok": outcome.is_ok(),
            "error": outcome.err().map(|e| e.to_string()),
        }));
    }
    crate::emit(&json!({"schema": "ghi.models-verify/1", "results": results}))?;
    if all_ok {
        Ok(())
    } else {
        Err(ErrorDoc::new(
            ErrorCode::EngineUnavailable,
            "a model is missing or failed verification",
        ))
    }
}

fn fetch(dir: &Path, id: &str, strict_offline: bool) -> Result<(), ErrorDoc> {
    let model = find(id)?;
    let policy = if strict_offline {
        NetPolicy::StrictOffline
    } else {
        NetPolicy::Default
    };
    let mut last_pct = u64::MAX;
    let mut progress = |done: u64, total: u64| {
        let pct = done * 100 / total.max(1);
        if pct != last_pct {
            last_pct = pct;
            crate::warn(&format!("{id}: {pct}% ({done}/{total} bytes)"));
        }
    };
    let report =
        ghi_models::download(&model, dir, policy, &mut progress, &UreqTransport).map_err(|e| {
            match e {
                ghi_models::DownloadError::Denied(_) => bad(e.to_string()),
                _ => ErrorDoc::new(ErrorCode::EngineUnavailable, e.to_string()),
            }
        })?;
    crate::emit(&json!({
        "schema": "ghi.models-fetch/1",
        "id": model.id,
        "path": ghi_models::path_in(dir, &model),
        "downloaded_bytes": report.downloaded,
        "resumed_from": report.resumed_from,
        "redirects": report.redirects,
        "host": report.final_host,
    }))
}

fn import(dir: &Path, file: &Path) -> Result<(), ErrorDoc> {
    let model = ghi_models::import_file(file, dir).map_err(|e| match e {
        ghi_models::ImportError::UnknownHash(_) => bad(e.to_string()),
        ghi_models::ImportError::Io(_) => ErrorDoc::new(ErrorCode::Internal, e.to_string()),
    })?;
    let _ = std::io::stderr().flush();
    crate::emit(&json!({
        "schema": "ghi.models-import/1",
        "id": model.id,
        "path": ghi_models::path_in(dir, &model),
        "size": model.size,
    }))
}
