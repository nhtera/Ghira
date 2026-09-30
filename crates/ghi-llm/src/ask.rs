// SPDX-License-Identifier: Apache-2.0
//! "Ask this meeting" (doc 02 §G): an answer with citations, or "not
//! discussed" together with what was searched (design rationale #3: the user
//! sees why nothing was found instead of a guess).
//!
//! A transcript that fits in the model's context is sent whole; a longer one
//! is narrowed to the best lexical matches and their neighbours.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::retrieval::{Index, query_terms};
use crate::run::{self, Outcome, PROMPT_OVERHEAD, RETRY_RESERVE, estimate_tokens};
use crate::schema::{self, Dialect, Shape};
use crate::template::OutLang;
use crate::transcript::{Aliases, Segment, Transcript, render};
use crate::validate::{Cites, Diagnostics, extract_json, plain_text};
use crate::{EngineInfo, Llm, LlmError, Message, Request, Result, prompt};

/// Best matches kept for a long transcript (each with ±1 neighbour).
const TOP_K: usize = 12;
const MAX_ANSWER_TOKENS: u32 = 768;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Answer {
    Answered {
        text: String,
        citations: Vec<u64>,
    },
    /// Nothing in the meeting answers it; `searched` are the (folded) terms looked for.
    NotDiscussed {
        searched: Vec<String>,
    },
}

#[derive(Debug, Clone)]
pub struct Run {
    pub answer: Answer,
    pub engine: EngineInfo,
    pub diagnostics: Diagnostics,
}

/// The segments a question is asked over, and the request.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub request: Request,
    ids: BTreeSet<u64>,
    searched: Vec<String>,
    aliases: Aliases,
}

/// Picks the segments and builds the request, or `None` when a long
/// transcript has no match at all (answered "not discussed" without a call).
pub fn prepare(
    t: &Transcript,
    aliases: &Aliases,
    question: &str,
    lang: OutLang,
    budget: u32,
    dialect: Dialect,
) -> Result<std::result::Result<Prepared, Answer>> {
    let question = plain_text(question);
    if question.is_empty() || t.is_empty() {
        return Err(LlmError::Invalid("empty question or transcript".into()));
    }
    let searched = query_terms(&question);
    let all: Vec<&Segment> = t.segments().iter().collect();
    let segs: Vec<&Segment> = if estimate_tokens(&render(t, aliases, &all)) <= budget {
        all
    } else {
        let hits = Index::new(t).search(&question, TOP_K);
        if hits.is_empty() {
            return Ok(Err(Answer::NotDiscussed { searched }));
        }
        let n = t.segments().len();
        // Best matches first, each with its neighbours, while they fit;
        // then back in time order.
        let mut picked: BTreeSet<usize> = BTreeSet::new();
        for i in hits {
            let mut with = picked.clone();
            with.extend(i.saturating_sub(1)..=(i + 1).min(n - 1));
            let segs: Vec<&Segment> = with.iter().map(|&j| &t.segments()[j]).collect();
            if !picked.is_empty() && estimate_tokens(&render(t, aliases, &segs)) > budget {
                break;
            }
            picked = with;
        }
        picked.iter().map(|&i| &t.segments()[i]).collect()
    };
    let ids: BTreeSet<u64> = segs.iter().map(|s| s.id).collect();
    let id_list: Vec<u64> = ids.iter().copied().collect();
    let shape = Shape {
        dialect,
        ids: &id_list,
        speakers: aliases.aliases(),
    };
    let request = Request {
        messages: vec![
            Message::system(prompt::ask_system(lang)),
            Message::user(prompt::ask_task(
                lang,
                &question,
                &render(t, aliases, &segs),
            )),
        ],
        schema: Some(schema::ask(&shape)),
        max_tokens: MAX_ANSWER_TOKENS,
        temperature: 0.2,
    };
    Ok(Ok(Prepared {
        request,
        ids,
        searched,
        aliases: aliases.clone(),
    }))
}

impl Prepared {
    pub fn parse(&self, reply: &str, diag: &mut Diagnostics) -> Result<Answer> {
        extract_json(reply)
            .and_then(|v| self.parse_value(&v, diag))
            .map_err(LlmError::InvalidOutput)
    }

