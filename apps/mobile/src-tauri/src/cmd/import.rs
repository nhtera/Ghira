// SPDX-License-Identifier: Apache-2.0
//! The share-extension inbox (M5): audio files the extension copied into the
//! App Group `inbox/<uuid>/` with a manifest. Contracts only (16-G).

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

#[tauri::command]
#[specta::specta]
pub async fn inbox_list() -> Result<Vec<InboxItem>, String> {
    Err("not yet".into())
}

/// Imports the item with these choices; returns the new meeting id.
#[tauri::command]
#[specta::specta]
pub async fn inbox_confirm(
    _id: String,
    _language: MeetingLanguage,
    _target: ProcessingTarget,
) -> Result<String, String> {
    Err("not yet".into())
}

/// Deletes the item and its file.
#[tauri::command]
#[specta::specta]
pub async fn inbox_dismiss(_id: String) -> Result<(), String> {
    Err("not yet".into())
}
