// SPDX-License-Identifier: Apache-2.0
//! Settings (D11) beyond the plain switches in system.rs: the custom
//! vocabulary (in `ghi-app`), "Export everything" (an encrypted archive) and
//! "Delete all data".

use std::sync::Arc;

use tauri::AppHandle;

pub use ghi_app::settings_cmd::*;

use crate::dialogs::{self, LastExport};
use crate::{CoreState, blocking};

/// Minimum password length for an export archive.
const MIN_PASSWORD_CHARS: usize = 8;

/// "Export everything": every meeting with its audio in one archive,
/// encrypted with `password` (a save dialog first). Returns the file name, or
/// `None` if the user cancelled.
#[tauri::command]
#[specta::specta]
pub async fn export_everything(
    app: AppHandle,
    core: CoreState<'_>,
    last: tauri::State<'_, Arc<LastExport>>,
    password: String,
) -> Result<Option<String>, String> {
    let password = zeroize::Zeroizing::new(password);
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(format!("use at least {MIN_PASSWORD_CHARS} characters"));
    }
    let name = format!("Ghira export {}.ghira", today());
    let Some(path) = dialogs::save_file(&app, name, "ghira", None).await? else {
        return Ok(None);
    };
    let last = last.inner().clone();
    blocking(&core, move |c| {
        if c.recording() {
            return Err("stop the recording first".into());
        }
        c.store()?
            .export_all(&path, &password)
            .map_err(|e| e.to_string())?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        *last.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
        Ok(Some(name))
    })
    .await
}

/// Today as `YYYY-MM-DD` (UTC).
fn today() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    // Civil date from days since 1970 (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// "Delete all data": everything goes (meetings, audio, keys, settings), then
/// the app restarts into onboarding. The UI asks for a typed confirmation
/// first. With `everywhere` and paired phones, each is queued to delete what
/// it got from this computer and this waits for the phones seen a moment ago
/// (`sync_delete_everywhere_status` says for whom; `sync_delete_everywhere_skip`
/// ends the wait); the sync identity is destroyed with the data.
#[tauri::command]
#[specta::specta]
pub async fn delete_all_data(
    app: AppHandle,
    core: CoreState<'_>,
    imports: tauri::State<'_, Arc<crate::import_cmd::Imports>>,
    sync: tauri::State<'_, Arc<ghi_app::sync_service::SyncService>>,
    everywhere: bool,
) -> Result<(), String> {
    imports.cancel_all();
    let sync = sync.inner().clone();
    blocking(&core, move |c| {
        c.delete_everything_with(&|| sync.delete_everywhere_prepare(everywhere))
    })
    .await?;
    app.restart();
}

#[cfg(test)]
mod tests {
    #[test]
    fn today_is_a_date() {
        let t = super::today();
        assert_eq!(t.len(), 10);
        assert!(t.starts_with("20"));
        assert_eq!(&t[4..5], "-");
    }
}
