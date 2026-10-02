// SPDX-License-Identifier: Apache-2.0
//! What this machine needs downloaded, for the onboarding screen: the tier
//! preset's speech models and notes LLM, with install state and the bytes of
//! an interrupted download (`<file>.part`), so "Downloading 34%" survives a
//! restart.

use std::fs;
use std::path::Path;

use ghi_net::fetch::part_path;
use serde::Serialize;

use crate::tier::{Tier, detect, preset, tier_for};
use crate::{find, path_in};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequiredModel {
    pub id: String,
    pub role: String,
    /// Pinned size in bytes.
    pub size: u64,
    /// In the directory with the pinned size (hash checked at load).
    pub installed: bool,
    /// Bytes of the `.part` of an unfinished download; 0 when installed or
    /// nothing was started. At most `size`.
    pub partial_bytes: u64,
}

/// The models a tier needs: speech first, then the LLM, then the embedding
/// model (Balanced and Max), then the voice model. The voice model is not in
/// `Preset::speech_models`: the final pass and `speech_ready` do not wait on
/// it. Cheap: file sizes only.
pub fn required_for_tier(dir: &Path, tier: Tier) -> Vec<RequiredModel> {
    let p = preset(tier);
    p.speech_models
        .iter()
        .map(String::as_str)
        .chain([p.llm_id])
        .chain(p.embed_id)
        .chain([p.voice_id])
        .filter_map(find)
        .map(|m| {
            let dest = path_in(dir, &m);
            let len = |p: &Path| fs::metadata(p).map(|md| md.len()).unwrap_or(0);
            let installed = len(&dest) == m.size;
            let partial_bytes = if installed {
                0
            } else {
                len(&part_path(&dest)).min(m.size)
            };
            RequiredModel {
                id: m.id,
                role: m.role,
                size: m.size,
                installed,
                partial_bytes,
            }
        })
        .collect()
}

/// [`required_for_tier`] for this machine's tier.
pub fn required_for_machine(dir: &Path) -> (Tier, Vec<RequiredModel>) {
    let tier = tier_for(&detect());
    (tier, required_for_tier(dir, tier))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_speech_then_llm_with_partial_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let all = required_for_tier(dir.path(), Tier::Balanced);
        let ids: Vec<_> = all.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "nemotron-3.5-asr",
                "nemotron-3-diarization",
                "qwen3-4b",
                "qwen3-embedding-0.6b",
                "campplus-zh-en"
            ]
        );
        let light = required_for_tier(dir.path(), Tier::Light);
        assert!(light.iter().all(|r| r.role != "embed"));
        assert!(all.iter().all(|r| !r.installed && r.partial_bytes == 0));

        let llm = find("qwen3-4b").unwrap();
        let part = part_path(&path_in(dir.path(), &llm));
        fs::write(&part, vec![0u8; 1234]).unwrap();
        let all = required_for_tier(dir.path(), Tier::Balanced);
        let r = all.iter().find(|r| r.id == "qwen3-4b").unwrap();
        assert_eq!((r.installed, r.partial_bytes), (false, 1234));
        assert_eq!(r.role, "llm");
    }

    #[test]
    fn voice_model_is_listed_last_but_not_a_speech_model() {
        let dir = tempfile::tempdir().unwrap();
        for tier in [Tier::Light, Tier::Balanced, Tier::Max] {
            let p = preset(tier);
            assert_eq!(p.voice_id, "campplus-zh-en");
            assert_eq!(find(p.voice_id).unwrap().role, "voice");
            assert!(!p.speech_models.iter().any(|id| id == p.voice_id));
            let all = required_for_tier(dir.path(), tier);
            assert_eq!(all.last().unwrap().id, "campplus-zh-en");
            assert_eq!(all.last().unwrap().role, "voice");
        }
    }
}
