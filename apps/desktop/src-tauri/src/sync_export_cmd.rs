// SPDX-License-Identifier: Apache-2.0
//! "Export for another device" (doc 07 §10, slice 15-M): the fallback when a
//! phone can't be reached over the network. The save and open dialogs are
//! Rust's (`rfd`); the webview sends the passphrase and gets file names and
//! counts back, never a path. The sealed archive itself is `ghi_sync::export`
//! (see `ghi_app::export_cmd::device_export`).

use std::sync::Arc;

use ghi_app::export_cmd::{DeviceTransfer, ERR_PASSPHRASE_SHORT, device_export, device_import};
use tauri::AppHandle;
use zeroize::Zeroizing;

use crate::dialogs::{self, LastExport};
use crate::settings_cmd::today;
use crate::{CoreState, blocking};

/// File extension of the sealed archive.
const EXTENSION: &str = "ghix";
/// The same minimum as "Export everything".
const MIN_PASSPHRASE_CHARS: usize = 8;

/// Seals the given meetings (every finished one when `meeting_gids` is
/// `None`) with `passphrase` into a file the user picks (a save dialog).
/// `None`: the dialog was cancelled.
#[tauri::command]
#[specta::specta]
pub async fn sync_export_for_device(
    app: AppHandle,
    core: CoreState<'_>,
    last: tauri::State<'_, Arc<LastExport>>,
    meeting_gids: Option<Vec<String>>,
    passphrase: String,
) -> Result<Option<DeviceTransfer>, String> {
    let passphrase = Zeroizing::new(passphrase);
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(ERR_PASSPHRASE_SHORT.into());
    }
    let name = format!("Ghira transfer {}.{EXTENSION}", today());
    let Some(path) = dialogs::save_file(&app, name, EXTENSION, None).await? else {
        return Ok(None);
    };
    let last = last.inner().clone();
    blocking(&core, move |c| {
        let done = device_export(&*c.store()?, meeting_gids.as_deref(), &passphrase, &path)?;
        *last.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
        Ok(Some(done))
    })
    .await
}

/// Merges a sealed export from another device (an open dialog first) into
/// this library with `passphrase`. `None`: the dialog was cancelled. A wrong
/// passphrase changes nothing.
#[tauri::command]
#[specta::specta]
pub async fn sync_import_from_device(
    app: AppHandle,
    core: CoreState<'_>,
    passphrase: String,
    title: String,
) -> Result<Option<DeviceTransfer>, String> {
    let passphrase = Zeroizing::new(passphrase);
    let Some(path) = dialogs::pick_file(&app, title, EXTENSION).await? else {
        return Ok(None);
    };
    blocking(&core, move |c| {
        device_import(&*c.store()?, &path, &passphrase).map(Some)
    })
    .await
}
