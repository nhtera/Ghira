// SPDX-License-Identifier: Apache-2.0
//! `ghi notes` and `ghi ask`: the notes engine (`ghi-llm`) on a
//! `ghi.transcript/1` file, and `ghi store notes` on a stored meeting.
//!
//! The local model runs in the `ghi-llm-worker` process (built next to
//! `ghi`; override with `GHI_LLM_WORKER`). Output is `ghi.notes/1` (the
//! harness fields plus the full notes) and `ghi.ask/1`.

use std::path::Path;
use std::time::Instant;

use ghi_llm::enhance::{self, NoteLine};
use ghi_llm::notes::{self, Notes, Options};
use ghi_llm::template::{self, OutLang, Template};
use ghi_llm::{LlmError, ask, local::LocalLlm};
use serde_json::{Value, json};

use ghi_llm::cloud::CloudProvider;
use ghi_llm::redact::{KnownEntities, Redactor};
use ghi_llm::schema::Dialect;
use ghi_llm::transcript::Aliases;
use ghi_llm::validate::plain_text;
use ghi_net::MeetingGate;

use crate::cmd::cloud::{CloudArgs, Exchange, exchange};
use crate::contract::{self, ErrorCode, ErrorDoc, NOTES};
use crate::engine::peak_rss_mb;
use crate::keystore::{open_store, store_error};

/// Default local model (registry id) and context size.
pub const DEFAULT_MODEL: &str = "qwen3-4b";
pub const DEFAULT_CTX: u32 = 32768;

#[derive(Debug, Clone, clap::Args)]
pub struct ModelArgs {
    /// Local model (registry id).
    #[arg(long, default_value = DEFAULT_MODEL)]
    pub model: String,
    /// Context size of the local model, in tokens.
    #[arg(long, default_value_t = DEFAULT_CTX)]
    pub n_ctx: u32,
}

#[derive(Debug, Clone, clap::Args)]
pub struct NotesArgs {
    /// Built-in template: general, one_on_one, standup, sales, interview, client, lecture.
    #[arg(long, default_value = "general")]
    pub template: String,
    /// A custom template (TOML), instead of --template.
    #[arg(long)]
    pub template_file: Option<std::path::PathBuf>,
    /// Notes the user typed, one per line (`[mm:ss] ` prefix = when typed), to enhance.
    #[arg(long)]
    pub user_notes: Option<std::path::PathBuf>,
}

pub fn llm_error(e: LlmError) -> ErrorDoc {
    let code = match e {
        LlmError::Invalid(_) => ErrorCode::BadInput,
        LlmError::Worker(_) | LlmError::Timeout => ErrorCode::EngineUnavailable,
        _ => ErrorCode::Internal,
    };
    ErrorDoc::new(code, e.to_string())
}

/// A `ghi.transcript/1` document as the engine's input.
pub fn to_llm(t: &contract::Transcript) -> Result<ghi_llm::Transcript, ErrorDoc> {
    let segs = t
        .segments
        .iter()
        .map(|s| ghi_llm::Segment {
            id: u64::from(s.id),
            t0_ms: (s.start * 1000.0).round() as i64,
            t1_ms: (s.end * 1000.0).round() as i64,
            speaker: s.speaker.clone(),
            text: s.text.clone(),
            lang: s.lang.map(|l| match l {
                contract::Lang::Vi => "vi".to_string(),
                contract::Lang::En => "en".to_string(),
            }),
        })
        .collect();
    ghi_llm::Transcript::new(segs).map_err(llm_error)
}

/// `auto` → the meeting's language.
pub fn out_lang(lang: contract::LangMode, t: &ghi_llm::Transcript) -> OutLang {
    let choice = match lang {
        contract::LangMode::Auto => "meeting",
        contract::LangMode::Vi => "vi",
        contract::LangMode::En => "en",
    };
    OutLang::resolve(choice, t).unwrap_or(OutLang::En)
}

pub fn load_template(args: &NotesArgs) -> Result<Template, ErrorDoc> {
    match &args.template_file {
        Some(path) => {
            let text = std::fs::read_to_string(path).map_err(|e| {
                ErrorDoc::new(ErrorCode::BadInput, format!("{}: {e}", path.display()))
            })?;
            Template::from_toml(&text).map_err(llm_error)
        }
        None => template::builtin(&args.template).map_err(llm_error),
    }
}