    fn parse_value(
        &self,
        v: &serde_json::Value,
        d: &mut Diagnostics,
    ) -> std::result::Result<Answer, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            discussed: bool,
            answer: String,
            cite: Vec<i64>,
        }
        let raw: Raw = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
        let allowed = |id: u64| self.ids.contains(&id);
        let citations = Cites::new(&allowed).keep(&raw.cite, d);
        let text = self.aliases.expand(&plain_text(&raw.answer));
        if !raw.discussed || text.is_empty() || citations.is_empty() {
            if raw.discussed {
                d.dropped_items += 1;
            }
            return Ok(Answer::NotDiscussed {
                searched: self.searched.clone(),
            });
        }
        Ok(Answer::Answered { text, citations })
    }
}

/// Answers `question` with the local model (or any [`Llm`]).
pub fn ask(llm: &mut dyn Llm, t: &Transcript, question: &str, lang: OutLang) -> Result<Run> {
    let aliases = Aliases::new(t);
    let budget = llm
        .context_tokens()
        .saturating_sub(MAX_ANSWER_TOKENS + PROMPT_OVERHEAD + RETRY_RESERVE)
        .max(512);
    let mut diag = Diagnostics::default();
    let answer = match prepare(t, &aliases, question, lang, budget, Dialect::Local)? {
        Err(answer) => answer,
        Ok(p) => match run::complete_json(llm, p.request.clone(), lang, &mut diag, |v, d| {
            p.parse_value(v, d)
        })? {
            Outcome::Done(a) => a,
            Outcome::Truncated => {
                return Err(LlmError::InvalidOutput(
                    "the answer hit the token limit".into(),
                ));
            }
        },
    };
    Ok(Run {
        answer,
        engine: llm.engine(),
        diagnostics: diag,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::tests::{Scripted, meeting};
    use crate::transcript::tests::seg;

    #[test]
    fn answers_with_valid_citations_only() {
        let t = meeting();
        let mut llm =
            Scripted::new(&[r#"{"discussed":true,"answer":"Nam gửi trước thứ Sáu","cite":[1,9]}"#]);
        let run = ask(&mut llm, &t, "Ai gửi tài liệu?", OutLang::Vi).unwrap();
        assert_eq!(
            run.answer,
            Answer::Answered {
                text: "Nam gửi trước thứ Sáu".into(),
                citations: vec![1]
            }
        );
        assert_eq!(run.diagnostics.dropped_cites, 1);
    }

    #[test]
    fn not_discussed_reports_the_search_terms() {
        let t = meeting();
        let mut llm = Scripted::new(&[r#"{"discussed":false,"answer":"","cite":[]}"#]);
        let run = ask(&mut llm, &t, "Hợp đồng Đà Nẵng?", OutLang::Vi).unwrap();
        assert_eq!(
            run.answer,
            Answer::NotDiscussed {
                searched: vec!["da".into(), "dong".into(), "hop".into(), "nang".into()]
            }
        );
        // "Discussed" without a valid citation is not an answer either.
        let mut llm = Scripted::new(&[r#"{"discussed":true,"answer":"Yes","cite":[42]}"#]);
        let run = ask(&mut llm, &t, "budget?", OutLang::En).unwrap();
        assert!(matches!(run.answer, Answer::NotDiscussed { .. }));
    }

    #[test]
    fn long_transcripts_are_narrowed_or_answered_without_a_call() {
        let mut segs: Vec<_> = (0..400u64)
            .map(|i| {
                seg(
                    i,
                    i as f64,
                    i as f64 + 1.0,
                    "S1",
                    &format!("filler sentence number {i} {}", "x".repeat(60)),
                    "en",
                )
            })
            .collect();
        segs[200].text = "The launch venue is Da Nang".into();
        let t = Transcript::new(segs).unwrap();
        let mut llm = Scripted::new(&[r#"{"discussed":true,"answer":"Da Nang","cite":[200]}"#]);
        llm.context = 6000;
        let run = ask(&mut llm, &t, "where is the launch?", OutLang::En).unwrap();
        assert!(matches!(run.answer, Answer::Answered { .. }));
        let prompt = &llm.requests[0].messages[1].content;
        assert!(prompt.contains("[s199]") && prompt.contains("[s201]") && !prompt.contains("[s5]"));

        let mut none = Scripted::new(&[]);
        none.context = 6000;
        let run = ask(&mut none, &t, "calendar sync?", OutLang::En).unwrap();
        assert!(matches!(run.answer, Answer::NotDiscussed { .. }));
        assert_eq!(run.diagnostics.requests, 0);
    }
}
