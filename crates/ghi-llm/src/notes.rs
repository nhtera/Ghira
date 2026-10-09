// SPDX-License-Identifier: Apache-2.0
//! Meeting notes from a transcript (doc 02 §E).
//!
//! If the whole transcript fits in the model's context it is summarised in
//! one call. Otherwise map-reduce: the transcript is cut into parts of about
//! `chunk_minutes` (snapped to a speaker change) that fit the context, each
//! part yields facts with citations, and the facts are reduced into the notes.
//! A reply cut off at the token limit falls back to smaller parts.
//!
//! Output is validated ([`crate::validate`]): only segment ids the call was
//! allowed to cite survive (the reduce step may only cite what the map steps
//! cited), items left without a citation are dropped, an action item's owner
//! must speak in the lines it cites (else it is unassigned), and all text is
//! plain. Speakers are aliased `SPK1..SPKn` in prompts and mapped back here.
//!
//! Cloud providers use the same prompt and parser in one call
//! ([`request`], [`parse`]): their bodies are fixed by the send preview.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::run::{self, Outcome, PROMPT_OVERHEAD, RETRY_RESERVE, estimate_tokens};
use crate::schema::{self, Dialect, MAX_QUOTES, MAX_TLDR, MAX_TOPICS, Shape};
use crate::template::{OutLang, Template};
use crate::transcript::{Aliases, Segment, Transcript, render};
use crate::validate::{Cites, Diagnostics, claim_words, is_weak, plain_text, snap, source_score};
use crate::{EngineInfo, Llm, LlmError, Message, Request, Result, prompt};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub text: String,
    /// Segment ids (at least one).
    pub citations: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionItem {
    pub text: String,
    /// The transcript's speaker (label or name); `None` = unassigned.
    pub owner: Option<String>,
    pub due: Option<String>,
    pub citations: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    pub text: String,
    pub speaker: Option<String>,
    pub citations: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topic {
    pub title: String,
    pub citations: Vec<u64>,
    /// Span of the cited segments.
    pub t0_ms: i64,
    pub t1_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub id: String,
    pub title: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notes {
    pub template: String,
    /// Output language (`en` / `vi`).
    pub lang: String,
    pub tldr: Vec<Item>,
    /// Decisions someone agreed or confirmed.
    pub decisions: Vec<Item>,
    /// Decisions only proposed (stored as block kind `proposal`). Notes saved
    /// before this field existed have none.
    #[serde(default)]
    pub proposals: Vec<Item>,
    pub action_items: Vec<ActionItem>,
    pub open_questions: Vec<Item>,
    pub key_quotes: Vec<Quote>,
    pub topics: Vec<Topic>,
    pub sections: Vec<Section>,
}

impl Notes {
    /// Rewrites every piece of text (e.g. restoring redacted values).
    pub fn map_text(&mut self, mut f: impl FnMut(&str) -> String) {
        let mut items = |v: &mut Vec<Item>| v.iter_mut().for_each(|i| i.text = f(&i.text));
        items(&mut self.tldr);
        items(&mut self.decisions);
        items(&mut self.proposals);
        items(&mut self.open_questions);
        for s in &mut self.sections {
            items(&mut s.items);
        }
        for a in &mut self.action_items {
            a.text = f(&a.text);
            a.due = a.due.as_deref().map(&mut f);
        }
        for q in &mut self.key_quotes {
            q.text = f(&q.text);
        }
        for t in &mut self.topics {
            t.title = f(&t.title);
        }
    }

    /// Every item's citations, for callers that check anchors.
    pub fn all_citations(&self) -> impl Iterator<Item = &[u64]> {
        let items = self
            .tldr
            .iter()
            .chain(&self.decisions)
            .chain(&self.proposals)
            .chain(&self.open_questions)
            .chain(self.sections.iter().flat_map(|s| &s.items))
            .map(|i| i.citations.as_slice());
        items
            .chain(self.action_items.iter().map(|a| a.citations.as_slice()))
            .chain(self.key_quotes.iter().map(|q| q.citations.as_slice()))
            .chain(self.topics.iter().map(|t| t.citations.as_slice()))
    }
}

/// What kind of moment the user marked while recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkKind {
    Star,
    Decision,
    Action,
    Question,
}

impl MarkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MarkKind::Star => "star",
            MarkKind::Decision => "decision",
            MarkKind::Action => "action",
            MarkKind::Question => "question",
        }
    }
}

/// A moment the user marked, as the transcript line it falls on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkHint {
    /// The transcript line (segment id as in the prompt).
    pub id: u64,
    pub kind: MarkKind,
    /// When it was marked (orders marks when there are too many).
    pub t_ms: i64,
}

/// Most marks one prompt carries.
pub const MAX_MARKS: usize = 24;

/// The marks one prompt carries: at most [`MAX_MARKS`], decision / action /
/// question before star, then the newest; in line order.
pub fn pick_marks(marks: &[MarkHint]) -> Vec<MarkHint> {
    let mut v: Vec<MarkHint> = marks.to_vec();
    v.sort_by_key(|m| (m.kind == MarkKind::Star, std::cmp::Reverse(m.t_ms), m.id));
    v.dedup_by_key(|m| (m.id, m.kind));
    v.truncate(MAX_MARKS);
    v.sort_by_key(|m| (m.id, m.t_ms));
    v
}

#[derive(Debug, Clone)]
pub struct Options {
    pub template: Template,
    pub lang: OutLang,
    /// User-written (pinned) notes kept on regenerate; the model is told not
    /// to repeat them.
    pub pinned: Vec<String>,
    pub max_output_tokens: u32,
    pub chunk_minutes: u32,
    /// Compact notes ([`schema::compact`]): no quotes or topics, fewer items
    /// per list (a phone, where every output token costs time and heat).
    pub compact: bool,
    /// Moments the user marked: local prompts ask the model to cover them.
    /// Never part of a cloud request.
    pub marks: Vec<MarkHint>,
}

impl Options {
    pub fn new(template: Template, lang: OutLang) -> Options {
        Options {
            template,
            lang,
            pinned: Vec::new(),
            max_output_tokens: 2048,
            chunk_minutes: 10,
            compact: false,
            marks: Vec::new(),
        }
    }