/// User notes: one per line, an optional `[mm:ss] ` prefix is when it was typed.
pub fn read_user_notes(path: &Path) -> Result<Vec<NoteLine>, ErrorDoc> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ErrorDoc::new(ErrorCode::BadInput, format!("{}: {e}", path.display())))?;
    Ok(text
        .lines()
        .map(|l| {
            let at = l.strip_prefix('[').and_then(|r| {
                let (ts, rest) = r.split_once(']')?;
                let (m, s) = ts.split_once(':')?;
                let ms =
                    (m.trim().parse::<i64>().ok()? * 60 + s.trim().parse::<i64>().ok()?) * 1000;
                Some((ms, rest.trim_start()))
            });
            match at {
                Some((ms, rest)) => NoteLine {
                    text: rest.to_string(),
                    t_ms: Some(ms),
                },
                None => NoteLine {
                    text: l.to_string(),
                    t_ms: None,
                },
            }
        })
        .collect())
}

pub fn open_local(m: &ModelArgs) -> Result<LocalLlm, ErrorDoc> {
    LocalLlm::open_registry(&m.model, m.n_ctx).map_err(llm_error)
}

fn items(v: &[notes::Item]) -> Vec<Value> {
    v.iter()
        .map(|i| json!({"text": i.text, "citations": i.citations}))
        .collect()
}

/// The `ghi.notes/1` document: the harness fields (`summary` = TL;DR,
/// `decisions`, `action_items`) plus everything else the engine produced.
pub fn notes_doc(
    n: &Notes,
    engine: &ghi_llm::EngineInfo,
    extra: serde_json::Map<String, Value>,
    wall_s: f64,
) -> Value {
    let mut doc = json!({
        "schema": NOTES,
        "engine": {"name": engine.name, "version": engine.version},
        "template": n.template,
        "lang": n.lang,
        "summary": items(&n.tldr),
        "decisions": items(&n.decisions),
        "action_items": n.action_items.iter().map(|a| json!({
            "text": a.text, "owner": a.owner, "due": a.due, "citations": a.citations,
        })).collect::<Vec<_>>(),
        "open_questions": items(&n.open_questions),
        "key_quotes": n.key_quotes.iter().map(|q| json!({
            "text": q.text, "speaker": q.speaker, "citations": q.citations,
        })).collect::<Vec<_>>(),
        "topics": n.topics.iter().map(|t| json!({
            "title": t.title, "citations": t.citations,
            "start": t.t0_ms as f64 / 1000.0, "end": t.t1_ms as f64 / 1000.0,
        })).collect::<Vec<_>>(),
        "sections": n.sections.iter().map(|s| json!({
            "id": s.id, "title": s.title, "items": items(&s.items),
        })).collect::<Vec<_>>(),
    });
    let obj = doc.as_object_mut().expect("object");
    obj.extend(extra);
    obj.insert(
        "perf".into(),
        json!({"wall_s": wall_s, "rtf": null, "peak_rss_mb": peak_rss_mb()}),
    );
    doc
}

/// `ghi notes <transcript>`: the local model, or a cloud provider after a
/// confirmed send preview.
pub fn run(
    transcript: &Path,
    lang: contract::LangMode,
    args: &NotesArgs,
    model: &ModelArgs,
    cloud: &CloudArgs,
) -> Result<(), ErrorDoc> {
    let started = Instant::now();
    let t = to_llm(&crate::read_transcript(transcript)?)?;
    let opts = Options::new(load_template(args)?, out_lang(lang, &t));
    let user_notes = args
        .user_notes
        .as_deref()
        .map(read_user_notes)
        .transpose()?;
    let provider = cloud.provider()?;
    if provider.is_some() && user_notes.is_some() {
        return Err(ErrorDoc::new(
            ErrorCode::BadInput,
            "--user-notes works with the local model only (for now)",
        ));
    }
    // A transcript file belongs to no stored meeting: nothing is locked.
    let gates = [MeetingGate {
        cloud_locked: false,
        sensitive: false,
    }];
    let no_log = |_: &Value| Ok(());
    let Some(mut out) = notes_with(&t, &opts, model, cloud, provider.as_ref(), &gates, &no_log)?
    else {
        return Ok(()); // previewed only
    };
    if let Some(lines) = user_notes {
        let mut llm = open_local(model)?;
        let e = enhance::enhance(&mut llm, &t, &lines, opts.lang).map_err(llm_error)?;
        out.run.diagnostics.add(&e.diagnostics);
        out.extra.insert("enhanced".into(), json!(e.lines));
    }
    out.extra
        .insert("diagnostics".into(), json!(out.run.diagnostics));
    crate::emit(&notes_doc(
        &out.run.notes,
        &out.run.engine,
        out.extra,
        started.elapsed().as_secs_f64(),
    ))
}

