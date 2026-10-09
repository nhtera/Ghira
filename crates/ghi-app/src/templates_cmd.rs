// SPDX-License-Identifier: Apache-2.0
//! Templates the user makes (desktop only; Settings → Templates).
//!
//! They live in the device-local setting `templates.user` (not synced, at
//! most [`ghi_core::user_templates::MAX_USER_TEMPLATES`]), are checked by the
//! same parser as the built-in files, and are used as `user:<gid>`. Like the
//! other content commands they refuse while the app is locked.

use ghi_core::user_templates::{self as store_of, Records, UserTemplate};
use ghi_llm::template::{Editor, EditorSection, OutLang, Template};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::core::Core;
use crate::{CoreState, blocking};

/// What the editor shows and sends.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TemplateForm {
    pub name: String,
    /// The language the text is written in: `en` or `vi`.
    pub language: String,
    /// A line on what the meetings are, for the model (optional).
    pub guidance: String,
    pub sections: Vec<FormSection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FormSection {
    /// Set for a section the template already has (kept for good); none for a new one.
    pub id: Option<String>,
    pub title: String,
    pub instruction: String,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UserTemplateView {
    /// `user:<gid>`.
    pub id: String,
    pub form: TemplateForm,
}

fn lang_of(s: &str) -> Result<OutLang, String> {
    match s {
        "en" => Ok(OutLang::En),
        "vi" => Ok(OutLang::Vi),
        _ => Err("the language is en or vi".into()),
    }
}

fn editor_of(form: &TemplateForm) -> Result<Editor, String> {
    Ok(Editor {
        name: form.name.clone(),
        lang: lang_of(&form.language)?,
        guidance: form.guidance.clone(),
        sections: form
            .sections
            .iter()
            .map(|s| EditorSection {
                id: s.id.clone(),
                title: s.title.clone(),
                instruction: s.instruction.clone(),
            })
            .collect(),
    })
}

fn form_of(t: &Template, lang: OutLang) -> TemplateForm {
    let e = t.to_editor(lang);
    TemplateForm {
        name: e.name,
        language: lang.code().into(),
        guidance: e.guidance,
        sections: e
            .sections
            .into_iter()
            .map(|s| FormSection {
                id: s.id,
                title: s.title,
                instruction: s.instruction,
            })
            .collect(),
    }
}

fn view(u: &UserTemplate) -> Option<UserTemplateView> {
    Some(UserTemplateView {
        id: u.id(),
        form: form_of(&u.template()?, u.out_lang()),
    })
}

fn new_gid(taken: &[String]) -> String {
    loop {
        let mut b = [0u8; 6];
        OsRng.fill_bytes(&mut b);
        let g = format!("t{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>());
        if !taken.contains(&g) {
            return g;
        }
    }
}

/// Section ids a template gave up that are remembered (so none is given twice).
const RETIRED_KEPT: usize = 200;

pub(crate) fn list_now(c: &Core) -> Result<Vec<UserTemplateView>, String> {
    let store = c.store()?;
    let _g = c.templates_guard();
    Ok(Records::load(&store)?.usable().into_iter().filter_map(view).collect())
}

pub(crate) fn create_now(c: &Core, form: &TemplateForm) -> Result<UserTemplateView, String> {
    let store = c.store()?;
    // The whole read-change-write: two creates at once cannot lose one.
    let _g = c.templates_guard();
    let mut all = Records::load(&store)?;
    if all.len() >= store_of::MAX_USER_TEMPLATES {
        return Err(format!(
            "at most {} templates: delete one first",
            store_of::MAX_USER_TEMPLATES
        ));
    }
    // A new template has no sections yet: none of the form's may name an id.
    let gid = new_gid(&all.gids());
    let t = Template::from_editor(&gid, &editor_of(form)?, &[], &[]).map_err(|e| e.to_string())?;
    let u = UserTemplate {
        gid,
        lang: form.language.clone(),
        toml: t.to_toml(),
        retired: Vec::new(),
    };
    let out = view(&u).ok_or("the template could not be read back")?;
    all.push(u);
    all.save(&store)?;
    Ok(out)
}

pub(crate) fn update_now(c: &Core, id: &str, form: &TemplateForm) -> Result<UserTemplateView, String> {
    let store = c.store()?;
    let _g = c.templates_guard();
    let mut all = Records::load(&store)?;
    let gid = store_of::gid_of(id).ok_or("not a template of yours")?;
    let u = all.find_mut(gid).ok_or("this template is gone")?;
    let old = u.template().ok_or("this template is gone")?;
    let known: Vec<String> = old.sections.iter().map(|s| s.id.clone()).collect();
    let t = Template::from_editor(&u.gid, &editor_of(form)?, &known, &u.retired)
        .map_err(|e| e.to_string())?;
    // A section the form dropped is retired: its id is never given to another.
    for k in known {
        if !t.sections.iter().any(|s| s.id == k) && !u.retired.contains(&k) {
            u.retired.push(k);
        }
    }
    if u.retired.len() > RETIRED_KEPT {
        let extra = u.retired.len() - RETIRED_KEPT;
        u.retired.drain(..extra);
    }
    u.lang = form.language.clone();
    u.toml = t.to_toml();
    let out = view(u).ok_or("the template could not be read back")?;
    all.save(&store)?;
    Ok(out)
}

pub(crate) fn delete_now(c: &Core, id: &str) -> Result<(), String> {
    let store = c.store()?;
    let _g = c.templates_guard();
    let mut all = Records::load(&store)?;
    let gid = store_of::gid_of(id).ok_or("not a template of yours")?;
    // Meetings that used it keep their notes and every section (their blocks name them).
    if !all.remove(gid) {
        return Err("this template is gone".into());
    }
    all.save(&store)
}

/// A form to start from: a copy of a built-in template (or one of yours) in
/// `language`. Nothing is saved: the person edits it and saves, or cancels.
pub(crate) fn duplicate_now(c: &Core, id: &str, language: &str) -> Result<TemplateForm, String> {
    let lang = lang_of(language)?;
    let store = c.store()?;
    let t = if store_of::gid_of(id).is_some() {
        store_of::find(&store, id).ok_or("this template is gone")?
    } else {
        ghi_llm::template::builtin(id).map_err(|e| e.to_string())?
    };
    let mut form = form_of(&t, lang);
    // The copy is a new template: its sections get ids of their own.
    for s in &mut form.sections {
        s.id = None;
        s.instruction = s.instruction.chars().take(ghi_llm::template::MAX_INSTRUCTION).collect();
        s.title = s.title.chars().take(ghi_llm::template::MAX_SECTION_TITLE).collect();
    }
    form.guidance = form.guidance.chars().take(ghi_llm::template::MAX_GUIDANCE).collect();
    form.sections.truncate(ghi_llm::template::MAX_SECTIONS);
    Ok(form)
}

/// A template form drafted from a description by `llm`. Nothing is saved.
pub(crate) fn draft_with(
    llm: &mut dyn ghi_llm::Llm,
    description: &str,
    language: &str,
) -> Result<TemplateForm, String> {
    let lang = lang_of(language)?;
    let e = ghi_llm::draft::draft(llm, description, lang).map_err(|e| e.to_string())?;
    Ok(TemplateForm {
        name: e.name,
        language: lang.code().into(),
        guidance: e.guidance,
        sections: e
            .sections
            .into_iter()
            .map(|s| FormSection {
                id: None,
                title: s.title,
                instruction: s.instruction,
            })
            .collect(),
    })
}

/// "Draft from description": the local model, like Ask (refused while
/// recording or while notes are being written, and while the app is locked).
pub(crate) fn draft_now(c: &Core, description: &str, language: &str) -> Result<TemplateForm, String> {
    lang_of(language)?;
    // Refused while the app is locked, like every content command.
    let store = c.store()?;
    crate::cloud_cmd::local_model_free(c, &store)?;
    let mut llm = (c.llm()?)(description.len())?;
    let form = draft_with(llm.as_mut(), description, language);
    drop(llm);
    form
}

/// Drafts a template from a description with the local model. Never saved:
/// the form is for the person to review.
#[tauri::command]
#[specta::specta]
pub async fn draft_template(
    core: CoreState<'_>,
    description: String,
    language: String,
) -> Result<TemplateForm, String> {
    blocking(&core, move |c| draft_now(c, &description, &language)).await
}

/// Your templates.
#[tauri::command]
#[specta::specta]
pub async fn user_templates(core: CoreState<'_>) -> Result<Vec<UserTemplateView>, String> {
    blocking(&core, list_now).await
}

#[tauri::command]
#[specta::specta]
pub async fn create_template(
    core: CoreState<'_>,
    form: TemplateForm,
) -> Result<UserTemplateView, String> {
    blocking(&core, move |c| create_now(c, &form)).await
}

#[tauri::command]
#[specta::specta]
pub async fn update_template(
    core: CoreState<'_>,
    id: String,
    form: TemplateForm,
) -> Result<UserTemplateView, String> {
    blocking(&core, move |c| update_now(c, &id, &form)).await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_template(core: CoreState<'_>, id: String) -> Result<(), String> {
    blocking(&core, move |c| delete_now(c, &id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn duplicate_template(
    core: CoreState<'_>,
    id: String,
    language: String,
) -> Result<TemplateForm, String> {
    blocking(&core, move |c| duplicate_now(c, &id, &language)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix() -> (tempfile::TempDir, std::sync::Arc<Core>) {
        let tmp = tempfile::tempdir().unwrap();
        let (core, _rx) = Core::for_test(tmp.path().join("data"));
        (tmp, core)
    }

    fn form(name: &str, sections: &[&str]) -> TemplateForm {
        TemplateForm {
            name: name.into(),
            language: "en".into(),
            guidance: "A retro.".into(),
            sections: sections
                .iter()
                .map(|t| FormSection {
                    id: None,
                    title: (*t).into(),
                    instruction: "What belongs here.".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn create_list_update_delete() {
        let (_t, c) = fix();
        let made = create_now(&c, &form("Retro", &["Went well", "Went badly"])).unwrap();
        assert!(made.id.starts_with("user:t"));
        assert_eq!(made.form.sections[0].id.as_deref(), Some("went_well"));
        assert_eq!(list_now(&c).unwrap().len(), 1);
        // the engine reads it like any template
        let t = ghi_core::user_templates::find(&c.store().unwrap(), &made.id).unwrap();
        assert_eq!(t.sections.len(), 2);

        // Rename a section, drop one, add one: ids stay, the dropped one is retired.
        let mut f = made.form.clone();
        f.sections[0].title = "Good".into();
        f.sections.remove(1);
        f.sections.push(FormSection {
            id: None,
            title: "Went badly".into(),
            instruction: "Again.".into(),
        });
        let upd = update_now(&c, &made.id, &f).unwrap();
        let ids: Vec<_> = upd.form.sections.iter().map(|s| s.id.clone().unwrap()).collect();
        assert_eq!(ids, ["went_well", "went_badly_2"], "went_badly stays retired");
        // A form cannot name an id the template never had.
        let mut forged = upd.form.clone();
        forged.sections[0].id = Some("tldr".into());
        assert!(update_now(&c, &made.id, &forged).is_err());

        delete_now(&c, &made.id).unwrap();
        assert!(list_now(&c).unwrap().is_empty());
        assert!(delete_now(&c, &made.id).is_err());
        assert!(update_now(&c, &made.id, &f).is_err());
    }

    #[test]
    fn invalid_forms_and_the_cap_are_refused() {
        let (_t, c) = fix();
        assert!(create_now(&c, &form(" ", &["A"])).is_err());
        let mut bad = form("X", &["A"]);
        bad.language = "fr".into();
        assert!(create_now(&c, &bad).is_err());
        let mut long = form("X", &["A"]);
        long.sections[0].instruction = "x".repeat(201);
        assert!(create_now(&c, &long).is_err());
        assert!(list_now(&c).unwrap().is_empty());
        for n in 0..store_of::MAX_USER_TEMPLATES {
            create_now(&c, &form(&format!("T{n}"), &["A"])).unwrap();
        }
        assert!(create_now(&c, &form("one more", &["A"])).is_err());
    }

    #[test]
    fn duplicating_makes_an_unsaved_form_in_the_chosen_language() {
        let (_t, c) = fix();
        let en = duplicate_now(&c, "standup", "en").unwrap();
        assert_eq!(en.name, "Standup");
        assert_eq!(en.sections.len(), 3);
        assert_eq!(en.sections[0].title, "Done");
        assert!(en.sections.iter().all(|s| s.id.is_none()), "a new template, new ids");
        let vi = duplicate_now(&c, "standup", "vi").unwrap();
        assert_eq!((vi.language.as_str(), vi.sections[0].title.as_str()), ("vi", "Đã làm"));
        assert!(duplicate_now(&c, "nope", "en").is_err());
        assert!(duplicate_now(&c, "standup", "fr").is_err());
        // Nothing was saved: only Save creates it.
        assert!(list_now(&c).unwrap().is_empty());
        // The form is valid as it is.
        let made = create_now(&c, &en).unwrap();
        let again = duplicate_now(&c, &made.id, "en").unwrap();
        assert_eq!(again.sections.len(), 3);
        assert_eq!(list_now(&c).unwrap().len(), 1);
    }

    #[test]
    fn creates_at_once_all_land() {
        let (_t, c) = fix();
        std::thread::scope(|s| {
            let hs: Vec<_> = (0..12)
                .map(|n| {
                    let c = &c;
                    s.spawn(move || create_now(c, &form(&format!("T{n}"), &["A"])).unwrap())
                })
                .collect();
            for h in hs {
                h.join().unwrap();
            }
        });
        let names: std::collections::BTreeSet<String> =
            list_now(&c).unwrap().into_iter().map(|u| u.form.name).collect();
        assert_eq!(names.len(), 12, "{names:?}");
    }

    #[test]
    fn an_edit_and_a_delete_at_once_do_not_undo_each_other() {
        let (_t, c) = fix();
        let a = create_now(&c, &form("A", &["X"])).unwrap();
        let b = create_now(&c, &form("B", &["X"])).unwrap();
        std::thread::scope(|s| {
            let (c1, c2) = (&c, &c);
            let (a1, b1) = (a.clone(), b.clone());
            let h1 = s.spawn(move || {
                let mut f = a1.form.clone();
                f.name = "A renamed".into();
                update_now(c1, &a1.id, &f).unwrap()
            });
            let h2 = s.spawn(move || delete_now(c2, &b1.id).unwrap());
            h1.join().unwrap();
            h2.join().unwrap();
        });
        let left = list_now(&c).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].form.name, "A renamed");
    }

    #[test]
    fn templates_this_version_cannot_read_survive_a_save_and_the_cap_counts_them() {
        let (_t, c) = fix();
        let store = c.store().unwrap();
        let mut list: Vec<serde_json::Value> = (0..store_of::MAX_USER_TEMPLATES)
            .map(|n| serde_json::json!({ "gid": format!("tf{n}"), "from": "a newer app" }))
            .collect();
        store.set_setting(store_of::KEY, &serde_json::Value::Array(list.clone())).unwrap();
        assert!(list_now(&c).unwrap().is_empty());
        assert!(create_now(&c, &form("X", &["A"])).is_err(), "twenty entries, readable or not");
        list.pop();
        store.set_setting(store_of::KEY, &serde_json::Value::Array(list)).unwrap();
        create_now(&c, &form("X", &["A"])).unwrap();
        let raw = store.get_setting(store_of::KEY).unwrap().unwrap();
        assert_eq!(raw.as_array().unwrap().len(), store_of::MAX_USER_TEMPLATES);
        assert_eq!(raw[0]["from"], "a newer app");
        // A value that is not a list is never overwritten.
        store.set_setting(store_of::KEY, &serde_json::json!("garbage")).unwrap();
        assert!(create_now(&c, &form("Y", &["A"])).is_err());
        assert_eq!(store.get_setting(store_of::KEY).unwrap().unwrap(), "garbage");
    }

    #[test]
    fn the_retired_ids_are_capped() {
        let (_t, c) = fix();
        let made = create_now(&c, &form("A", &["X"])).unwrap();
        let mut f = made.form.clone();
        // Churn sections: each round drops the section and adds a new one.
        let mut cur = made;
        for n in 0..210 {
            f = cur.form.clone();
            f.sections = vec![FormSection {
                id: None,
                title: format!("Part {n}"),
                instruction: "i".into(),
            }];
            cur = update_now(&c, &cur.id, &f).unwrap();
        }
        let raw = c.store().unwrap().get_setting(store_of::KEY).unwrap().unwrap();
        assert!(raw[0]["retired"].as_array().unwrap().len() <= 200);
        assert_eq!(cur.form.sections.len(), 1);
        let _ = f;
    }

    #[test]
    fn a_locked_app_refuses_every_template_command() {
        let (_t, c) = fix();
        let made = create_now(&c, &form("Retro", &["A"])).unwrap();
        c.set_locked(true);
        let locked = "the app is locked";
        assert_eq!(list_now(&c).unwrap_err(), locked);
        assert_eq!(create_now(&c, &form("B", &["A"])).unwrap_err(), locked);
        assert_eq!(update_now(&c, &made.id, &made.form).unwrap_err(), locked);
        assert_eq!(delete_now(&c, &made.id).unwrap_err(), locked);
        assert_eq!(duplicate_now(&c, "standup", "en").unwrap_err(), locked);
        c.set_locked(false);
        assert_eq!(list_now(&c).unwrap().len(), 1);
    }
}

#[cfg(test)]
mod draft_tests {
    use super::*;
    use ghi_llm::{Completion, EngineInfo, Llm, Request};

    struct Say(String, std::sync::atomic::AtomicUsize);

    impl Llm for Say {
        fn engine(&self) -> EngineInfo {
            EngineInfo {
                name: "say".into(),
                version: "1".into(),
            }
        }
        fn context_tokens(&self) -> u32 {
            8192
        }
        fn complete(&mut self, _: &Request) -> ghi_llm::Result<Completion> {
            self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Completion {
                text: self.0.clone(),
                tokens_in: 1,
                tokens_out: 1,
                truncated: false,
            })
        }
    }

    const REPLY: &str = r#"{"name":"Retro","guidance":"Weekly retros.","sections":[{"title":"Went well","instruction":"What worked."},{"title":"Went badly","instruction":"What did not."}]}"#;

    #[test]
    fn a_draft_is_a_form_with_no_ids_that_creates_cleanly_and_is_not_saved() {
        let tmp = tempfile::tempdir().unwrap();
        let (c, _rx) = Core::for_test(tmp.path().join("data"));
        let mut llm = Say(REPLY.into(), 0.into());
        let form = draft_with(&mut llm, "A weekly retro", "en").unwrap();
        assert_eq!(form.name, "Retro");
        assert!(form.sections.iter().all(|s| s.id.is_none()));
        // Drafting saves nothing; only creating does.
        assert!(list_now(&c).unwrap().is_empty());
        let made = create_now(&c, &form).unwrap();
        assert_eq!(made.form.sections.len(), 2);
        assert!(draft_with(&mut llm, "x", "fr").is_err());
    }

    #[test]
    fn a_locked_app_refuses_before_any_model_is_opened() {
        let tmp = tempfile::tempdir().unwrap();
        let (c, _rx) = Core::for_test(tmp.path().join("data"));
        c.set_locked(true);
        assert_eq!(draft_now(&c, "A weekly retro", "en").unwrap_err(), "the app is locked");
        c.set_locked(false);
        // Unlocked, but no model is set up in this core: refused in words, nothing drafted.
        let e = draft_now(&c, "A weekly retro", "en").unwrap_err();
        assert!(!e.is_empty());
        assert!(list_now(&c).unwrap().is_empty());
    }
}