    /// Tokens the marks block takes in a local prompt.
    fn marks_tokens(&self) -> u32 {
        estimate_tokens(&prompt::marks_block(self.lang, &pick_marks(&self.marks)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Strategy {
    Single,
    MapReduce { parts: usize },
}

#[derive(Debug, Clone)]
pub struct Run {
    pub notes: Notes,
    pub engine: EngineInfo,
    pub strategy: Strategy,
    pub diagnostics: Diagnostics,
}

/// Notes with the local model (or any [`Llm`]).
pub fn generate(llm: &mut dyn Llm, t: &Transcript, opts: &Options) -> Result<Run> {
    if t.is_empty() {
        return Err(LlmError::Invalid("the transcript is empty".into()));
    }
    let aliases = Aliases::new(t);
    let mut diag = Diagnostics::default();
    let all: Vec<&Segment> = t.segments().iter().collect();
    // Pinned notes are in every prompt too.
    let budget = transcript_budget(llm, opts.max_output_tokens)
        .saturating_sub(estimate_tokens(&opts.pinned.join("\n")))
        .saturating_sub(opts.marks_tokens())
        .max(512);
    let rendered = render(t, &aliases, &all);
    // The estimate is high; count exactly before falling back to map-reduce.
    let fits = estimate_tokens(&rendered) <= budget || llm.count_tokens(&rendered)? <= budget;
    if fits {
        let (req, _) = request(t, &all, &aliases, opts, Dialect::Local);
        let allowed = |id: u64| t.get(id).is_some();
        let outcome = run::complete_json(llm, req, opts.lang, &mut diag, |v, d| {
            parse_notes(v, &Ctx::new(t, &aliases, opts, &allowed), d)
        })?;
        if let Outcome::Done(notes) = outcome {
            return Ok(Run {
                notes,
                engine: llm.engine(),
                strategy: Strategy::Single,
                diagnostics: diag,
            });
        }
    }
    map_reduce(llm, t, &aliases, opts, budget, diag)
}

/// Where a step-by-step notes run is (map-reduce with the parts' facts kept):
/// saved after every part, so a run stopped on a phone (the app left the
/// screen) continues from the next part instead of starting over.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Steps {
    /// Which transcript these steps belong to (lines and last line id): a
    /// changed transcript starts over.
    fingerprint: (usize, u64),
    /// The parts (segment indices), as cut (and split when a reply ran long).
    parts: Vec<Vec<usize>>,
    /// Parts done.
    pub done: usize,
    facts: Vec<Fact>,
}

impl Steps {
    /// Parts in all (the reduce step comes after them).
    pub fn parts(&self) -> usize {
        self.parts.len()
    }
}

fn fingerprint(t: &Transcript) -> (usize, u64) {
    (t.segments().len(), t.segments().last().map_or(0, |s| s.id))
}

/// Notes step by step: a transcript that spans more than
/// `opts.chunk_minutes` (plus a minute) is written as map steps of about that
/// long and a reduce step, whatever the context, and `saved` steps of the same
/// transcript are skipped; `on_step` gets the state after every part (the
/// caller saves it). A shorter transcript is one call, as in [`generate`].
pub fn generate_steps(
    llm: &mut dyn Llm,
    t: &Transcript,
    opts: &Options,
    saved: Option<Steps>,
    on_step: &mut dyn FnMut(&Steps),
) -> Result<Run> {
    if t.is_empty() {
        return Err(LlmError::Invalid("the transcript is empty".into()));
    }
    let segs = t.segments();
    let span_ms = segs.last().map_or(0, |s| s.t1_ms) - segs.first().map_or(0, |s| s.t0_ms);
    let step_ms = i64::from(opts.chunk_minutes.max(1)) * 60_000 + 60_000;
    if !t.has_times() || span_ms <= step_ms {
        return generate(llm, t, opts);
    }
    let aliases = Aliases::new(t);
    let mut diag = Diagnostics::default();
    let budget = transcript_budget(llm, opts.max_output_tokens)
        .saturating_sub(estimate_tokens(&opts.pinned.join("\n")))
        .saturating_sub(opts.marks_tokens())
        .max(512);
    let mut st = match saved {
        Some(s) if s.fingerprint == fingerprint(t) && s.done <= s.parts.len() => s,
        _ => Steps {
            fingerprint: fingerprint(t),
            parts: chunk(t, &aliases, budget, opts.chunk_minutes),
            done: 0,
            facts: Vec::new(),
        },
    };
    while st.done < st.parts.len() {
        let i = st.done;
        let of = st.parts.len();
        match map_part(llm, t, &aliases, opts, &st.parts[i], i + 1, of, &mut diag)? {
            Some(f) => {
                st.facts.extend(f);
                st.done += 1;
                on_step(&st);
            }
            // Too much to say for one reply: split the part (once it can be split).
            None if st.parts[i].len() > 1 => {
                let half = st.parts[i].len() / 2;
                let second = st.parts[i].split_off(half);
                st.parts.insert(i + 1, second);
            }
            None => return Err(LlmError::InvalidOutput("output hit the token limit".into())),
        }
    }
    let n = st.parts.len();
    reduce(llm, t, &aliases, opts, budget, st.facts, n, diag)
}

/// Tokens of transcript one call can take.
fn transcript_budget(llm: &dyn Llm, max_output: u32) -> u32 {
    llm.context_tokens()
        .saturating_sub(max_output + PROMPT_OVERHEAD + RETRY_RESERVE)
        .max(512)
}

/// The notes schema for `opts` (compact on request).
fn notes_schema(opts: &Options, shape: &Shape) -> serde_json::Value {
    let full = schema::notes(&opts.template, shape);
    if opts.compact {
        schema::compact(full, shape.dialect)
    } else {
        full
    }
}

/// The single-call notes request over `segments` (also what a cloud provider
/// is sent). Returns the request and the ids it may cite.
pub fn request(
    t: &Transcript,
    segments: &[&Segment],
    aliases: &Aliases,
    opts: &Options,
    dialect: Dialect,
) -> (Request, Vec<u64>) {
    let ids: Vec<u64> = segments.iter().map(|s| s.id).collect();
    let shape = Shape {
        dialect,
        ids: &ids,
        speakers: aliases.aliases(),
    };
    // Marks are local-only: a cloud request never carries them.
    let marks = match dialect {
        Dialect::Local => {
            let on_page: Vec<MarkHint> = opts
                .marks
                .iter()
                .copied()
                .filter(|m| ids.contains(&m.id))
                .collect();
            pick_marks(&on_page)
        }
        Dialect::Cloud => Vec::new(),
    };
    let user = format!(
        "{}\n{}",
        prompt::notes_task(&opts.template, opts.lang, &opts.pinned, &marks),
        prompt::transcript_block(opts.lang, &render(t, aliases, segments))
    );
    let req = Request {
        messages: vec![
            Message::system(prompt::notes_system(opts.lang)),
            Message::user(user),
        ],
        schema: Some(notes_schema(opts, &shape)),
        max_tokens: opts.max_output_tokens,
        temperature: 0.3,
    };
    (req, ids)
}

/// Parses and validates a notes reply (the cloud path; the local path does
/// the same inside its retry loop).
pub fn parse(
    reply: &str,
    t: &Transcript,
    aliases: &Aliases,
    opts: &Options,
    diag: &mut Diagnostics,
) -> Result<Notes> {
    let allowed = |id: u64| t.get(id).is_some();
    crate::validate::extract_json(reply)
        .and_then(|v| parse_notes(&v, &Ctx::new(t, aliases, opts, &allowed), diag))
        .map_err(LlmError::InvalidOutput)
}

struct Ctx<'a> {
    t: &'a Transcript,
    aliases: &'a Aliases,
    template: &'a Template,
    lang: OutLang,
    cites: Cites<'a>,
}

impl<'a> Ctx<'a> {
    fn new(
        t: &'a Transcript,
        aliases: &'a Aliases,
        opts: &'a Options,
        allowed: &'a dyn Fn(u64) -> bool,
    ) -> Ctx<'a> {
        Ctx {
            t,
            aliases,
            template: &opts.template,
            lang: opts.lang,
            cites: Cites::new(allowed),
        }
    }

    fn segments(&self, ids: &[u64]) -> Vec<&'a Segment> {
        ids.iter().filter_map(|id| self.t.get(*id)).collect()
    }

