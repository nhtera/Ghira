// SPDX-License-Identifier: Apache-2.0
//! Synced settings (slice 15-C1): an allowlist, last writer wins per key.

use super::not_yet;
use super::records::SettingRec;
use crate::Result;
use crate::store::Store;

/// The only keys that sync: the `"app"` JSON fields (serde names) and the
/// vocabulary keys. Everything else is device-local.
pub const SYNCED_KEYS: [&str; 7] = [
    "meetingLanguage",
    "cloudRedact",
    "audioRetentionDays",
    "consentMessageEn",
    "consentMessageVi",
    "vocabulary",
    "vocabulary.ignored",
];

/// Whether `key` is on the allowlist.
pub fn is_synced_key(key: &str) -> bool {
    SYNCED_KEYS.contains(&key)
}

impl Store {
    /// Records a local change to a synced key (`value` is JSON text). Keys
    /// off the allowlist are an error.
    pub fn put_synced(&self, _key: &str, _value_json: &str) -> Result<()> {
        not_yet("sync::settings::put_synced")
    }

    /// Applies a peer's value: writes `synced_settings` and the underlying
    /// setting. Unknown keys are ignored.
    pub fn apply_synced(&self, _rec: &SettingRec) -> Result<()> {
        not_yet("sync::settings::apply_synced")
    }
}
