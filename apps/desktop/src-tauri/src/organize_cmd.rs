// SPDX-License-Identifier: Apache-2.0
//! Folders and tags (phase 14d, D7): one optional folder per meeting, many
//! tags. Names are plain text, unique ignoring case, limited in length and
//! count. Every command refuses while the app is locked. Errors are codes:
//! `duplicate`, `tooLong`, `limit`, `notFound`, `storage`.
//!
//! W0-B stub: the commands and types are final, the bodies are inert (slice
//! S6): the lists are empty, the rest answer `notImplemented`.

use serde::Serialize;
use specta::Type;

use crate::{CoreState, blocking};

/// Not built yet (the W0-B stubs).
pub(crate) const NOT_IMPLEMENTED: &str = "notImplemented";

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FolderRow {
    pub gid: String,
    pub name: String,
    /// Meetings in it.
    pub meetings: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TagRow {
    pub gid: String,
    pub name: String,
    /// Meetings with it.
    pub meetings: u32,
}

/// All folders, by name.
#[tauri::command]
#[specta::specta]
pub async fn list_folders(core: CoreState<'_>) -> Result<Vec<FolderRow>, String> {
    blocking(&core, |c| {
        c.store()?;
        Ok(Vec::new())
    })
    .await
}

/// Makes a folder. Errors: `duplicate`, `tooLong`, `limit`.
#[tauri::command]
#[specta::specta]
pub async fn create_folder(core: CoreState<'_>, name: String) -> Result<FolderRow, String> {
    blocking(&core, move |_| {
        let _ = name;
        Err(NOT_IMPLEMENTED.into())
    })
    .await
}

/// Renames a folder. Errors: `duplicate`, `tooLong`, `notFound`.
#[tauri::command]
#[specta::specta]
pub async fn rename_folder(
    core: CoreState<'_>,
    folder: String,
    name: String,
) -> Result<(), String> {
    blocking(&core, move |_| {
        let _ = (folder, name);
        Err(NOT_IMPLEMENTED.into())
    })
    .await
}

/// Deletes a folder; its meetings stay (in no folder). Returns how many were
/// in it. Errors: `notFound`.
#[tauri::command]
#[specta::specta]
pub async fn delete_folder(core: CoreState<'_>, folder: String) -> Result<u32, String> {
    blocking(&core, move |_| {
        let _ = folder;
        Err(NOT_IMPLEMENTED.into())
    })
    .await
}

/// Moves meetings into a folder (`null`: out of any folder). Returns how many
/// changed. Errors: `notFound`.
#[tauri::command]
#[specta::specta]
pub async fn move_to_folder(
    core: CoreState<'_>,
    meetings: Vec<String>,
    folder: Option<String>,
) -> Result<u32, String> {
    blocking(&core, move |_| {
        let _ = (meetings, folder);
        Err(NOT_IMPLEMENTED.into())
    })
    .await
}

/// All tags, by name.
#[tauri::command]
#[specta::specta]
pub async fn list_tags(core: CoreState<'_>) -> Result<Vec<TagRow>, String> {
    blocking(&core, |c| {
        c.store()?;
        Ok(Vec::new())
    })
    .await
}

/// Gets the tag with this name, making it if new (so the same name is never
/// two tags). Errors: `tooLong`, `limit`.
#[tauri::command]
#[specta::specta]
pub async fn create_tag(core: CoreState<'_>, name: String) -> Result<TagRow, String> {
    blocking(&core, move |_| {
        let _ = name;
        Err(NOT_IMPLEMENTED.into())
    })
    .await
}

/// Renames a tag. Errors: `duplicate`, `tooLong`, `notFound`.
#[tauri::command]
#[specta::specta]
pub async fn rename_tag(core: CoreState<'_>, tag: String, name: String) -> Result<(), String> {
    blocking(&core, move |_| {
        let _ = (tag, name);
        Err(NOT_IMPLEMENTED.into())
    })
    .await
}

/// Deletes a tag from every meeting. Returns how many had it. Errors:
/// `notFound`.
#[tauri::command]
#[specta::specta]
pub async fn delete_tag(core: CoreState<'_>, tag: String) -> Result<u32, String> {
    blocking(&core, move |_| {
        let _ = tag;
        Err(NOT_IMPLEMENTED.into())
    })
    .await
}

/// Adds a tag to meetings. Returns how many gained it. Errors: `limit` (20
/// tags per meeting), `notFound`.
#[tauri::command]
#[specta::specta]
pub async fn tag_meetings(
    core: CoreState<'_>,
    meetings: Vec<String>,
    tag: String,
) -> Result<u32, String> {
    blocking(&core, move |_| {
        let _ = (meetings, tag);
        Err(NOT_IMPLEMENTED.into())
    })
    .await
}

/// Removes a tag from meetings. Returns how many lost it. Errors: `notFound`.
#[tauri::command]
#[specta::specta]
pub async fn untag_meetings(
    core: CoreState<'_>,
    meetings: Vec<String>,
    tag: String,
) -> Result<u32, String> {
    blocking(&core, move |_| {
        let _ = (meetings, tag);
        Err(NOT_IMPLEMENTED.into())
    })
    .await
}
