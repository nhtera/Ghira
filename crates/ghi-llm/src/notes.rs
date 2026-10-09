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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    let mut seen = HashSet::new();
    v.retain(|m| seen.insert((m.id, m.kind)));
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
    /// Terms said in the meeting, to be written as given (the user's own
    /// vocabulary, attendees, enabled glossary packs; already filtered to the
    /// ones the transcript says). Local prompts only: never part of a cloud
    /// request.
    pub spellings: Vec<String>,
}

/// Most spellings one prompt carries.
pub const MAX_SPELLINGS: usize = 40;

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
            spellings: Vec::new(),
        }
    }

    /// The spellings one prompt carries: at most [`MAX_SPELLINGS`], distinct, in order.
    pub fn spellings(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for s in &self.spellings {
            let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
            if !s.is_empty() && !out.contains(&s) {
                out.push(s);
            }
            if out.len() == MAX_SPELLINGS {
                break;
            }
        }
        out
    }

    /// Tokens the spellings block takes in a local prompt.
    fn spellings_tokens(&self) -> u32 {
        estimate_tokens(&prompt::spellings_block(self.lang, &self.spellings()))
    }

    /// Tokens the marks block takes in a local prompt.
    fn marks_tokens(&self) -> u32 {
        estimate_tokens(&prompt::marks_block(self.lang, &pick_marks(&self.marks)))
    }

    /// The task text the template adds beyond the General template's: its
    /// guidance and its sections' instructions (a user's template can be 8
    /// sections of 200 characters plus 400 of guidance, in Vietnamese).
    fn template_task(&self) -> (String, String) {
        let general = crate::template::builtin("general").expect("general is built in");
        (
            prompt::notes_task(&self.template, self.lang, &[], &[], &[]),
            prompt::notes_task(&general, self.lang, &[], &[], &[]),
        )
    }

    /// Tokens of that extra text; comes out of the transcript's share of the context.
    pub fn template_tokens(&self) -> u32 {
        let (own, general) = self.template_task();
        estimate_tokens(&own).saturating_sub(estimate_tokens(&general))
    }

    /// Bytes of that extra text, for sizing the model's context up front.
    pub fn template_bytes(&self) -> usize {
        let (own, general) = self.template_task();
        own.len().saturating_sub(general.len())
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
    let budget = transcript_budget(llm, output_room(opts))
        .saturating_sub(estimate_tokens(&opts.pinned.join("\n")))
        .saturating_sub(opts.marks_tokens())
        .saturating_sub(opts.template_tokens())
        .saturating_sub(opts.spellings_tokens())
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
    let budget = transcript_budget(llm, output_room(opts))
        .saturating_sub(estimate_tokens(&opts.pinned.join("\n")))
        .saturating_sub(opts.marks_tokens())
        .saturating_sub(opts.template_tokens())
        .saturating_sub(opts.spellings_tokens())
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

/// Output tokens the reduce step of a long meeting may write (full notes). A
/// reduce over many parts' facts used to write past 2,048 and fail; its
/// lists are bounded now ([`bound_reduce`]) and this covers the bound.
const REDUCE_OUTPUT_TOKENS: u32 = 4096;
/// Item caps of the reduce step's lists (a template section gets
/// [`REDUCE_SECTION_CAP`]), citations per item, and characters per text.
const REDUCE_CAPS: &[(&str, usize)] = &[
    ("tldr", schema::MAX_TLDR),
    ("decisions", 12),
    ("action_items", 15),
    ("open_questions", 8),
    ("key_quotes", schema::MAX_QUOTES),
    ("topics", 8),
];
const REDUCE_SECTION_CAP: usize = 8;
const REDUCE_CITES: usize = 4;
const REDUCE_TEXT_CHARS: usize = 240;

/// Tokens a notes call may write, which the transcript budget leaves room
/// for: the reduce step's, unless the notes are compact.
fn output_room(opts: &Options) -> u32 {
    if opts.compact {
        opts.max_output_tokens
    } else {
        opts.max_output_tokens.max(REDUCE_OUTPUT_TOKENS)
    }
}

/// Bounds a local notes schema for the reduce step: fewer items per list,
/// fewer citations and shorter texts, so the reply fits [`REDUCE_OUTPUT_TOKENS`]
/// however many facts it summarises. Cloud schemas are returned as they are.
fn bound_reduce(mut schema: serde_json::Value, d: Dialect) -> serde_json::Value {
    if d != Dialect::Local {
        return schema;
    }
    let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return schema;
    };
    for (key, prop) in props.iter_mut() {
        let cap = REDUCE_CAPS
            .iter()
            .find(|(k, _)| k == key)
            .map_or(REDUCE_SECTION_CAP, |(_, c)| *c);
        let lower = prop
            .get("maxItems")
            .and_then(Value::as_u64)
            .map_or(cap, |m| (m as usize).min(cap));
        prop["maxItems"] = serde_json::json!(lower);
        let Some(fields) = prop
            .pointer_mut("/items/properties")
            .and_then(Value::as_object_mut)
        else {
            continue;
        };
        for name in ["text", "title"] {
            if let Some(f) = fields.get_mut(name) {
                f["maxLength"] = serde_json::json!(REDUCE_TEXT_CHARS);
            }
        }
        if let Some(c) = fields.get_mut("cite") {
            c["maxItems"] = serde_json::json!(REDUCE_CITES);
        }
    }
    schema
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
    // Spellings (the user's vocabulary, attendees, glossary packs) are local-only too.
    let spellings = match dialect {
        Dialect::Local => opts.spellings(),
        Dialect::Cloud => Vec::new(),
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
        prompt::notes_task(&opts.template, opts.lang, &opts.pinned, &marks, &spellings),
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
    /// Required by the schema. A reply without one (a cloud model may leave
    /// it out) reads as proposed: never over-claim a commitment. Notes saved
    /// before statuses existed are a different path (stored `Notes`, whose
    /// decisions are all decided).
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
            Some("decided") => false,
            Some("proposed") => true,
            None => {
                d.missing_status += 1;
                true
            }
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
    // Reduce: the facts, in transcript order, with the lines they cite. The
    // lines the user marked are in it too: a marked line no fact cites gets a
    // fact of its own with the line's text (the model may only cite what it
    // is shown), and neither it nor a fact citing a marked line is trimmed.
    let marks = pick_marks(
        &opts
            .marks
            .iter()
            .copied()
            .filter(|m| t.get(m.id).is_some())
            .collect::<Vec<_>>(),
    );
    add_marked_facts(&mut facts, &marks, t, aliases);
    trim_facts(&mut facts, &marks, budget, &mut diag);
    let max_tokens = output_room(opts);
    let mut retries_left = 2;
    loop {
        let cited: HashSet<u64> = facts.iter().flat_map(|f| f.cites.iter().copied()).collect();
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
            prompt::notes_task(&opts.template, opts.lang, &opts.pinned, &marks, &opts.spellings()),
            prompt::reduce_block(opts.lang, &lines)
        );
        let schema = notes_schema(opts, &shape);
        let req = Request {
            messages: vec![
                Message::system(prompt::notes_system(opts.lang)),
                Message::user(user),
            ],
            schema: Some(if opts.compact {
                schema
            } else {
                bound_reduce(schema, Dialect::Local)
            }),
            max_tokens,
            temperature: 0.3,
        };
        let allowed = |id: u64| cited.contains(&id);
        match run::complete_json(llm, req, opts.lang, &mut diag, |v, d| {
            parse_notes(v, &Ctx::new(t, aliases, opts, &allowed), d)
        })? {
            Outcome::Done(mut notes) => {
                keep_proposals(&mut notes, &facts);
                return Ok(Run {
                    notes,
                    engine: llm.engine(),
                    strategy: Strategy::MapReduce { parts: n },
                    diagnostics: diag,
                });
            }
            // Still too long: say it about fewer facts (minor points first).
            Outcome::Truncated if retries_left > 0 => {
                retries_left -= 1;
                let before = facts.len();
                let target = estimate_tokens(&render_facts(&facts)) * 3 / 4;
                trim_facts(&mut facts, &marks, target, &mut diag);
                if facts.len() == before {
                    retries_left = 0;
                }
            }
            Outcome::Truncated => {
                return Err(LlmError::InvalidOutput(
                    "the notes hit the token limit".into(),
                ));
            }
        }
    }
}

