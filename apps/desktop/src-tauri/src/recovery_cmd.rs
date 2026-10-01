// SPDX-License-Identifier: Apache-2.0
//! The optional 24-word recovery key (onboarding, doc 05 §2.2): created,
//! shown once, then confirmed by the user typing some words back before it is
//! stored. The phrase lives only in memory until confirmed and is never
//! logged; it is dropped on confirm or cancel.

use std::sync::{Arc, Mutex};

use ghi_store::recovery::RecoveryPhrase;

use crate::{CoreState, blocking};

#[derive(Default)]
pub struct PendingRecovery(Mutex<Option<RecoveryPhrase>>);

/// Whether a recovery key is set up.
#[tauri::command]
#[specta::specta]
pub async fn has_recovery_key(core: CoreState<'_>) -> Result<bool, String> {
    blocking(&core, |c| Ok(c.store()?.has_recovery_phrase())).await
}

/// A new phrase to write down (not stored until confirmed).
#[tauri::command]
#[specta::specta]
pub fn create_recovery_key(pending: tauri::State<'_, Arc<PendingRecovery>>) -> Vec<String> {
    let phrase = RecoveryPhrase::generate();
    let words = phrase.words().into_iter().map(String::from).collect();
    *pending.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(phrase);
    words
}

/// Stores the pending phrase if `words` (the whole phrase, typed back) match.
#[tauri::command]
#[specta::specta]
pub async fn confirm_recovery_key(
    core: CoreState<'_>,
    pending: tauri::State<'_, Arc<PendingRecovery>>,
    words: Vec<String>,
) -> Result<bool, String> {
    let pending = pending.inner().clone();
    blocking(&core, move |c| {
        let mut slot = pending.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(phrase) = slot.as_ref() else {
            return Err("no recovery key to confirm".into());
        };
        let typed = RecoveryPhrase::parse(&words.join(" "));
        if !typed.is_ok_and(|t| t.words() == phrase.words()) {
            return Ok(false);
        }
        c.store()?
            .set_recovery_phrase(phrase)
            .map_err(|e| e.to_string())?;
        *slot = None;
        Ok(true)
    })
    .await
}

/// Forgets a phrase that was shown but not confirmed.
#[tauri::command]
#[specta::specta]
pub fn cancel_recovery_key(pending: tauri::State<'_, Arc<PendingRecovery>>) {
    *pending.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
}
