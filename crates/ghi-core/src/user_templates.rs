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

/// The stored list, entry by entry. An entry this version cannot read (a
/// newer app wrote it, or a rule got stricter) is kept as it is and only left
/// out of what is shown: saving never drops what it did not understand.
pub struct Records {
    entries: Vec<Entry>,
}

enum Entry {
    Known(UserTemplate),
    Unread(serde_json::Value),
}

impl Records {
    /// Reads the list. A stored value that is not a list at all is an error:
    /// nothing may be written over it.
    pub fn load(store: &Store) -> Result<Records, String> {
        let Some(v) = store.get_setting(KEY).map_err(|e| e.to_string())? else {
            return Ok(Records { entries: Vec::new() });
        };
        let serde_json::Value::Array(list) = v else {
            return Err("the stored templates could not be read, so they were left as they are".into());
        };
        let entries = list
            .into_iter()
            .map(|raw| match serde_json::from_value::<UserTemplate>(raw.clone()) {
                Ok(u) => Entry::Known(u),
                Err(_) => Entry::Unread(raw),
            })
            .collect();
        Ok(Records { entries })
    }

    /// The templates that can be used: known and valid under today's rules.
    pub fn usable(&self) -> Vec<&UserTemplate> {
        self.entries
            .iter()
            .filter_map(|e| match e {
                Entry::Known(u) if u.template().is_some() => Some(u),
                _ => None,
            })
            .collect()
    }

    /// How many entries there are, readable or not (the cap counts them all).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every gid in use (an unreadable entry's too, if it has one).
    pub fn gids(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|e| match e {
                Entry::Known(u) => Some(u.gid.clone()),
                Entry::Unread(v) => v.get("gid").and_then(|g| g.as_str()).map(str::to_string),
            })
            .collect()
    }

    pub fn find_mut(&mut self, gid: &str) -> Option<&mut UserTemplate> {
        self.entries.iter_mut().find_map(|e| match e {
            Entry::Known(u) if u.gid == gid && u.template().is_some() => Some(u),
            _ => None,
        })
    }

    pub fn push(&mut self, u: UserTemplate) {
        self.entries.push(Entry::Known(u));
    }

    /// Removes a usable template; false if there was none.
    pub fn remove(&mut self, gid: &str) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|e| !matches!(e, Entry::Known(u) if u.gid == gid));
        self.entries.len() != before
    }

    pub fn save(&self, store: &Store) -> Result<(), String> {
        let list: Vec<serde_json::Value> = self
            .entries
            .iter()
            .map(|e| match e {
                Entry::Known(u) => serde_json::to_value(u).map_err(|e| e.to_string()),
                Entry::Unread(v) => Ok(v.clone()),
            })
            .collect::<Result<_, _>>()?;
        store
            .set_setting(KEY, &serde_json::Value::Array(list))
            .map_err(|e| e.to_string())
    }
}

/// The user's usable templates.
pub fn load(store: &Store) -> Result<Vec<UserTemplate>, String> {
    Ok(Records::load(store)?.usable().into_iter().cloned().collect())
}

/// The template behind `user:<gid>`, if it exists and is usable.
pub fn find(store: &Store, id: &str) -> Option<Template> {
    let gid = gid_of(id)?;
    load(store)
        .ok()?
        .into_iter()
        .find(|u| u.gid == gid)
        .and_then(|u| u.template())
}

/// A template by id: a built-in, or `user:<gid>` (this device's own).
pub fn template_of(store: &Store, id: &str) -> Option<Template> {
    if gid_of(id).is_some() {
        find(store, id)
    } else {
        ghi_llm::template::builtin(id).ok()
    }
}

/// A section of the notes as it is listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListedSection {
    pub id: String,
    pub title_en: String,
    pub title_vi: String,
}

