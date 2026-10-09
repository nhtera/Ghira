// SPDX-License-Identifier: Apache-2.0
//! Follow-up email draft (doc 02 §N, P1): a short email to the people in a
//! meeting, written by the local model from the meeting's notes (summary,
//! decisions, action items with owners), never from the raw transcript. The
//! user picks the language and the tone, edits it and copies it; nothing is
//! sent from the app.

use ghi_llm::run::{Outcome, complete_json};
use ghi_llm::template::OutLang;
use ghi_llm::validate::{Diagnostics, plain_text};
use ghi_llm::{Llm, Message, Request};
use ghi_store::store::{Provenance, Store};
use serde_json::{Value, json};

use crate::notes_job::{ENHANCED_PREFIX, speaker_label};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Friendly,
    Neutral,
    Formal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Email {
    pub subject: String,
    pub body: String,
}

/// At most this much notes text goes into the prompt.
const MAX_NOTES_CHARS: usize = 12_000;
const MAX_OUTPUT_TOKENS: u32 = 900;

fn store_err(e: ghi_store::StoreError) -> String {
    e.to_string()
}

/// The meeting's notes as plain lines for the prompt.
fn notes_text(store: &Store, meeting: &str) -> Result<String, String> {
    let speakers = store.speakers(meeting).map_err(store_err)?;
    let name = |gid: &Option<String>| {
        gid.as_ref()
            .and_then(|g| speakers.iter().find(|s| &s.gid == g))
            .map(speaker_label)
    };
    let mut lines = Vec::new();
    for b in store.note_blocks(meeting).map_err(store_err)? {
        if b.body.trim().is_empty() || b.kind.starts_with(ENHANCED_PREFIX) {
            continue;
        }
        let label = match b.kind.as_str() {
            "tldr" => "Summary",
            "decision" => "Decision",
            "proposal" => "Proposed, not agreed",
            "answer" => "Saved answer",
            "question" => "Open question",
            "topic" | "quote" => continue,
            _ if b.provenance == Provenance::User => "Note",
            _ => "Point",
        };
        let body = b.body.split_whitespace().collect::<Vec<_>>().join(" ");
        lines.push(format!("{label}: {body}"));
    }
    for a in store.action_items(meeting).map_err(store_err)? {
        let owner = name(&a.owner_speaker_gid)
            .map(|n| format!(" (owner: {n})"))
            .unwrap_or_default();
        let due = a
            .due_text
            .map(|d| format!(" (due: {d})"))
            .unwrap_or_default();
        let done = if a.done { " [done]" } else { "" };
        lines.push(format!("Action: {}{owner}{due}{done}", a.text.trim()));
    }
    let mut text = lines.join("\n");
    if text.chars().count() > MAX_NOTES_CHARS {
        text = text.chars().take(MAX_NOTES_CHARS).collect();
    }
    Ok(text)
}

fn request(title: &str, notes: &str, lang: OutLang, tone: Tone) -> Request {
    let tone = match tone {
        Tone::Friendly => "friendly and warm, like a message to teammates",
        Tone::Neutral => "neutral and clear",
        Tone::Formal => "formal and polite, like a message to a client",
    };
    let language = match lang {
        OutLang::En => "English",
        OutLang::Vi => "Vietnamese (natural, with full diacritics)",
    };
    let system = format!(
        "You write short follow-up emails after meetings. Write in {language}. The tone is \
         {tone}. Use only the facts in the notes; never invent names, dates or numbers. Lines marked \
         \"Proposed, not agreed\" were only suggested: never present them as decided. \
         Structure: a one-line greeting, two or three sentences on what was decided, the \
         action items as a list (\"- item (owner, due)\"), and a one-line closing. Plain \
         text only: no Markdown headings, no bold. Do not sign with a name. Reply as JSON \
         {{\"subject\": string, \"body\": string}}."
    );
    let user = format!("Meeting: {title}\n\nNotes:\n{notes}");
    Request {
        messages: vec![Message::system(system), Message::user(user)],
        schema: Some(json!({
            "type": "object",
            "properties": {
                "subject": {"type": "string"},
                "body": {"type": "string"}
            },
            "required": ["subject", "body"],
            "additionalProperties": false
        })),
        max_tokens: MAX_OUTPUT_TOKENS,
        temperature: 0.4,
    }
}

fn parse(v: &Value) -> Result<Email, String> {
    let field = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("`{k}` must be a non-empty string"))
    };
    Ok(Email {
        subject: plain_text(field("subject")?).chars().take(200).collect(),
        body: plain_text(field("body")?),
    })
}

