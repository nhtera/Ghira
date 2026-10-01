// SPDX-License-Identifier: Apache-2.0
//! Hardware detection, tiers and the presets the core's scheduler reads.
//!
//! Doc 05 §6: 8 GB is "Light", 16 GB "Balanced", 32 GB and up "Max". Machines
//! report a little less than their nominal RAM (Windows reserves some), so the
//! cut-offs sit between the nominal sizes. The RAM budgets are the share of
//! memory Ghira plans to use (speech models, NeMo's ~1.3 GB host reserve per
//! session, the LLM with its KV cache); they are data for the scheduler, to be
//! tuned on real hardware.

use serde::Serialize;

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Light,
    Balanced,
    Max,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Light => "light",
            Tier::Balanced => "balanced",
            Tier::Max => "max",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hw {
    pub ram_bytes: u64,
    pub chip: String,
    /// A GPU the engines can use (Metal on Apple silicon). Best effort: false
    /// where it is not detected.
    pub gpu: bool,
}

/// Reads the machine's RAM and chip. Never fails: unknown values are 0 and
/// `"unknown"`, which tier as Light.
pub fn detect() -> Hw {
    let (ram_bytes, chip) = detect_os();
    let gpu = cfg!(all(target_os = "macos", target_arch = "aarch64"));
    Hw {
        ram_bytes,
        chip,
        gpu,
    }
}

#[cfg(target_os = "macos")]
fn detect_os() -> (u64, String) {
    let sysctl = |key: &str| {
        std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", key])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    let ram = sysctl("hw.memsize")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let chip = sysctl("machdep.cpu.brand_string").unwrap_or_else(|| "unknown".into());
    (ram, chip)
}

#[cfg(target_os = "linux")]
fn detect_os() -> (u64, String) {
    let ram = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            let line = s.lines().find(|l| l.starts_with("MemTotal:"))?;
            let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
            Some(kb * 1024)
        })
        .unwrap_or(0);
    (ram, "unknown".into())
}

#[cfg(target_os = "windows")]
fn detect_os() -> (u64, String) {
    let ram = std::process::Command::new("wmic")
        .args(["ComputerSystem", "get", "TotalPhysicalMemory", "/value"])
        .output()
        .ok()
        .and_then(|o| {
            let text = String::from_utf8_lossy(&o.stdout).into_owned();
            let value = text
                .lines()
                .find_map(|l| l.trim().strip_prefix("TotalPhysicalMemory="))?;
            value.trim().parse().ok()
        })
        .unwrap_or(0);
    (ram, "unknown".into())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn detect_os() -> (u64, String) {
    (0, "unknown".into())
}

/// Light below 12 GiB (8 GB machines), Balanced below 28 GiB (16 to 24 GB),
/// Max from 28 GiB (32 GB machines).
pub fn tier_for(hw: &Hw) -> Tier {
    if hw.ram_bytes < 12 * GIB {
        Tier::Light
    } else if hw.ram_bytes < 28 * GIB {
        Tier::Balanced
    } else {
        Tier::Max
    }
}

/// What a tier runs with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preset {
    pub tier: Tier,
    /// Live ASR chunk (ms): 1120 on Light, 560 elsewhere.
    pub asr_chunk_ms: u32,
    /// Chunk for the final pass (ms).
    pub final_asr_chunk_ms: u32,
    /// Registry id of the notes LLM. No 8B is pinned yet, so every tier uses
    /// the 4B.
    pub llm_id: &'static str,
    /// Memory Ghira plans to use at most on this tier.
    pub ram_budget_bytes: u64,
    /// Registry ids of the speech models (ASR, diarization).
    pub speech_models: Vec<String>,
}

pub fn preset(tier: Tier) -> Preset {
    let (asr_chunk_ms, ram_budget_bytes) = match tier {
        Tier::Light => (1120, 5 * GIB + GIB / 2),
        Tier::Balanced => (560, 10 * GIB),
        Tier::Max => (560, 20 * GIB),
    };
    Preset {
        tier,
        asr_chunk_ms,
        final_asr_chunk_ms: 1120,
        llm_id: "qwen3-4b",
        ram_budget_bytes,
        speech_models: vec!["nemotron-3.5-asr".into(), "nemotron-3-diarization".into()],
    }
}

/// Never run the LLM during a recording on Light (doc 05 §6).
pub fn llm_allowed_while_recording(tier: Tier) -> bool {
    tier != Tier::Light
}

/// Free the speech models before loading the LLM on 16 GB and below.
pub fn unload_speech_before_llm(tier: Tier) -> bool {
    tier != Tier::Max
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(gib: f64) -> Hw {
        Hw {
            ram_bytes: (gib * GIB as f64) as u64,
            chip: "test".into(),
            gpu: false,
        }
    }

    #[test]
    fn tiers_follow_ram() {
        for (gib, tier) in [
            (0.0, Tier::Light),
            (7.7, Tier::Light),
            (8.0, Tier::Light),
            (11.9, Tier::Light),
            (15.7, Tier::Balanced),
            (16.0, Tier::Balanced),
            (24.0, Tier::Balanced),
            (31.5, Tier::Max),
            (32.0, Tier::Max),
            (64.0, Tier::Max),
        ] {
            assert_eq!(tier_for(&hw(gib)), tier, "{gib} GiB");
        }
    }

    #[test]
    fn presets_match_the_decision_record() {
        assert_eq!(preset(Tier::Light).asr_chunk_ms, 1120);
        assert_eq!(preset(Tier::Balanced).asr_chunk_ms, 560);
        assert_eq!(preset(Tier::Max).asr_chunk_ms, 560);
        for t in [Tier::Light, Tier::Balanced, Tier::Max] {
            let p = preset(t);
            assert_eq!(p.final_asr_chunk_ms, 1120);
            assert!(crate::find(p.llm_id).is_some());
            for id in &p.speech_models {
                assert!(crate::find(id).is_some(), "{id}");
            }
        }
        let budgets: Vec<_> = [Tier::Light, Tier::Balanced, Tier::Max]
            .map(|t| preset(t).ram_budget_bytes)
            .to_vec();
        assert!(budgets.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn scheduling_rules() {
        assert!(!llm_allowed_while_recording(Tier::Light));
        assert!(llm_allowed_while_recording(Tier::Balanced));
        assert!(unload_speech_before_llm(Tier::Light));
        assert!(unload_speech_before_llm(Tier::Balanced));
        assert!(!unload_speech_before_llm(Tier::Max));
    }

    #[test]
    fn detect_returns_something_sane_on_dev_hosts() {
        let hw = detect();
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            assert!(hw.ram_bytes > 0);
        }
        assert!(!hw.chip.is_empty());
    }
}