/// Longest stretch of a marked line shown to the reduce step.
const MARKED_LINE_CHARS: usize = 200;

/// A fact for each marked line that no fact cites: what the line says, of the
/// kind the user tagged (a star is a point). Placed in transcript order.
fn add_marked_facts(facts: &mut Vec<Fact>, marks: &[MarkHint], t: &Transcript, aliases: &Aliases) {
    let cited: HashSet<u64> = facts.iter().flat_map(|f| f.cites.iter().copied()).collect();
    for m in marks.iter().filter(|m| !cited.contains(&m.id)) {
        let Some(seg) = t.get(m.id) else { continue };
        let text = plain_text(&seg.text);
        let text: String = text.chars().take(MARKED_LINE_CHARS).collect();
        if text.is_empty() {
            continue;
        }
        let fact = Fact {
            kind: match m.kind {
                MarkKind::Star => "point",
                MarkKind::Decision => "decision",
                MarkKind::Action => "action",
                MarkKind::Question => "question",
            }
            .to_string(),
            text,
            speaker: seg
                .speaker
                .as_deref()
                .and_then(|s| aliases.alias(s))
                .map(str::to_string),
            owner: None,
            due: None,
            cites: vec![m.id],
        };
        let at = facts
            .iter()
            .position(|f| f.cites.first().is_some_and(|&c| c > m.id))
            .unwrap_or(facts.len());
        facts.insert(at, fact);
    }
}