    /// Plain text + valid citations, or `None` (dropped) if either is empty.
    fn anchored(
        &self,
        text: &str,
        cite: &[i64],
        d: &mut Diagnostics,
    ) -> Option<(String, Vec<u64>)> {
        let text = self.aliases.expand(&plain_text(text));
        let mut cites = self.cites.keep(cite, d);
        if text.is_empty() || cites.is_empty() {
            d.dropped_items += 1;
            return None;
        }
        // Word overlap is only meaningful when notes and transcript share a language.
        let same_lang = self
            .segments(&cites)
            .iter()
            .all(|s| s.lang.as_deref().is_none_or(|l| l == self.lang.code()));
        if same_lang {
            let near = |id: u64| {
                let i = self.t.index_of(id).unwrap_or(0);
                let segs = self.t.segments();
                segs[i.saturating_sub(2)..(i + 3).min(segs.len())]
                    .iter()
                    .filter(|s| self.cites.allows(s.id))
                    .collect::<Vec<_>>()
            };
            snap(&text, &mut cites, near, |id| self.t.get(id), d);
        }
        let segs = self.segments(&cites);
        if same_lang {
            let claim = claim_words(&text);
            let cited: Vec<HashSet<String>> = segs.iter().map(|s| claim_words(&s.text)).collect();
            let cited: Vec<&HashSet<String>> = cited.iter().collect();
            if source_score(&claim, &cited).is_some_and(is_weak) {
                d.weak_anchors += 1;
            }
        }
        Some((text, cites))
    }

