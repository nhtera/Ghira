// SPDX-License-Identifier: Apache-2.0
//! What this device can do (phase 16 D4): the live transcript and the
//! on-phone final pass need an A16-class chip and 6 GB of RAM.
//!
//! Live iff the model is an `iPhone<major>,<n>` with major >= 15 and RAM >=
//! 6 GB, or the model is unknown (a newer family than this build knows) and
//! RAM >= 6 GB. Everything else records only and has the `Phone` target
//! disabled. The simulator reports `SIMULATOR_MODEL_IDENTIFIER` and the host's
//! RAM. `GHI_DEVICE_TIER=live|record-only` overrides it in test-hooks builds.

use crate::cmd::lifecycle::{DeviceTier, TierClass};
use crate::platform;

/// RAM (GiB) a live device needs.
pub const MIN_RAM_GB: f64 = 6.0;
/// First `iPhone<major>` generation (A16) that is live.
pub const MIN_IPHONE_MAJOR: u32 = 15;

/// The `15` of `iPhone15,4`; `None` for anything else.
fn iphone_major(model_id: &str) -> Option<u32> {
    model_id
        .strip_prefix("iPhone")?
        .split(',')
        .next()?
        .parse()
        .ok()
}

pub fn classify(model_id: &str, ram_gb: f64) -> TierClass {
    // 5.9 GiB reads as "6 GB" on devices that report slightly under.
    let ram_ok = ram_gb >= MIN_RAM_GB - 0.2;
    let model_ok = match iphone_major(model_id) {
        Some(major) => major >= MIN_IPHONE_MAJOR,
        // Nothing read (a failed call): do not guess.
        None if model_id.is_empty() => false,
        // A non-empty id of another shape is a family newer than this build
        // knows (or unknown): RAM decides.
        None => true,
    };
    if ram_ok && model_ok {
        TierClass::Live
    } else {
        TierClass::RecordOnly
    }
}

/// The test override, honoured only in test-hooks builds.
fn forced() -> Option<TierClass> {
    if !cfg!(feature = "test-hooks") {
        return None;
    }
    match std::env::var("GHI_DEVICE_TIER").ok()?.as_str() {
        "live" => Some(TierClass::Live),
        "record-only" => Some(TierClass::RecordOnly),
        _ => None,
    }
}

pub fn detect() -> DeviceTier {
    let simulator = cfg!(target_abi = "sim");
    let model_id = if simulator {
        std::env::var("SIMULATOR_MODEL_IDENTIFIER").unwrap_or_else(|_| platform::device_model())
    } else {
        platform::device_model()
    };
    let ram_gb = platform::physical_memory() as f64 / (1024.0 * 1024.0 * 1024.0);
    describe(model_id, ram_gb, simulator, forced())
}

/// The tier of a device; `forced` wins (tests).
pub fn describe(
    model_id: String,
    ram_gb: f64,
    simulator: bool,
    forced: Option<TierClass>,
) -> DeviceTier {
    let tier = forced.unwrap_or_else(|| classify(&model_id, ram_gb));
    DeviceTier {
        model_id,
        ram_gb,
        simulator,
        tier,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tier_table() {
        use TierClass::{Live, RecordOnly};
        for (model, ram, want) in [
            ("iPhone12,1", 4.0, RecordOnly), // iPhone 11
            ("iPhone14,5", 4.0, RecordOnly), // iPhone 13
            ("iPhone14,2", 6.0, RecordOnly), // iPhone 13 Pro: A15
            ("iPhone15,2", 6.0, Live),       // iPhone 14 Pro: counts as A16 family
            ("iPhone15,4", 6.0, Live),       // iPhone 15
            ("iPhone15,4", 4.0, RecordOnly), // RAM too low
            ("iPhone16,1", 8.0, Live),       // iPhone 15 Pro
            ("iPhone17,3", 8.0, Live),
            ("iPhone18,1", 12.0, Live), // newer than this table
            ("iPhone15,4", 5.9, Live),  // reported slightly under 6
            ("Future1,1", 8.0, Live),   // an id shape this build does not know
            ("Future1,1", 4.0, RecordOnly),
            ("", 8.0, RecordOnly), // nothing read: never Live
            ("", 0.0, RecordOnly), // nothing known (host tests)
        ] {
            assert_eq!(classify(model, ram), want, "{model} {ram} GB");
        }
    }

    #[test]
    fn a_forced_tier_wins() {
        let t = describe("iPhone12,1".into(), 4.0, true, Some(TierClass::Live));
        assert_eq!(t.tier, TierClass::Live);
        assert!(t.simulator);
        let t = describe("iPhone16,1".into(), 8.0, false, None);
        assert_eq!(t.tier, TierClass::Live);
    }
}
