// SPDX-License-Identifier: Apache-2.0
//! Settings (D11) beyond the plain switches in system.rs: the custom
//! vocabulary, "Export everything" (an encrypted archive) and "Delete all
//! data".

use std::sync::Arc;

use ghi_core::vocab::{self, IGNORED_SETTING, MAX_TERMS, TERMS_SETTING};
use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

use crate::dialogs::{self, LastExport};
use crate::{CoreState, blocking};

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Vocabulary {
    /// The user's own terms (names, products, jargon), as written.
    pub terms: Vec<String>,
    /// Names learned from the speakers the user named (removable).
    pub learned: Vec<String>,
    pub max_terms: u32,
}

fn vocabulary_of(store: &ghi_store::store::Store) -> Result<Vocabulary, String> {
    Ok(Vocabulary {
        terms: vocab::user_terms(store)?,
        learned: vocab::learned_terms(store)?,
        max_terms: MAX_TERMS as u32,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn vocabulary(core: CoreState<'_>) -> Result<Vocabulary, String> {
    blocking(&core, move |c| vocabulary_of(&*c.store()?)).await
}

/// Replaces the user's terms (trimmed, deduplicated, at most 200).
#[tauri::command]
#[specta::specta]
pub async fn set_vocabulary(core: CoreState<'_>, terms: Vec<String>) -> Result<Vocabulary, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let mut out: Vec<String> = Vec::new();
        for t in terms {
            let t: String = t.trim().chars().take(80).collect();
            if !t.is_empty() && !out.iter().any(|o| ghi_text::fold(o) == ghi_text::fold(&t)) {
                out.push(t);
            }
        }
        if out.len() > MAX_TERMS {
            return Err(format!("at most {MAX_TERMS} terms"));
        }
        store
            .set_setting(TERMS_SETTING, &serde_json::json!(out))
            .map_err(|e| e.to_string())?;
        vocabulary_of(&store)
    })
    .await
}

/// Removes a learned name from the vocabulary (it stays removed).
#[tauri::command]
#[specta::specta]
pub async fn ignore_learned_term(core: CoreState<'_>, term: String) -> Result<Vocabulary, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let mut ignored: Vec<String> = store
            .get_setting(IGNORED_SETTING)
            .map_err(|e| e.to_string())?
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let folded = ghi_text::fold(&term);
        if !ignored.iter().any(|i| ghi_text::fold(i) == folded) && ignored.len() < 2000 {
            ignored.push(term);
        }
        store
            .set_setting(IGNORED_SETTING, &serde_json::json!(ignored))
            .map_err(|e| e.to_string())?;
        vocabulary_of(&store)
    })
    .await
}

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

/// "Delete all data": everything goes (meetings, audio, keys, settings),
/// then the app restarts into onboarding. The UI asks for a typed
/// confirmation first.
#[tauri::command]
#[specta::specta]
pub async fn delete_all_data(
    app: AppHandle,
    core: CoreState<'_>,
    imports: tauri::State<'_, Arc<crate::import_cmd::Imports>>,
) -> Result<(), String> {
    imports.cancel_all();
    blocking(&core, move |c| c.delete_everything()).await?;
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