    fn speaker(&self, alias: Option<&str>) -> Option<String> {
        alias.and_then(|a| self.aliases.original(a)).map(plain_text)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawItem {
    text: String,
    cite: Vec<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDecision {
    text: String,
    /// Required by the schema; a missing one reads as decided (notes written
    /// before statuses existed).
    #[serde(default)]
    status: Option<String>,
    cite: Vec<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAction {
    text: String,
    owner: Option<String>,
    due: Option<String>,
    cite: Vec<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawQuote {
    text: String,
    speaker: Option<String>,
    cite: Vec<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTopic {
    title: String,
    cite: Vec<i64>,
}

fn field<T: serde::de::DeserializeOwned>(
    obj: &serde_json::Map<String, Value>,
    key: &str,
) -> std::result::Result<T, String> {
    let v = obj.get(key).ok_or_else(|| format!("missing key `{key}`"))?;
    serde_json::from_value(v.clone()).map_err(|e| format!("`{key}`: {e}"))
}

/// The text an item is compared by when dropping repeats.
trait Keyed {
    fn key(&self) -> &str;
}

impl Keyed for Item {
    fn key(&self) -> &str {
        &self.text
    }
}

impl Keyed for ActionItem {
    fn key(&self) -> &str {
        &self.text
    }
}

impl Keyed for Quote {
    fn key(&self) -> &str {
        &self.text
    }
}

impl Keyed for Topic {
    fn key(&self) -> &str {
        &self.title
    }
}

/// Drops repeats (same folded text) within one list, then keeps at most `max`.
fn cap<T: Keyed>(v: Vec<T>, max: usize, d: &mut Diagnostics) -> Vec<T> {
    let mut seen = HashSet::new();
    let mut v: Vec<T> = v
        .into_iter()
        .filter(|x| {
            let new = seen.insert(ghi_text::fold(x.key()));
            if !new {
                d.duplicates += 1;
            }
            new
        })
        .collect();
    if v.len() > max {
        d.truncated_lists += 1;
        v.truncate(max);
    }
    v
}

fn parse_notes(v: &Value, ctx: &Ctx, d: &mut Diagnostics) -> std::result::Result<Notes, String> {
    let obj = v.as_object().ok_or("the reply is not a JSON object")?;
    let expected: HashSet<&str> = schema::CORE_KEYS
        .iter()
        .copied()
        .chain(ctx.template.sections.iter().map(|s| s.id.as_str()))
        .collect();
    if let Some(extra) = obj.keys().find(|k| !expected.contains(k.as_str())) {
        return Err(format!("unexpected key `{extra}`"));
    }
    let items = |key: &str, d: &mut Diagnostics| -> std::result::Result<Vec<Item>, String> {
        let raw: Vec<RawItem> = field(obj, key)?;
        Ok(raw
            .iter()
            .filter_map(|r| ctx.anchored(&r.text, &r.cite, d))
            .map(|(text, citations)| Item { text, citations })
            .collect())
    };

    let tldr = cap(items("tldr", d)?, MAX_TLDR, d);
    let (mut decisions, mut proposals) = (Vec::new(), Vec::new());
    for r in field::<Vec<RawDecision>>(obj, "decisions")? {
        let proposed = match r.status.as_deref() {
            None | Some("decided") => false,
            Some("proposed") => true,
            Some(other) => return Err(format!("`decisions`: unknown status `{other}`")),
        };
        if let Some((text, citations)) = ctx.anchored(&r.text, &r.cite, d) {
            let item = Item { text, citations };
            if proposed {
                proposals.push(item);
            } else {
                decisions.push(item);
            }
        }
    }
    let decisions = cap(decisions, usize::MAX, d);
    let proposals = cap(proposals, usize::MAX, d);
    let open_questions = cap(items("open_questions", d)?, usize::MAX, d);

    let mut action_items = Vec::new();
    for r in field::<Vec<RawAction>>(obj, "action_items")? {
        let Some((text, citations)) = ctx.anchored(&r.text, &r.cite, d) else {
            continue;
        };
        // The owner must speak in the cited lines; never guess.
        let owner = ctx.speaker(r.owner.as_deref()).filter(|o| {
            ctx.segments(&citations)
                .iter()
                .any(|s| s.speaker.as_deref().map(plain_text).as_deref() == Some(o.as_str()))
        });
        if r.owner.is_some() && owner.is_none() {
            d.unassigned_owners += 1;
        }
        let due = r.due.as_deref().map(plain_text).filter(|s| !s.is_empty());
        action_items.push(ActionItem {
            text,
            owner,
            due,
            citations,
        });
    }

    let mut key_quotes = Vec::new();
    for r in field::<Vec<RawQuote>>(obj, "key_quotes")? {
        if let Some((text, citations)) = ctx.anchored(&r.text, &r.cite, d) {
            // Like an owner, a quote's speaker must speak in the cited lines.
            let speaker = ctx.speaker(r.speaker.as_deref()).filter(|sp| {
                ctx.segments(&citations)
                    .iter()
                    .any(|s| s.speaker.as_deref().map(plain_text).as_deref() == Some(sp.as_str()))
            });
            key_quotes.push(Quote {
                text,
                speaker,
                citations,
            });
        }
    }
    let action_items = cap(action_items, usize::MAX, d);
    let key_quotes = cap(key_quotes, MAX_QUOTES, d);

    let mut topics = Vec::new();
    for r in field::<Vec<RawTopic>>(obj, "topics")? {
        if let Some((title, citations)) = ctx.anchored(&r.title, &r.cite, d) {
            let segs = ctx.segments(&citations);
            topics.push(Topic {
                title,
                t0_ms: segs.iter().map(|s| s.t0_ms).min().unwrap_or(0),
                t1_ms: segs.iter().map(|s| s.t1_ms).max().unwrap_or(0),
                citations,
            });
        }
    }
    let topics = cap(topics, MAX_TOPICS, d);

    let mut sections = Vec::new();
    for s in &ctx.template.sections {
        sections.push(Section {
            id: s.id.clone(),
            title: match ctx.lang {
                OutLang::En => s.title_en.clone(),
                OutLang::Vi => s.title_vi.clone(),
            },
            items: cap(items(&s.id, d)?, usize::MAX, d),
        });
    }

    Ok(Notes {
        template: ctx.template.id.clone(),
        lang: ctx.lang.code().to_string(),
        tldr,
        decisions,
        proposals,
        action_items,
        open_questions,
        key_quotes,
        topics,
        sections,
    })
}

/// One extracted fact of the map step (speakers still aliased).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Fact {
    kind: String,
    text: String,
    speaker: Option<String>,
    owner: Option<String>,
    due: Option<String>,
    cites: Vec<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFacts {
    facts: Vec<RawFact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFact {
    kind: String,
    text: String,
    speaker: Option<String>,
    owner: Option<String>,
    due: Option<String>,
    cite: Vec<i64>,
}

fn map_reduce(
    llm: &mut dyn Llm,
    t: &Transcript,
    aliases: &Aliases,
    opts: &Options,
    budget: u32,
    mut diag: Diagnostics,
) -> Result<Run> {
    // Parts leave room for the facts of the others in the reduce prompt.
    let mut parts = chunk(t, aliases, budget, opts.chunk_minutes);
    let mut facts = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        match map_part(
            llm,
            t,
            aliases,
            opts,
            &parts[i],
            i + 1,
            parts.len(),
            &mut diag,
        )? {
            Some(f) => {
                facts.extend(f);
                i += 1;
            }
            // Too much to say for one reply: split the part (once it can be split).
            None if parts[i].len() > 1 => {
                let half = parts[i].len() / 2;
                let second = parts[i].split_off(half);
                parts.insert(i + 1, second);
            }
            None => return Err(LlmError::InvalidOutput("output hit the token limit".into())),
        }
    }
    let n = parts.len();
    reduce(llm, t, aliases, opts, budget, facts, n, diag)
}

/// The reduce step over the map steps' facts (`n` parts).
#[allow(clippy::too_many_arguments)]
fn reduce(
    llm: &mut dyn Llm,
    t: &Transcript,
    aliases: &Aliases,
    opts: &Options,
    budget: u32,
    mut facts: Vec<Fact>,
    n: usize,
    mut diag: Diagnostics,
) -> Result<Run> {
    // Reduce: the facts, in transcript order, with the lines they cite. If
    // they don't fit, minor points go first, then quotes, then the tail.
    for minor in ["point", "quote"] {
        if estimate_tokens(&render_facts(&facts)) <= budget {
            break;
        }
        facts.retain(|f| f.kind != minor);
        diag.truncated_lists += 1;
    }
    while facts.len() > 1 && estimate_tokens(&render_facts(&facts)) > budget {
        facts.truncate(facts.len() * 3 / 4);
        diag.truncated_lists += 1;
    }
    // Marked lines may be cited too, even when the map steps left them out.
    let marks = pick_marks(
        &opts
            .marks
            .iter()
            .copied()
            .filter(|m| t.get(m.id).is_some())
            .collect::<Vec<_>>(),
    );
    let cited: HashSet<u64> = facts
        .iter()
        .flat_map(|f| f.cites.iter().copied())
        .chain(marks.iter().map(|m| m.id))
        .collect();
    let lines = render_facts(&facts);
    let mut ids: Vec<u64> = cited.iter().copied().collect();
    ids.sort_unstable();
    let shape = Shape {
        dialect: Dialect::Local,
        ids: &ids,
        speakers: aliases.aliases(),
    };
    let user = format!(
        "{}\n{}",
        prompt::notes_task(&opts.template, opts.lang, &opts.pinned, &marks),
        prompt::reduce_block(opts.lang, &lines)
    );
    let req = Request {
        messages: vec![
            Message::system(prompt::notes_system(opts.lang)),
            Message::user(user),
        ],
        schema: Some(notes_schema(opts, &shape)),
        max_tokens: opts.max_output_tokens,
        temperature: 0.3,
    };
    let allowed = |id: u64| cited.contains(&id);
    match run::complete_json(llm, req, opts.lang, &mut diag, |v, d| {
        parse_notes(v, &Ctx::new(t, aliases, opts, &allowed), d)
    })? {
        Outcome::Done(mut notes) => Ok(Run {
            notes: {
                keep_proposals(&mut notes, &facts);
                notes
            },
            engine: llm.engine(),
            strategy: Strategy::MapReduce { parts: n },
            diagnostics: diag,
        }),
        Outcome::Truncated => Err(LlmError::InvalidOutput(
            "the notes hit the token limit".into(),
        )),
    }
}

/// The map steps know which lines were only suggested; a small model reducing
/// them sometimes files a suggestion ("hay là ... nhỉ") under open questions.
/// An open question that cites only lines of `proposal` facts (and none of a
/// `decision` fact) is a proposed decision.
fn keep_proposals(notes: &mut Notes, facts: &[Fact]) {
    let of = |kind: &str| -> HashSet<u64> {
        facts
            .iter()
            .filter(|f| f.kind == kind)
            .flat_map(|f| f.cites.iter().copied())
            .collect()
    };
    let (proposed, decided) = (of("proposal"), of("decision"));
    let (moved, kept): (Vec<Item>, Vec<Item>) = std::mem::take(&mut notes.open_questions)
        .into_iter()
        .partition(|q| {
            q.citations
                .iter()
                .all(|c| proposed.contains(c) && !decided.contains(c))
        });
    notes.open_questions = kept;
    for q in moved {
        if !notes.proposals.iter().any(|p| p.text == q.text) {
            notes.proposals.push(q);
        }
    }
}

/// Facts as reduce-prompt lines: `- [kind] text (speaker SPK1, ...) cite: 3, 4`.
fn render_facts(facts: &[Fact]) -> String {
    facts
        .iter()
        .map(|f| {
            let mut extra = Vec::new();
            if let Some(s) = &f.speaker {
                extra.push(format!("speaker {s}"));
            }
            if let Some(o) = &f.owner {
                extra.push(format!("owner {o}"));
            }
            if let Some(due) = &f.due {
                extra.push(format!("due {due}"));
            }
            let cites: Vec<String> = f.cites.iter().map(u64::to_string).collect();
            let extra = if extra.is_empty() {
                String::new()
            } else {
                format!(" ({})", extra.join(", "))
            };
            // Spelled out: a small model leaves "proposal" facts out of the
            // decisions otherwise.
            let kind = match f.kind.as_str() {
                "proposal" => "proposal, decisions with status proposed",
                "decision" => "decision, decisions with status decided",
                k => k,
            };
            format!(
                "- [{}] {}{} cite: {}",
                kind,
                f.text,
                extra,
                cites.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[allow(clippy::too_many_arguments)]
fn map_part(
    llm: &mut dyn Llm,
    t: &Transcript,
    aliases: &Aliases,
    opts: &Options,
    part: &[usize],
    n: usize,
    of: usize,
    diag: &mut Diagnostics,
) -> Result<Option<Vec<Fact>>> {
    let segs: Vec<&Segment> = part.iter().map(|&i| &t.segments()[i]).collect();
    let ids: Vec<u64> = segs.iter().map(|s| s.id).collect();
    let shape = Shape {
        dialect: Dialect::Local,
        ids: &ids,
        speakers: aliases.aliases(),
    };
    let in_part: HashSet<u64> = ids.iter().copied().collect();
    let marks = pick_marks(
        &opts
            .marks
            .iter()
            .copied()
            .filter(|m| in_part.contains(&m.id))
            .collect::<Vec<_>>(),
    );
    let req = Request {
        messages: vec![
            Message::system(prompt::map_system(opts.lang)),
            Message::user(prompt::map_task(
                opts.lang,
                n,
                of,
                &render(t, aliases, &segs),
                &marks,
            )),
        ],
        schema: Some(if opts.compact {
            schema::compact_facts(schema::facts(&shape), Dialect::Local)
        } else {
            schema::facts(&shape)
        }),
        max_tokens: if opts.compact {
            opts.max_output_tokens.min(schema::COMPACT_FACTS_TOKENS)
        } else {
            opts.max_output_tokens
        },
        temperature: 0.3,
    };
    let allowed = |id: u64| in_part.contains(&id);
    let cites = Cites::new(&allowed);
    let outcome = run::complete_json(llm, req, opts.lang, diag, |v, d| {
        let raw: RawFacts = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for f in raw.facts {
            if !schema::FACT_KINDS.contains(&f.kind.as_str()) {
                return Err(format!("unknown fact kind `{}`", f.kind));
            }
            // Facts stay aliased: they are prompt text for the reduce step.
            let text = plain_text(&f.text);
            let c = cites.keep(&f.cite, d);
            if text.is_empty() || c.is_empty() {
                d.dropped_items += 1;
                continue;
            }
            let alias = |a: Option<String>| a.filter(|a| aliases.original(a).is_some());
            out.push(Fact {
                kind: f.kind,
                text,
                speaker: alias(f.speaker),
                owner: alias(f.owner),
                due: f.due.map(|s| plain_text(&s)).filter(|s| !s.is_empty()),
                cites: c,
            });
        }
        Ok(out)
    })?;
    Ok(match outcome {
        Outcome::Done(f) => Some(f),
        Outcome::Truncated => None,
    })
}

/// Parts of about `minutes` (ending at a speaker change when possible), each
/// within `budget` tokens. Transcripts without times are cut by tokens only.
fn chunk(t: &Transcript, aliases: &Aliases, budget: u32, minutes: u32) -> Vec<Vec<usize>> {
    let span_ms = i64::from(minutes.max(1)) * 60_000;
    let grace_ms = 60_000;
    let max_tokens = budget.max(512);
    let timed = t.has_times();
    let mut parts: Vec<Vec<usize>> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut tokens = 0u32;
    let segs = t.segments();
    for (i, s) in segs.iter().enumerate() {
        let line = estimate_tokens(&crate::transcript::render_line(s, aliases, timed)) + 1;
        if let Some(&first) = cur.first() {
            let elapsed = s.t0_ms - segs[first].t0_ms;
            let turn = s.speaker != segs[i - 1].speaker;
            let long = timed && (elapsed >= span_ms + grace_ms || (elapsed >= span_ms && turn));
            if tokens + line > max_tokens || long {
                parts.push(std::mem::take(&mut cur));
                tokens = 0;
            }
        }
        cur.push(i);
        tokens += line;
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::template;
    use crate::transcript::tests::seg;
    use crate::{Completion, Llm};
    use serde_json::json;

    /// Replies with scripted texts in order and records the requests.
    pub struct Scripted {
        pub replies: Vec<(String, bool)>,
        pub requests: Vec<Request>,
        pub context: u32,
    }

    impl Scripted {
        pub fn new(replies: &[&str]) -> Scripted {
            Scripted {
                replies: replies.iter().map(|r| (r.to_string(), false)).collect(),
                requests: Vec::new(),
                context: 16384,
            }
        }
    }

    impl Llm for Scripted {
        fn engine(&self) -> EngineInfo {
            EngineInfo {
                name: "scripted".into(),
                version: "1".into(),
            }
        }
        fn context_tokens(&self) -> u32 {
            self.context
        }
        fn complete(&mut self, req: &Request) -> Result<Completion> {
            self.requests.push(req.clone());
            assert!(!self.replies.is_empty(), "unexpected request");
            let (text, truncated) = self.replies.remove(0);
            Ok(Completion {
                text,
                tokens_in: 10,
                tokens_out: 5,
                truncated,
            })
        }
    }

    pub fn meeting() -> Transcript {
        Transcript::new(vec![
            seg(
                0,
                0.0,
                4.0,
                "Linh",
                "Mình chốt scope cho bản beta nhé, không làm lịch.",
                "vi",
            ),
            seg(
                1,
                4.0,
                8.0,
                "Nam",
                "OK, tôi sẽ gửi tài liệu scope trước thứ Sáu.",
                "vi",
            ),
            seg(
                2,
                8.0,
                12.0,
                "Linh",
                "Còn chuyện ngân sách quý sau thì chưa rõ.",
                "vi",
            ),
        ])
        .unwrap()
    }

    fn reply(body: &str) -> String {
        let empty = r#""tldr":[],"decisions":[],"action_items":[],"open_questions":[],"key_quotes":[],"topics":[]"#;
        let mut v: serde_json::Map<String, Value> =
            serde_json::from_str(&format!("{{{empty}}}")).unwrap();
        let extra: serde_json::Map<String, Value> = serde_json::from_str(body).unwrap();
        v.extend(extra);
        Value::Object(v).to_string()
    }

    fn opts() -> Options {
        Options::new(template::builtin("general").unwrap(), OutLang::Vi)
    }

    #[test]
    fn single_pass_maps_aliases_owners_and_citations() {
        let t = meeting();
        let mut llm = Scripted::new(&[&reply(
            r#"{"tldr":[{"text":"Chốt scope beta","cite":[0]},{"text":"Bịa","cite":[99]}],
                "decisions":[{"text":"Chốt scope","cite":[0]},{"text":"chot  SCOPE","cite":[0]}],
                "action_items":[
                  {"text":"Gửi tài liệu scope","owner":"SPK2","due":"thứ Sáu","cite":[1]},
                  {"text":"Lo ngân sách","owner":"SPK2","due":null,"cite":[2]}],
                "key_quotes":[{"text":"không làm lịch","speaker":"SPK1","cite":[0]}],
                "topics":[{"title":"Scope beta","cite":[0,1]}]}"#,
        )]);
        let run = generate(&mut llm, &t, &opts()).unwrap();
        assert_eq!(run.strategy, Strategy::Single);
        let n = &run.notes;
        assert_eq!(
            n.tldr.len(),
            1,
            "the item citing a missing segment is dropped"
        );
        assert_eq!(n.action_items[0].owner.as_deref(), Some("Nam"));
        assert_eq!(n.action_items[0].due.as_deref(), Some("thứ Sáu"));
        assert_eq!(n.action_items[1].owner, None, "Nam does not speak in s2");
        assert_eq!(n.key_quotes[0].speaker.as_deref(), Some("Linh"));
        assert_eq!((n.topics[0].t0_ms, n.topics[0].t1_ms), (0, 8000));
        let d = &run.diagnostics;
        assert_eq!(
            (d.dropped_items, d.unassigned_owners, d.requests),
            (1, 1, 1)
        );
        // Names never reach the prompt; aliases and ids do.
        let prompt = &llm.requests[0].messages[1].content;
        assert!(prompt.contains("[s1] (00:04) SPK2: OK"));
        assert!(!prompt.contains("Linh") && !prompt.contains("Nam:"));
        let schema = llm.requests[0].schema.as_ref().unwrap().to_string();
        assert!(schema.contains("\"enum\":[0,1,2]"));
    }

    #[test]
    fn invalid_output_is_retried_with_the_error_then_fails() {
        let t = meeting();
        let mut llm =
            Scripted::new(&["not json", &reply(r#"{"tldr":[{"text":"ok","cite":[0]}]}"#)]);
        let run = generate(&mut llm, &t, &opts()).unwrap();
        assert_eq!(run.diagnostics.retries, 1);
        let retry = &llm.requests[1].messages;
        assert_eq!(retry.len(), 4);
        assert!(retry[3].content.contains("không hợp lệ"));

        let mut bad = Scripted::new(&[r#"{"tldr":[]}"#, "{}", r#"{"x":1}"#]);
        assert!(matches!(
            generate(&mut bad, &t, &opts()),
            Err(LlmError::InvalidOutput(_))
        ));
        assert_eq!(bad.requests.len(), 3);
    }

    #[test]
    fn unknown_keys_and_missing_sections_are_invalid() {
        let t = meeting();
        let a = Aliases::new(&t);
        let o = Options::new(template::builtin("standup").unwrap(), OutLang::En);
        let mut d = Diagnostics::default();
        let missing = reply("{}");
        assert!(parse(&missing, &t, &a, &o, &mut d).is_err());
        let full = reply(r#"{"done":[],"next":[],"blockers":[{"text":"CI is red","cite":[2]}]}"#);
        let n = parse(&full, &t, &a, &o, &mut d).unwrap();
        assert_eq!(n.sections[2].title, "Blockers");
        assert_eq!(n.sections[2].items[0].citations, vec![2]);
        let extra = reply(r#"{"done":[],"next":[],"blockers":[],"x":[]}"#);
        assert!(parse(&extra, &t, &a, &o, &mut d).is_err());
    }

    #[test]
    fn long_meetings_map_then_reduce_citing_only_mapped_lines() {
        let mut segs = Vec::new();
        for i in 0..40u64 {
            let sp = if i % 2 == 0 { "S1" } else { "S2" };
            segs.push(seg(
                i,
                i as f64 * 30.0,
                i as f64 * 30.0 + 25.0,
                sp,
                &"word ".repeat(40),
                "en",
            ));
        }
        let t = Transcript::new(segs).unwrap();
        let facts = r#"{"facts":[{"kind":"decision","text":"Ship it","speaker":"SPK1","owner":null,"due":null,"cite":[0]}]}"#;
        // 40 × 30 s = 20 min at 10-minute parts → 2 map calls + reduce.
        let mut llm = Scripted::new(&[
            facts,
            r#"{"facts":[{"kind":"action","text":"Send doc","speaker":"SPK2","owner":"SPK2","due":null,"cite":[21]}]}"#,
            &reply(
                r#"{"decisions":[{"text":"Ship it","cite":[0]}],"tldr":[{"text":"Not mapped","cite":[5]}]}"#,
            ),
        ]);
        llm.context = 6500;
        let run = generate(
            &mut llm,
            &t,
            &Options::new(template::builtin("general").unwrap(), OutLang::En),
        )
        .unwrap();
        assert_eq!(run.strategy, Strategy::MapReduce { parts: 2 });
        assert_eq!(run.notes.decisions.len(), 1);
        assert!(run.notes.tldr.is_empty(), "s5 was not cited by any fact");
        let reduce = &llm.requests[2].messages[1].content;
        assert!(reduce.contains("- [action] Send doc (speaker SPK2, owner SPK2) cite: 21"));
        // Map calls only allow their own part's ids.
        let map1 = llm.requests[0].schema.as_ref().unwrap().to_string();
        assert!(map1.contains("\"enum\":[0,1,") && !map1.contains(",21,"));
    }

    /// Stopped after two of four parts, the saved steps resume at the third:
    /// the first two parts are not read again, their facts reach the reduce.
    #[test]
    fn step_notes_resume_after_the_last_saved_part() {
        struct Stops(Scripted, usize);
        impl Llm for Stops {
            fn engine(&self) -> EngineInfo {
                self.0.engine()
            }
            fn context_tokens(&self) -> u32 {
                self.0.context_tokens()
            }
            fn complete(&mut self, req: &Request) -> Result<Completion> {
                if self.0.requests.len() == self.1 {
                    return Err(LlmError::Worker("stopped".into()));
                }
                self.0.complete(req)
            }
        }
        let mut segs = Vec::new();
        for i in 0..40u64 {
            let sp = if i % 2 == 0 { "S1" } else { "S2" };
            segs.push(seg(
                i,
                i as f64 * 30.0,
                i as f64 * 30.0 + 25.0,
                sp,
                "word word",
                "en",
            ));
        }
        let t = Transcript::new(segs).unwrap();
        let fact = |text: &str, cite: u64| {
            format!(
                r#"{{"facts":[{{"kind":"decision","text":"{text}","speaker":"SPK1","owner":null,"due":null,"cite":[{cite}]}}]}}"#
            )
        };
        let mut opts = Options::new(template::builtin("general").unwrap(), OutLang::En);
        opts.chunk_minutes = 5;
        // 20 min at 5-minute parts: 4 map steps, stopped after 2.
        let (f1, f2, f3, f4) = (
            fact("One", 0),
            fact("Two", 11),
            fact("Three", 21),
            fact("Four", 31),
        );
        let mut first = Stops(Scripted::new(&[&f1, &f2]), 2);
        let mut saved = None;
        let r = generate_steps(&mut first, &t, &opts, None, &mut |s| {
            saved = Some(s.clone())
        });
        assert!(r.is_err());
        let saved = saved.expect("saved after each part");
        assert_eq!((saved.done, saved.parts()), (2, 4));
        // Resumed: parts 3 and 4, then the reduce with all four parts' facts.
        let reduce = reply(r#"{"decisions":[{"text":"One","cite":[0]}]}"#);
        let mut second = Scripted::new(&[&f3, &f4, &reduce]);
        let mut steps = Vec::new();
        let run = generate_steps(&mut second, &t, &opts, Some(saved), &mut |s| {
            steps.push(s.done)
        })
        .unwrap();
        assert_eq!(steps, [3, 4]);
        assert_eq!(second.requests.len(), 3, "parts 1-2 not read again");
        let prompt = &second.requests[2].messages[1].content;
        for f in ["One", "Two", "Three", "Four"] {
            assert!(prompt.contains(f), "{f} reaches the reduce");
        }
        assert_eq!(run.strategy, Strategy::MapReduce { parts: 4 });
        // Another transcript does not take these steps.
        let mut other = Scripted::new(&[&reply(r#"{"tldr":[{"text":"a","cite":[0]}]}"#)]);
        let short = meeting();
        generate_steps(&mut other, &short, &opts, None, &mut |_| {
            panic!("a short meeting is one call")
        })
        .unwrap();
    }

    #[test]
    fn a_truncated_part_is_split() {
        let t = meeting();
        let mut llm = Scripted::new(&[
            "{",
            "{",
            r#"{"facts":[{"kind":"point","text":"a","speaker":null,"owner":null,"due":null,"cite":[0]}]}"#,
            r#"{"facts":[]}"#,
            &reply(r#"{"tldr":[{"text":"a","cite":[0]}]}"#),
        ]);
        // The single pass, then the one map part, hit the token limit.
        llm.replies[0].1 = true;
        llm.replies[1].1 = true;
        let run = generate(&mut llm, &t, &opts()).unwrap();
        assert_eq!(run.strategy, Strategy::MapReduce { parts: 2 });
        assert_eq!(run.notes.tldr.len(), 1);
    }

    #[test]
    fn gold_transcripts_without_times_chunk_by_tokens() {
        let segs: Vec<_> = (0..30u64)
            .map(|i| seg(i, 0.0, 0.0, "", &"x".repeat(300), ""))
            .collect();
        let t = Transcript::new(segs).unwrap();
        let parts = chunk(&t, &Aliases::new(&t), 1000, 10);
        assert!(parts.len() > 1);
        assert_eq!(parts.iter().map(Vec::len).sum::<usize>(), 30);
    }

    fn hint(id: u64, kind: MarkKind, t_ms: i64) -> MarkHint {
        MarkHint { id, kind, t_ms }
    }

    #[test]
    fn marks_are_capped_tagged_first_then_newest() {
        let mut marks: Vec<MarkHint> = (0..30)
            .map(|i| hint(i, MarkKind::Star, i as i64 * 1000))
            .collect();
        marks.push(hint(100, MarkKind::Decision, 0));
        marks.push(hint(100, MarkKind::Decision, 0));
        marks.push(hint(101, MarkKind::Question, 1));
        let picked = pick_marks(&marks);
        assert_eq!(picked.len(), MAX_MARKS);
        let ids: Vec<u64> = picked.iter().map(|m| m.id).collect();
        assert!(ids.contains(&100) && ids.contains(&101), "tagged marks win");
        assert_eq!(ids.iter().filter(|&&i| i == 100).count(), 1, "repeats go");
        assert!(!ids.contains(&0), "the oldest stars go first");
        assert!(ids.contains(&29));
        assert!(ids.windows(2).all(|w| w[0] <= w[1]), "in line order");
    }

    #[test]
    fn single_pass_prompt_lists_marks_in_english_and_vietnamese() {
        let t = meeting();
        let marks = vec![
            hint(0, MarkKind::Decision, 1000),
            hint(2, MarkKind::Star, 9000),
            hint(1, MarkKind::Action, 5000),
            // Not a line of this transcript: never reaches the prompt.
            hint(77, MarkKind::Star, 9500),
        ];
        for lang in [OutLang::En, OutLang::Vi] {
            let mut o = Options::new(template::builtin("general").unwrap(), lang);
            o.marks = marks.clone();
            let mut llm = Scripted::new(&[&reply(r#"{"tldr":[{"text":"ok","cite":[0]}]}"#)]);
            generate(&mut llm, &t, &o).unwrap();
            let prompt = &llm.requests[0].messages[1].content;
            assert!(
                prompt.contains("[s0 decision] [s1 action] [s2 star]"),
                "{prompt}"
            );
            assert!(!prompt.contains("s77"));
            let want = match lang {
                OutLang::En => "The user marked these moments as important",
                OutLang::Vi => "Người dùng đã đánh dấu các thời điểm quan trọng",
            };
            assert!(prompt.contains(want));
            // After the output keys, before the transcript.
            assert!(prompt.find(want).unwrap() < prompt.find("[s0] (00:00)").unwrap());
        }
        let mut llm = Scripted::new(&[&reply(r#"{"tldr":[{"text":"ok","cite":[0]}]}"#)]);
        generate(&mut llm, &t, &opts()).unwrap();
        let prompt = &llm.requests[0].messages[1].content;
        assert!(!prompt.contains("đánh dấu"), "no marks, no block");
    }

    #[test]
    fn a_cloud_request_never_carries_marks() {
        let t = meeting();
        let mut o = Options::new(template::builtin("general").unwrap(), OutLang::En);
        o.marks = vec![hint(1, MarkKind::Decision, 5000)];
        let aliases = Aliases::new(&t);
        let all: Vec<&Segment> = t.segments().iter().collect();
        let (local, _) = request(&t, &all, &aliases, &o, Dialect::Local);
        let (cloud, _) = request(&t, &all, &aliases, &o, Dialect::Cloud);
        assert!(local.messages[1].content.contains("[s1 decision]"));
        let sent = serde_json::to_string(&cloud.messages).unwrap();
        assert!(!sent.contains("marked") && !sent.contains("[s1 decision]"));
    }

    #[test]
    fn marks_shrink_the_transcript_budget() {
        let mut llm = Scripted::new(&[]);
        llm.context = 8192;
        let plain = transcript_budget(&llm, 2048);
        let mut o = Options::new(template::builtin("general").unwrap(), OutLang::En);
        o.marks = (0..10).map(|i| hint(i, MarkKind::Decision, 0)).collect();
        assert!(o.marks_tokens() > 0);
        let with = plain.saturating_sub(o.marks_tokens());
        assert!(with < plain);
    }

    /// Marks in the first and the last of three parts: each part's prompt
    /// carries only its own, and the reduce step may cite both even though no
    /// fact did (a >32k-token transcript, so it cannot be one call).
    #[test]
    fn marks_in_early_and_late_parts_both_reach_the_reduce() {
        let segs: Vec<_> = (0..60u64)
            .map(|i| {
                let sp = if i % 2 == 0 { "S1" } else { "S2" };
                let text = format!("line{i} {}", "word ".repeat(330));
                seg(i, i as f64 * 30.0, i as f64 * 30.0 + 25.0, sp, &text, "en")
            })
            .collect();
        let t = Transcript::new(segs).unwrap();
        let total: u32 = t.segments().iter().map(|s| estimate_tokens(&s.text)).sum();
        assert!(total > 32_000, "{total}");
        let fact = |cite: u64| {
            format!(
                r#"{{"facts":[{{"kind":"point","text":"P{cite}","speaker":null,"owner":null,"due":null,"cite":[{cite}]}}]}}"#
            )
        };
        let (f1, f2, f3) = (fact(5), fact(25), fact(45));
        let reduce = reply(
            r#"{"decisions":[{"text":"Early","cite":[3]}],"tldr":[{"text":"Late","cite":[55]},{"text":"Unmarked","cite":[30]}]}"#,
        );
        let mut llm = Scripted::new(&[&f1, &f2, &f3, &reduce]);
        let mut o = Options::new(template::builtin("general").unwrap(), OutLang::En);
        o.marks = vec![
            hint(3, MarkKind::Decision, 90_000),
            hint(55, MarkKind::Star, 1_650_000),
        ];
        let run = generate_steps(&mut llm, &t, &o, None, &mut |_| {}).unwrap();
        assert_eq!(run.strategy, Strategy::MapReduce { parts: 3 });
        let map = |i: usize| llm.requests[i].messages[1].content.clone();
        assert!(map(0).contains("[s3 decision]") && !map(0).contains("s55 star"));
        assert!(!map(1).contains("marked these") && !map(1).contains("[s3 decision]"));
        assert!(map(2).contains("[s55 star]") && !map(2).contains("[s3 decision]"));
        let reduce_req = &llm.requests[3];
        assert!(
            reduce_req.messages[1]
                .content
                .contains("[s3 decision] [s55 star]")
        );
        let schema = reduce_req.schema.as_ref().unwrap().to_string();
        assert!(schema.contains("3") && schema.contains("55"));
        assert_eq!(run.notes.decisions[0].citations, vec![3]);
        let tldr: Vec<&str> = run.notes.tldr.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(tldr, ["Late"], "55 reachable, unmarked uncited 30 is not");
    }

    #[test]
    fn decisions_split_into_decided_and_proposed() {
        let t = meeting();
        let mut llm = Scripted::new(&[&reply(
            r#"{"decisions":[
                {"text":"Chốt scope beta","status":"decided","cite":[0]},
                {"text":"Có thể xem lại ngân sách","status":"proposed","cite":[2]},
                {"text":"Bịa","status":"proposed","cite":[99]},
                {"text":"Gửi tài liệu","cite":[1]}]}"#,
        )]);
        let run = generate(&mut llm, &t, &opts()).unwrap();
        let texts = |v: &[Item]| v.iter().map(|i| i.text.clone()).collect::<Vec<_>>();
        assert_eq!(
            texts(&run.notes.decisions),
            ["Chốt scope beta", "Gửi tài liệu"],
            "no status reads as decided (old stored JSON)"
        );
        assert_eq!(texts(&run.notes.proposals), ["Có thể xem lại ngân sách"]);
        assert_eq!(run.diagnostics.dropped_items, 1);
        assert!(run.notes.all_citations().any(|c| c == [2]));
        // The model is told what the two statuses mean, and the grammar forces one.
        let req = &llm.requests[0];
        assert!(req.messages[1].content.contains("\"decided\" chỉ khi"));
        let decisions = &req.schema.as_ref().unwrap()["properties"]["decisions"]["items"];
        assert_eq!(
            decisions["properties"]["status"]["enum"],
            json!(["decided", "proposed"])
        );
        assert!(
            decisions["required"]
                .as_array()
                .unwrap()
                .contains(&json!("status"))
        );
        // Anything else is invalid output.
        let mut bad = Scripted::new(&[
            &reply(r#"{"decisions":[{"text":"x","status":"maybe","cite":[0]}]}"#),
            &reply(r#"{"decisions":[{"text":"x","status":"maybe","cite":[0]}]}"#),
            &reply(r#"{"decisions":[{"text":"x","status":"maybe","cite":[0]}]}"#),
        ]);
        assert!(generate(&mut bad, &t, &opts()).is_err());
        // Notes stored before proposals existed still load.
        let old = serde_json::to_value(&run.notes).unwrap();
        let mut old = old.as_object().unwrap().clone();
        old.remove("proposals");
        let back: Notes = serde_json::from_value(Value::Object(old)).unwrap();
        assert!(back.proposals.is_empty());
    }

    #[test]
    fn map_reduce_keeps_proposals_apart_from_decisions() {
        let mut segs = Vec::new();
        for i in 0..40u64 {
            let sp = if i % 2 == 0 { "S1" } else { "S2" };
            segs.push(seg(
                i,
                i as f64 * 30.0,
                i as f64 * 30.0 + 25.0,
                sp,
                &"word ".repeat(40),
                "en",
            ));
        }
        let t = Transcript::new(segs).unwrap();
        let mut llm = Scripted::new(&[
            r#"{"facts":[{"kind":"proposal","text":"Maybe a dark mode","speaker":"SPK1","owner":null,"due":null,"cite":[0]}]}"#,
            r#"{"facts":[{"kind":"decision","text":"Ship Friday","speaker":"SPK2","owner":null,"due":null,"cite":[21]}]}"#,
            &reply(
                r#"{"decisions":[{"text":"Ship Friday","status":"decided","cite":[21]},{"text":"Dark mode","status":"proposed","cite":[0]}]}"#,
            ),
        ]);
        llm.context = 6500;
        let o = Options::new(template::builtin("general").unwrap(), OutLang::En);
        let run = generate(&mut llm, &t, &o).unwrap();
        assert_eq!(run.strategy, Strategy::MapReduce { parts: 2 });
        assert_eq!(run.notes.decisions[0].text, "Ship Friday");
        assert_eq!(run.notes.proposals[0].text, "Dark mode");
        let reduce = &llm.requests[2].messages[1].content;
        assert!(reduce.contains("- [proposal, decisions with status proposed] Maybe a dark mode"));
        assert!(reduce.contains("- [decision, decisions with status decided] Ship Friday"));
        // The map step may extract proposals.
        assert!(
            llm.requests[0]
                .schema
                .as_ref()
                .unwrap()
                .to_string()
                .contains("\"proposal\"")
        );
    }

    #[test]
    fn a_suggestion_filed_as_a_question_by_the_reduce_is_a_proposal() {
        let mut segs = Vec::new();
        for i in 0..40u64 {
            let sp = if i % 2 == 0 { "S1" } else { "S2" };
            segs.push(seg(
                i,
                i as f64 * 30.0,
                i as f64 * 30.0 + 25.0,
                sp,
                &"word ".repeat(40),
                "en",
            ));
        }
        let t = Transcript::new(segs).unwrap();
        let mut llm = Scripted::new(&[
            r#"{"facts":[{"kind":"proposal","text":"Try dark mode","speaker":"SPK1","owner":null,"due":null,"cite":[0]}]}"#,
            r#"{"facts":[{"kind":"decision","text":"Ship Friday","speaker":"SPK2","owner":null,"due":null,"cite":[21]},{"kind":"question","text":"Budget?","speaker":"SPK2","owner":null,"due":null,"cite":[22]}]}"#,
            &reply(
                r#"{"decisions":[{"text":"Ship Friday","status":"decided","cite":[21]}],
                    "open_questions":[{"text":"Try dark mode","cite":[0]},{"text":"Budget?","cite":[22]}]}"#,
            ),
        ]);
        llm.context = 6500;
        let o = Options::new(template::builtin("general").unwrap(), OutLang::En);
        let run = generate(&mut llm, &t, &o).unwrap();
        assert_eq!(run.notes.proposals[0].text, "Try dark mode");
        assert_eq!(run.notes.open_questions.len(), 1, "a real question stays");
        assert_eq!(run.notes.open_questions[0].text, "Budget?");
    }
}
