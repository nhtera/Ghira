// SPDX-License-Identifier: Apache-2.0
//! Settings (D11) beyond the plain switches in `system`: the custom
//! vocabulary.

use ghi_core::vocab::{self, IGNORED_SETTING, MAX_TERMS, TERMS_SETTING};
use serde::Serialize;
use specta::Type;

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