/// The sections of a meeting's notes: the template's own, then any `section:<id>`
/// among the block `kinds` that it does not list (it was edited or deleted, or
/// the notes came from a device with other templates). Their titles are the
/// ids as words. One rule for the notes tab, the exports and the phone.
pub fn sections_for<'a>(
    store: &Store,
    template: Option<&str>,
    kinds: impl IntoIterator<Item = &'a str>,
) -> Vec<ListedSection> {
    let mut out: Vec<ListedSection> = template_of(store, template.unwrap_or("general"))
        .map(|t| {
            t.sections
                .into_iter()
                .map(|s| ListedSection {
                    id: s.id,
                    title_en: s.title_en,
                    title_vi: s.title_vi,
                })
                .collect()
        })
        .unwrap_or_default();
    for kind in kinds {
        let Some(id) = kind.strip_prefix("section:") else {
            continue;
        };
        if !out.iter().any(|s| s.id == id) {
            let title = ghi_llm::template::humanize_id(id);
            out.push(ListedSection {
                id: id.to_string(),
                title_en: title.clone(),
                title_vi: title,
            });
        }
    }
    out
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

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(
            dir.path(),
            std::sync::Arc::new(ghi_store::keys::MemoryKeyStore::default()),
            ghi_store::keys::Protection::default(),
        )
        .unwrap();
        (dir, store)
    }

    #[test]
    fn saved_templates_are_found_by_their_user_id() {
        let (_d, store) = store();
        assert!(load(&store).unwrap().is_empty());
        let mut r = Records::load(&store).unwrap();
        r.push(record("t1"));
        r.save(&store).unwrap();
        assert_eq!(load(&store).unwrap().len(), 1);
        assert_eq!(find(&store, "user:t1").unwrap().name, "Retro");
        assert!(find(&store, "user:nope").is_none());
        assert!(find(&store, "general").is_none());
        assert_eq!(template_of(&store, "general").unwrap().id, "general");
        assert_eq!(template_of(&store, "user:t1").unwrap().name, "Retro");
        assert!(!ghi_store::sync::settings::is_synced_key(KEY), "stays on this device");
    }

    #[test]
    fn what_this_version_cannot_read_is_kept_when_saving_and_never_dropped() {
        let (_d, store) = store();
        // A record from a newer app (an unknown shape), a record that parses but breaks
        // today's rules (nine sections), and one good one.
        let nine: String = (0..9)
            .map(|i| format!("[[section]]\nid = \"s{i}\"\ntitle_en = \"t\"\ntitle_vi = \"t\"\ninstruction = \"i\"\n"))
            .collect();
        let big = UserTemplate {
            gid: "t9".into(),
            lang: "en".into(),
            toml: format!("id = \"t9\"\nname = \"Big\"\nguidance_en = \"\"\nguidance_vi = \"\"\n{nine}"),
            retired: vec![],
        };
        assert!(big.template().is_none(), "over the cap today");
        let future = serde_json::json!({ "gid": "tf", "shape": "newer", "sections": 3 });
        store
            .set_setting(
                KEY,
                &serde_json::json!([future, serde_json::to_value(&big).unwrap(), serde_json::to_value(record("t1")).unwrap()]),
            )
            .unwrap();
        let mut r = Records::load(&store).unwrap();
        assert_eq!(r.usable().len(), 1, "only the good one shows");
        assert_eq!(r.len(), 3);
        assert_eq!(r.gids(), ["tf", "t9", "t1"]);
        // Edit and add through the same list, save: nothing is lost.
        r.push(record("t2"));
        assert!(r.find_mut("t9").is_none(), "an unusable one is not editable");
        assert!(!r.remove("tf"), "an unreadable one is not deletable either");
        r.save(&store).unwrap();
        let raw = store.get_setting(KEY).unwrap().unwrap();
        let gids: Vec<&str> = raw.as_array().unwrap().iter().map(|v| v["gid"].as_str().unwrap()).collect();
        assert_eq!(gids, ["tf", "t9", "t1", "t2"]);
        assert_eq!(raw[0], future);
    }

    #[test]
    fn a_stored_value_that_is_not_a_list_is_never_written_over() {
        let (_d, store) = store();
        store.set_setting(KEY, &serde_json::json!({ "not": "a list" })).unwrap();
        assert!(Records::load(&store).is_err());
        assert!(load(&store).is_err());
        assert!(find(&store, "user:t1").is_none());
        // untouched
        assert_eq!(store.get_setting(KEY).unwrap().unwrap()["not"], "a list");
    }

    #[test]
    fn sections_come_from_the_template_then_the_blocks_the_template_does_not_list() {
        let (_d, store) = store();
        let mut r = Records::load(&store).unwrap();
        r.push(record("t1"));
        r.save(&store).unwrap();
        let ids = |t: Option<&str>, kinds: &[&str]| -> Vec<String> {
            sections_for(&store, t, kinds.iter().copied()).into_iter().map(|s| s.id).collect()
        };
        assert_eq!(ids(Some("user:t1"), &["section:went_well", "decision", "section:old_one"]), ["went_well", "old_one"]);
        assert_eq!(ids(Some("user:gone"), &["section:went_well"]), ["went_well"]);
        assert_eq!(ids(Some("standup"), &["section:done", "section:x_y"]), ["done", "next", "blockers", "x_y"]);
        let s = sections_for(&store, Some("user:gone"), ["section:went_well"]);
        assert_eq!(s[0].title_en, "Went well");
    }
}
