// SPDX-License-Identifier: Apache-2.0
//! Speech models on the phone (types are `Mobile*` so they never clash with the
//! desktop's `ModelsStatus`/`ModelDownload`) (M1 step, Settings → Models). Contracts only;
//! 16-G downloads through `ghi-models::download` over `ghi-net`.

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

#[tauri::command]
#[specta::specta]
pub async fn models_status() -> Result<MobileModelsStatus, String> {
    Err("not yet".into())
}

/// Downloads what is missing, resuming `.part` files. `wifi_only: false` is the
/// user's "download over cellular this time". Progress comes as `MobileEvent::ModelDownload`.
#[tauri::command]
#[specta::specta]
pub async fn models_download(_wifi_only: bool) -> Result<(), String> {
    Err("not yet".into())
}

#[tauri::command]
#[specta::specta]
pub async fn models_cancel() -> Result<(), String> {
    Err("not yet".into())
}