/// Notes and what to add to the output document about how they were made.
struct NotesOut {
    run: notes::Run,
    extra: serde_json::Map<String, Value>,
    /// Set when a cloud request was sent (for the audit log).
    sent: Option<Value>,
}

fn cloud_engine(p: &CloudProvider) -> ghi_llm::EngineInfo {
    ghi_llm::EngineInfo {
        name: format!("{}:{}", p.name(), p.model),
        version: "cloud".into(),
    }
}

/// Notes from the local model, or from `provider`; `None` after `--preview`.
/// A failed cloud request falls back to the local model.
fn notes_with(
    t: &ghi_llm::Transcript,
    opts: &Options,
    model: &ModelArgs,
    cloud: &CloudArgs,
    provider: Option<&CloudProvider>,
    gates: &[MeetingGate],
    on_sent: &dyn Fn(&Value) -> Result<(), ErrorDoc>,
) -> Result<Option<NotesOut>, ErrorDoc> {
    let local = |mut extra: serde_json::Map<String, Value>| -> Result<NotesOut, ErrorDoc> {
        let mut llm = open_local(model)?;
        let run = notes::generate(&mut llm, t, opts).map_err(llm_error)?;
        extra.insert("strategy".into(), json!(run.strategy));
        Ok(NotesOut {
            run,
            extra,
            sent: None,
        })
    };
    let Some(provider) = provider else {
        return local(serde_json::Map::new()).map(Some);
    };
    let mut redactor = redactor_for(t, cloud);
    let red = redactor.redact_transcript(t);
    let aliases = Aliases::new(&red);
    let all: Vec<&ghi_llm::Segment> = red.segments().iter().collect();
    // Cloud AI gets transcript text only: never the user's own notes.
    let cloud_opts = Options {
        pinned: Vec::new(),
        ..opts.clone()
    };
    let opts = &cloud_opts;
    let (req, _) = notes::request(&red, &all, &aliases, opts, Dialect::Cloud);
    let fallback = |reason: String, sent: Option<Value>| -> Result<Option<NotesOut>, ErrorDoc> {
        let mut extra = serde_json::Map::new();
        extra.insert("fallback".into(), json!({"to": "local", "reason": reason}));
        let mut out = local(extra)?;
        out.sent = sent;
        Ok(Some(out))
    };
    match exchange(cloud, provider, &req, gates, &redactor.summary())? {
        Exchange::Previewed => Ok(None),
        Exchange::Failed { reason, sent } => {
            if let Some(s) = &sent {
                on_sent(s)?;
            }
            fallback(reason, sent)
        }
        Exchange::Reply { completion, sent } => {
            on_sent(&sent)?;
            let mut diag = ghi_llm::validate::Diagnostics {
                requests: 1,
                tokens_in: u64::from(completion.tokens_in),
                tokens_out: u64::from(completion.tokens_out),
                ..Default::default()
            };
            if completion.truncated {
                return fallback("the reply hit the token limit".into(), Some(sent));
            }
            let mut n = match notes::parse(&completion.text, &red, &aliases, opts, &mut diag) {
                Ok(n) => n,
                Err(e) => return fallback(e.to_string(), Some(sent)),
            };
            // Put redacted values back locally; count what could not be.
            let mut unresolved = 0;
            n.map_text(|s| {
                let r = redactor.restore(s);
                unresolved += r.unresolved;
                // Restored values are plain text too.
                plain_text(&r.text)
            });
            let mut extra = serde_json::Map::new();
            extra.insert("strategy".into(), json!({"kind": "single"}));
            extra.insert("cloud".into(), sent.clone());
            extra.insert("unresolved_placeholders".into(), json!(unresolved));
            Ok(Some(NotesOut {
                run: notes::Run {
                    notes: n,
                    engine: cloud_engine(provider),
                    strategy: notes::Strategy::Single,
                    diagnostics: diag,
                },
                extra,
                sent: Some(sent),
            }))
        }
    }
}

