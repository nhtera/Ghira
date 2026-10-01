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

use ghi_llm::notes::{self, Notes, Options};
use ghi_llm::template::{OutLang, Template};
use ghi_llm::{Llm, Transcript};
use ghi_store::store::{NewActionItem, NewNoteBlock, Provenance, ReplacedNotes, Segment, Store};

use crate::events::{Event, Stage};
use crate::jobs::{JobCtx, JobHandler, Outcome};

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

/// Replaces the meeting's AI notes with `n`; citations become time anchors.
pub fn save_notes(
    store: &Store,
    meeting: &str,
    n: &Notes,
    segs: &[Segment],
) -> Result<ReplacedNotes, String> {
    let anchors = |ids: &[u64]| -> Result<Vec<ghi_store::anchors::Anchor>, String> {
        ids.iter()
            .filter_map(|&i| segs.get(i as usize))
            .map(|s| store.anchor_for_segment(meeting, s).map_err(store_err))
            .collect()
    };
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
    store
        .replace_ai_notes(meeting, blocks, actions)
        .map_err(store_err)
}

/// Opens the local model for a transcript of `transcript_bytes`.
pub type LlmFactory = Arc<dyn Fn(usize) -> Result<Box<dyn Llm + Send>, String> + Send + Sync>;

/// `notes_live` / `notes_final`.
pub struct NotesJob {
    pub kind: &'static str,
    /// 1 = from the live transcript, 2 = after the final pass.
    pub version: u32,
    pub template: Template,
    pub llm: LlmFactory,
}

impl JobHandler for NotesJob {
    fn kind(&self) -> &'static str {
        self.kind
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
        ctx.progress(Some(Stage::WritingNotes), 0.0);
        let (t, segs) = stored_transcript(ctx.store, meeting)?;
        if !t.is_empty() {
            let mut opts = Options::new(
                self.template.clone(),
                OutLang::resolve("meeting", &t).unwrap_or(OutLang::En),
            );
            opts.pinned = kept_texts(ctx.store, meeting)?;
            let bytes: usize = segs.iter().map(|s| s.text.len()).sum();
            let mut llm = (self.llm)(bytes)?;
            let run = notes::generate(llm.as_mut(), &t, &opts).map_err(|e| e.to_string())?;
            drop(llm); // frees the model before anything else loads
            save_notes(ctx.store, meeting, &run.notes, &segs)?;
        }
        if self.version >= 2 {
            ctx.store
                .set_meeting_status(meeting, "ready")
                .map_err(store_err)?;
        }
        ctx.events.emit(Event::NotesReady {
            meeting: meeting.to_string(),
            version: self.version,
        });
        Ok(Outcome::Done)
    }
}
