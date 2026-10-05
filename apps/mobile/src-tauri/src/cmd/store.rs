// SPDX-License-Identifier: Apache-2.0
//! The encrypted store can't be opened (a key that is not on this phone, a
//! damaged database): what the UI shows instead of the library. Both
//! commands work while the app is locked or starting; they never return
//! meeting content.

use ghi_app::store_problem::StoreProblem;
use serde::Serialize;
use specta::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum StoreStatus {
    Ready,
    /// `problem` is a stable code; the words are the UI's.
    Unavailable {
        problem: StoreProblem,
    },
}

/// What the last attempt to open the store found (`Ready`: nothing failed).
/// It never opens the store itself: the startup call already did, and a
/// second open could prompt for the key again. "Try again" is `lock_state`.
#[tauri::command]
#[specta::specta]
pub async fn store_status(core: ghi_app::CoreState<'_>) -> Result<StoreStatus, String> {
    Ok(core
        .open_problem()
        .map_or(StoreStatus::Ready, |problem| StoreStatus::Unavailable {
            problem,
        }))
}

/// Removes the store that can't be opened and its key, and opens a new empty
/// one. `confirm` is the typed phrase (`DELETE` / `XÓA`, checked again here),
/// refused while a recording or an import runs (`busy`). Rust also refuses
/// unless the store still can't be read for good (`keyMissing`, `damaged`):
/// the error is that problem's code, or `storeReadable`.
#[tauri::command]
#[specta::specta]
pub async fn store_start_fresh(
    core: ghi_app::CoreState<'_>,
    recorder: tauri::State<'_, std::sync::Arc<crate::session::Recorder>>,
    inbox: tauri::State<'_, std::sync::Arc<crate::inbox::Inbox>>,
    confirm: String,
) -> Result<(), String> {
    let (recorder, inbox) = (recorder.inner().clone(), inbox.inner().clone());
    ghi_app::blocking(&core, move |c| {
        crate::privacy_cmd::start_fresh(c, c.data_dir(), inbox.root(), &confirm, &|| {
            recorder.latest().is_some() || inbox.importing()
        })
    })
    .await
}

/// The webview reports that a screen failed to render, for the diagnostics log
/// (`ghi-diag`). Only the error's type name goes in: letters and digits, at
/// most 48, never a message (it could hold meeting text).
#[tauri::command]
#[specta::specta]
pub async fn log_ui_failure(kind: String) -> Result<(), String> {
    let kind: String = kind
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(48)
        .collect();
    log::error!("ui failure: {kind}");
    Ok(())
}
