// SPDX-License-Identifier: Apache-2.0
//! Notes from a stored meeting, shared by the jobs and `ghi store notes`.
//!
//! - `notes_live`: right after stop, from the live transcript (≤3 min [RT-7]).
//! - `notes_final`: after the final pass, from transcript v2.
//!
//! Both keep what the user wrote, pinned, edited or ticked off; the model is
//! told about those so it doesn't repeat them. Local model only here: a
//! cloud send needs the per-meeting preview and confirmation (phase 11 UI).

use std::sync::Arc;

use ghi_llm::enhance::{self, NoteLine};
use ghi_llm::notes::{self, Notes, Options};
use ghi_llm::template::{OutLang, Template};
use ghi_llm::{Llm, Transcript};
use ghi_store::store::{NewActionItem, NewNoteBlock, Provenance, ReplacedNotes, Segment, Store};

use crate::events::{Event, Stage};
use crate::jobs::{JobCtx, JobHandler, JobLease, Outcome};

pub const NOTES_FINAL_JOB: &str = "notes_final";

fn store_err(e: ghi_store::StoreError) -> String {
    e.to_string()
}

/// What the app shows for a speaker: the name, "Me", or "Speaker N".
pub fn speaker_label(s: &ghi_store::store::Speaker) -> String {
    match (&s.display_name, s.is_me, s.label_idx) {
        (Some(n), _, _) => n.clone(),
        (None, true, _) => "Me".into(),
        (None, false, i) if i >= 0 => format!("Speaker {}", i + 1),
        _ => "Speaker".into(),
    }
}

/// The meeting's transcript for the notes engine, speakers named as shown
/// (never by gid), and the stored segments in the same order.
pub fn stored_transcript(
    store: &Store,
    meeting: &str,
) -> Result<(Transcript, Vec<Segment>), String> {
    let segs = store.segments(meeting).map_err(store_err)?;
    let t = Transcript::new(
        segs.iter()
            .enumerate()
            .map(|(i, s)| ghi_llm::Segment {
                id: i as u64,
                t0_ms: s.t0_ms,
                t1_ms: s.t1_ms,
                speaker: s.speaker_gid.clone(),
                text: s.text.clone(),
                lang: s.lang.clone(),
            })
            .collect(),
    )
    .map_err(|e| e.to_string())?;
    let names = store
        .speakers(meeting)
        .map_err(store_err)?
        .into_iter()
        .map(|sp| {
            let label = speaker_label(&sp);
            (sp.gid, label)
        })
        .collect();
    Ok((t.with_speaker_names(names), segs))
}

/// Texts the user owns (written, pinned, edited, done), for `Options::pinned`.
pub fn kept_texts(store: &Store, meeting: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = store
        .note_blocks(meeting)
        .map_err(store_err)?
        .into_iter()
        .filter(|b| b.pinned || b.provenance != Provenance::Ai)
        .map(|b| b.body)
        .collect();
    out.extend(
        store
            .action_items(meeting)
            .map_err(store_err)?
            .into_iter()
            .filter(|a| a.done || a.provenance != Provenance::Ai)
            .map(|a| a.text),
    );
    Ok(out)
}

/// Kind of the AI lines that expand one of the user's own notes
/// (`enhanced:<user block gid>`). An empty body says the transcript has
/// nothing for that note ("not found").
pub const ENHANCED_PREFIX: &str = "enhanced:";

/// Time anchors for segment indexes (citations).
fn anchors_for(
    store: &Store,
    meeting: &str,
    segs: &[Segment],
    ids: &[u64],
) -> Result<Vec<ghi_store::anchors::Anchor>, String> {
    ids.iter()
        .filter_map(|&i| segs.get(i as usize))
        .map(|s| store.anchor_for_segment(meeting, s).map_err(store_err))
        .collect()
}

