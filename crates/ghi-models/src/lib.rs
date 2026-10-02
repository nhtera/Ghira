// SPDX-License-Identifier: Apache-2.0
//! Model registry, pinned downloads, SHA-256 checks and device tiers.
//!
//! The registry (`registry.toml`, embedded at build time), path resolution,
//! hardware tiers and presets ([`tier`]), hash checks, status and offline
//! import ([`verify`]), and pinned downloads through `ghi-net` ([`download`]).
//! `tools/scripts/fetch-models.sh` stays as the dev download path.

pub mod download;
pub mod required;
pub mod tier;
pub mod verify;

pub use download::{DownloadError, download};
pub use required::{RequiredModel, required_for_machine, required_for_tier};
pub use tier::{
    Hw, Preset, Tier, detect, llm_allowed_while_recording, preset, tier_for,
    unload_speech_before_llm,
};
pub use verify::{
    ImportError, ModelStatus, VerifyError, import_file, import_from, installed, status,
    status_verified, verify_file, verify_for_load,
};

use std::path::{Path, PathBuf};

use serde::Deserialize;

const REGISTRY: &str = include_str!("../registry.toml");

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// One pinned model file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub id: String,
    pub role: String,
    pub repo: String,
    pub revision: String,
    pub file: String,
    pub sha256: String,
    pub size: u64,
    pub license: String,
    /// Chat template family for LLMs (`qwen3`); absent for other roles.
    #[serde(default)]
    pub chat_format: Option<String>,
    /// Fallback sources tried in order after Hugging Face: base URLs (https)
    /// that serve the pinned file as `<base>/<file>`. The hash still decides.
    #[serde(default)]
    pub mirrors: Vec<String>,
}

#[derive(Deserialize)]
struct Registry {
    model: Vec<Model>,
}

/// All pinned models.
pub fn registry() -> Vec<Model> {
    toml::from_str::<Registry>(REGISTRY)
        .expect("registry.toml is valid (checked by tests)")
        .model
}

/// The registry entry with this id.
pub fn find(id: &str) -> Option<Model> {
    registry().into_iter().find(|m| m.id == id)
}

/// Directory holding downloaded models: `$GHI_MODELS_DIR`, else `./models`.
pub fn models_dir() -> PathBuf {
    std::env::var_os("GHI_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("models"))
}

/// Local path of a model file inside `dir`.
pub fn path_in(dir: &Path, model: &Model) -> PathBuf {
    dir.join(&model.file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_valid_and_pinned() {
        let models = registry();
        assert!(!models.is_empty());
        let mut ids: Vec<_> = models.iter().map(|m| m.id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), models.len(), "duplicate model ids");
        for m in &models {
            assert_eq!(
                m.revision.len(),
                40,
                "{}: revision must be a full commit",
                m.id
            );
            assert_eq!(m.sha256.len(), 64, "{}: sha256", m.id);
            assert!(m.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(m.size > 0);
            assert!(
                !m.file.contains('/') && !m.file.contains('\\'),
                "{}: file name",
                m.id
            );
        }
    }

    #[test]
    fn mirrors_are_https_when_present() {
        for m in registry() {
            for mirror in &m.mirrors {
                assert!(mirror.starts_with("https://"), "{}: {mirror}", m.id);
            }
        }
    }

    #[test]
    fn speech_models_are_registered() {
        assert_eq!(find("nemotron-3.5-asr").unwrap().role, "asr");
        assert_eq!(find("nemotron-3-diarization").unwrap().role, "diarization");
        assert!(find("nope").is_none());
    }

    #[test]
    fn embedding_model_is_registered() {
        let m = find("qwen3-embedding-0.6b").unwrap();
        assert_eq!(m.role, "embed");
        assert_eq!(m.license, "Apache-2.0");
        assert_eq!(m.chat_format, None);
    }

    #[test]
    fn llm_declares_its_chat_format() {
        let m = find("qwen3-4b").unwrap();
        assert_eq!(m.role, "llm");
        assert_eq!(m.chat_format.as_deref(), Some("qwen3"));
        assert_eq!(find("nemotron-3.5-asr").unwrap().chat_format, None);
    }
}
