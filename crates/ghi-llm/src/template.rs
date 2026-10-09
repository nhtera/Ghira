// SPDX-License-Identifier: Apache-2.0
//! Note templates as data (`templates/<id>.toml`).
//!
//! Every template produces the common notes (TL;DR, decisions, action items,
//! open questions, key quotes, topics) plus its own sections. The JSON schema
//! the model must follow is generated from the template ([`crate::schema`]),
//! so a custom template is just another TOML file.

use serde::{Deserialize, Serialize};

use crate::{LlmError, Result};

const BUILTIN: &[(&str, &str)] = &[
    ("general", include_str!("../templates/general.toml")),
    ("one_on_one", include_str!("../templates/one_on_one.toml")),
    ("standup", include_str!("../templates/standup.toml")),
    ("sales", include_str!("../templates/sales.toml")),
    ("interview", include_str!("../templates/interview.toml")),
    ("client", include_str!("../templates/client.toml")),
    ("lecture", include_str!("../templates/lecture.toml")),
    (
        "consultation",
        include_str!("../templates/consultation.toml"),
    ),
];

/// Most sections a template may add (keeps the schema small for local models).
pub const MAX_SECTIONS: usize = 8;
/// Caps of a template made in the editor (characters).
pub const MAX_NAME: usize = 60;
pub const MAX_GUIDANCE: usize = 400;
pub const MAX_SECTION_TITLE: usize = 60;
pub const MAX_INSTRUCTION: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub guidance_en: String,
    pub guidance_vi: String,
    #[serde(default, rename = "section")]
    pub sections: Vec<SectionSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SectionSpec {
    /// JSON key in the model output: lowercase ASCII letters, digits, `_`.
    pub id: String,
    pub title_en: String,
    pub title_vi: String,
    /// What belongs in the section, for the model.
    pub instruction: String,
}

impl Template {
    /// Parses and checks a template (built-in or a user's custom file).
    pub fn from_toml(text: &str) -> Result<Template> {
        let t: Template =
            toml::from_str(text).map_err(|e| LlmError::Invalid(format!("template: {e}")))?;
        t.check()?;
        Ok(t)
    }

