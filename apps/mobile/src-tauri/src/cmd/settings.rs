// SPDX-License-Identifier: Apache-2.0
//! Phone-only settings, beside the shared `AppSettings` (`get_settings` /
//! `update_settings`: meeting language, cloud provider, audio retention,
//! consent message). Stored by 16-G in the store's settings table under a
//! separate `mobile` key, so the desktop settings type stays unchanged.

use serde::{Deserialize, Serialize};
use specta::Type;

use super::types::ProcessingTarget;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MobileSettings {
    /// What new recordings and imports use (Settings → Processing). `Desktop`
    /// is rejected until phase 15.
    pub default_target: ProcessingTarget,
    /// Download models over Wi-Fi only (default true).
    pub models_wifi_only: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn mobile_settings() -> Result<MobileSettings, String> {
    Err("not yet".into())
}

#[tauri::command]
#[specta::specta]
pub async fn set_mobile_settings(_settings: MobileSettings) -> Result<MobileSettings, String> {
    Err("not yet".into())
}
