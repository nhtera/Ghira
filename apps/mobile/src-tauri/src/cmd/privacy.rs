// SPDX-License-Identifier: Apache-2.0
//! Privacy actions (Settings → Privacy); the work is in `privacy_cmd.rs`.
//!
//! - **Retention** is `AppSettings.audio_retention_days` through the shared
//!   `get_settings` / `update_settings`: it deletes *audio* older than N days
//!   and keeps the text. v1 has no whole-meeting retention, so there is no
//!   mobile retention command.
//! - **App lock** is `ghi_app::lock_cmd` (LocalAuthentication, Face ID or the
//!   passcode). The phone locks when it goes to the background, after the
//!   chosen minutes (0: at once; `core.rs`); a recording keeps going and the
//!   UI is covered. The store key is not bound to the Keychain's user presence
//!   in v1 (the lock gates the commands and events, as on the desktop).
//! - **Cloud keys** are `ghi_app::cloud_cmd`.

/// Exports everything as an encrypted archive and presents the system share
/// sheet (the path never reaches the webview). Resolves once the sheet is
/// presented; Swift deletes the archive when the sheet closes.
///
/// `password` seals the archive (at least 8 characters, error
/// `passwordTooShort`): without it the file is useless, and anyone with the
/// file and the password gets every meeting and key.
#[tauri::command]
#[specta::specta]
pub async fn privacy_export_all_share(
    core: ghi_app::CoreState<'_>,
    password: String,
) -> Result<(), String> {
    // Wiped when this call ends, whatever happens.
    let password = zeroize::Zeroizing::new(password);
    ghi_app::blocking(&core, move |c| {
        let store = c.store()?;
        // The export can take a while with a lot of audio.
        let bg = crate::platform::begin_bg_task("export");
        let r = crate::privacy_cmd::export_and_share(
            &store,
            c.data_dir(),
            &password,
            &crate::share::share_file,
        );
        if bg != 0 {
            crate::platform::end_bg_task(bg);
        }
        r
    })
    .await
}

/// Deletes every meeting, key and setting, including the Keychain items.
///
/// `confirm` must be the phrase the user typed in the typed confirmation
/// (`DELETE` or `XÓA`, see `privacy_cmd::DELETE_PHRASES`; case and accents
/// are ignored); Rust checks it again, so a stray call from the webview
/// cannot wipe the phone. Refused while a recording or import runs (`busy`).
#[tauri::command]
#[specta::specta]
pub async fn privacy_delete_all(
    core: ghi_app::CoreState<'_>,
    recorder: tauri::State<'_, std::sync::Arc<crate::session::Recorder>>,
    inbox: tauri::State<'_, std::sync::Arc<crate::inbox::Inbox>>,
    confirm: String,
) -> Result<(), String> {
    let (recorder, inbox) = (recorder.inner().clone(), inbox.inner().clone());
    ghi_app::blocking(&core, move |c| {
        crate::privacy_cmd::delete_all(c, c.data_dir(), inbox.root(), &confirm, &|| {
            recorder.latest().is_some() || inbox.importing()
        })
    })
    .await
}