/// Redacts the speakers' names (not `S1`-style labels) and `--redact-names`.
fn redactor_for(t: &ghi_llm::Transcript, cloud: &CloudArgs) -> Redactor {
    let is_label = |s: &str| {
        s.strip_prefix('S')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    };
    let mut people: Vec<String> = t
        .speakers()
        .into_iter()
        .map(|s| t.speaker_name(s).to_string())
        .filter(|n| !is_label(n))
        .collect();
    people.extend(cloud.redact_names.iter().cloned());
    Redactor::new(KnownEntities {
        people,
        orgs: Vec::new(),
        terms: Vec::new(),
    })
}

/// `ghi ask <transcript> <question>` (`ghi.ask/1`): the local model, or a
/// cloud provider after a confirmed send preview.
pub fn ask(
    transcript: &Path,
    question: &str,
    lang: contract::LangMode,
    model: &ModelArgs,
    cloud: &CloudArgs,
) -> Result<(), ErrorDoc> {
    let started = Instant::now();
    let t = to_llm(&crate::read_transcript(transcript)?)?;
    let lang = out_lang(lang, &t);
    let local = || -> Result<ask::Run, ErrorDoc> {
        let mut llm = open_local(model)?;
        ask::ask(&mut llm, &t, question, lang).map_err(llm_error)
    };
    let mut extra = serde_json::Map::new();
    let run = match cloud.provider()? {
        None => local()?,
        Some(provider) => {
            let mut redactor = redactor_for(&t, cloud);
            let red = redactor.redact_transcript(&t);
            let q = redactor.redact(question);
            let aliases = Aliases::new(&red);
            let prepared = ask::prepare(&red, &aliases, &q, lang, u32::MAX, Dialect::Cloud)
                .map_err(llm_error)?;
            let gates = [MeetingGate {
                cloud_locked: false,
                sensitive: false,
            }];
            match prepared {
                // Nothing matched: answered without sending anything (the
                // terms shown are the user's, not the redacted ones).
                Err(_) => ask::Run {
                    answer: ask::Answer::NotDiscussed {
                        searched: ghi_llm::retrieval::query_terms(question),
                    },
                    engine: cloud_engine(&provider),
                    diagnostics: Default::default(),
                },
                Ok(p) => match exchange(cloud, &provider, &p.request, &gates, &redactor.summary())?
                {
                    Exchange::Previewed => return Ok(()),
                    Exchange::Failed { reason, .. } => {
                        extra.insert("fallback".into(), json!({"to": "local", "reason": reason}));
                        local()?
                    }
                    Exchange::Reply { completion, sent } => {
                        let mut diag = ghi_llm::validate::Diagnostics {
                            requests: 1,
                            tokens_in: u64::from(completion.tokens_in),
                            tokens_out: u64::from(completion.tokens_out),
                            ..Default::default()
                        };
                        extra.insert("cloud".into(), sent);
                        match p.parse(&completion.text, &mut diag) {
                            Ok(mut answer) => {
                                match &mut answer {
                                    ask::Answer::Answered { text, .. } => {
                                        *text = plain_text(&redactor.restore(text).text);
                                    }
                                    ask::Answer::NotDiscussed { searched } => {
                                        *searched = ghi_llm::retrieval::query_terms(question);
                                    }
                                }
                                ask::Run {
                                    answer,
                                    engine: cloud_engine(&provider),
                                    diagnostics: diag,
                                }
                            }
                            Err(e) => {
                                extra.insert(
                                    "fallback".into(),
                                    json!({"to": "local", "reason": e.to_string()}),
                                );
                                local()?
                            }
                        }
                    }
                },
            }
        }
    };
    let mut doc = json!({
        "schema": "ghi.ask/1",
        "engine": {"name": run.engine.name, "version": run.engine.version},
        "answer": run.answer,
        "diagnostics": run.diagnostics,
        "perf": {"wall_s": started.elapsed().as_secs_f64(), "rtf": null, "peak_rss_mb": peak_rss_mb()},
    });
    doc.as_object_mut().expect("object").extend(extra);
    crate::emit(&doc)
}

