// SPDX-License-Identifier: Apache-2.0
//! The share-extension inbox (M5): audio files the extension copied into the
//! App Group `inbox/<uuid>/` with a manifest (`inbox.rs`).

use serde::{Deserialize, Serialize};
use specta::Type;

use super::types::ProcessingTarget;
use ghi_app::system::MeetingLanguage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum InboxState {
    /// Waiting for the user to confirm language and target (the extension closed early).
    Pending,
    Importing,
    /// Could not be imported (unsupported type such as CAF, too large, unreadable).
    Rejected,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InboxItem {
    /// The inbox folder's uuid.
    pub id: String,
    /// The shared file's name (shown as the title).
    pub name: String,
    pub size_bytes: f64,
    pub language: MeetingLanguage,
    pub target: ProcessingTarget,
    pub state: InboxState,
    /// Set when `state` is `Rejected`: a code the UI turns into words.
    pub reason: Option<String>,
}

type InboxHandle<'a> = tauri::State<'a, std::sync::Arc<crate::inbox::Inbox>>;

/// The items waiting, importing or rejected. Items the share extension's user
/// already confirmed are imported by the app on its own.
#[tauri::command]
#[specta::specta]
pub async fn inbox_list(
    core: ghi_app::CoreState<'_>,
    inbox: InboxHandle<'_>,
) -> Result<Vec<InboxItem>, String> {
    // File names are content: nothing while the app is locked.
    if core.locked() {
        return Err("the app is locked".into());
    }
    let inbox = inbox.inner().clone();
    tauri::async_runtime::spawn_blocking(move || inbox.list())
        .await
        .map_err(|e| e.to_string())
}

/// Imports the item with these choices; returns the new meeting id. Errors are
/// codes: `notFound`, `busy`, `desktopUnavailable`, and the item's rejection
/// reason (`unsupportedType`, `tooLarge`, `unreadable`, ...).
#[tauri::command]
#[specta::specta]
pub async fn inbox_confirm(
    core: ghi_app::CoreState<'_>,
    recorder: tauri::State<'_, std::sync::Arc<crate::session::Recorder>>,
    inbox: InboxHandle<'_>,
    id: String,
    language: MeetingLanguage,
    target: ProcessingTarget,
) -> Result<String, String> {
    if core.locked() {
        return Err("the app is locked".into());
    }
    let (core, recorder, inbox) = (
        core.inner().clone(),
        recorder.inner().clone(),
        inbox.inner().clone(),
    );
    tauri::async_runtime::spawn_blocking(move || {
        crate::core::import_item(&core, &recorder, &inbox, &id, language, target)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Deletes the item and its file.
#[tauri::command]
#[specta::specta]
pub async fn inbox_dismiss(
    core: ghi_app::CoreState<'_>,
    inbox: InboxHandle<'_>,
    id: String,
) -> Result<(), String> {
    if core.locked() {
        return Err("the app is locked".into());
    }
    let inbox = inbox.inner().clone();
    tauri::async_runtime::spawn_blocking(move || inbox.dismiss(&id))
        .await
        .map_err(|e| e.to_string())?
}