/// Drafts the email. Fails when the meeting has no notes yet.
pub fn draft(
    store: &Store,
    meeting: &str,
    llm: &mut dyn Llm,
    lang: OutLang,
    tone: Tone,
) -> Result<Email, String> {
    let m = store.get_meeting(meeting).map_err(store_err)?;
    let notes = notes_text(store, meeting)?;
    if notes.trim().is_empty() {
        return Err("this meeting has no notes yet".into());
    }
    let mut diag = Diagnostics::default();
    match complete_json(
        llm,
        request(&m.title, &notes, lang, tone),
        lang,
        &mut diag,
        |v, _| parse(v),
    )
    .map_err(|e| e.to_string())?
    {
        Outcome::Done(e) => Ok(e),
        Outcome::Truncated => Err("the draft was too long; try again".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_llm::{Completion, EngineInfo};
    use ghi_store::keys::{MemoryKeyStore, Protection};
    use ghi_store::store::{NewActionItem, NewMeeting, NewNoteBlock, NewSpeaker};
    use std::sync::{Arc, Mutex};

    struct Scripted(Arc<Mutex<Vec<String>>>, String);

    impl Llm for Scripted {
        fn engine(&self) -> EngineInfo {
            EngineInfo {
                name: "scripted".into(),
                version: "1".into(),
            }
        }
        fn context_tokens(&self) -> u32 {
            8192
        }
        fn complete(&mut self, req: &Request) -> ghi_llm::Result<Completion> {
            let text: Vec<&str> = req.messages.iter().map(|m| m.content.as_str()).collect();
            self.0.lock().unwrap().push(text.join("\n"));
            Ok(Completion {
                text: self.1.clone(),
                tokens_in: 10,
                tokens_out: 10,
                truncated: false,
            })
        }
    }

    fn store() -> (tempfile::TempDir, Store, String) {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        let m = s
            .create_meeting(NewMeeting {
                title: "Beta sync".into(),
                ..Default::default()
            })
            .unwrap()
            .gid;
        (tmp, s, m)
    }

    #[test]
    fn drafts_from_the_notes_in_the_chosen_language_and_tone() {
        let (_tmp, s, m) = store();
        let lan = s
            .add_speaker(
                &m,
                NewSpeaker {
                    display_name: Some("Lan".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        s.add_note_block(
            &m,
            NewNoteBlock {
                kind: "decision".into(),
                provenance: Provenance::Ai,
                body: "Beta ships in November".into(),
                anchors: Vec::new(),
                pinned: false,
            },
        )
        .unwrap();
        s.add_note_block(
            &m,
            NewNoteBlock {
                kind: "proposal".into(),
                provenance: Provenance::Ai,
                body: "Maybe move support to a new vendor".into(),
                anchors: Vec::new(),
                pinned: false,
            },
        )
        .unwrap();
        s.add_note_block(
            &m,
            NewNoteBlock {
                kind: "answer".into(),
                provenance: Provenance::Ai,
                body: "Q: Who owns QA?\nA: Nam.".into(),
                anchors: Vec::new(),
                pinned: true,
            },
        )
        .unwrap();
        s.add_action_item(
            &m,
            NewActionItem {
                text: "Send the deck".into(),
                owner_speaker_gid: Some(lan),
                due_text: Some("Friday".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut llm = Scripted(
            seen.clone(),
            r#"{"subject":"Ghi chú buổi họp","body":"Chào mọi người,\n- Send the deck (Lan, Friday)"}"#
                .into(),
        );
        let e = draft(&s, &m, &mut llm, OutLang::Vi, Tone::Formal).unwrap();
        assert_eq!(e.subject, "Ghi chú buổi họp");
        assert!(e.body.contains("Send the deck"));
        let prompt = seen.lock().unwrap().join("\n");
        assert!(prompt.contains("Vietnamese") && prompt.contains("formal"));
        assert!(prompt.contains("Decision: Beta ships in November"));
        // A proposal is its own group, not a "Point", and the model is told so.
        assert!(prompt.contains("Proposed, not agreed: Maybe move support to a new vendor"));
        assert!(!prompt.contains("Point: Maybe"));
        assert!(prompt.contains("Saved answer: Q: Who owns QA? A: Nam."));
        assert!(!prompt.contains("Point: Q:"));
        assert!(prompt.contains("never present them as decided"));
        assert!(prompt.contains("Action: Send the deck (owner: Lan) (due: Friday)"));
    }

    #[test]
    fn no_notes_no_draft() {
        let (_tmp, s, m) = store();
        let mut llm = Scripted(Arc::default(), String::new());
        assert!(draft(&s, &m, &mut llm, OutLang::En, Tone::Neutral).is_err());
    }
}
