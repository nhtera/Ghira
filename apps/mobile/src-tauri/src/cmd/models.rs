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
    /// The notes model (optional, 8 GB phones).
    Notes,
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
        crate::models_cmd::start(
            core,
            downloads,
            wifi_only,
            ghi_net::NetPolicy::Default,
            crate::models_cmd::needed(),
        )
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

/// The notes model on this phone: `None` when the phone cannot write notes
/// itself (below 8 GB), else its row (missing, downloading, ready, …).
#[tauri::command]
#[specta::specta]
pub async fn notes_model_status(
    core: ghi_app::CoreState<'_>,
    downloads: tauri::State<'_, std::sync::Arc<crate::models_cmd::Downloads>>,
) -> Result<Option<MobileModelItem>, String> {
    let downloads = downloads.inner().clone();
    ghi_app::blocking(&core, move |c| {
        Ok(crate::models_cmd::notes_model().and_then(|(id, role)| {
            crate::models_cmd::row(
                &c.models(),
                id,
                role,
                &downloads,
                &ghi_app::core::damaged_models(),
            )
            .map(|(item, _)| item)
        }))
    })
    .await
}

/// Downloads the notes model (2.5 GB), Wi-Fi only unless the user allows
/// cellular this time; refused on a phone that cannot write notes.
#[tauri::command]
#[specta::specta]
pub async fn models_download_notes(
    core: ghi_app::CoreState<'_>,
    downloads: tauri::State<'_, std::sync::Arc<crate::models_cmd::Downloads>>,
    wifi_only: bool,
) -> Result<(), String> {
    let downloads = downloads.inner().clone();
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let model =
            crate::models_cmd::notes_model().ok_or("this phone cannot write notes itself")?;
        // One download at a time (the speech models may be waiting for Wi-Fi).
        if downloads.running() {
            return Err("busy".into());
        }
        let strict = core
            .store_even_locked()?
            .get_setting(ghi_app::system::SETTINGS_KEY)
            .map_err(|e| e.to_string())
            .map(|v| ghi_app::system::from_stored(v).strict_offline)?;
        if strict {
            return Err("offline".into());
        }
        let wifi_only = wifi_only && wifi_only_setting(&core);
        crate::models_cmd::start(
            core,
            downloads,
            wifi_only,
            ghi_net::NetPolicy::Default,
            vec![model],
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Removes the notes model to free its 2.5 GB (notes then come from the
/// cloud or the computer again). Refused while notes are being written or a
/// download runs; notes still waiting to be written are called off, and their
/// meetings settle (they would wait for the model forever).
#[tauri::command]
#[specta::specta]
pub async fn models_remove_notes(
    core: ghi_app::CoreState<'_>,
    downloads: tauri::State<'_, std::sync::Arc<crate::models_cmd::Downloads>>,
) -> Result<(), String> {
    let downloads = downloads.inner().clone();
    ghi_app::blocking(&core, move |c| {
        if downloads.running() {
            return Err("busy".into());
        }
        let store = c.store_even_locked()?;
        crate::models_cmd::remove_notes_model(&store, &c.models())
    })
    .await
}

/// Writes (or rewrites) a meeting's notes on this phone. Only a phone that
/// can run the notes model and has it installed: anywhere else the job would
/// wait for a model that never comes and leave the meeting "processing".
#[tauri::command]
#[specta::specta]
pub async fn write_notes(core: ghi_app::CoreState<'_>, meeting: String) -> Result<(), String> {
    ghi_app::blocking(&core, move |c| {
        if crate::models_cmd::notes_model().is_none() {
            return Err("this phone cannot write notes itself".into());
        }
        if !ghi_app::core::llm_ready(&c.models()) {
            return Err("noModel".into());
        }
        let store = c.store()?;
        let speech_waits = !ghi_app::core::speech_ready(&c.models());
        ghi_app::detail::queue_notes_again(
            &store,
            &meeting,
            None,
            ghi_app::detail::NotesLanguage::Meeting,
            speech_waits,
        )?;
        c.notify_jobs();
        Ok(())
    })
    .await
}
