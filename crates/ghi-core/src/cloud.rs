// SPDX-License-Identifier: Apache-2.0
//! "Improve with cloud" and cloud Ask for a stored meeting (doc 02 §K, D6):
//! opt-in per request, transcript text only, never audio, never the user's
//! own notes.
//!
//! 1. [`plan`] builds the request: the transcript with the speakers' names
//!    (and any extra names) redacted when asked, as the exact bytes, and the
//!    send preview the user reviews (host, payload, SHA-256, tokens, cost).
//!    A cloud-locked or sensitive meeting is refused before anything is built.
//! 2. [`send`] sends those bytes and nothing else: a `ghi_net::CloudGrant`
//!    is minted for exactly them. The reply's placeholders are put back
//!    locally, then the notes are saved (what the user wrote, pinned, edited
//!    or ticked off stays) or the answer is returned. Every request that may
//!    have left the device is logged; the meeting is marked as cloud-used.
//!    A failure is reported so the caller can fall back to the local model.

use ghi_llm::ask::{self, Answer};
use ghi_llm::cloud::{CloudProvider, Prepared};
use ghi_llm::notes::{self, Options};
use ghi_llm::preview::{Prices, SendPreview, preview};
use ghi_llm::redact::{KnownEntities, Redactor};
use ghi_llm::schema::Dialect;
use ghi_llm::template::{OutLang, Template};
use ghi_llm::transcript::Aliases;
use ghi_llm::validate::{Diagnostics, plain_text};
use ghi_llm::{Request, Transcript};
use ghi_net::{CloudGrant, MeetingGate, NetPolicy, Secret};
use ghi_store::store::{ReplacedNotes, Segment, Store};

use crate::notes_job::{previous_enhanced, save_notes_with, stored_transcript};

/// Transcript a cloud Ask may carry: the relevant parts, not the meeting.
const ASK_BUDGET_TOKENS: u32 = 12_000;

fn store_err(e: ghi_store::StoreError) -> String {
    e.to_string()
}

/// What to ask the cloud model for.
#[derive(Debug, Clone)]
pub enum Task {
    Notes { template: Template, lang: OutLang },
    Ask { question: String, lang: OutLang },
}

enum Prepared2 {
    Notes {
        opts: Options,
    },
    Ask {
        prepared: Box<ask::Prepared>,
        question: String,
    },
}

/// A request ready to send, held by the app between preview and confirm.
pub struct Plan {
    pub meeting: String,
    pub preview: SendPreview,
    /// Redacted values by kind (`person`, `email`, …) and count.
    pub redactions: Vec<(&'static str, usize)>,
    provider: CloudProvider,
    prepared: Prepared,
    gate: MeetingGate,
    redactor: Redactor,
    red: Transcript,
    aliases: Aliases,
    segs: Vec<Segment>,
    task: Prepared2,
    /// The transcript the request was built from (a send after an edit or a
    /// final pass is refused: review it again).
    version: i64,
    /// The notes template asked for was not a built-in one (the user's own),
    /// so the built-in General template is what the request carries.
    pub template_fallback: bool,
}

/// The template a cloud notes request may carry: a built-in one as it is.
/// Anything else (a template the user made: its titles and instructions are
/// theirs and stay on the device) becomes the built-in General template. This
/// holds whoever calls [`plan`], whatever id the template has.
fn cloud_template(template: Template) -> (Template, bool) {
    match ghi_llm::template::builtin(&template.id) {
        Ok(b) if b == template => (template, false),
        _ => (
            ghi_llm::template::builtin("general").expect("general is built in"),
            true,
        ),
    }
}

/// Longest excerpt of the request's text shown in the send sheet (characters).
pub const EXCERPT_MAX: usize = 4000;

impl Plan {
    /// The transcript version the request was built from.
    pub fn version(&self) -> i64 {
        self.version
    }

    /// The lines the request was built from (an answer's citation numbers
    /// point into these).
    pub fn segments(&self) -> &[Segment] {
        &self.segs
    }

    /// The question of an Ask plan, as the user typed it (not redacted).
    pub fn ask_question(&self) -> Option<&str> {
        match &self.task {
            Prepared2::Ask { question, .. } => Some(question),
            Prepared2::Notes { .. } => None,
        }
    }

