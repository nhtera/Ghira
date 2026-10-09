// SPDX-License-Identifier: Apache-2.0
//! "Draft from description": a note template made by the local model from a
//! sentence or two, in the editor's shape. Nothing is saved here: the person
//! reviews the form and saves it, or not. The output is bounded by the grammar
//! (`schema::template_draft`), cleaned to plain text, cut to the editor's caps
//! and checked by the same rules as a template made by hand
//! ([`Template::from_editor`]).

use serde::Deserialize;

use crate::run::{self, Outcome};
use crate::template::{
    Editor, EditorSection, MAX_GUIDANCE, MAX_INSTRUCTION, MAX_NAME, MAX_SECTION_TITLE,
    MAX_SECTIONS, OutLang, Template,
};
use crate::validate::{Diagnostics, plain_text};
use crate::{Llm, LlmError, Message, Request, Result, prompt, schema};

/// The longest description read (characters).
pub const MAX_DESCRIPTION: usize = 600;
const MAX_DRAFT_TOKENS: u32 = 900;

/// Plain text, no Markdown emphasis or heading marks, at most `max` characters.
fn clip(s: &str, max: usize) -> String {
    let text: String = plain_text(s).chars().filter(|c| !matches!(c, '*' | '`')).collect();
    text.trim_start_matches(['#', '>', ' '])
        .chars()
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

/// A template draft for `description`, written in `lang`.
pub fn draft(llm: &mut dyn Llm, description: &str, lang: OutLang) -> Result<Editor> {
    let description = clip(description, MAX_DESCRIPTION);
    if description.is_empty() {
        return Err(LlmError::Invalid("describe the meetings first".into()));
    }
    let req = Request {
        messages: vec![
            Message::system(prompt::draft_system(lang)),
            Message::user(prompt::draft_task(lang, &description)),
        ],
        schema: Some(schema::template_draft()),
        max_tokens: MAX_DRAFT_TOKENS,
        temperature: 0.4,
    };
    let mut diag = Diagnostics::default();
    let outcome = run::complete_json(llm, req, lang, &mut diag, |v, _| parse(v, lang))?;
    match outcome {
        Outcome::Done(e) => Ok(e),
        Outcome::Truncated => Err(LlmError::InvalidOutput("the draft was cut off".into())),
    }
}

/// Titles of the standard sections (folded, so accents and case do not matter):
/// every note has these already, so a draft that repeats one gets it dropped.
const STANDARD: &[&str] = &[
    "summary", "tldr", "tl dr", "decisions", "decision", "action items", "actions", "action", "open questions", "questions",
    "key quotes", "quotes", "topics", "tom tat", "quyet dinh", "viec can lam", "cau hoi con mo", "cau hoi mo", "cau hoi",
    "trich dan chinh", "trich dan", "chu de",
];

fn is_standard(title: &str) -> bool {
    let folded: String = ghi_text::fold(title)
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    STANDARD.contains(&folded.as_str())
}

fn parse(v: &serde_json::Value, lang: OutLang) -> std::result::Result<Editor, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Raw {
        name: String,
        guidance: String,
        sections: Vec<RawSection>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct RawSection {
        title: String,
        instruction: String,
    }
    let raw: Raw = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    let editor = Editor {
        name: clip(&raw.name, MAX_NAME),
        lang,
        guidance: clip(&raw.guidance, MAX_GUIDANCE),
        sections: raw
            .sections
            .iter()
            .map(|s| EditorSection {
                id: None,
                title: clip(&s.title, MAX_SECTION_TITLE),
                instruction: clip(&s.instruction, MAX_INSTRUCTION),
            })
            .filter(|s| !s.title.is_empty() && !s.instruction.is_empty() && !is_standard(&s.title))
            .take(MAX_SECTIONS)
            .collect(),
    };
    if editor.sections.is_empty() {
        return Err("the template needs sections of its own, not the standard ones".into());
    }
    // Never hand back something the editor would refuse to save.
    Template::from_editor("draft", &editor, &[], &[]).map_err(|e| e.to_string())?;
    Ok(editor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Completion, EngineInfo};

    struct Reply(Vec<String>, Vec<Request>);

    impl Llm for Reply {
        fn engine(&self) -> EngineInfo {
            EngineInfo {
                name: "scripted".into(),
                version: "1".into(),
            }
        }
        fn context_tokens(&self) -> u32 {
            8192
        }
        fn complete(&mut self, req: &Request) -> Result<Completion> {
            self.1.push(req.clone());
            Ok(Completion {
                text: self.0.remove(0),
                tokens_in: 1,
                tokens_out: 1,
                truncated: false,
            })
        }
    }

    fn json(name: &str, sections: &[(&str, &str)]) -> String {
        serde_json::json!({
            "name": name,
            "guidance": "Weekly retros.",
            "sections": sections.iter().map(|(t, i)| serde_json::json!({"title": t, "instruction": i})).collect::<Vec<_>>(),
        })
        .to_string()
    }

    #[test]
    fn a_description_becomes_an_editor_form_that_validates_and_has_no_ids() {
        let mut llm = Reply(vec![json("Retro", &[("Went well", "What worked."), ("Went badly", "What did not.")])], vec![]);
        let e = draft(&mut llm, "A weekly team retro", OutLang::En).unwrap();
        assert_eq!(e.name, "Retro");
        assert_eq!(e.sections.len(), 2);
        assert!(e.sections.iter().all(|s| s.id.is_none()), "ids come at save");
        Template::from_editor("t1", &e, &[], &[]).unwrap();
        // The request is grammar-bounded and the description is in the user message, not the rules.
        let r = &llm.1[0];
        assert!(r.schema.is_some());
        assert!(!r.messages[0].content.contains("weekly team retro"));
        assert!(r.messages[1].content.contains("A weekly team retro"));
        assert_eq!(r.messages.len(), 2);
    }

    #[test]
    fn output_is_cleaned_cut_to_the_caps_and_empty_sections_dropped() {
        let long = "x".repeat(500);
        let mut llm = Reply(
            vec![json(
                &format!("**{long}**"),
                &[("[Link](http://x.y) Risks", &long), ("", "no title"), ("No instruction", ""), ("<b>Plans</b>", "Next steps")],
            )],
            vec![],
        );
        let e = draft(&mut llm, "x", OutLang::Vi).unwrap();
        assert!(e.name.chars().count() <= MAX_NAME);
        assert!(!e.name.contains('*'));
        assert_eq!(e.sections.len(), 2, "{:?}", e.sections);
        assert!(!e.sections[0].title.contains("http") && !e.sections[0].title.contains('['));
        assert!(e.sections[0].instruction.chars().count() <= MAX_INSTRUCTION);
        assert_eq!(e.sections[1].title, "Plans");
        assert_eq!(e.lang, OutLang::Vi);
    }

    #[test]
    fn an_unusable_draft_is_retried_once_with_the_reason_and_then_refused() {
        // Nothing usable (no sections left): the same rules as a hand-made template refuse it.
        let bad = json("", &[("A", "B")]);
        let good = json("Retro", &[("A", "B")]);
        let mut llm = Reply(vec![bad.clone(), good], vec![]);
        let e = draft(&mut llm, "x", OutLang::En).unwrap();
        assert_eq!(e.name, "Retro");
        assert_eq!(llm.1.len(), 2);
        assert!(llm.1[1].messages.last().unwrap().content.contains("not valid"));
        let mut llm = Reply(vec![bad.clone(), bad.clone(), bad.clone(), bad], vec![]);
        assert!(draft(&mut llm, "x", OutLang::En).is_err());
    }

    #[test]
    fn standard_sections_are_dropped_in_either_language() {
        let mut llm = Reply(
            vec![json(
                "Retro",
                &[("Summary", "x"), ("Action Items", "x"), ("Went well", "What worked."), ("TÓM TẮT", "x"), ("Việc cần làm", "x"), ("Câu hỏi mở", "x"), ("Rủi ro", "Điều có thể sai.")],
            )],
            vec![],
        );
        let e = draft(&mut llm, "x", OutLang::En).unwrap();
        assert_eq!(e.sections.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Went well", "Rủi ro"]);
        // Nothing of its own left: tried again, then refused.
        let only = json("Retro", &[("Summary", "x"), ("Topics", "x")]);
        let mut llm = Reply(vec![only.clone(), only.clone(), only.clone(), only], vec![]);
        assert!(draft(&mut llm, "x", OutLang::En).is_err());
    }

    #[test]
    fn an_empty_description_is_refused_before_the_model_is_asked() {
        let mut llm = Reply(vec![], vec![]);
        assert!(draft(&mut llm, "  <b></b> ", OutLang::En).is_err());
        assert!(llm.1.is_empty());
    }

    #[test]
    fn the_grammar_bounds_the_sections() {
        let s = schema::template_draft();
        assert_eq!(s["properties"]["sections"]["maxItems"], MAX_SECTIONS);
        assert_eq!(s["properties"]["sections"]["minItems"], 1);
        assert_eq!(s["additionalProperties"], false);
    }
}