/// Replaces the meeting's AI notes with `n`; citations become time anchors.
pub fn save_notes(
    store: &Store,
    meeting: &str,
    n: &Notes,
    segs: &[Segment],
    model: &str,
) -> Result<ReplacedNotes, String> {
    save_notes_with(store, meeting, n, segs, Vec::new(), model)
}

/// The user's own notes, expanded from the transcript with citations (doc 02
/// §D): AI blocks of kind `enhanced:<gid>` to add with the notes.
pub fn enhance_user_notes(
    store: &Store,
    meeting: &str,
    llm: &mut dyn Llm,
    t: &Transcript,
    segs: &[Segment],
    lang: OutLang,
) -> Result<Vec<NewNoteBlock>, String> {
    let all = store.note_blocks(meeting).map_err(store_err)?;
    // A note whose expansion the user edited keeps that one.
    let kept = |gid: &str| {
        let kind = format!("{ENHANCED_PREFIX}{gid}");
        all.iter()
            .any(|b| b.kind == kind && b.provenance != Provenance::Ai)
    };
    let users: Vec<_> = all
        .iter()
        .filter(|b| b.provenance == Provenance::User && !b.body.trim().is_empty())
        .filter(|b| !kept(&b.gid))
        .collect();
    if users.is_empty() || t.is_empty() {
        return Ok(Vec::new());
    }
    let lines: Vec<NoteLine> = users
        .iter()
        .map(|b| NoteLine {
            text: b.body.clone(),
            t_ms: b.anchors.first().map(|a| a.t0_ms),
        })
        .collect();
    let run = enhance::enhance(llm, t, &lines, lang).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (b, e) in users.iter().zip(run.lines) {
        let kind = format!("{ENHANCED_PREFIX}{}", b.gid);
        if e.not_found || e.points.is_empty() {
            out.push(NewNoteBlock {
                kind,
                provenance: Provenance::Ai,
                body: String::new(),
                anchors: Vec::new(),
                pinned: false,
            });
            continue;
        }
        for p in e.points {
            out.push(NewNoteBlock {
                kind: kind.clone(),
                provenance: Provenance::Ai,
                body: p.text,
                anchors: anchors_for(store, meeting, segs, &p.citations)?,
                pinned: false,
            });
        }
    }
    Ok(out)
}

/// The current AI expansions of the user's notes, to keep when enhancing
/// fails (a regenerate replaces every AI block).
pub fn previous_enhanced(store: &Store, meeting: &str) -> Result<Vec<NewNoteBlock>, String> {
    Ok(store
        .note_blocks(meeting)
        .map_err(store_err)?
        .into_iter()
        .filter(|b| b.provenance == Provenance::Ai && b.kind.starts_with(ENHANCED_PREFIX))
        .map(|b| NewNoteBlock {
            kind: b.kind,
            provenance: Provenance::Ai,
            body: b.body,
            anchors: b.anchors,
            pinned: false,
        })
        .collect())
}

/// [`save_notes`] plus `extra` AI blocks (the enhanced user notes). `model`
/// is recorded as the one that wrote the notes (the detail's footer).
pub fn save_notes_with(
    store: &Store,
    meeting: &str,
    n: &Notes,
    segs: &[Segment],
    extra: Vec<NewNoteBlock>,
    model: &str,
) -> Result<ReplacedNotes, String> {
    save_notes_leased(store, meeting, n, segs, extra, model, None).map_err(|e| match e {
        SaveError::Fenced => store_err(ghi_store::StoreError::Fenced),
        SaveError::Other(e) => e,
    })
}

/// Why [`save_notes_leased`] saved nothing.
pub(crate) enum SaveError {
    /// The lease is no longer granted.
    Fenced,
    Other(String),
}

impl From<String> for SaveError {
    fn from(e: String) -> Self {
        SaveError::Other(e)
    }
}

