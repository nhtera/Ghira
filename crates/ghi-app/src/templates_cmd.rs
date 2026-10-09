// SPDX-License-Identifier: Apache-2.0
//! Templates the user makes (desktop only; Settings → Templates).
//!
//! They live in the device-local setting `templates.user` (not synced, at
//! most [`ghi_core::user_templates::MAX_USER_TEMPLATES`]), are checked by the
//! same parser as the built-in files, and are used as `user:<gid>`. Like the
//! other content commands they refuse while the app is locked.

use ghi_core::user_templates::{self as store_of, UserTemplate};
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

fn new_gid(taken: &[UserTemplate]) -> String {
    loop {
        let mut b = [0u8; 6];
        OsRng.fill_bytes(&mut b);
        let g = format!("t{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>());
        if !taken.iter().any(|u| u.gid == g) {
            return g;
        }
    }
}

pub(crate) fn list_now(c: &Core) -> Result<Vec<UserTemplateView>, String> {
    let store = c.store()?;
    Ok(store_of::load(&store)?.iter().filter_map(view).collect())
}

pub(crate) fn create_now(c: &Core, form: &TemplateForm) -> Result<UserTemplateView, String> {
    let store = c.store()?;
    let mut all = store_of::load(&store)?;
    if all.len() >= store_of::MAX_USER_TEMPLATES {
        return Err(format!(
            "at most {} templates: delete one first",
            store_of::MAX_USER_TEMPLATES
        ));
    }
    // A new template has no sections yet: none of the form's may name an id.
    let gid = new_gid(&all);
    let t = Template::from_editor(&gid, &editor_of(form)?, &[], &[]).map_err(|e| e.to_string())?;
    let u = UserTemplate {
        gid,
        lang: form.language.clone(),
        toml: t.to_toml(),
        retired: Vec::new(),
    };
    let out = view(&u).ok_or("the template could not be read back")?;
    all.push(u);
    store_of::save(&store, &all)?;
    Ok(out)
}

pub(crate) fn update_now(c: &Core, id: &str, form: &TemplateForm) -> Result<UserTemplateView, String> {
    let store = c.store()?;
    let mut all = store_of::load(&store)?;
    let gid = store_of::gid_of(id).ok_or("not a template of yours")?;
    let u = all
        .iter_mut()
        .find(|u| u.gid == gid)
        .ok_or("this template is gone")?;
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
    u.lang = form.language.clone();
    u.toml = t.to_toml();
    let out = view(u).ok_or("the template could not be read back")?;
    store_of::save(&store, &all)?;
    Ok(out)
}

pub(crate) fn delete_now(c: &Core, id: &str) -> Result<(), String> {
    let store = c.store()?;
    let mut all = store_of::load(&store)?;
    let gid = store_of::gid_of(id).ok_or("not a template of yours")?;
    let before = all.len();
    all.retain(|u| u.gid != gid);
    if all.len() == before {
        return Err("this template is gone".into());
    }
    // Meetings that used it keep their notes and every section (their blocks name them).
    store_of::save(&store, &all)
}

/// A copy of a built-in template (or one of yours) to edit, in `language`.
pub(crate) fn duplicate_now(c: &Core, id: &str, language: &str) -> Result<UserTemplateView, String> {
    let lang = lang_of(language)?;
    let store = c.store()?;
    let t = if store_of::gid_of(id).is_some() {
        store_of::find(&store, id).ok_or("this template is gone")?
    } else {
        ghi_llm::template::builtin(id).map_err(|e| e.to_string())?
    };
    let mut form = form_of(&t, lang);
    form.name = format!("{} (copy)", form.name.trim_end());
    form.name = form.name.chars().take(ghi_llm::template::MAX_NAME).collect();
    // The copy is a new template: its sections get ids of their own.
    for s in &mut form.sections {
        s.id = None;
    }
    form.guidance = form.guidance.chars().take(ghi_llm::template::MAX_GUIDANCE).collect();
    for s in &mut form.sections {
        s.instruction = s.instruction.chars().take(ghi_llm::template::MAX_INSTRUCTION).collect();
    }
    form.sections.truncate(ghi_llm::template::MAX_SECTIONS);
    create_now(c, &form)
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
) -> Result<UserTemplateView, String> {
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
    fn duplicating_a_builtin_makes_an_editable_copy_in_the_chosen_language() {
        let (_t, c) = fix();
        let en = duplicate_now(&c, "standup", "en").unwrap();
        assert_eq!(en.form.name, "Standup (copy)");
        assert_eq!(en.form.sections.len(), 3);
        assert_eq!(en.form.sections[0].title, "Done");
        let vi = duplicate_now(&c, &en.id, "vi").unwrap();
        assert_eq!(vi.form.language, "vi");
        assert!(duplicate_now(&c, "nope", "en").is_err());
        assert_eq!(list_now(&c).unwrap().len(), 2);
    }

    #[test]
    fn a_locked_app_refuses_every_template_command() {
        let (_t, c) = fix();
        let made = create_now(&c, &form("Retro", &["A"])).unwrap();
        c.set_locked(true);
        assert!(list_now(&c).is_err());
        assert!(create_now(&c, &form("B", &["A"])).is_err());
        assert!(update_now(&c, &made.id, &made.form).is_err());
        assert!(delete_now(&c, &made.id).is_err());
        assert!(duplicate_now(&c, "standup", "en").is_err());
        c.set_locked(false);
        assert_eq!(list_now(&c).unwrap().len(), 1);
    }
}