    /// The request's user text before and after redaction, for the sheet's
    /// "on this Mac" / "what is sent" box, each cut to [`EXCERPT_MAX`]. `None`
    /// when the request shape has no user text. Never the system prompt.
    pub fn excerpts(&self) -> Option<(String, String)> {
        let after = ghi_llm::preview::user_text(&self.preview.payload);
        if after.is_empty() {
            return None;
        }
        let before = self.redactor.restore(&after).text;
        let cut = |s: String| match s.char_indices().nth(EXCERPT_MAX) {
            Some((i, _)) => format!("{}…", s[..i].trim_end()),
            None => s,
        };
        Some((cut(before), cut(after)))
    }

    /// A notes request (else an Ask).
    pub fn is_notes(&self) -> bool {
        matches!(self.task, Prepared2::Notes { .. })
    }
}

/// What [`plan`] found.
pub enum Planned {
    /// Review this, then [`send`] it.
    Send(Box<Plan>),
    /// An Ask nothing in the meeting matches: answered without a request.
    NotDiscussed(Answer),
}

/// The result of a send.
pub enum Sent {
    Notes(ReplacedNotes),
    Answer(Answer),
}

pub enum Outcome {
    Done(Sent),
    /// Nothing usable came back; `left_device`: the request may have been
    /// sent (it is logged). Fall back to the local model.
    Failed {
        reason: String,
        left_device: bool,
    },
}

/// The meeting's own privacy switches.
fn gate(store: &Store, meeting: &str) -> Result<MeetingGate, String> {
    let m = store.get_meeting(meeting).map_err(store_err)?;
    Ok(MeetingGate {
        cloud_locked: m.cloud_locked,
        sensitive: m.sensitive,
    })
}

/// The names the user gave this meeting's speakers (never "Me" or
/// "Speaker N", which aren't names) and the calendar attendees, each also by its first and last word
/// (a given name: "Sarah" of "Sarah Chen", "Lan" of "Nguyễn Thị Lan"), plus
/// `extra_names`.
fn people(store: &Store, meeting: &str, extra_names: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |n: &str| {
        let n = n.trim();
        if n.chars().count() >= 2 && !out.iter().any(|o| o == n) {
            out.push(n.to_string());
        }
    };
    let named = store.speakers(meeting).map_err(store_err)?;
    // The people in the calendar invite are known names too (doc 05 layer 2).
    let invited = crate::calendar::info(store, meeting)
        .map(|i| i.attendees)
        .unwrap_or_default();
    for full in named
        .iter()
        .filter_map(|s| s.display_name.as_deref())
        .chain(invited.iter().map(String::as_str))
        .chain(extra_names.iter().map(String::as_str))
    {
        add(full);
        let words: Vec<&str> = full.split_whitespace().collect();
        if words.len() > 1 {
            for w in [words[0], words[words.len() - 1]] {
                if w.chars().count() >= 3 {
                    add(w);
                }
            }
        }
    }
    Ok(out)
}

fn redactor_for(people: Vec<String>) -> Redactor {
    Redactor::new(KnownEntities {
        people,
        orgs: Vec::new(),
        terms: Vec::new(),
    })
}

/// Builds the request and its preview. `redact`: replace names and personal
/// data with placeholders (restored locally in the reply).
pub fn plan(
    store: &Store,
    meeting: &str,
    provider: CloudProvider,
    task: Task,
    redact: bool,
    extra_names: &[String],
    prices: &Prices,
) -> Result<Planned, String> {
    let gate = gate(store, meeting)?;
    if gate.cloud_locked || gate.sensitive {
        return Err("cloud AI is off for this meeting".into());
    }
    let version = store
        .get_meeting(meeting)
        .map_err(store_err)?
        .transcript_version;
    let (t, segs) = stored_transcript(store, meeting)?;
    if t.is_empty() {
        return Err("this meeting has no transcript yet".into());
    }
    let mut redactor = if redact {
        redactor_for(people(store, meeting, extra_names)?)
    } else {
        Redactor::new(KnownEntities::default())
    };
    let red = if redact {
        redactor.redact_transcript(&t)
    } else {
        t.clone()
    };
    let aliases = Aliases::new(&red);
    let mut template_fallback = false;
    let (request, state): (Request, Prepared2) = match task {
        Task::Notes { template, lang } => {
            let (template, fell_back) = cloud_template(template);
            template_fallback = fell_back;
            // Transcript text only: never the user's own notes (so no
            // `pinned` either; what they wrote stays on this device). Marks
            // stay local too: `Options::marks` is empty here, and the cloud
            // dialect leaves it out regardless.
            let opts = Options::new(template, lang);
            let all: Vec<&ghi_llm::Segment> = red.segments().iter().collect();
            let (req, _) = notes::request(&red, &all, &aliases, &opts, Dialect::Cloud);
            (req, Prepared2::Notes { opts })
        }
        Task::Ask { question, lang } => {
            let q = if redact {
                redactor.redact(&question)
            } else {
                question.clone()
            };
            let p = ask::prepare(&red, &aliases, &q, lang, ASK_BUDGET_TOKENS, Dialect::Cloud)
                .map_err(|e| e.to_string())?;
            match p {
                Ok(p) => (
                    p.request.clone(),
                    Prepared2::Ask {
                        prepared: Box::new(p),
                        question,
                    },
                ),
                // Nothing matched: answered without sending anything (the
                // terms shown are the user's, not the redacted ones).
                Err(_) => {
                    return Ok(Planned::NotDiscussed(Answer::NotDiscussed {
                        searched: ghi_llm::retrieval::query_terms(&question),
                    }));
                }
            }
        }
    };
    let prepared = provider.prepare(&request).map_err(|e| e.to_string())?;
    let pv = preview(&provider, &prepared, prices);
    Ok(Planned::Send(Box::new(Plan {
        meeting: meeting.to_string(),
        preview: pv,
        template_fallback,
        redactions: redactor.summary(),
        provider,
        prepared,
        gate,
        redactor,
        red,
        aliases,
        segs,
        task: state,
        version,
    })))
}

/// Sends a reviewed plan (exactly its bytes) and saves or returns the result.
pub fn send(store: &Store, plan: Plan, key: &Secret, policy: NetPolicy) -> Result<Outcome, String> {
    // The meeting may have been locked, or its transcript changed, since
    // the preview.
    let now = gate(store, &plan.meeting)?;
    if store
        .get_meeting(&plan.meeting)
        .map_err(store_err)?
        .transcript_version
        != plan.version
    {
        return Err("the transcript changed since this preview: review it again".into());
    }
    if now.cloud_locked || now.sensitive || plan.gate.cloud_locked || plan.gate.sensitive {
        return Err("cloud AI is off for this meeting".into());
    }
    let mut grant = CloudGrant::mint(policy, &plan.prepared.url, &plan.prepared.body, &[now])
        .map_err(|e| format!("cloud send refused: {e}"))?;
    let log = |tokens_in: u64, tokens_out: u64| {
        store
            .record_cloud_request(
                &plan.meeting,
                plan.provider.name(),
                &plan.provider.model,
                tokens_in,
                tokens_out,
            )
            .map_err(store_err)
    };
    let completion = match plan.provider.send(&plan.prepared, &mut grant, key) {
        Ok(c) => c,
        Err(e) => {
            // Once a send was attempted, the body may have left the device.
            log(0, 0)?;
            return Ok(Outcome::Failed {
                reason: e.to_string(),
                left_device: true,
            });
        }
    };
    log(
        u64::from(completion.tokens_in),
        u64::from(completion.tokens_out),
    )?;
    let failed = |reason: String| {
        Ok(Outcome::Failed {
            reason,
            left_device: true,
        })
    };
    if completion.truncated {
        return failed("the reply hit the token limit".into());
    }
    let mut diag = Diagnostics {
        requests: 1,
        tokens_in: u64::from(completion.tokens_in),
        tokens_out: u64::from(completion.tokens_out),
        ..Default::default()
    };
    match plan.task {
        Prepared2::Notes { opts } => {
            let mut n =
                match notes::parse(&completion.text, &plan.red, &plan.aliases, &opts, &mut diag) {
                    Ok(n) => n,
                    Err(e) => return failed(e.to_string()),
                };
            // Put the redacted values back, locally; plain text only.
            n.map_text(|s| plain_text(&plan.redactor.restore(s).text));
            // The user's own notes keep their expansions (cloud gets no notes).
            let keep = previous_enhanced(store, &plan.meeting)?;
            let saved = save_notes_with(
                store,
                &plan.meeting,
                &n,
                &plan.segs,
                keep,
                &plan.provider.model,
            )?;
            Ok(Outcome::Done(Sent::Notes(saved)))
        }
        Prepared2::Ask { prepared, question } => {
            match prepared.parse(&completion.text, &mut diag) {
                Ok(mut answer) => {
                    match &mut answer {
                        Answer::Answered { text, .. } => {
                            *text = plain_text(&plan.redactor.restore(text).text);
                        }
                        Answer::NotDiscussed { searched } => {
                            *searched = ghi_llm::retrieval::query_terms(&question);
                        }
                    }
                    Ok(Outcome::Done(Sent::Answer(answer)))
                }
                Err(e) => failed(e.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_store::keys::{MemoryKeyStore, Protection};
    use ghi_store::store::{NewMeeting, NewSegment, NewSpeaker};
    use std::sync::Arc;

    fn meeting(store: &Store) -> String {
        let m = store.create_meeting(NewMeeting::default()).unwrap().gid;
        let lan = store
            .add_speaker(
                &m,
                NewSpeaker {
                    display_name: Some("Nguyễn Thị Lan".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .add_segments(
                &m,
                vec![NewSegment {
                    speaker_gid: Some(lan),
                    t0_ms: 0,
                    t1_ms: 3000,
                    text: "Nguyễn Thị Lan will send me the pricing deck on Friday, Lan said".into(),
                    lang: Some("en".into()),
                    ..Default::default()
                }],
            )
            .unwrap();
        m
    }

    fn store() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        (tmp, s)
    }

    fn notes_task() -> Task {
        Task::Notes {
            template: ghi_llm::template::builtin("general").unwrap(),
            lang: OutLang::En,
        }
    }

    /// Whatever the caller passes, a template the user made never reaches the payload.
    #[test]
    fn a_template_that_is_not_built_in_is_swapped_for_general_inside_plan() {
        use ghi_llm::template::{Editor, EditorSection};
        let (_tmp, store) = store();
        let m = meeting(&store);
        let mine = Template::from_editor(
            "t77",
            &Editor {
                name: "Zebra quarterly".into(),
                lang: OutLang::En,
                guidance: "XYLOPHONE-GUIDANCE zebras".into(),
                sections: vec![EditorSection {
                    id: None,
                    title: "Quokka findings".into(),
                    instruction: "QUOKKA-INSTRUCTION list every marsupial".into(),
                }],
            },
            &[],
            &[],
        )
        .unwrap();
        // A user's template that borrows a built-in's id is no more the built-in than any other.
        let mut disguised = mine.clone();
        disguised.id = "standup".into();
        let provider = CloudProvider::preset("openai", "gpt-4.1-mini").unwrap();
        let send = |template: Template| {
            let Planned::Send(p) = plan(
                &store,
                &m,
                provider.clone(),
                Task::Notes {
                    template,
                    lang: OutLang::En,
                },
                true,
                &[],
                &Prices::builtin(),
            )
            .unwrap() else {
                panic!("a request")
            };
            p
        };
        for t in [mine.clone(), disguised] {
            let p = send(t);
            assert!(p.template_fallback);
            for secret in ["XYLOPHONE", "QUOKKA", "Zebra", "Quokka", "t77"] {
                assert!(
                    !p.preview.payload.contains(secret),
                    "{secret} left the device: {}",
                    p.preview.payload
                );
            }
            assert!(
                p.preview.payload.contains("pricing deck"),
                "the transcript is still sent"
            );
        }
        // A built-in template goes as it is, with no fallback.
        let standup = ghi_llm::template::builtin("standup").unwrap();
        let p = send(standup);
        assert!(!p.template_fallback);
        assert!(
            p.preview.payload.contains("blockers"),
            "{}",
            p.preview.payload
        );
    }

    #[test]
    fn the_preview_is_the_redacted_request_and_holds_no_audio_or_notes() {
        let (_tmp, store) = store();
        let m = meeting(&store);
        let provider = CloudProvider::preset("openai", "gpt-4.1-mini").unwrap();
        let Planned::Send(p) = plan(
            &store,
            &m,
            provider.clone(),
            notes_task(),
            true,
            &[],
            &Prices::builtin(),
        )
        .unwrap() else {
            panic!("a request")
        };
        assert_eq!(p.preview.host, "api.openai.com");
        assert!(!p.preview.payload.contains("Nguyễn Thị Lan"), "redacted");
        assert!(
            !p.preview.payload.contains("Lan said"),
            "the given name too"
        );
        assert!(
            p.preview.payload.contains("send me"),
            "\"me\" is not a name"
        );
        assert!(p.preview.payload.contains("pricing deck"));
        assert!(p.redactions.iter().any(|(_, n)| *n > 0));
        assert_eq!(p.preview.sha256, ghi_net::sha256_hex(&p.prepared.body));
        // The sheet's before / after box: names back on this Mac, hidden in
        // what is sent, and never the system prompt.
        let (before, after) = p.excerpts().expect("user text");
        assert!(before.contains("Nguyễn Thị Lan"), "{before}");
        assert!(!after.contains("Nguyễn Thị Lan"), "{after}");
        assert!(after.contains("pricing deck"));
        assert!(!after.contains("JSON"), "not the system prompt: {after}");
        // Same input, same bytes: the user confirms what is sent.
        let Planned::Send(again) = plan(
            &store,
            &m,
            provider,
            notes_task(),
            true,
            &[],
            &Prices::builtin(),
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(again.preview.sha256, p.preview.sha256);
    }

    #[test]
    fn the_preview_holds_no_marks_block() {
        let (_tmp, store) = store();
        let m = meeting(&store);
        store
            .add_mark(&m, 1000, ghi_store::store::MarkTag::Decision)
            .unwrap();
        store
            .add_mark(&m, 2000, ghi_store::store::MarkTag::Star)
            .unwrap();
        let Planned::Send(p) = plan(
            &store,
            &m,
            CloudProvider::preset("openai", "gpt-4.1-mini").unwrap(),
            notes_task(),
            true,
            &[],
            &Prices::builtin(),
        )
        .unwrap() else {
            panic!("a request")
        };
        let body = String::from_utf8_lossy(&p.prepared.body).to_string();
        for text in [&p.preview.payload, &body] {
            assert!(text.contains("pricing deck"), "the transcript is sent");
            assert!(!text.contains("marked these"), "{text}");
            assert!(
                !text.contains("decision]") && !text.contains("star]"),
                "{text}"
            );
        }
        // The same marks do reach a local prompt (see tests/notes_marks.rs).
    }

    #[test]
    fn a_locked_or_sensitive_meeting_is_refused() {
        let (_tmp, store) = store();
        let m = meeting(&store);
        store.set_cloud_locked(&m, true).unwrap();
        let provider = CloudProvider::preset("anthropic", "claude-haiku-4-5").unwrap();
        let r = plan(
            &store,
            &m,
            provider,
            notes_task(),
            true,
            &[],
            &Prices::builtin(),
        );
        assert!(r.is_err());
    }

    #[test]
    fn strict_offline_refuses_the_send_and_nothing_is_logged() {
        let (_tmp, store) = store();
        let m = meeting(&store);
        let provider = CloudProvider::preset("openai", "gpt-4.1-mini").unwrap();
        let Planned::Send(p) = plan(
            &store,
            &m,
            provider,
            notes_task(),
            true,
            &[],
            &Prices::builtin(),
        )
        .unwrap() else {
            panic!()
        };
        let r = send(&store, *p, &Secret::new("k"), NetPolicy::StrictOffline);
        assert!(r.is_err(), "refused before anything leaves");
        assert!(!store.get_meeting(&m).unwrap().cloud_used);
    }

    #[test]
    fn an_ask_with_no_match_needs_no_request() {
        let (_tmp, store) = store();
        let m = meeting(&store);
        let provider = CloudProvider::preset("openai", "gpt-4.1-mini").unwrap();
        let r = plan(
            &store,
            &m,
            provider,
            Task::Ask {
                question: "what about the kubernetes migration budget".into(),
                lang: OutLang::En,
            },
            true,
            &[],
            &Prices::builtin(),
        )
        .unwrap();
        // A one-line meeting is short: the whole transcript may still be sent.
        match r {
            Planned::NotDiscussed(Answer::NotDiscussed { searched }) => {
                assert!(!searched.is_empty())
            }
            Planned::Send(p) => assert!(p.preview.payload.contains("pricing deck")),
            Planned::NotDiscussed(_) => panic!(),
        }
    }

    #[test]
    fn calendar_attendees_are_known_names_in_the_redaction() {
        let (_tmp, store) = store();
        let m = meeting(&store);
        let info = crate::calendar::CalendarInfo {
            event: "e".into(),
            title: "Sync".into(),
            attendees: vec!["Hoàng Gia Bảo".into()],
            emails: Vec::new(),
            calendar: None,
        };
        store
            .set_calendar_info(&m, Some(&serde_json::to_value(&info).unwrap()))
            .unwrap();
        let p = people(&store, &m, &[]).unwrap();
        assert!(p.contains(&"Nguyễn Thị Lan".to_string()));
        assert!(p.contains(&"Hoàng Gia Bảo".to_string()));
        assert!(p.contains(&"Hoàng".to_string()) && p.contains(&"Bảo".to_string()));
    }
}
