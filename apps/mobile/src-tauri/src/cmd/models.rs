// SPDX-License-Identifier: Apache-2.0
//! Speech models on the phone (types are `Mobile*` so they never clash with the
//! desktop's `ModelsStatus`/`ModelDownload`) (M1 step, Settings → Models). The work
//! is in `models_cmd.rs` (`ghi-models::download` over `ghi-net`).

use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum MobileModelRole {
    Asr,
    Diarization,
    Voice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum MobileModelState {
    Missing,
    Downloading,
    Ready,
    /// Waiting for Wi-Fi (the default is Wi-Fi only).
    WaitingForWifi,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MobileModelItem {
    pub id: String,
    pub role: MobileModelRole,
    pub size_bytes: f64,
    pub received_bytes: f64,
    pub state: MobileModelState,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MobileModelsStatus {
    pub items: Vec<MobileModelItem>,
    /// Total download size of what is missing.
    pub missing_bytes: f64,
    pub wifi_only: bool,
}

/// The settings the downloads follow (a locked app still downloads).
fn wifi_only_setting(core: &ghi_app::core::Core) -> bool {
    core.store_even_locked()
        .and_then(|s| super::settings::load(&s))
        .map_or(true, |m| m.models_wifi_only)
}

#[tauri::command]
#[specta::specta]
pub async fn models_status(
    core: ghi_app::CoreState<'_>,
    downloads: tauri::State<'_, std::sync::Arc<crate::models_cmd::Downloads>>,
) -> Result<MobileModelsStatus, String> {
    let downloads = downloads.inner().clone();
    ghi_app::blocking(&core, move |c| {
        Ok(crate::models_cmd::status(
            &c.models(),
            wifi_only_setting(c),
            &downloads,
            &ghi_app::core::damaged_models(),
        ))
    })
    .await
}

/// Downloads what is missing, resuming `.part` files. `wifi_only: false` is the
/// user's "download over cellular this time". Progress comes as `MobileEvent::ModelDownload`.
/// Refused in strict offline mode (`offline`). A second call while one runs does nothing.
#[tauri::command]
#[specta::specta]
pub async fn models_download(
    core: ghi_app::CoreState<'_>,
    downloads: tauri::State<'_, std::sync::Arc<crate::models_cmd::Downloads>>,
    wifi_only: bool,
) -> Result<(), String> {
    let downloads = downloads.inner().clone();
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let strict = core
            .store_even_locked()?
            .get_setting(ghi_app::system::SETTINGS_KEY)
            .map_err(|e| e.to_string())
            .map(|v| ghi_app::system::from_stored(v).strict_offline)?;
        if strict {
            return Err("offline".into());
        }
        // Cellular is allowed when either the setting or this call says so.
        let wifi_only = wifi_only && wifi_only_setting(&core);
        crate::models_cmd::start(core, downloads, wifi_only, ghi_net::NetPolicy::Default)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn models_cancel(
    downloads: tauri::State<'_, std::sync::Arc<crate::models_cmd::Downloads>>,
) -> Result<(), String> {
    downloads.cancel();
    Ok(())
}