/// [`save_notes_with`] under a lease (`job_uuid`, epoch): the commit also
/// closes the lease atomically.
pub(crate) fn save_notes_leased(
    store: &Store,
    meeting: &str,
    n: &Notes,
    segs: &[Segment],
    extra: Vec<NewNoteBlock>,
    model: &str,
    lease: Option<&JobLease>,
) -> Result<ReplacedNotes, SaveError> {
    let anchors = |ids: &[u64]| anchors_for(store, meeting, segs, ids);
    let mut blocks = Vec::new();
    let mut block = |kind: &str, text: &str, ids: &[u64]| -> Result<(), String> {
        blocks.push(NewNoteBlock {
            kind: kind.to_string(),
            provenance: Provenance::Ai,
            body: text.to_string(),
            anchors: anchors(ids)?,
            pinned: false,
        });
        Ok(())
    };
    for i in &n.tldr {
        block("tldr", &i.text, &i.citations)?;
    }
    for i in &n.decisions {
        block("decision", &i.text, &i.citations)?;
    }
    for i in &n.proposals {
        block("proposal", &i.text, &i.citations)?;
    }
    for i in &n.open_questions {
        block("question", &i.text, &i.citations)?;
    }
    for q in &n.key_quotes {
        block("quote", &q.text, &q.citations)?;
    }
    for tp in &n.topics {
        block("topic", &tp.title, &tp.citations)?;
    }
    for sec in &n.sections {
        for i in &sec.items {
            block(&format!("section:{}", sec.id), &i.text, &i.citations)?;
        }
    }
    blocks.extend(extra);
    let mut actions = Vec::new();
    for a in &n.action_items {
        let all = anchors(&a.citations)?;
        actions.push(NewActionItem {
            text: a.text.clone(),
            owner_speaker_gid: a.owner.clone(),
            due_text: a.due.clone(),
            anchor: all.first().cloned(),
            anchors: all,
            provenance: Provenance::Ai,
            ..Default::default()
        });
    }
    let saved = match lease {
        // The final pass under this lease already closed it (`granted ->
        // done`, in its own commit): the notes that follow carry the epoch
        // but have no state change left to make. A lease that is revoked or
        // expired is not `done`, so it still fences the commit.
        Some(l) if lease_is_done(store, &l.job_uuid) => {
            store.replace_ai_notes_epoch(meeting, blocks, actions, l.epoch, None)
        }
        Some(l) => {
            store.replace_ai_notes_epoch(meeting, blocks, actions, l.epoch, Some(&l.job_uuid))
        }
        None => store.replace_ai_notes(meeting, blocks, actions),
    }
    .map_err(|e| match e {
        ghi_store::StoreError::Fenced => SaveError::Fenced,
        e => SaveError::Other(store_err(e)),
    })?;
    // Not worth failing the save over.
    let _ = store.set_notes_model(meeting, Some(model));
    Ok(saved)
}

/// Whether the lease was closed by a result commit (see [`save_notes_leased`]).
fn lease_is_done(store: &Store, job_uuid: &str) -> bool {
    store
        .lease_state(job_uuid)
        .ok()
        .flatten()
        .is_some_and(|l| l.state == "done")
}

/// Opens the local model for a transcript of `transcript_bytes`.
pub type LlmFactory = Arc<dyn Fn(usize) -> Result<Box<dyn Llm + Send>, String> + Send + Sync>;

/// The template for notes: the one asked for, else the meeting's own, else the
/// one its calendar event suggests, else `default`. `resolve` turns an id
/// into a template (a built-in, or one of the user's own as `user:<gid>`); an
/// id it does not know (a template since deleted, or made on another device)
/// is passed over.
fn choose_template(
    asked: Option<String>,
    meeting: Option<String>,
    suggested: Option<String>,
    default: &Template,
    resolve: &dyn Fn(&str) -> Option<Template>,
) -> Template {
    [asked, meeting, suggested]
        .into_iter()
        .flatten()
        .find_map(|id| resolve(&id))
        .unwrap_or_else(|| default.clone())
}

