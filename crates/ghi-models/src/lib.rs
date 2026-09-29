// SPDX-License-Identifier: Apache-2.0
//! Model registry, pinned downloads, SHA-256 checks and device tiers.
//!
//! Phase 3 has the registry (`registry.toml`, embedded at build time) and path
//! resolution. Downloads go through `tools/scripts/fetch-models.sh` for now;
//! the in-app downloader (through `ghi-net`) and tiers come later.

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
    fn speech_models_are_registered() {
        assert_eq!(find("nemotron-3.5-asr").unwrap().role, "asr");
        assert_eq!(find("nemotron-3-diarization").unwrap().role, "diarization");
        assert!(find("nope").is_none());
    }
}