    /// The template as TOML text (what [`Template::from_toml`] reads back).
    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("a template is plain strings")
    }

    /// A template made from the editor's form. `id` is the template's own
    /// (`user_<n>`); `known` are the section ids already in use in it, which
    /// a form section may keep (and which stay for good: ids are never
    /// edited); `retired` are ids that were used once and were removed, never
    /// given again (notes written under one would show under a new section).
    /// New sections get an id made from their title (`went_well`; the id is
    /// all that notes keep of a section if the template is deleted). The text is written in
    /// `form.lang`; it fills both language fields.
    pub fn from_editor(
        id: &str,
        form: &Editor,
        known: &[String],
        retired: &[String],
    ) -> Result<Template> {
        let bad = |what: String| Err(LlmError::Invalid(format!("template: {what}")));
        let one_line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let name = one_line(&form.name);
        if name.is_empty() || name.chars().count() > MAX_NAME {
            return bad(format!("a name of 1 to {MAX_NAME} characters"));
        }
        let guidance = one_line(&form.guidance);
        if guidance.chars().count() > MAX_GUIDANCE {
            return bad(format!("guidance of at most {MAX_GUIDANCE} characters"));
        }
        if form.sections.len() > MAX_SECTIONS {
            return bad(format!("at most {MAX_SECTIONS} sections"));
        }
        let mut taken: Vec<String> = known.iter().chain(retired).cloned().collect();
        let mut sections: Vec<SectionSpec> = Vec::new();
        for f in &form.sections {
            let title = one_line(&f.title);
            let instruction = one_line(&f.instruction);
            if title.is_empty() || title.chars().count() > MAX_SECTION_TITLE {
                return bad(format!("a section title of 1 to {MAX_SECTION_TITLE} characters"));
            }
            if instruction.is_empty() || instruction.chars().count() > MAX_INSTRUCTION {
                return bad(format!("a section instruction of 1 to {MAX_INSTRUCTION} characters"));
            }
            let sid = match &f.id {
                Some(k) if known.contains(k) && !sections.iter().any(|s| &s.id == k) => k.clone(),
                // An id the template never had is not the form's to make up.
                Some(k) => return bad(format!("unknown section id `{k}`")),
                None => {
                    let k = fresh_id(&title, &taken);
                    taken.push(k.clone());
                    k
                }
            };
            sections.push(SectionSpec {
                id: sid,
                title_en: title.clone(),
                title_vi: title,
                instruction,
            });
        }
        let t = Template {
            id: id.to_string(),
            name,
            guidance_en: guidance.clone(),
            guidance_vi: guidance,
            sections,
        };
        t.check()?;
        Ok(t)
    }

    /// The form to edit this template in, in the language its text is written in.
    pub fn to_editor(&self, lang: OutLang) -> Editor {
        Editor {
            name: self.name.clone(),
            lang,
            guidance: self.guidance(lang).to_string(),
            sections: self
                .sections
                .iter()
                .map(|s| EditorSection {
                    id: Some(s.id.clone()),
                    title: match lang {
                        OutLang::Vi => s.title_vi.clone(),
                        OutLang::En => s.title_en.clone(),
                    },
                    instruction: s.instruction.clone(),
                })
                .collect(),
        }
    }

    pub fn guidance(&self, lang: OutLang) -> &str {
        match lang {
            OutLang::Vi => &self.guidance_vi,
            OutLang::En => &self.guidance_en,
        }
    }

    fn check(&self) -> Result<()> {
        let bad = |what: String| Err(LlmError::Invalid(format!("template {}: {what}", self.id)));
        if !is_key(&self.id) {
            return bad("id must be lowercase letters, digits and _".into());
        }
        if self.sections.len() > MAX_SECTIONS {
            return bad(format!("at most {MAX_SECTIONS} sections"));
        }
        for (i, s) in self.sections.iter().enumerate() {
            if !is_key(&s.id) || crate::schema::CORE_KEYS.contains(&s.id.as_str()) {
                return bad(format!("section id `{}` is not allowed", s.id));
            }
            if self.sections[..i].iter().any(|o| o.id == s.id) {
                return bad(format!("duplicate section `{}`", s.id));
            }
        }
        Ok(())
    }
}

/// A template as the editor shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    pub name: String,
    /// The language the text is written in.
    pub lang: OutLang,
    /// A line on what the meetings are, for the model (optional).
    pub guidance: String,
    pub sections: Vec<EditorSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorSection {
    /// `None`: a new section (it gets its id when the template is made).
    pub id: Option<String>,
    pub title: String,
    /// One line: what belongs in the section.
    pub instruction: String,
}

/// An id for a new section from its title: folded to ASCII, `_` between
/// words, at most 24 characters, not taken and not one of the common keys.
fn fresh_id(title: &str, taken: &[String]) -> String {
    let folded = ghi_text::fold(title);
    let mut base = String::new();
    for c in folded.chars() {
        if c.is_ascii_alphanumeric() {
            base.push(c.to_ascii_lowercase());
        } else if !base.ends_with('_') && !base.is_empty() {
            base.push('_');
        }
    }
    let mut base: String = base.trim_end_matches('_').chars().take(24).collect();
    if !base.starts_with(|c: char| c.is_ascii_lowercase()) {
        base = format!("s_{base}").trim_end_matches('_').to_string();
    }
    let mut id = base.clone();
    let mut n = 1;
    while taken.contains(&id) || crate::schema::CORE_KEYS.contains(&id.as_str()) {
        n += 1;
        id = format!("{base}_{n}");
    }
    id
}

