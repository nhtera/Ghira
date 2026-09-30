// SPDX-License-Identifier: Apache-2.0
//! Note templates as data (`templates/<id>.toml`).
//!
//! Every template produces the common notes (TL;DR, decisions, action items,
//! open questions, key quotes, topics) plus its own sections. The JSON schema
//! the model must follow is generated from the template ([`crate::schema`]),
//! so a custom template is just another TOML file.

use serde::Deserialize;

use crate::{LlmError, Result};

const BUILTIN: &[(&str, &str)] = &[
    ("general", include_str!("../templates/general.toml")),
    ("one_on_one", include_str!("../templates/one_on_one.toml")),
    ("standup", include_str!("../templates/standup.toml")),
    ("sales", include_str!("../templates/sales.toml")),
    ("interview", include_str!("../templates/interview.toml")),
    ("client", include_str!("../templates/client.toml")),
    ("lecture", include_str!("../templates/lecture.toml")),
];

/// Most sections a template may add (keeps the schema small for local models).
const MAX_SECTIONS: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub guidance_en: String,
    pub guidance_vi: String,
    #[serde(default, rename = "section")]
    pub sections: Vec<SectionSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
        assert_eq!(ids.len(), 7);
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
}