/// Makes the facts fit `budget`: minor points go first, then quotes, then the
/// tail. Facts citing a marked line stay.
fn trim_facts(facts: &mut Vec<Fact>, marks: &[MarkHint], budget: u32, diag: &mut Diagnostics) {
    let marked: HashSet<u64> = marks.iter().map(|m| m.id).collect();
    let keeps = |f: &Fact| f.cites.iter().any(|c| marked.contains(c));
    for minor in ["point", "quote"] {
        if estimate_tokens(&render_facts(facts)) <= budget {
            return;
        }
        facts.retain(|f| f.kind != minor || keeps(f));
        diag.truncated_lists += 1;
    }
    while estimate_tokens(&render_facts(facts)) > budget {
        let loose: Vec<usize> = (0..facts.len()).filter(|&i| !keeps(&facts[i])).collect();
        if loose.is_empty() {
            break;
        }
        // A quarter of the loose facts, from the end.
        let drop = (loose.len() / 4).max(1);
        for &i in loose.iter().rev().take(drop) {
            facts.remove(i);
        }
        diag.truncated_lists += 1;
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
    let (proposed, decided, asked) = (of("proposal"), of("decision"), of("question"));
    // A line a question fact also cites may be a real question that happens
    // to contain a suggestion ("should we maybe switch vendors, and who pays?").
    let (moved, kept): (Vec<Item>, Vec<Item>) = std::mem::take(&mut notes.open_questions)
        .into_iter()
        .partition(|q| {
            q.citations
                .iter()
                .all(|c| proposed.contains(c) && !decided.contains(c) && !asked.contains(c))
        });
    notes.open_questions = kept;
    for q in moved {
        if !notes.proposals.iter().any(|p| p.text == q.text) {
            notes.proposals.push(q);
        }
    }
    // The same sentence as a question and as a decision or proposal is said once.
    let said: HashSet<String> = notes
        .decisions
        .iter()
        .chain(&notes.proposals)
        .map(|i| ghi_text::fold(&i.text))
        .collect();
    notes
        .open_questions
        .retain(|q| !said.contains(&ghi_text::fold(&q.text)));
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
                "decisions":[{"text":"Chốt scope","status":"decided","cite":[0]},{"text":"chot  SCOPE","status":"decided","cite":[0]}],
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
                r#"{"decisions":[{"text":"Ship it","status":"decided","cite":[0]}],"tldr":[{"text":"Not mapped","cite":[5]}]}"#,
            ),
        ]);
        llm.context = 8548;
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
        // The same line marked twice with other marks between: still once.
        let again = pick_marks(&[
            hint(5, MarkKind::Decision, 10),
            hint(6, MarkKind::Action, 11),
            hint(5, MarkKind::Decision, 12),
            hint(7, MarkKind::Question, 13),
            hint(5, MarkKind::Decision, 14),
        ]);
        let ids: Vec<(u64, MarkKind)> = again.iter().map(|m| (m.id, m.kind)).collect();
        assert_eq!(
            ids,
            [
                (5, MarkKind::Decision),
                (6, MarkKind::Action),
                (7, MarkKind::Question)
            ]
        );
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

    /// Answers a map step with no facts and a notes request with one TL;DR item.
    struct Auto(u32);

    impl Llm for Auto {
        fn engine(&self) -> EngineInfo {
            EngineInfo {
                name: "auto".into(),
                version: "1".into(),
            }
        }
        fn context_tokens(&self) -> u32 {
            self.0
        }
        fn complete(&mut self, req: &Request) -> Result<Completion> {
            let facts = req
                .schema
                .as_ref()
                .unwrap()
                .to_string()
                .contains("\"facts\"");
            Ok(Completion {
                text: if facts {
                    r#"{"facts":[]}"#.into()
                } else {
                    reply(r#"{"tldr":[{"text":"ok","cite":[0]}]}"#)
                },
                tokens_in: 1,
                tokens_out: 1,
                truncated: false,
            })
        }
    }

    /// The marks block comes out of the transcript's share of the context: a
    /// transcript that just fits alone is read in parts once 24 marks are added.
    #[test]
    fn marks_can_push_a_borderline_transcript_into_map_reduce() {
        let segs: Vec<_> = (0..40u64)
            .map(|i| {
                seg(
                    i,
                    i as f64 * 5.0,
                    i as f64 * 5.0 + 4.0,
                    "S1",
                    &"word ".repeat(60),
                    "en",
                )
            })
            .collect();
        let t = Transcript::new(segs).unwrap();
        let all: Vec<&Segment> = t.segments().iter().collect();
        let rendered = estimate_tokens(&render(&t, &Aliases::new(&t), &all));
        // The room left for the transcript is exactly what it needs, plus a little.
        let overhead = REDUCE_OUTPUT_TOKENS + PROMPT_OVERHEAD + RETRY_RESERVE;
        let context = rendered + overhead + 10;
        let mut o = Options::new(template::builtin("general").unwrap(), OutLang::En);
        let plain = generate(&mut Auto(context), &t, &o).unwrap();
        assert_eq!(plain.strategy, Strategy::Single);
        o.marks = (0..30)
            .map(|i| hint(i, MarkKind::Decision, i as i64))
            .collect();
        assert!(o.marks_tokens() > 10);
        let marked = generate(&mut Auto(context), &t, &o).unwrap();
        assert!(
            matches!(marked.strategy, Strategy::MapReduce { parts } if parts > 1),
            "{:?}",
            marked.strategy
        );
    }

    /// A maximum-size Vietnamese user template (8 sections of 200 characters, 400 of
    /// guidance) comes out of the transcript's share too, so a retry cannot overflow.
    fn big_template() -> Template {
        use crate::template::{Editor, EditorSection};
        let sections = (0..8)
            .map(|i| EditorSection {
                id: None,
                title: format!("Phần số {i}"),
                instruction: format!(
                    "{} {}",
                    "Nội dung quan trọng của cuộc họp về kế hoạch".repeat(5),
                    i
                )
                .chars()
                .take(200)
                .collect(),
            })
            .collect();
        Template::from_editor(
            "t1",
            &Editor {
                name: "Họp lớn".into(),
                lang: OutLang::Vi,
                guidance: "Cuộc họp kế hoạch hằng tuần của nhóm sản phẩm. "
                    .repeat(10)
                    .chars()
                    .take(400)
                    .collect(),
                sections,
            },
            &[],
            &[],
        )
        .unwrap()
    }

    #[test]
    fn spellings_are_a_local_prompt_block_in_both_languages_and_never_in_a_cloud_request() {
        let t = Transcript::new(vec![seg(1, 0.0, 4.0, "S1", "we use nemotron and Coreml here", "en")]).unwrap();
        let all: Vec<&Segment> = t.segments().iter().collect();
        let mut o = Options::new(template::builtin("general").unwrap(), OutLang::En);
        o.spellings = vec!["Nemotron".into(), "CoreML".into()];
        let text = |d: Dialect, o: &Options| request(&t, &all, &Aliases::new(&t), o, d).0.messages[1].content.clone();
        let local = text(Dialect::Local, &o);
        assert!(local.contains("These terms are said in the meeting. Write them exactly like this: Nemotron; CoreML\n"), "{local}");
        // After the template and before the transcript.
        assert!(local.find("- topics:").unwrap() < local.find("Nemotron; CoreML").unwrap());
        assert!(local.find("Nemotron; CoreML").unwrap() < local.find("Transcript:").unwrap());
        let cloud = text(Dialect::Cloud, &o);
        assert!(!cloud.contains("Nemotron") && !cloud.contains("CoreML") && !cloud.contains("exactly like this"), "{cloud}");
        o.lang = OutLang::Vi;
        let vi = text(Dialect::Local, &o);
        assert!(vi.contains("Các thuật ngữ sau được nhắc đến trong cuộc họp. Hãy viết đúng như sau: Nemotron; CoreML\n"), "{vi}");
        o.spellings.clear();
        assert!(!text(Dialect::Local, &o).contains("exactly like this"));
        assert_eq!(prompt::spellings_block(OutLang::En, &[]), "");
    }

    #[test]
    fn at_most_forty_distinct_spellings_are_carried() {
        let mut o = Options::new(template::builtin("general").unwrap(), OutLang::En);
        o.spellings = (0..60).map(|i| format!("Term{}", i % 50)).collect();
        o.spellings.insert(1, "  Term0  ".into());
        o.spellings.push(String::new());
        let s = o.spellings();
        assert_eq!(s.len(), MAX_SPELLINGS);
        assert_eq!(s[0], "Term0");
        assert_eq!(s.iter().collect::<HashSet<_>>().len(), MAX_SPELLINGS, "distinct");
    }

    /// The spellings come out of the transcript's share like marks do.
    #[test]
    fn spellings_can_push_a_borderline_transcript_into_map_reduce() {
        let segs: Vec<_> = (0..40u64)
            .map(|i| seg(i, i as f64 * 5.0, i as f64 * 5.0 + 4.0, "S1", &"word ".repeat(60), "en"))
            .collect();
        let t = Transcript::new(segs).unwrap();
        let all: Vec<&Segment> = t.segments().iter().collect();
        let rendered = estimate_tokens(&render(&t, &Aliases::new(&t), &all));
        let mut o = Options::new(template::builtin("general").unwrap(), OutLang::En);
        // The smallest context that reads it whole, found by trying (the reserved output room is the engine's business).
        let mut context = rendered;
        while generate(&mut Auto(context), &t, &o).unwrap().strategy != Strategy::Single {
            context += 16;
        }
        context += 10;
        assert_eq!(generate(&mut Auto(context), &t, &o).unwrap().strategy, Strategy::Single);
        o.spellings = (0..40).map(|i| format!("Pharmacokinetics{i}")).collect();
        assert!(o.spellings_tokens() > 10);
        let run = generate(&mut Auto(context), &t, &o).unwrap();
        assert!(matches!(run.strategy, Strategy::MapReduce { parts } if parts > 1), "{:?}", run.strategy);
    }

    /// Answers with notes that have every key of the template (empty lists).
    struct AutoKeys(u32, Vec<String>);

    impl Llm for AutoKeys {
        fn engine(&self) -> EngineInfo {
            EngineInfo {
                name: "auto-keys".into(),
                version: "1".into(),
            }
        }
        fn context_tokens(&self) -> u32 {
            self.0
        }
        fn complete(&mut self, req: &Request) -> Result<Completion> {
            let facts = req
                .schema
                .as_ref()
                .unwrap()
                .to_string()
                .contains("\"facts\"");
            let extra: String = self
                .1
                .iter()
                .map(|k| format!("\"{k}\":[]"))
                .collect::<Vec<_>>()
                .join(",");
            Ok(Completion {
                text: if facts {
                    r#"{"facts":[]}"#.into()
                } else {
                    reply(&format!("{{{extra}}}"))
                },
                tokens_in: 1,
                tokens_out: 1,
                truncated: false,
            })
        }
    }

    #[test]
    fn a_big_user_template_is_counted_and_can_push_a_borderline_transcript_into_map_reduce() {
        let segs: Vec<_> = (0..40u64)
            .map(|i| {
                seg(
                    i,
                    i as f64 * 5.0,
                    i as f64 * 5.0 + 4.0,
                    "S1",
                    &"word ".repeat(60),
                    "vi",
                )
            })
            .collect();
        let t = Transcript::new(segs).unwrap();
        let all: Vec<&Segment> = t.segments().iter().collect();
        let rendered = estimate_tokens(&render(&t, &Aliases::new(&t), &all));
        let overhead = REDUCE_OUTPUT_TOKENS + PROMPT_OVERHEAD + RETRY_RESERVE;
        let context = rendered + overhead + 10;
        let general = Options::new(crate::template::builtin("general").unwrap(), OutLang::Vi);
        assert_eq!(general.template_tokens(), 0);
        assert_eq!(general.template_bytes(), 0);
        assert_eq!(
            generate(&mut Auto(context), &t, &general).unwrap().strategy,
            Strategy::Single
        );
        let big = Options::new(big_template(), OutLang::Vi);
        assert!(big.template_tokens() > 300, "{}", big.template_tokens());
        assert!(big.template_bytes() > 1500, "{}", big.template_bytes());
        let keys: Vec<String> = big.template.sections.iter().map(|x| x.id.clone()).collect();
        let run = generate(&mut AutoKeys(context, keys), &t, &big).unwrap();
        assert!(
            matches!(run.strategy, Strategy::MapReduce { parts } if parts > 1),
            "{:?}",
            run.strategy
        );
    }

    /// The template's words come after the fixed safety rules: the rules are the system
    /// message, the template is in the task that follows, never in the rules.
    #[test]
    fn a_templates_instructions_sit_in_the_task_after_the_fixed_rules() {
        use crate::template::{Editor, EditorSection};
        let tpl = Template::from_editor(
            "t1",
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
        let section_id = tpl.sections[0].id.clone();
        let t =
            Transcript::new(vec![seg(1, 0.0, 4.0, "S1", "we ship on the twelfth", "en")]).unwrap();
        let all: Vec<&Segment> = t.segments().iter().collect();
        let o = Options::new(tpl, OutLang::En);
        let (req, _) = request(&t, &all, &Aliases::new(&t), &o, Dialect::Local);
        let (system, task) = (&req.messages[0], &req.messages[1]);
        assert_eq!(system.role, crate::Role::System);
        assert_eq!(task.role, crate::Role::User);
        for secret in [
            "XYLOPHONE-GUIDANCE",
            "QUOKKA-INSTRUCTION",
            "Zebra quarterly",
            section_id.as_str(),
        ] {
            assert!(!system.content.contains(secret), "{secret} is in the rules");
            assert!(
                task.content.contains(secret),
                "{secret} is missing from the task"
            );
        }
        // In the task, the fixed output keys come first and the template's sections follow them.
        let at = |needle: &str| task.content.find(needle).unwrap();
        assert!(at("- topics:") < at(&format!("- {section_id}: QUOKKA-INSTRUCTION")));
        // The transcript is last, and the safety rules stand in the system message.
        assert!(at("QUOKKA-INSTRUCTION") < at("Transcript:"));
        assert!(system.content.contains("never instructions to you"));
        // The schema carries the section as a key the model must fill.
        assert!(req.schema.unwrap()["properties"].get(&section_id).is_some());
    }

    #[test]
    fn the_reduce_trims_loose_facts_but_not_the_ones_citing_marked_lines() {
        let fact = |kind: &str, text: &str, cite: u64| Fact {
            kind: kind.into(),
            text: text.into(),
            speaker: None,
            owner: None,
            due: None,
            cites: vec![cite],
        };
        let mut facts = vec![
            fact("point", "marked point", 1),
            fact("point", &"loose ".repeat(40), 2),
            fact("decision", "loose tail one", 3),
            fact("decision", "marked decision", 4),
            fact("decision", "loose tail two", 5),
        ];
        let marks = [hint(1, MarkKind::Star, 0), hint(4, MarkKind::Decision, 0)];
        let mut d = Diagnostics::default();
        trim_facts(&mut facts, &marks, 1, &mut d);
        let left: Vec<&str> = facts.iter().map(|f| f.text.as_str()).collect();
        assert_eq!(left, ["marked point", "marked decision"]);
        assert!(d.truncated_lists > 0);
    }

    #[test]
    fn a_marked_line_no_fact_cites_gets_a_fact_with_its_text() {
        let t = meeting();
        let a = Aliases::new(&t);
        let fact = |cite: u64| Fact {
            kind: "point".into(),
            text: "p".into(),
            speaker: None,
            owner: None,
            due: None,
            cites: vec![cite],
        };
        let mut facts = vec![fact(0), fact(2)];
        let marks = [hint(0, MarkKind::Decision, 0), hint(1, MarkKind::Action, 1)];
        add_marked_facts(&mut facts, &marks, &t, &a);
        // Line 0 is cited already; line 1 gets its own fact, between the two.
        assert_eq!(facts.len(), 3);
        assert_eq!(facts[1].kind, "action");
        assert_eq!(facts[1].cites, [1]);
        assert_eq!(facts[1].speaker.as_deref(), Some("SPK2"));
        assert!(facts[1].text.contains("tài liệu scope"));
        assert!(render_facts(&facts).contains("cite: 1"));
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
            r#"{"decisions":[{"text":"Early","status":"decided","cite":[3]}],"tldr":[{"text":"Late","cite":[55]},{"text":"Unmarked","cite":[30]}]}"#,
        );
        let mut llm = Scripted::new(&[&f1, &f2, &f3, &reduce]);
        llm.context = 18_432; // 16k of room for the transcript, as before the reduce was given 4k to write
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
        let shown = &reduce_req.messages[1].content;
        assert!(shown.contains("[s3 decision] [s55 star]"));
        // A marked line is citable only because the reduce step is shown it:
        // a fact with the line's own text (a star is a point), cap 200 chars.
        let line = |kind: &str, n: u64| {
            shown
                .lines()
                .find(|l| {
                    l.starts_with(&format!("- [{kind}")) && l.ends_with(&format!("cite: {n}"))
                })
                .unwrap_or_else(|| panic!("no fact for line {n} in {shown}"))
                .to_string()
        };
        let early = line("decision", 3);
        assert!(
            early.contains("line3 word word") && early.len() < 300,
            "{early}"
        );
        assert!(line("point", 55).contains("line55 word word"));
        let schema = reduce_req.schema.as_ref().unwrap().to_string();
        assert!(schema.contains("\"enum\":[3,5,25,45,55]"), "{schema}");
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
        assert_eq!(texts(&run.notes.decisions), ["Chốt scope beta"]);
        // A fresh reply with no status is never taken as a commitment.
        assert_eq!(
            texts(&run.notes.proposals),
            ["Có thể xem lại ngân sách", "Gửi tài liệu"]
        );
        assert_eq!(run.diagnostics.missing_status, 1);
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
        assert_eq!(back.decisions.len(), 1, "stored decisions stay decided");
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
        llm.context = 8548;
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
        llm.context = 8548;
        let o = Options::new(template::builtin("general").unwrap(), OutLang::En);
        let run = generate(&mut llm, &t, &o).unwrap();
        assert_eq!(run.notes.proposals[0].text, "Try dark mode");
        assert_eq!(run.notes.open_questions.len(), 1, "a real question stays");
        assert_eq!(run.notes.open_questions[0].text, "Budget?");
    }

    #[test]
    fn a_question_that_holds_a_suggestion_stays_a_question() {
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
            r#"{"facts":[{"kind":"proposal","text":"Maybe switch vendors","speaker":"SPK1","owner":null,"due":null,"cite":[3]},{"kind":"question","text":"Who would pay for it?","speaker":"SPK1","owner":null,"due":null,"cite":[3]},{"kind":"proposal","text":"Try dark mode","speaker":"SPK2","owner":null,"due":null,"cite":[5]}]}"#,
            r#"{"facts":[{"kind":"decision","text":"Ship Friday","speaker":"SPK2","owner":null,"due":null,"cite":[21]}]}"#,
            &reply(
                r#"{"decisions":[{"text":"Ship Friday","status":"decided","cite":[21]}],
                    "open_questions":[
                      {"text":"Should we maybe switch vendors, and who would pay for it?","cite":[3]},
                      {"text":"Try dark mode","cite":[5]},
                      {"text":"Ship Friday","cite":[21]}]}"#,
            ),
        ]);
        llm.context = 8548;
        let o = Options::new(template::builtin("general").unwrap(), OutLang::En);
        let run = generate(&mut llm, &t, &o).unwrap();
        let q: Vec<&str> = run
            .notes
            .open_questions
            .iter()
            .map(|i| i.text.as_str())
            .collect();
        assert_eq!(
            q,
            ["Should we maybe switch vendors, and who would pay for it?"]
        );
        assert_eq!(run.notes.proposals[0].text, "Try dark mode");
    }

    #[test]
    fn the_reduce_is_bounded_and_retries_with_fewer_facts_when_still_cut_off() {
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
        let fact = |kind: &str, text: &str, cite: u64| {
            format!(
                r#"{{"kind":"{kind}","text":"{text}","speaker":"SPK1","owner":null,"due":null,"cite":[{cite}]}}"#
            )
        };
        let map1 = format!(
            r#"{{"facts":[{},{}]}}"#,
            fact("decision", "Ship Friday", 0),
            fact("point", "minor aside", 1)
        );
        let map2 = r#"{"facts":[]}"#;
        let mut llm = Scripted::new(&[
            &map1,
            map2,
            "{",
            &reply(r#"{"tldr":[{"text":"ok","cite":[0]}]}"#),
        ]);
        llm.replies[2].1 = true; // the first reduce reply is cut off
        llm.context = 8548;
        let o = Options::new(template::builtin("general").unwrap(), OutLang::En);
        let run = generate(&mut llm, &t, &o).unwrap();
        assert_eq!(run.strategy, Strategy::MapReduce { parts: 2 });
        let (first, second) = (&llm.requests[2], &llm.requests[3]);
        assert!(first.messages[1].content.contains("minor aside"));
        assert!(
            !second.messages[1].content.contains("minor aside"),
            "fewer facts"
        );
        // The reduce writes up to 4,096 tokens, and its lists are bounded.
        assert_eq!(first.max_tokens, REDUCE_OUTPUT_TOKENS);
        let p = &first.schema.as_ref().unwrap()["properties"];
        assert_eq!(p["decisions"]["maxItems"], 12);
        assert_eq!(p["action_items"]["maxItems"], 15);
        assert_eq!(p["tldr"]["maxItems"], 5);
        assert_eq!(
            p["decisions"]["items"]["properties"]["text"]["maxLength"],
            240
        );
        assert_eq!(p["decisions"]["items"]["properties"]["cite"]["maxItems"], 4);
        // The map steps are not bounded this way.
        assert!(
            llm.requests[0]
                .schema
                .as_ref()
                .unwrap()
                .to_string()
                .find("maxLength")
                .is_none()
        );
        // Two cut-offs in a row with nothing left to drop: an error, not a loop.
        let mut cut = Scripted::new(&[&map1, map2, "{", "{", "{"]);
        for r in &mut cut.replies[2..] {
            r.1 = true;
        }
        cut.context = 8548;
        assert!(generate(&mut cut, &t, &o).is_err());
    }
}
