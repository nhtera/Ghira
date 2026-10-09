// SPDX-License-Identifier: Apache-2.0
//! Settings (D11) beyond the plain switches in `system`: the custom
//! vocabulary and the final pass's transcription engine.

use ghi_core::vocab::{self, IGNORED_SETTING, MAX_TERMS, TERMS_SETTING};
use serde::{Deserialize, Serialize};
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
    /// The bundled glossary packs, with the ones switched on.
    pub packs: Vec<PackInfo>,
}

/// A bundled glossary pack (`<domain>-<lang>`; the UI names it by id).
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PackInfo {
    pub id: String,
    pub domain: String,
    pub lang: String,
    pub terms: u32,
    pub enabled: bool,
}

fn vocabulary_of(store: &ghi_store::store::Store) -> Result<Vocabulary, String> {
    let on = vocab::enabled_packs(store)?;
    Ok(Vocabulary {
        terms: vocab::user_terms(store)?,
        learned: vocab::learned_terms(store)?,
        max_terms: MAX_TERMS as u32,
        packs: vocab::packs()
            .iter()
            .map(|p| PackInfo {
                id: p.id.clone(),
                domain: p.domain.clone(),
                lang: p.lang.clone(),
                terms: p.terms.len() as u32,
                enabled: on.contains(&p.id),
            })
            .collect(),
    })
}

/// Switches the glossary packs on (the ids listed) and the rest off. Unknown
/// ids are refused; the choice syncs to the paired devices.
#[tauri::command]
#[specta::specta]
pub async fn set_vocabulary_packs(
    core: CoreState<'_>,
    ids: Vec<String>,
) -> Result<Vocabulary, String> {
    blocking(&core, move |c| store_packs(&*c.store()?, ids)).await
}

pub(crate) fn store_packs(
    store: &ghi_store::store::Store,
    ids: Vec<String>,
) -> Result<Vocabulary, String> {
    if let Some(bad) = ids
        .iter()
        .find(|i| !ghi_store::sync::settings::PACK_IDS.contains(&i.as_str()))
    {
        return Err(format!("unknown glossary pack {bad}"));
    }
    // The known ids asked for, in pack order, once each; then the ids this
    // build does not know that are already stored (a newer device's packs), so
    // saving here does not wipe them through last-writer-wins.
    let stored: Vec<String> = store
        .get_setting(vocab::PACKS_SETTING)
        .map_err(|e| e.to_string())?
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let mut out: Vec<String> = ghi_store::sync::settings::PACK_IDS
        .into_iter()
        .filter(|p| ids.iter().any(|i| i == p))
        .map(String::from)
        .collect();
    for id in stored {
        if !ghi_store::sync::settings::PACK_IDS.contains(&id.as_str())
            && ghi_store::sync::settings::is_pack_id(&id)
            && !out.contains(&id)
        {
            out.push(id);
        }
    }
    let value = serde_json::json!(out);
    store
        .set_setting(vocab::PACKS_SETTING, &value)
        .map_err(|e| e.to_string())?;
    sync_setting(store, vocab::PACKS_SETTING, &value);
    vocabulary_of(store)
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
    blocking(&core, move |c| store_terms(&*c.store()?, terms)).await
}

pub(crate) fn store_terms(
    store: &ghi_store::store::Store,
    terms: Vec<String>,
) -> Result<Vocabulary, String> {
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
    let value = serde_json::json!(out);
    store
        .set_setting(TERMS_SETTING, &value)
        .map_err(|e| e.to_string())?;
    sync_setting(store, TERMS_SETTING, &value);
    vocabulary_of(store)
}

/// Records a vocabulary change for the paired devices (the allowlist, last
/// writer wins). The local change stands whatever happens here.
fn sync_setting(store: &ghi_store::store::Store, key: &str, value: &serde_json::Value) {
    if let Err(e) = store.put_synced(key, &value.to_string()) {
        log::warn!("synced setting {key} not recorded: {e}");
    }
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
        let value = serde_json::json!(ignored);
        store
            .set_setting(IGNORED_SETTING, &value)
            .map_err(|e| e.to_string())?;
        sync_setting(&store, IGNORED_SETTING, &value);
        vocabulary_of(&store)
    })
    .await
}
// ------------------------------------------------- transcription engine

/// The recognizer of the transcript written after a meeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AsrEngine {
    /// Nemotron, the default: fast, best on phone-quality and mixed speech.
    Nemo,
    /// Whisper large-v3-turbo: slower, an extra download; Nemotron reads
    /// what it leaves out.
    Whisper,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionEngine {
    pub engine: AsrEngine,
    /// This build can use Whisper (the computer; never the phone).
    pub whisper_available: bool,
    /// Whisper's models are downloaded (until then the transcript uses Nemotron).
    pub whisper_installed: bool,
    /// Bytes Whisper's models take.
    pub whisper_bytes: f64,
}

fn engine_of(store: &ghi_store::store::Store, models: &std::path::Path) -> TranscriptionEngine {
    use crate::core::{WHISPER_BUILT, WHISPER_MODELS};
    TranscriptionEngine {
        engine: if WHISPER_BUILT && crate::core::whisper_chosen(store) {
            AsrEngine::Whisper
        } else {
            AsrEngine::Nemo
        },
        whisper_available: WHISPER_BUILT,
        whisper_installed: ghi_models::installed(models, &WHISPER_MODELS),
        whisper_bytes: WHISPER_MODELS
            .iter()
            .filter_map(|id| ghi_models::find(id))
            .map(|m| m.size as f64)
            .sum(),
    }
}

