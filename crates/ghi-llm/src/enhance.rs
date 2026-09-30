// SPDX-License-Identifier: Apache-2.0
//! Enhance the user's notes (doc 02 §D, Granola-style).
//!
//! Each line the user typed keeps its text exactly; the AI adds up to four
//! supporting points from the transcript, each cited, or flags the line
//! `not_found` when the transcript doesn't support it. The context of a line
//! is the ±90 s around when it was typed plus its best lexical matches.
//! Lines are sent in batches that fit the model's context; a cloud provider
//! gets one batch (see [`plan`]).

use std::collections::{BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::notes::Item;
use crate::retrieval::Index;
use crate::run::{self, Outcome, PROMPT_OVERHEAD, RETRY_RESERVE, estimate_tokens};
use crate::schema::{self, Dialect, MAX_POINTS, Shape};
use crate::template::OutLang;
use crate::transcript::{Aliases, Segment, Transcript, render};
use crate::validate::{
    Cites, Diagnostics, content_words, extract_json, plain_text, snap, supported,
};
use crate::{EngineInfo, Llm, LlmError, Message, Request, Result, prompt};

/// ±this much audio around a line's typing time is its context.
const WINDOW_MS: i64 = 90_000;
/// Lexical matches added to a line's context.
const MATCHES: usize = 4;
/// Lines per request for a large (cloud) model. The local model gets one line
/// per request: small models mix up the lines of a batch.
pub const CLOUD_BATCH_LINES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteLine {
    pub text: String,
    /// Audio time when the line was typed (notepad keystroke anchor).
    pub t_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Enhanced {
    /// Exactly what the user typed.
    pub user_text: String,
    pub points: Vec<Item>,
    /// The transcript has nothing that supports the line.
    pub not_found: bool,
}

#[derive(Debug, Clone)]
pub struct Run {
    pub lines: Vec<Enhanced>,
    pub engine: EngineInfo,
    pub diagnostics: Diagnostics,
}

/// A group of lines sent in one request, with each line's context.
#[derive(Debug, Clone)]
pub struct Batch {
    /// Indexes into the caller's lines.
    pub lines: Vec<usize>,
    /// Per line: segment ids it may cite.
    contexts: Vec<BTreeSet<u64>>,
    aliases: Aliases,
    lang: OutLang,
    /// Text and language of every segment in the batch, to check support.
    segments: HashMap<u64, Segment>,
    /// Content words of each line (for the relevance check).
    words: Vec<HashSet<String>>,
}

/// Groups the lines that have any context into batches of at most
/// `max_lines` lines and `budget` transcript tokens.
pub fn plan(
    t: &Transcript,
    aliases: &Aliases,
    lines: &[NoteLine],
    lang: OutLang,
    budget: u32,
    max_lines: usize,
) -> Vec<Batch> {
    let ix = Index::new(t);
    let empty = || Batch {
        lines: Vec::new(),
        contexts: Vec::new(),
        aliases: aliases.clone(),
        lang,
        segments: HashMap::new(),
        words: Vec::new(),
    };
    let mut batches: Vec<Batch> = Vec::new();
    let mut cur = empty();
    for (i, line) in lines.iter().enumerate() {
        let ctx = context(t, &ix, line);
        if ctx.is_empty() {
            continue;
        }
        let mut union: BTreeSet<u64> = cur.contexts.iter().flatten().copied().collect();
        union.extend(&ctx);
        let fits = estimate_tokens(&render_ids(t, aliases, &union)) <= budget;
        if !cur.lines.is_empty() && (!fits || cur.lines.len() >= max_lines.max(1)) {
            batches.push(std::mem::replace(&mut cur, empty()));
        }
        for id in &ctx {
            if let Some(s) = t.get(*id) {
                cur.segments.insert(*id, s.clone());
            }
        }
        cur.lines.push(i);
        cur.contexts.push(ctx);
        cur.words.push(content_words(&plain_text(&line.text)));
    }
    if !cur.lines.is_empty() {
        batches.push(cur);
    }
    batches
}

fn context(t: &Transcript, ix: &Index, line: &NoteLine) -> BTreeSet<u64> {
    let text = plain_text(&line.text);
    if text.is_empty() {
        return BTreeSet::new();
    }
    // A lexical match must share two content words with the line (one if the
    // line has only one), so a common word alone doesn't make context.
    let words = content_words(&text);
    let need = words.len().min(2);
    if need == 0 && line.t_ms.is_none() {
        return BTreeSet::new();
    }
    let mut ids: BTreeSet<u64> = ix
        .search(&text, MATCHES)
        .into_iter()
        .map(|i| &t.segments()[i])
        .filter(|s| need > 0 && content_words(&s.text).intersection(&words).count() >= need)
        .map(|s| s.id)
        .collect();
    if let (Some(at), true) = (line.t_ms, t.has_times()) {
        ids.extend(
            t.segments()
                .iter()
                .filter(|s| s.t1_ms >= at - WINDOW_MS && s.t0_ms <= at + WINDOW_MS)
                .map(|s| s.id),
        );
    }
    ids
}

fn render_ids(t: &Transcript, aliases: &Aliases, ids: &BTreeSet<u64>) -> String {
    let mut segs: Vec<&Segment> = ids.iter().filter_map(|id| t.get(*id)).collect();
    segs.sort_by_key(|s| t.index_of(s.id));
    render(t, aliases, &segs)
}

impl Batch {
    pub fn request(
        &self,
        t: &Transcript,
        aliases: &Aliases,
        lines: &[NoteLine],
        lang: OutLang,
        dialect: Dialect,
    ) -> Request {
        let union: BTreeSet<u64> = self.contexts.iter().flatten().copied().collect();
        let ids: Vec<u64> = union.iter().copied().collect();
        let notes = self
            .lines
            .iter()
            .enumerate()
            .map(|(n, &i)| format!("{}. {}", n + 1, plain_text(&lines[i].text)))
            .collect::<Vec<_>>()
            .join("\n");
        let shape = Shape {
            dialect,
            ids: &ids,
            speakers: aliases.aliases(),
        };
        Request {
            messages: vec![
                Message::system(prompt::enhance_system(lang)),
                Message::user(prompt::enhance_task(
                    lang,
                    &notes,
                    &render_ids(t, aliases, &union),
                )),
            ],
            schema: Some(schema::enhance(&shape, self.lines.len())),
            max_tokens: 1024,
            temperature: 0.3,
        }
    }

    /// Per batch line: points (possibly empty) and whether it was found.
    pub fn parse(&self, reply: &str, diag: &mut Diagnostics) -> Result<Vec<(Vec<Item>, bool)>> {
        extract_json(reply)
            .and_then(|v| self.parse_value(&v, diag))
            .map_err(LlmError::InvalidOutput)
    }

    fn parse_value(
        &self,
        v: &serde_json::Value,
        d: &mut Diagnostics,
    ) -> std::result::Result<Vec<(Vec<Item>, bool)>, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            lines: Vec<RawLine>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawLine {
            line: i64,
            found: bool,
            points: Vec<RawPoint>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawPoint {
            text: String,
            cite: Vec<i64>,
        }
        let raw: Raw = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
        let mut by_line: HashMap<usize, RawLine> = HashMap::new();
        for l in raw.lines {
            match usize::try_from(l.line) {
                Ok(n) if (1..=self.lines.len()).contains(&n) => {
                    by_line.entry(n).or_insert(l);
                }
                _ => return Err(format!("line {} is not one of the notes", l.line)),
            }
        }
        let mut out = Vec::with_capacity(self.lines.len());
        for (n, ctx) in self.contexts.iter().enumerate() {
            let allowed = |id: u64| ctx.contains(&id);
            let cites = Cites::new(&allowed);
            let mut points = Vec::new();
            if let Some(l) = by_line.remove(&(n + 1))
                && l.found
            {
                for p in l.points.iter().take(MAX_POINTS) {
                    let text = self.aliases.expand(&plain_text(&p.text));
                    let mut c = cites.keep(&p.cite, d);
                    let near = |id: u64| {
                        (id.saturating_sub(2)..=id + 2)
                            .filter(|n| ctx.contains(n))
                            .filter_map(|n| self.segments.get(&n))
                            .collect::<Vec<_>>()
                    };
                    let lang_ok = c
                        .iter()
                        .filter_map(|id| self.segments.get(id))
                        .all(|s| s.lang.as_deref().is_none_or(|l| l == self.lang.code()));
                    if lang_ok {
                        snap(&text, &mut c, near, |id| self.segments.get(&id), d);
                    }
                    // Points expand what was said about the line, so (in the
                    // same language) they share words with what they cite and
                    // with the line itself.
                    let segs: Vec<&Segment> =
                        c.iter().filter_map(|id| self.segments.get(id)).collect();
                    let same_lang = segs
                        .iter()
                        .all(|s| s.lang.as_deref().is_none_or(|l| l == self.lang.code()));
                    let on_topic = !content_words(&text).is_disjoint(&self.words[n]);
                    if text.is_empty()
                        || c.is_empty()
                        || (same_lang && !(supported(&text, &segs) && on_topic))
                    {
                        d.dropped_items += 1;
                    } else {
                        points.push(Item { text, citations: c });
                    }
                }
            }
            let found = !points.is_empty();
            out.push((points, found));
        }
        Ok(out)
    }
}

/// Assembles the result: every input line, in order, with its user text untouched.
pub fn assemble(
    lines: &[NoteLine],
    batches: &[Batch],
    results: Vec<Vec<(Vec<Item>, bool)>>,
) -> Vec<Enhanced> {
    let mut out: Vec<Enhanced> = lines
        .iter()
        .map(|l| Enhanced {
            user_text: l.text.clone(),
            points: Vec::new(),
            not_found: !plain_text(&l.text).is_empty(),
        })
        .collect();
    for (b, res) in batches.iter().zip(results) {
        for (&i, (points, found)) in b.lines.iter().zip(res) {
            out[i].points = points;
            out[i].not_found = !found;
        }
    }
    out
}

/// Enhances `lines` with the local model (or any [`Llm`]).
pub fn enhance(
    llm: &mut dyn Llm,
    t: &Transcript,
    lines: &[NoteLine],
    lang: OutLang,
) -> Result<Run> {
    let aliases = Aliases::new(t);
    let budget = llm
        .context_tokens()
        .saturating_sub(1024 + PROMPT_OVERHEAD + RETRY_RESERVE)
        .max(512);
    let batches = plan(t, &aliases, lines, lang, budget, 1);
    let mut diag = Diagnostics::default();
    let mut results = Vec::with_capacity(batches.len());
    for b in &batches {
        let req = b.request(t, &aliases, lines, lang, Dialect::Local);
        match run::complete_json(llm, req, lang, &mut diag, |v, d| b.parse_value(v, d))? {
            Outcome::Done(r) => results.push(r),
            Outcome::Truncated => {
                return Err(LlmError::InvalidOutput(
                    "enhance hit the token limit".into(),
                ));
            }
        }
    }
    Ok(Run {
        lines: assemble(lines, &batches, results),
        engine: llm.engine(),
        diagnostics: diag,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notes::tests::{Scripted, meeting};

    fn line(text: &str, t: Option<f64>) -> NoteLine {
        NoteLine {
            text: text.into(),
            t_ms: t.map(|s| (s * 1000.0) as i64),
        }
    }

    #[test]
    fn keeps_user_text_and_flags_unsupported_lines() {
        let t = meeting();
        let lines = vec![
            line("scope beta", Some(2.0)),
            line("zzz unrelated", None),
            line("", None),
            line("ngân sách?", None),
        ];
        // One request per line with context (the local model gets one line at a time).
        let mut llm = Scripted::new(&[
            r#"{"lines":[{"line":1,"found":true,"points":[
                {"text":"Beta không làm lịch","cite":[0]},{"text":"Bịa","cite":[2]},
                {"text":"Không làm lịch","cite":[0]}]}]}"#,
            r#"{"lines":[{"line":1,"found":true,"points":[{"text":"x","cite":[1]}]}]}"#,
        ]);
        let run = enhance(&mut llm, &t, &lines, OutLang::Vi).unwrap();
        let r = &run.lines;
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].user_text, "scope beta");
        assert_eq!(
            r[0].points.len(),
            1,
            "unsupported or off-topic points are dropped"
        );
        assert!(!r[0].not_found);
        assert!(
            r[1].not_found && r[1].points.is_empty(),
            "no context → no call"
        );
        assert!(!r[2].not_found, "empty lines are left alone");
        // "ngân sách?" may only cite its own context (s2).
        assert!(r[3].not_found);
        assert_eq!(run.diagnostics.dropped_items, 3);
        assert_eq!(llm.requests.len(), 2);
        assert!(
            llm.requests[0].messages[1]
                .content
                .starts_with("Ghi chú của người dùng:\n1. scope beta\n")
        );
        assert!(
            llm.requests[1].messages[1]
                .content
                .contains("1. ngân sách?")
        );
    }

    #[test]
    fn one_common_word_is_not_context() {
        let t = meeting();
        let ix = Index::new(&t);
        // "làm" is a word of s0, but the line shares nothing else with it.
        assert!(context(&t, &ix, &line("làm thêm 3 kỹ sư iOS", None)).is_empty());
        assert_eq!(
            context(&t, &ix, &line("ngân sách quý", None))
                .into_iter()
                .collect::<Vec<_>>(),
            vec![2]
        );
    }

    #[test]
    fn line_numbers_outside_the_batch_are_invalid() {
        let t = meeting();
        let lines = vec![line("scope", None)];
        let mut llm = Scripted::new(&[
            r#"{"lines":[{"line":7,"found":false,"points":[]}]}"#,
            r#"{"lines":[]}"#,
        ]);
        let run = enhance(&mut llm, &t, &lines, OutLang::En).unwrap();
        assert_eq!(run.diagnostics.retries, 1);
        assert!(run.lines[0].not_found);
    }
}