/// What to call a section whose template is gone: its id as words.
pub fn humanize_id(id: &str) -> String {
    let words = id.replace('_', " ");
    let mut c = words.trim().chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

fn is_key(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A built-in template by id.
pub fn builtin(id: &str) -> Result<Template> {
    let (_, text) = BUILTIN
        .iter()
        .find(|(k, _)| *k == id)
        .ok_or_else(|| LlmError::Invalid(format!("unknown template `{id}`")))?;
    Template::from_toml(text)
}

/// Ids of the built-in templates, in menu order.
pub fn builtin_ids() -> impl Iterator<Item = &'static str> {
    BUILTIN.iter().map(|(k, _)| *k)
}

/// The language notes are written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutLang {
    En,
    Vi,
}

impl OutLang {
    pub fn code(self) -> &'static str {
        match self {
            OutLang::En => "en",
            OutLang::Vi => "vi",
        }
    }

    /// `meeting` → the transcript's dominant language (English if unknown).
    pub fn resolve(choice: &str, transcript: &crate::Transcript) -> Result<OutLang> {
        match choice {
            "en" => Ok(OutLang::En),
            "vi" => Ok(OutLang::Vi),
            "meeting" | "auto" => Ok(match transcript.dominant_lang() {
                Some("vi") => OutLang::Vi,
                _ => OutLang::En,
            }),
            other => Err(LlmError::Invalid(format!("output language `{other}`"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_parses_and_has_distinct_sections() {
        let ids: Vec<_> = builtin_ids().collect();
        assert_eq!(ids.len(), 8);
        for id in ids {
            let t = builtin(id).unwrap();
            assert_eq!(t.id, id);
            assert!(!t.guidance_vi.is_empty() && !t.guidance_en.is_empty());
        }
        assert!(builtin("nope").is_err());
    }

    #[test]
    fn custom_templates_are_checked() {
        let ok = "id = \"retro\"\nname = \"Retro\"\nguidance_en = \"g\"\nguidance_vi = \"g\"\n\
                  [[section]]\nid = \"went_well\"\ntitle_en = \"Went well\"\ntitle_vi = \"Tốt\"\ninstruction = \"i\"\n";
        assert_eq!(Template::from_toml(ok).unwrap().sections.len(), 1);
        for bad in [
            ok.replace("went_well", "tldr"),
            ok.replace("went_well", "Went Well"),
            ok.replace("retro", "1retro"),
            format!("{ok}extra = 1\n"),
        ] {
            assert!(Template::from_toml(&bad).is_err(), "{bad}");
        }
    }

    fn form() -> Editor {
        Editor {
            name: "  Weekly  retro ".into(),
            lang: OutLang::En,
            guidance: "A team retro.".into(),
            sections: vec![
                EditorSection {
                    id: None,
                    title: "Went well".into(),
                    instruction: "What worked,\n and who did it.".into(),
                },
                EditorSection {
                    id: None,
                    title: "Went badly".into(),
                    instruction: "What did not.".into(),
                },
            ],
        }
    }

    #[test]
    fn a_form_becomes_a_template_that_round_trips_through_toml() {
        let t = Template::from_editor("user_1", &form(), &[], &[]).unwrap();
        assert_eq!(t.name, "Weekly retro");
        assert_eq!(t.sections[0].id, "went_well");
        assert_eq!(t.sections[1].id, "went_badly");
        assert_eq!(t.sections[0].instruction, "What worked, and who did it.");
        assert_eq!(t.sections[0].title_vi, "Went well");
        assert_eq!(Template::from_toml(&t.to_toml()).unwrap(), t);
        // and back to a form with the same words and the ids kept
        let e = t.to_editor(OutLang::En);
        assert_eq!(e.sections[1].id.as_deref(), Some("went_badly"));
        assert_eq!(e.name, "Weekly retro");
        let again = Template::from_editor("user_1", &e, &["went_well".into(), "went_badly".into()], &[]).unwrap();
        assert_eq!(again, t);
    }

    #[test]
    fn the_caps_are_enforced() {
        let with = |f: &dyn Fn(&mut Editor)| {
            let mut e = form();
            f(&mut e);
            Template::from_editor("user_1", &e, &[], &[])
        };
        assert!(with(&|e| e.name = " ".into()).is_err());
        assert!(with(&|e| e.name = "x".repeat(MAX_NAME + 1)).is_err());
        assert!(with(&|e| e.guidance = "x".repeat(MAX_GUIDANCE + 1)).is_err());
        assert!(with(&|e| e.sections[0].title = String::new()).is_err());
        assert!(with(&|e| e.sections[0].instruction = "x".repeat(MAX_INSTRUCTION + 1)).is_err());
        assert!(with(&|e| e.sections[0].instruction = "x".repeat(MAX_INSTRUCTION)).is_ok());
        let one = form().sections[0].clone();
        assert!(with(&|e| e.sections = vec![one.clone(); MAX_SECTIONS]).is_ok());
        assert!(with(&|e| e.sections = vec![one.clone(); MAX_SECTIONS + 1]).is_err());
        // 200 characters count as characters, not bytes
        assert!(with(&|e| e.sections[0].instruction = "đ".repeat(MAX_INSTRUCTION)).is_ok());
    }

    #[test]
    fn section_ids_are_immutable_and_never_reused() {
        let t = Template::from_editor("user_1", &form(), &[], &[]).unwrap();
        let known: Vec<String> = t.sections.iter().map(|s| s.id.clone()).collect();
        // Remove the first section, add a new one: the old id stays retired.
        let mut e = t.to_editor(OutLang::En);
        let removed = e.sections.remove(0);
        e.sections.push(EditorSection {
            id: None,
            title: "Actions".into(),
            instruction: "Follow-ups.".into(),
        });
        let retired = vec![removed.id.clone().unwrap()];
        let t2 = Template::from_editor("user_1", &e, &known, &retired).unwrap();
        let ids: Vec<&str> = t2.sections.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["went_badly", "actions"]);
        // Renaming a section keeps its id.
        e.sections[0].title = "Went badly (renamed)".into();
        let t3 = Template::from_editor("user_1", &e, &known, &retired).unwrap();
        assert_eq!(t3.sections[0].id, "went_badly");
        assert_eq!(t3.sections[0].title_en, "Went badly (renamed)");
        // A form cannot make up an id, or use one twice.
        e.sections[1].id = Some("tldr".into());
        assert!(Template::from_editor("user_1", &e, &known, &retired).is_err());
        let mut dup = t.to_editor(OutLang::En);
        dup.sections[1].id = dup.sections[0].id.clone();
        assert!(Template::from_editor("user_1", &dup, &known, &[]).is_err());
    }

    #[test]
    fn new_ids_come_from_the_title_and_are_unique_and_allowed() {
        let mut f = form();
        f.sections = ["Quyết định chính", "Quyết định chính", "TL;DR", "Đã làm!", "123 go"]
            .iter()
            .map(|t| EditorSection {
                id: None,
                title: t.to_string(),
                instruction: "i".into(),
            })
            .collect();
        f.sections.push(EditorSection {
            id: None,
            title: "Decisions".into(),
            instruction: "i".into(),
        });
        let ids: Vec<String> = Template::from_editor("user_1", &f, &[], &[])
            .unwrap()
            .sections
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, ["quyet_dinh_chinh", "quyet_dinh_chinh_2", "tl_dr", "da_lam", "s_123_go", "decisions_2"].map(String::from));
        assert_eq!(humanize_id("went_well"), "Went well");
    }

    #[test]
    fn a_vietnamese_template_edits_in_vietnamese() {
        let mut f = form();
        f.lang = OutLang::Vi;
        f.name = "Họp tuần".into();
        let t = Template::from_editor("user_2", &f, &[], &[]).unwrap();
        assert_eq!(t.to_editor(OutLang::Vi).name, "Họp tuần");
    }
}