/// Stores the engine choice. Whisper needs a build that has it.
pub fn store_engine(
    store: &ghi_store::store::Store,
    models: &std::path::Path,
    engine: AsrEngine,
) -> Result<TranscriptionEngine, String> {
    let value = match engine {
        AsrEngine::Whisper if !crate::core::WHISPER_BUILT => {
            return Err("this build has no Whisper".into());
        }
        AsrEngine::Whisper => "whisper",
        AsrEngine::Nemo => "nemo",
    };
    store
        .set_setting(crate::core::ASR_FINAL_KEY, &serde_json::json!(value))
        .map_err(|e| e.to_string())?;
    Ok(engine_of(store, models))
}

#[tauri::command]
#[specta::specta]
pub async fn transcription_engine(core: CoreState<'_>) -> Result<TranscriptionEngine, String> {
    blocking(&core, move |c| Ok(engine_of(&*c.store()?, &c.models()))).await
}

/// Chooses the engine for transcripts written from now on (and for Transcribe
/// again). Whisper's models are then listed with the others to download.
#[tauri::command]
#[specta::specta]
pub async fn set_transcription_engine(
    core: CoreState<'_>,
    engine: AsrEngine,
) -> Result<TranscriptionEngine, String> {
    blocking(&core, move |c| {
        store_engine(&*c.store()?, &c.models(), engine)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_store::keys::{MemoryKeyStore, Protection};

    #[test]
    fn the_engine_choice_is_stored_and_needs_a_whisper_build() {
        let t = tempfile::tempdir().unwrap();
        let store = ghi_store::store::Store::open(
            t.path(),
            std::sync::Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        let models = t.path().join("models");
        let e = engine_of(&store, &models);
        assert_eq!(e.engine, AsrEngine::Nemo, "the default");
        assert!(!e.whisper_installed);
        assert!(e.whisper_bytes > 500e6);
        assert!(crate::core::wanted_optional_models(&store).is_empty());

        let chose = store_engine(&store, &models, AsrEngine::Whisper);
        if crate::core::WHISPER_BUILT {
            assert_eq!(chose.unwrap().engine, AsrEngine::Whisper);
            assert_eq!(
                crate::core::wanted_optional_models(&store),
                crate::core::WHISPER_MODELS
            );
        } else {
            assert!(chose.is_err());
            assert_eq!(engine_of(&store, &models).engine, AsrEngine::Nemo);
        }
        let back = store_engine(&store, &models, AsrEngine::Nemo).unwrap();
        assert_eq!(back.engine, AsrEngine::Nemo);
        assert!(crate::core::wanted_optional_models(&store).is_empty());
    }

    fn open() -> (tempfile::TempDir, ghi_store::store::Store) {
        let t = tempfile::tempdir().unwrap();
        let store = ghi_store::store::Store::open(
            t.path(),
            std::sync::Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        (t, store)
    }

    #[test]
    fn glossary_packs_are_validated_stored_and_synced() {
        let (_t, store) = open();
        let v = vocabulary_of(&store).unwrap();
        assert_eq!(v.packs.len(), ghi_store::sync::settings::PACK_IDS.len());
        assert!(v.packs.iter().all(|p| !p.enabled && p.terms > 0));

        let v = store_packs(
            &store,
            vec!["tech-en".into(), "medical-vi".into(), "tech-en".into()],
        )
        .unwrap();
        let on: Vec<&str> = v
            .packs
            .iter()
            .filter(|p| p.enabled)
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(on, ["medical-vi", "tech-en"], "pack order, once each");
        assert_eq!(
            store.get_setting(vocab::PACKS_SETTING).unwrap().unwrap(),
            serde_json::json!(["medical-vi", "tech-en"])
        );
        // The final pass reads the same setting.
        assert_eq!(
            vocab::enabled_packs(&store).unwrap(),
            ["medical-vi", "tech-en"]
        );
        let synced = store
            .changes_since(0, 256)
            .unwrap()
            .changes
            .into_iter()
            .any(|c| {
                matches!(&c.record, ghi_store::sync::records::Record::Setting(s)
                    if s.key == vocab::PACKS_SETTING
                        && s.value_json.as_deref() == Some("[\"medical-vi\",\"tech-en\"]"))
            });
        assert!(synced, "the choice is in the change feed");

        // Unknown ids change nothing.
        for bad in [
            vec!["klingon-en".to_string()],
            vec!["tech-en".into(), "x".into()],
        ] {
            assert!(store_packs(&store, bad).is_err());
        }
        assert_eq!(
            vocab::enabled_packs(&store).unwrap(),
            ["medical-vi", "tech-en"]
        );
        // Off again.
        let v = store_packs(&store, vec![]).unwrap();
        assert!(v.packs.iter().all(|p| !p.enabled));
    }

    #[test]
    fn packs_from_a_newer_device_survive_a_save_here() {
        let (_t, store) = open();
        store
            .set_setting(
                vocab::PACKS_SETTING,
                &serde_json::json!(["klingon-en", "tech-en", "Bad Id"]),
            )
            .unwrap();
        let v = store_packs(&store, vec!["medical-vi".into()]).unwrap();
        let on: Vec<&str> = v
            .packs
            .iter()
            .filter(|p| p.enabled)
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(on, ["medical-vi"], "tech-en was switched off here");
        assert_eq!(
            store.get_setting(vocab::PACKS_SETTING).unwrap().unwrap(),
            serde_json::json!(["medical-vi", "klingon-en"]),
            "the unknown pack stays, the malformed id goes"
        );
        // Still only the ids this build knows are applied.
        assert_eq!(vocab::enabled_packs(&store).unwrap(), ["medical-vi"]);
    }
}