/// The app setting `notesLanguage` (`meeting`, `en`, `vi`); `meeting` (the
/// transcript's own language) when unset or unreadable.
fn default_notes_language(store: &Store) -> String {
    store
        .get_setting("app")
        .ok()
        .flatten()
        .and_then(|v| {
            v.get("notesLanguage")
                .and_then(|l| l.as_str().map(str::to_string))
        })
        .filter(|l| matches!(l.as_str(), "meeting" | "en" | "vi"))
        .unwrap_or_else(|| "meeting".into())
}

/// `notes_live` / `notes_final`.
pub struct NotesJob {
    pub kind: &'static str,
    /// 1 = from the live transcript, 2 = after the final pass.
    pub version: u32,
    pub template: Template,
    pub llm: LlmFactory,
    /// The local model is installed (else the job waits for it).
    pub ready: crate::jobs::Ready,
}

impl JobHandler for NotesJob {
    fn kind(&self) -> &'static str {
        self.kind
    }

    fn ready(&self) -> bool {
        (self.ready)()
    }

    /// The transcript is there without (new) notes: the meeting is usable.
    fn failed(&self, ctx: &JobCtx) {
        if self.version >= 2
            && let Ok(m) = ctx.meeting()
        {
            let _ = ctx.store.set_meeting_status(m, "ready");
        }
    }

    fn run(&self, ctx: &JobCtx) -> Result<Outcome, String> {
        if ctx.preempted() {
            return Ok(Outcome::Yield(ctx.job.payload.clone()));
        }
        let meeting = ctx.meeting()?;
        // Notes from the live transcript are moot once the final pass made v2
        // or is about to (its notes come next): one LLM run, not two.
        if self.version == 1 {
            let v2 = ctx
                .store
                .get_meeting(meeting)
                .map_err(store_err)?
                .transcript_version
                >= 2;
            let pending = |kind| ctx.store.active_job(meeting, kind).map_err(store_err);
            if v2 || pending(NOTES_FINAL_JOB)?.is_some() {
                return Ok(Outcome::Done);
            }
        }
        ctx.progress(Some(Stage::WritingNotes), 0.0);
        let (t, segs) = stored_transcript(ctx.store, meeting)?;
        if t.is_empty() && self.version == 1 {
            // Recorded without a live transcript: nothing to write yet.
            return Ok(Outcome::Done);
        }
        if !t.is_empty() {
            // A regenerate may pick the template and language (payload); else
            // the meeting's template, else one its calendar event suggests,
            // else the default.
            let payload = &ctx.job.payload;
            let suggested = crate::calendar::info(ctx.store, meeting).and_then(|i| {
                crate::calendar::suggest_template(&i.title, &i.attendees).map(str::to_string)
            });
            let template = choose_template(
                payload
                    .get("template")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                ctx.store.get_meeting(meeting).map_err(store_err)?.template,
                suggested,
                &self.template,
                &|id| crate::user_templates::template_of(ctx.store, id),
            );
            let setting = default_notes_language(ctx.store);
            let lang = payload
                .get("lang")
                .and_then(|v| v.as_str())
                .unwrap_or(&setting);
            let mut opts = Options::new(
                template,
                OutLang::resolve(lang, &t)
                    .or_else(|_| OutLang::resolve("meeting", &t))
                    .unwrap_or(OutLang::En),
            );
            opts.pinned = kept_texts(ctx.store, meeting)?;
            // What the user marked while recording steers the local model; a
            // cloud send never gets it (`cloud::plan` builds its own options).
            opts.marks = crate::marks::load_hints(ctx.store, meeting, &segs);
            // Terms the transcript says, in the user's spelling (their vocabulary,
            // attendees, enabled glossary packs): local prompts only.
            opts.spellings = crate::vocab::spellings_for_prompt(
                ctx.store,
                meeting,
                &segs,
                ghi_llm::notes::MAX_SPELLINGS,
            );
            // Everything in the prompt sizes the model's context: the
            // transcript, the texts the user keeps (saved answers can be
            // ~10 x 1.5k characters on a short meeting) and the template's own
            // words (a user's can be 8 sections of 200 characters).
            let bytes: usize = segs.iter().map(|s| s.text.len()).sum::<usize>()
                + opts.pinned.iter().map(String::len).sum::<usize>()
                + opts.template_bytes()
                + opts.spellings().iter().map(String::len).sum::<usize>();
            let mut llm = (self.llm)(bytes)?;
            // The model's own progress, as the meeting's: reading the
            // transcript to 40 %, writing to 80 % (about 1,800 tokens
            // expected), then the user's notes.
            let shown = Arc::new(std::sync::atomic::AtomicU32::new(0));
            {
                let shown = shown.clone();
                let est_in = (bytes as f64 / 3.0).max(1.0);
                llm.set_progress(Box::new(move |read, written| {
                    let p = if written == 0 {
                        0.05 + 0.35 * (f64::from(read) / est_in).min(1.0)
                    } else {
                        0.4 + 0.4 * (f64::from(written) / 1_800.0).min(1.0)
                    };
                    shown.fetch_max((p * 1000.0) as u32, std::sync::atomic::Ordering::Relaxed);
                }));
            }
            // A model that can be stopped mid-answer (an in-process engine)
            // is stopped as soon as a recording starts or the app leaves the
            // screen: the run then yields and starts over later.
            let stop = llm.stopper();
            let stop_requested = ctx.stop_signal();
            let finished = std::sync::atomic::AtomicBool::new(false);
            // Ends the watcher however the run ends (a panic included), so the
            // scope below never waits on it forever.
            struct Finished<'a>(&'a std::sync::atomic::AtomicBool);
            impl Drop for Finished<'_> {
                fn drop(&mut self) {
                    self.0.store(true, std::sync::atomic::Ordering::Relaxed);
                }
            }
            let written = std::thread::scope(|scope| {
                scope.spawn(|| {
                    let mut reported = 0;
                    let mut stopped = false;
                    while !finished.load(std::sync::atomic::Ordering::Relaxed) {
                        if !stopped
                            && let Some(stop) = &stop
                            && stop_requested()
                        {
                            stop();
                            stopped = true;
                        }
                        let now = shown.load(std::sync::atomic::Ordering::Relaxed);
                        if now > reported + 9 {
                            reported = now;
                            ctx.progress(Some(Stage::WritingNotes), now as f32 / 1000.0);
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                });
                let _finished = Finished(&finished);
                (|| -> Result<_, String> {
                    let run =
                        notes::generate(llm.as_mut(), &t, &opts).map_err(|e| e.to_string())?;
                    // The final notes also expand the user's own lines; a failure
                    // there leaves them as typed (the notes still count).
                    let extra = if self.version >= 2 {
                        ctx.progress(Some(Stage::WritingNotes), 0.8);
                        enhance_user_notes(ctx.store, meeting, llm.as_mut(), &t, &segs, opts.lang)
                            .or_else(|_| previous_enhanced(ctx.store, meeting))?
                    } else {
                        Vec::new()
                    };
                    Ok((run, extra))
                })()
            });
            let (run, extra) = match written {
                Ok(w) => w,
                // Stopped (or failed) while asked to stop: try again later.
                Err(_) if stop.is_some() && stop_requested() => {
                    return Ok(Outcome::Yield(ctx.job.payload.clone()));
                }
                Err(e) => return Err(e),
            };
            let model = llm.engine().name;
            drop(llm); // frees the model before anything else loads
            // The last look before the commit: a recording or a lost lease
            // ends the run here (the lease's own margin included).
            if ctx.preempted() {
                return Ok(Outcome::Yield(ctx.job.payload.clone()));
            }
            if !ctx.may_commit() {
                return Ok(ctx.abandon_fenced());
            }
            match save_notes_leased(
                ctx.store,
                meeting,
                &run.notes,
                &segs,
                extra,
                &model,
                ctx.lease().as_ref(),
            ) {
                Ok(_) => {}
                Err(SaveError::Fenced) => return Ok(ctx.abandon_fenced()),
                Err(SaveError::Other(e)) => return Err(e),
            }
        }
        if self.version >= 2 {
            ctx.store
                .set_meeting_status(meeting, "ready")
                .map_err(store_err)?;
            // The transcript is final: queue its semantic index. It waits for
            // the embedding model; a failure here costs nothing (the app
            // queues what is missing at startup).
            let _ = crate::index_job::queue_one(ctx.store, meeting);
        }
        ctx.events.emit(Event::NotesReady {
            meeting: meeting.to_string(),
            version: self.version,
        });
        Ok(Outcome::Done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_notes_records_the_model_that_wrote_them() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(
            tmp.path(),
            std::sync::Arc::new(ghi_store::keys::MemoryKeyStore::default()),
            ghi_store::keys::Protection::default(),
        )
        .unwrap();
        let m = store
            .create_meeting(ghi_store::store::NewMeeting::default())
            .unwrap()
            .gid;
        let item = |t: &str| ghi_llm::notes::Item {
            text: t.into(),
            citations: vec![0],
        };
        let notes = Notes {
            template: "general".into(),
            lang: "vi".into(),
            tldr: vec![],
            decisions: vec![item("Ship Friday")],
            proposals: vec![item("Maybe dark mode")],
            action_items: vec![],
            open_questions: vec![],
            key_quotes: vec![],
            topics: vec![],
            sections: vec![],
        };
        // The CLI, the job and a cloud send all go through this.
        store
            .add_segments(
                &m,
                vec![ghi_store::store::NewSegment {
                    t0_ms: 0,
                    t1_ms: 1000,
                    text: "x".into(),
                    ..Default::default()
                }],
            )
            .unwrap();
        let segs = store.segments(&m).unwrap();
        save_notes(&store, &m, &notes, &segs, "qwen3-4b").unwrap();
        assert_eq!(store.notes_model(&m).unwrap().as_deref(), Some("qwen3-4b"));
        // A proposed decision is its own kind of block, apart from decisions.
        let blocks = store.note_blocks(&m).unwrap();
        let kinds: Vec<_> = blocks
            .iter()
            .map(|b| (b.kind.as_str(), b.body.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [("decision", "Ship Friday"), ("proposal", "Maybe dark mode")]
        );
        save_notes_with(&store, &m, &notes, &segs, Vec::new(), "gpt-4.1-mini").unwrap();
        assert_eq!(
            store.notes_model(&m).unwrap().as_deref(),
            Some("gpt-4.1-mini")
        );
    }

    #[test]
    fn template_order_is_asked_then_meeting_then_calendar_then_default() {
        let d = ghi_llm::template::builtin("general").unwrap();
        let builtin = |id: &str| ghi_llm::template::builtin(id).ok();
        let id = |a: Option<&str>, m: Option<&str>, c: Option<&str>| {
            choose_template(
                a.map(str::to_string),
                m.map(str::to_string),
                c.map(str::to_string),
                &d,
                &builtin,
            )
            .id
        };
        assert_eq!(id(Some("sales"), Some("client"), Some("standup")), "sales");
        assert_eq!(id(None, Some("client"), Some("standup")), "client");
        assert_eq!(id(None, None, Some("standup")), "standup");
        assert_eq!(id(None, None, None), "general");
        // An unknown id is passed over, not an error.
        assert_eq!(id(Some("nope"), None, Some("interview")), "interview");
        assert_eq!(id(Some("nope"), Some("also-nope"), None), "general");
    }
}