/// A stored meeting's current transcript as the engine's input. Segment ids
/// are positions in `segments`; speakers are speaker gids, so an action
/// item's owner maps straight back to a speaker row.
/// `ghi store notes <meeting>`: notes from the stored transcript, saved as AI
/// note blocks and action items with time anchors (doc 05 §2.3).
pub fn store_notes(
    dir: &Path,
    meeting: &str,
    lang: contract::LangMode,
    args: &NotesArgs,
    model: &ModelArgs,
    cloud: &CloudArgs,
) -> Result<(), ErrorDoc> {
    let started = Instant::now();
    let store = open_store(dir)?;
    let internal = |e: String| ErrorDoc::new(ErrorCode::Internal, e);
    // An unknown meeting is the caller's mistake (bad_input), not internal.
    store.get_meeting(meeting).map_err(store_error)?;
    let (t, segs) = ghi_core::notes_job::stored_transcript(&store, meeting).map_err(internal)?;
    if t.is_empty() {
        return Err(ErrorDoc::new(
            ErrorCode::BadInput,
            format!("meeting {meeting} has no transcript"),
        ));
    }
    let mut opts = Options::new(load_template(args)?, out_lang(lang, &t));
    // What the user wrote, pinned, edited or ticked off stays; the model is
    // told not to repeat it.
    opts.pinned = ghi_core::notes_job::kept_texts(&store, meeting).map_err(internal)?;
    // The meeting's own privacy switches gate a cloud send (RT-6).
    let m = store.get_meeting(meeting).map_err(store_error)?;
    let gates = [MeetingGate {
        cloud_locked: m.cloud_locked,
        sensitive: m.sensitive,
    }];
    let provider = cloud.provider()?;
    // Logged as soon as a request may have left, before any fallback.
    let log = |sent: &Value| {
        store
            .record_cloud_request(
                meeting,
                sent["provider"].as_str().unwrap_or("?"),
                sent["model"].as_str().unwrap_or("?"),
                sent["tokens_in"].as_u64().unwrap_or(0),
                sent["tokens_out"].as_u64().unwrap_or(0),
            )
            .map_err(store_error)
    };
    let Some(out) = notes_with(&t, &opts, model, cloud, provider.as_ref(), &gates, &log)? else {
        return Ok(()); // previewed only
    };
    let run = out.run;

    let n = &run.notes;
    let r = ghi_core::notes_job::save_notes(&store, meeting, n, &segs, &run.engine.name)
        .map_err(internal)?;
    let mut extra = out.extra;
    extra.insert("meeting".into(), json!(meeting));
    extra.insert(
        "saved".into(),
        json!({"removed": r.removed, "kept": r.kept, "added": r.added}),
    );
    extra.insert("diagnostics".into(), json!(run.diagnostics));
    let mut doc = notes_doc(n, &run.engine, extra, started.elapsed().as_secs_f64());
    doc["schema"] = json!("ghi.store-notes/1");
    crate::emit(&doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_notes_take_an_optional_time_prefix() {
        let dir = std::env::temp_dir().join(format!("ghi-notes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("n.txt");
        std::fs::write(&p, "[01:05] chốt scope\nno time\n[bad] x\n").unwrap();
        let lines = read_user_notes(&p).unwrap();
        assert_eq!(lines[0].t_ms, Some(65_000));
        assert_eq!(lines[0].text, "chốt scope");
        assert_eq!(lines[1].t_ms, None);
        assert_eq!(lines[2].text, "[bad] x");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn notes_doc_keeps_the_harness_contract() {
        let n = Notes {
            template: "general".into(),
            lang: "en".into(),
            tldr: vec![notes::Item {
                text: "Beta scope agreed".into(),
                citations: vec![0],
            }],
            decisions: vec![],
            action_items: vec![notes::ActionItem {
                text: "Send doc".into(),
                owner: Some("S2".into()),
                due: None,
                citations: vec![1],
            }],
            open_questions: vec![],
            key_quotes: vec![],
            topics: vec![],
            sections: vec![],
        };
        let engine = ghi_llm::EngineInfo {
            name: "qwen3-4b".into(),
            version: "bc640142".into(),
        };
        let doc = notes_doc(&n, &engine, serde_json::Map::new(), 1.5);
        let parsed: contract::Notes = serde_json::from_value(doc).unwrap();
        assert_eq!(parsed.schema, NOTES);
        assert_eq!(parsed.summary[0].citations, vec![0]);
        assert_eq!(parsed.action_items[0].owner.as_deref(), Some("S2"));
    }
}
