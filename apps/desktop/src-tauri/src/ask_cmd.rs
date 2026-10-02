// SPDX-License-Identifier: Apache-2.0
//! Ask across meetings (D9) and the library's "related" results (phase 14b).
//!
//! Both run on this device. Retrieval is hybrid (keywords + meaning, fused:
//! `ghi_core::ask_all`); the meaning half needs the embedding model, which
//! 8 GB Macs don't get, so there it is keywords only. The answer comes from
//! the local notes model, like "Ask this meeting", and cites lines of each
//! meeting it used.

use std::collections::HashMap;

use ghi_core::ask_all::{self, Passage, Scope};
use ghi_llm::ask::Answer;
use ghi_llm::template::OutLang;
use ghi_store::store::{Meeting, Store};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::cloud_cmd::{local_model_free, out_lang};
use crate::detail::{Citation, NotesLanguage, QUOTE_CHARS, segment_citation, shorten};
use crate::{CoreState, blocking};

fn err(e: ghi_store::StoreError) -> String {
    e.to_string()
}

/// Where to look (empty lists: everywhere / everyone).
#[derive(Debug, Clone, Default, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AskScope {
    pub meetings: Vec<String>,
    /// Meeting start, unix ms, inclusive.
    pub from_ms: Option<f64>,
    pub to_ms: Option<f64>,
    pub persons: Vec<String>,
}

impl AskScope {
    fn core(self) -> Scope {
        let ms = |v: Option<f64>| v.filter(|x| x.is_finite()).map(|x| x as i64);
        Scope {
            meetings: self.meetings,
            from_ms: ms(self.from_ms),
            to_ms: ms(self.to_ms),
            person_gids: self.persons,
        }
    }
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MeetingRef {
    pub meeting: String,
    pub title: String,
    pub started_at: f64,
}

impl MeetingRef {
    fn of(m: &Meeting) -> MeetingRef {
        MeetingRef {
            meeting: m.gid.clone(),
            title: m.title.clone(),
            started_at: m.started_at as f64,
        }
    }
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AskAllCitation {
    pub meeting: MeetingRef,
    pub citation: Citation,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AskAllAnswer {
    /// False: "Not discussed in these meetings".
    pub answered: bool,
    pub text: String,
    pub citations: Vec<AskAllCitation>,
    /// The terms looked for (not discussed).
    pub searched: Vec<String>,
    /// The meetings the answer was looked for in (passages read), newest
    /// first.
    pub sources: Vec<MeetingRef>,
    /// Meaning search took part (false: keywords only).
    pub semantic: bool,
}

/// A passage close in meaning to the search text.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelatedHit {
    pub meeting: MeetingRef,
    pub t0_ms: f64,
    pub t1_ms: f64,
    /// The passage's first words.
    pub quote: String,
}

/// Meetings by gid, each read once.
struct Meetings<'a>(&'a Store, HashMap<String, Meeting>);

impl Meetings<'_> {
    fn get(&mut self, gid: &str) -> Result<&Meeting, String> {
        if !self.1.contains_key(gid) {
            let m = self.0.get_meeting(gid).map_err(err)?;
            self.1.insert(gid.to_string(), m);
        }
        Ok(&self.1[gid])
    }
}

fn lang_for(
    language: NotesLanguage,
    store: &Store,
    passages: &[Passage],
) -> Result<OutLang, String> {
    match (language, passages.first()) {
        (NotesLanguage::Meeting, None) => Ok(OutLang::En),
        // The language of the best-matching meeting.
        (NotesLanguage::Meeting, Some(p)) => out_lang(language, store, &p.meeting_gid),
        (l, _) => out_lang(l, store, ""),
    }
}

/// Ask across meetings, answered on this device by the local model.
#[tauri::command]
#[specta::specta]
pub async fn ask_all_meetings(
    core: CoreState<'_>,
    question: String,
    scope: AskScope,
    language: NotesLanguage,
) -> Result<AskAllAnswer, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        local_model_free(c, &store)?;
        let question: String = question.chars().take(1000).collect();
        if question.trim().is_empty() {
            return Err("ask a question".into());
        }
        let scope = scope.core();
        let (passages, semantic) = c.with_query_embedder(&store, |e| {
            let semantic = e.is_some();
            ask_all::retrieve(&store, e, &question, &scope, ask_all::ASK_PASSAGES)
                .map(|p| (p, semantic))
        })?;
        let lang = lang_for(language, &store, &passages)?;
        let bytes: usize = 2 * ask_all::ASK_PASSAGES * ghi_core::index_job::MAX_CHARS;
        let mut llm = (c.llm()?)(bytes)?;
        let r = ask_all::ask_all(&store, llm.as_mut(), &passages, &question, lang)?;
        drop(llm);

        let mut meetings = Meetings(&store, HashMap::new());
        let mut sources: Vec<MeetingRef> = Vec::new();
        for p in &passages {
            if !sources.iter().any(|s| s.meeting == p.meeting_gid) {
                sources.push(MeetingRef::of(meetings.get(&p.meeting_gid)?));
            }
        }
        sources.sort_by(|a, b| b.started_at.total_cmp(&a.started_at));
        // The app may have locked while the model was answering.
        c.store()?;
        Ok(match r.answer {
            Answer::Answered { text, .. } => AskAllAnswer {
                answered: true,
                text,
                citations: r
                    .citations
                    .iter()
                    .map(|(gid, s)| {
                        let m = meetings.get(gid)?;
                        Ok(AskAllCitation {
                            citation: segment_citation(s, m.transcript_version),
                            meeting: MeetingRef::of(m),
                        })
                    })
                    .collect::<Result<_, String>>()?,
                searched: Vec::new(),
                sources,
                semantic,
            },
            Answer::NotDiscussed { searched } => AskAllAnswer {
                answered: false,
                text: String::new(),
                citations: Vec::new(),
                searched,
                sources,
                semantic,
            },
        })
    })
    .await
}

/// Passages close in meaning to `text`, at most one per meeting (empty when
/// meaning search is off or the model isn't installed yet).
#[tauri::command]
#[specta::specta]
pub async fn related_meetings(
    core: CoreState<'_>,
    text: String,
    scope: AskScope,
    limit: u32,
) -> Result<Vec<RelatedHit>, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let text: String = text.chars().take(500).collect();
        if text.trim().chars().count() < 3 {
            return Ok(Vec::new());
        }
        let scope = scope.core();
        let limit = limit.clamp(1, 20) as usize;
        let passages = c.with_query_embedder(&store, |e| match e {
            // More passages than meetings asked for: several may share one.
            Some(e) => ask_all::related(&store, e, &text, &scope, limit * 4),
            None => Ok(Vec::new()),
        })?;
        let mut meetings = Meetings(&store, HashMap::new());
        let mut out: Vec<RelatedHit> = Vec::new();
        for p in passages {
            if out.len() == limit {
                break;
            }
            if out.iter().any(|h| h.meeting.meeting == p.meeting_gid) {
                continue;
            }
            let m = MeetingRef::of(meetings.get(&p.meeting_gid)?);
            let words: Vec<String> = ask_all::passage_lines(&store, &p)
                .into_iter()
                .map(|s| s.text)
                .collect();
            out.push(RelatedHit {
                meeting: m,
                t0_ms: p.t0_ms as f64,
                t1_ms: p.t1_ms as f64,
                quote: shorten(&words.join(" "), QUOTE_CHARS),
            });
        }
        // The app may have locked meanwhile.
        c.store()?;
        Ok(out)
    })
    .await
}
