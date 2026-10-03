// SPDX-License-Identifier: Apache-2.0
//! Privacy actions (Settings → Privacy). Contracts only (16-G).
//!
//! - **Retention** is `AppSettings.audio_retention_days` through the shared
//!   `get_settings` / `update_settings`: it deletes *audio* older than N days
//!   and keeps the text. v1 has no whole-meeting retention, so there is no
//!   mobile retention command.
//! - **App lock** is `ghi_app::lock_cmd`, which is macOS-only today
//!   (LocalAuthentication under `cfg(target_os = "macos")`). 16-G must extend
//!   it to iOS: lock on background through the lifecycle (immediately when not
//!   recording), and bind the store key to the Keychain with user presence so
//!   Face ID is needed to read it. Until then the commands exist but cannot lock.
//! - **Cloud keys** are `ghi_app::cloud_cmd`.

/// Exports everything as an encrypted archive and presents the system share
/// sheet (the path never reaches the webview). Resolves when the sheet closes.
#[tauri::command]
#[specta::specta]
pub async fn privacy_export_all_share() -> Result<(), String> {
    Err("not yet".into())
}

/// Deletes every meeting, key and setting, including the Keychain items.
///
/// `confirm` must be the exact phrase the user typed in the typed confirmation
/// (`mobile.privacy.deleteAll.phrase` in the current language); Rust checks it
/// again, so a stray call from the webview cannot wipe the phone.
#[tauri::command]
#[specta::specta]
pub async fn privacy_delete_all(_confirm: String) -> Result<(), String> {
    Err("not yet".into())
}
