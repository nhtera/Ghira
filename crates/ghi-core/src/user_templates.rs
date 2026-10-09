// SPDX-License-Identifier: Apache-2.0
//! Note templates the user made (desktop only, device-local: the setting
//! `templates.user` is not one of the synced keys). Stored as a list of
//! records; each holds the template as TOML, the language its text is in and
//! the section ids it once had and gave up (never reused).
//!
//! The id a meeting and a regenerate use is `user:<gid>`. The notes job and
//! the app both read templates through [`find`].

use ghi_llm::template::{OutLang, Template};
use ghi_store::store::Store;
use serde::{Deserialize, Serialize};

/// The settings key of the list.
pub const KEY: &str = "templates.user";
/// Templates a user may have.
pub const MAX_USER_TEMPLATES: usize = 20;
/// Prefix of a user template's id (`user:<gid>`).
pub const PREFIX: &str = "user:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserTemplate {
    /// `t` + hex: also the template's own `id`.
    pub gid: String,
    /// The language the text is written in (`en` / `vi`).
    pub lang: String,
    pub toml: String,
    /// Section ids removed from the template: never given to a new section.
    #[serde(default)]
    pub retired: Vec<String>,
}

impl UserTemplate {
    pub fn template(&self) -> Option<Template> {
        Template::from_toml(&self.toml).ok()
    }

    pub fn out_lang(&self) -> OutLang {
        if self.lang == "vi" {
            OutLang::Vi
        } else {
            OutLang::En
        }
    }

    /// `user:<gid>`.
    pub fn id(&self) -> String {
        format!("{PREFIX}{}", self.gid)
    }
}

/// The gid in a `user:<gid>` id.
pub fn gid_of(id: &str) -> Option<&str> {
    id.strip_prefix(PREFIX)
}

/// The user's templates (an entry that no longer parses is left out).
pub fn load(store: &Store) -> Result<Vec<UserTemplate>, String> {
    let Some(v) = store.get_setting(KEY).map_err(|e| e.to_string())? else {
        return Ok(Vec::new());
    };
    let list: Vec<UserTemplate> = serde_json::from_value(v).unwrap_or_default();
    Ok(list.into_iter().filter(|u| u.template().is_some()).collect())
}

pub fn save(store: &Store, list: &[UserTemplate]) -> Result<(), String> {
    let v = serde_json::to_value(list).map_err(|e| e.to_string())?;
    store.set_setting(KEY, &v).map_err(|e| e.to_string())
}

/// The template behind `user:<gid>`, if it exists.
pub fn find(store: &Store, id: &str) -> Option<Template> {
    let gid = gid_of(id)?;
    load(store)
        .ok()?
        .into_iter()
        .find(|u| u.gid == gid)
        .and_then(|u| u.template())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_llm::template::{Editor, EditorSection};

    fn record(gid: &str) -> UserTemplate {
        let t = Template::from_editor(
            gid,
            &Editor {
                name: "Retro".into(),
                lang: OutLang::En,
                guidance: String::new(),
                sections: vec![EditorSection {
                    id: None,
                    title: "Went well".into(),
                    instruction: "What worked.".into(),
                }],
            },
            &[],
            &[],
        )
        .unwrap();
        UserTemplate {
            gid: gid.into(),
            lang: "en".into(),
            toml: t.to_toml(),
            retired: vec![],
        }
    }

    #[test]
    fn saved_templates_are_found_by_their_user_id_and_a_broken_entry_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(
            dir.path(),
            std::sync::Arc::new(ghi_store::keys::MemoryKeyStore::default()),
            ghi_store::keys::Protection::default(),
        )
        .unwrap();
        assert!(load(&store).unwrap().is_empty());
        let mut broken = record("t2");
        broken.toml = "not toml {{".into();
        save(&store, &[record("t1"), broken]).unwrap();
        assert_eq!(load(&store).unwrap().len(), 1);
        assert_eq!(find(&store, "user:t1").unwrap().name, "Retro");
        assert!(find(&store, "user:t2").is_none());
        assert!(find(&store, "user:nope").is_none());
        assert!(find(&store, "general").is_none());
        assert!(!ghi_store::sync::settings::is_synced_key(KEY), "stays on this device");
    }
}
