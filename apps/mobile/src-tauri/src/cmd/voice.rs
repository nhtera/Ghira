// SPDX-License-Identifier: Apache-2.0
//! "Me" voice enrollment on the phone (M1 optional step, Settings → Voice
//! profile). Contracts only (16-G). `voice_status` is `ghi_app::voice_cmd`.
//! Consent must be given before the passage is read; without it nothing is stored.

#[tauri::command]
#[specta::specta]
pub async fn voice_set_consent(_given: bool) -> Result<(), String> {
    Err("not yet".into())
}

/// Starts capturing the passage. Needs consent and the voice model.
#[tauri::command]
#[specta::specta]
pub async fn voice_enroll_start() -> Result<(), String> {
    Err("not yet".into())
}

/// Finishes: stores the profile. Errors are the `ghi_app::voice_cmd` codes.
#[tauri::command]
#[specta::specta]
pub async fn voice_enroll_stop() -> Result<(), String> {
    Err("not yet".into())
}

/// Cancels and wipes the buffered audio.
#[tauri::command]
#[specta::specta]
pub async fn voice_enroll_cancel() -> Result<(), String> {
    Err("not yet".into())
}
