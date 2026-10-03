// SPDX-License-Identifier: Apache-2.0
//! Meeting detail (D6): the header, the notes (blocks with resolved
//! citations, action items), the transcript (segments with word timings,
//! marks, topics), their edits, Regenerate, the templates, and search (D3).
//!
//! Every edit names its meeting and is refused for an item of another
//! meeting. Text is returned as text; the webview renders text nodes only
//! [RT-6]. Edits of AI-written blocks and items flip them to `ai_edited` in
//! the store, so a regenerate keeps them.

use ghi_store::anchors::Anchor;
use ghi_store::search::{HitKind, SearchFilter, SearchQuery};
use ghi_store::store::{Item, NewActionItem, NewNoteBlock, Provenance, Segment, Store};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::library::{MeetingJob, active_jobs};
use crate::speakers_cmd::{MeetingSpeaker, speakers_with};
use crate::{CoreState, blocking};

fn err(e: ghi_store::StoreError) -> String {
    e.to_string()
}

/// Text the user typed: a sane upper bound.
const MAX_TEXT_CHARS: usize = 10_000;
/// Characters of transcript shown as a citation's quote.
pub const QUOTE_CHARS: usize = 280;

fn cap(text: String) -> String {
    if text.chars().count() > MAX_TEXT_CHARS {
        text.chars().take(MAX_TEXT_CHARS).collect()
    } else {
        text
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    User,
    Ai,
    /// AI-written, then changed by the user (kept by a regenerate).
    AiEdited,
}

impl From<Provenance> for Origin {
    fn from(p: Provenance) -> Origin {
        match p {
            Provenance::User => Origin::User,
            Provenance::Ai => Origin::Ai,
            Provenance::AiEdited => Origin::AiEdited,
        }
    }
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetail {
    pub gid: String,
    pub title: String,
    /// Unix ms.
    pub started_at: f64,
    pub duration_ms: f64,
    /// `live`, `import`, …
    pub source: String,
    /// `call` or `room`.
    pub mode: String,
    /// The transcript's language (`en`, `vi`, `mixed`), if known.
    pub language: Option<String>,
    /// Notes template id (`None`: the default, `general`).
    pub template: Option<String>,
    pub status: String,
    pub cloud_locked: bool,
    pub sensitive: bool,
    pub cloud_used: bool,
    pub consent_confirmed: bool,
    pub transcript_version: f64,
    /// Some audio is kept (retention may have removed it).
    pub audio_available: bool,
    pub speakers: Vec<MeetingSpeaker>,
    pub job: Option<MeetingJob>,
}

/// A citation resolved against the current transcript.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub t0_ms: f64,
    pub t1_ms: f64,
    /// The cited words (overlapping segments, shortened).
    pub quote: String,
    pub speaker_gid: Option<String>,
    /// Made against an older transcript (the text was found by time).
    pub stale: bool,
    /// Nothing in the transcript at that time ("not found").
    pub missing: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NoteBlockView {
    pub gid: String,
    /// `tldr`, `decision`, `question`, `quote`, `topic`, `section:<id>`,
    /// `enhanced:<user block gid>` (the AI's expansion of a user note; an
    /// empty text means "not found"), or the user's `note` / `decision` /
    /// `action` / `question`.
    pub kind: String,
    pub origin: Origin,
    pub text: String,
    pub pinned: bool,
    pub citations: Vec<Citation>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActionItemView {
    pub gid: String,
    pub text: String,
    pub owner_speaker_gid: Option<String>,
    /// The due date as spoken ("thứ Sáu").
    pub due_text: Option<String>,
    pub done: bool,
    pub origin: Origin,
    pub citations: Vec<Citation>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TemplateSection {
    pub id: String,
    pub title_en: String,
    pub title_vi: String,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInfo {
    pub id: String,
    pub name: String,
    pub sections: Vec<TemplateSection>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MeetingNotes {
    pub blocks: Vec<NoteBlockView>,
    pub action_items: Vec<ActionItemView>,
    /// The meeting template's own sections (for `section:<id>` blocks).
    pub sections: Vec<TemplateSection>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WordTiming {
    pub t0_ms: f64,
    pub t1_ms: f64,
    /// 0..1, if the engine gave one.
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SegmentView {
    pub gid: String,
    pub speaker_gid: Option<String>,
    pub t0_ms: f64,
    pub t1_ms: f64,
    pub text: String,
    pub language: Option<String>,
    pub confidence: Option<f64>,
    /// The user changed the text ("Edited").
    pub edited: bool,
    /// Another speaker talked over this line: some words may be wrong.
    pub overlap: bool,
    /// One per space-separated word of `text`, in order (empty when the
    /// counts don't match, e.g. after an edit).
    pub words: Vec<WordTiming>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MarkView {
    pub t_ms: f64,
    /// `mark`, `decision`, `action`, `question`.
    pub tag: String,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TopicView {
    pub title: String,
    pub t_ms: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MeetingTranscript {
    pub version: f64,
    pub segments: Vec<SegmentView>,
    pub marks: Vec<MarkView>,
    /// Topic headers (from the notes), in time order.
    pub topics: Vec<TopicView>,
}

pub fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max).collect();
    if let Some(i) = s.rfind(' ').filter(|&i| i > max / 2) {
        s.truncate(i);
    }
    s.push('…');
    s
}

/// Resolves an anchor against the current segments (in time order).
fn citation(a: &Anchor, segs: &[Segment], version: i64) -> Citation {
    let hit: Vec<&Segment> = if a.t1_ms <= a.t0_ms {
        segs.iter()
            .filter(|s| s.t0_ms <= a.t0_ms && s.t1_ms > a.t0_ms)
            .collect()
    } else {
        segs.iter()
            .filter(|s| s.t0_ms < a.t1_ms && s.t1_ms > a.t0_ms)
            .collect()
    };
    let text = hit
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    Citation {
        t0_ms: a.t0_ms as f64,
        t1_ms: a.t1_ms.max(a.t0_ms) as f64,
        quote: shorten(&text, QUOTE_CHARS),
        speaker_gid: hit.first().and_then(|s| s.speaker_gid.clone()),
        stale: a.transcript_version != version,
        missing: hit.is_empty(),
    }
}

/// A citation of one whole segment (an answer's source line).
pub fn segment_citation(s: &Segment, version: i64) -> Citation {
    Citation {
        t0_ms: s.t0_ms as f64,
        t1_ms: s.t1_ms as f64,
        quote: shorten(&s.text, QUOTE_CHARS),
        speaker_gid: s.speaker_gid.clone(),
        stale: s.version != version,
        missing: false,
    }
}

fn sections_of(template: Option<&str>) -> Vec<TemplateSection> {
    ghi_llm::template::builtin(template.unwrap_or("general"))
        .map(|t| {
            t.sections
                .into_iter()
                .map(|s| TemplateSection {
                    id: s.id,
                    title_en: s.title_en,
                    title_vi: s.title_vi,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn notes_of(store: &Store, meeting: &str) -> Result<MeetingNotes, String> {
    let m = store.get_meeting(meeting).map_err(err)?;
    let segs = store.segments(meeting).map_err(err)?;
    let v = m.transcript_version;
    let blocks = store
        .note_blocks(meeting)
        .map_err(err)?
        .into_iter()
        .map(|b| NoteBlockView {
            citations: b.anchors.iter().map(|a| citation(a, &segs, v)).collect(),
            gid: b.gid,
            kind: b.kind,
            origin: b.provenance.into(),
            text: b.body,
            pinned: b.pinned,
        })
        .collect();
    let action_items = store
        .action_items(meeting)
        .map_err(err)?
        .into_iter()
        .map(|a| ActionItemView {
            citations: a.anchors.iter().map(|x| citation(x, &segs, v)).collect(),
            gid: a.gid,
            text: a.text,
            owner_speaker_gid: a.owner_speaker_gid,
            due_text: a.due_text,
            done: a.done,
            origin: a.provenance.into(),
        })
        .collect();
    Ok(MeetingNotes {
        blocks,
        action_items,
        sections: sections_of(m.template.as_deref()),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn meeting_detail(core: CoreState<'_>, meeting: String) -> Result<MeetingDetail, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let m = store.get_meeting(&meeting).map_err(err)?;
        let segs = store.segments(&meeting).map_err(err)?;
        let speakers = speakers_with(&store, &meeting, &segs)?;
        let job = active_jobs(c, &store)?.remove(&meeting);
        Ok(MeetingDetail {
            audio_available: store.audio_available(&meeting).map_err(err)?,
            speakers,
            job,
            gid: m.gid,
            title: m.title,
            started_at: m.started_at as f64,
            duration_ms: m.duration_ms as f64,
            source: m.source,
            mode: m.mode,
            language: m.lang,
            template: m.template,
            status: m.status,
            cloud_locked: m.cloud_locked,
            sensitive: m.sensitive,
            cloud_used: m.cloud_used,
            consent_confirmed: m.consent_confirmed,
            transcript_version: m.transcript_version as f64,
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn meeting_notes(core: CoreState<'_>, meeting: String) -> Result<MeetingNotes, String> {
    blocking(&core, move |c| notes_of(&*c.store()?, &meeting)).await
}

#[tauri::command]
#[specta::specta]
pub async fn meeting_transcript(
    core: CoreState<'_>,
    meeting: String,
) -> Result<MeetingTranscript, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let m = store.get_meeting(&meeting).map_err(err)?;
        let segs = store.segments(&meeting).map_err(err)?;
        let mut all_words = store.meeting_words(&meeting).map_err(err)?;
        let mut segments = Vec::with_capacity(segs.len());
        for s in segs {
            let words = all_words.remove(&s.gid).unwrap_or_default();
            let words = if words.len() == s.text.split_whitespace().count() {
                words
                    .iter()
                    .map(|w| WordTiming {
                        t0_ms: w.t0_ms as f64,
                        t1_ms: w.t1_ms as f64,
                        confidence: w.conf.map(f64::from),
                    })
                    .collect()
            } else {
                Vec::new()
            };
            segments.push(SegmentView {
                gid: s.gid,
                speaker_gid: s.speaker_gid,
                t0_ms: s.t0_ms as f64,
                t1_ms: s.t1_ms as f64,
                text: s.text,
                language: s.lang,
                confidence: s.confidence.map(f64::from),
                edited: s.edited,
                overlap: s.overlap,
                words,
            });
        }
        let marks = store
            .marks(&meeting)
            .map_err(err)?
            .into_iter()
            .map(|k| MarkView {
                t_ms: k.t_ms as f64,
                tag: k.tag.as_str().to_string(),
            })
            .collect();
        let mut topics: Vec<TopicView> = store
            .note_blocks(&meeting)
            .map_err(err)?
            .into_iter()
            .filter(|b| b.kind == "topic")
            .filter_map(|b| {
                let t = b.anchors.iter().map(|a| a.t0_ms).min()?;
                Some(TopicView {
                    title: b.body,
                    t_ms: t as f64,
                })
            })
            .collect();
        topics.sort_by(|a, b| a.t_ms.total_cmp(&b.t_ms));
        Ok(MeetingTranscript {
            version: m.transcript_version as f64,
            segments,
            marks,
            topics,
        })
    })
    .await
}

// ----------------------------------------------------------------- edits

/// The item belongs to `meeting` (one indexed lookup, nothing decrypted).
fn owns(store: &Store, meeting: &str, item: Item, gid: &str) -> Result<(), String> {
    match store.meeting_of(item, gid) {
        Ok(m) if m == meeting => Ok(()),
        _ => Err(match item {
            Item::Segment => "not a line of this meeting",
            Item::NoteBlock => "not a note of this meeting",
            Item::ActionItem => "not an action item of this meeting",
            Item::Speaker => "not a speaker of this meeting",
        }
        .into()),
    }
}

fn own_segment(store: &Store, meeting: &str, segment: &str) -> Result<(), String> {
    owns(store, meeting, Item::Segment, segment)
}

fn own_block(store: &Store, meeting: &str, block: &str) -> Result<(), String> {
    owns(store, meeting, Item::NoteBlock, block)
}

fn own_action(store: &Store, meeting: &str, item: &str) -> Result<(), String> {
    owns(store, meeting, Item::ActionItem, item)
}

fn own_speaker(store: &Store, meeting: &str, speaker: &str) -> Result<(), String> {
    owns(store, meeting, Item::Speaker, speaker)?;
    let merged = store
        .speakers(meeting)
        .map_err(err)?
        .iter()
        .any(|s| s.gid == speaker && s.merged_into.is_some());
    if merged {
        Err("not a speaker of this meeting".into())
    } else {
        Ok(())
    }
}

/// Corrects a transcript line ("Edited"; the final pass keeps it).
#[tauri::command]
#[specta::specta]
pub async fn update_segment_text(
    core: CoreState<'_>,
    meeting: String,
    segment: String,
    text: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        own_segment(&store, &meeting, &segment)?;
        let text = cap(text);
        if text.trim().is_empty() {
            return Err("a line can't be empty".into());
        }
        store.update_segment_text(&segment, &text).map_err(err)
    })
    .await
}

/// Moves a transcript line to another speaker of the meeting.
#[tauri::command]
#[specta::specta]
pub async fn set_segment_speaker(
    core: CoreState<'_>,
    meeting: String,
    segment: String,
    speaker: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        own_segment(&store, &meeting, &segment)?;
        own_speaker(&store, &meeting, &speaker)?;
        store
            .set_segment_speaker(&segment, Some(&speaker))
            .map_err(err)
    })
    .await
}

/// Edits a note block (an AI block becomes the user's: `aiEdited`).
#[tauri::command]
#[specta::specta]
pub async fn update_note_block(
    core: CoreState<'_>,
    meeting: String,
    block: String,
    text: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        own_block(&store, &meeting, &block)?;
        store.update_note_block(&block, &cap(text)).map_err(err)
    })
    .await
}

/// Adds one of the user's own notes ("My notes").
#[tauri::command]
#[specta::specta]
pub async fn add_note_block(
    core: CoreState<'_>,
    meeting: String,
    text: String,
) -> Result<NoteBlockView, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let b = store
            .add_note_block(
                &meeting,
                NewNoteBlock {
                    kind: "note".into(),
                    provenance: Provenance::User,
                    body: cap(text),
                    anchors: Vec::new(),
                    pinned: false,
                },
            )
            .map_err(err)?;
        Ok(NoteBlockView {
            gid: b.gid,
            kind: b.kind,
            origin: b.provenance.into(),
            text: b.body,
            pinned: b.pinned,
            citations: Vec::new(),
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_note_block(
    core: CoreState<'_>,
    meeting: String,
    block: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        own_block(&store, &meeting, &block)?;
        // The AI's expansion of the user's note goes with it (one transaction).
        let enhanced = format!("{}{block}", ghi_core::notes_job::ENHANCED_PREFIX);
        let mut gone: Vec<String> = store
            .note_blocks(&meeting)
            .map_err(err)?
            .into_iter()
            .filter(|b| b.kind == enhanced)
            .map(|b| b.gid)
            .collect();
        gone.push(block);
        store.delete_note_blocks(&gone).map_err(err)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn add_action_item(
    core: CoreState<'_>,
    meeting: String,
    text: String,
    owner: Option<String>,
) -> Result<ActionItemView, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        if let Some(o) = &owner {
            own_speaker(&store, &meeting, o)?;
        }
        let a = store
            .add_action_item(
                &meeting,
                NewActionItem {
                    text: cap(text),
                    owner_speaker_gid: owner,
                    provenance: Provenance::User,
                    ..Default::default()
                },
            )
            .map_err(err)?;
        Ok(ActionItemView {
            gid: a.gid,
            text: a.text,
            owner_speaker_gid: a.owner_speaker_gid,
            due_text: a.due_text,
            done: a.done,
            origin: a.provenance.into(),
            citations: Vec::new(),
        })
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn update_action_item(
    core: CoreState<'_>,
    meeting: String,
    item: String,
    text: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        own_action(&store, &meeting, &item)?;
        store
            .update_action_item_text(&item, &cap(text))
            .map_err(err)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn set_action_done(
    core: CoreState<'_>,
    meeting: String,
    item: String,
    done: bool,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        own_action(&store, &meeting, &item)?;
        store.set_action_done(&item, done).map_err(err)
    })
    .await
}

/// Sets (or clears) who owns an action item: one of the meeting's speakers.
#[tauri::command]
#[specta::specta]
pub async fn set_action_owner(
    core: CoreState<'_>,
    meeting: String,
    item: String,
    owner: Option<String>,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        own_action(&store, &meeting, &item)?;
        if let Some(o) = &owner {
            own_speaker(&store, &meeting, o)?;
        }
        store.set_action_owner(&item, owner.as_deref()).map_err(err)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_action_item(
    core: CoreState<'_>,
    meeting: String,
    item: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        own_action(&store, &meeting, &item)?;
        store.delete_action_item(&item).map_err(err)
    })
    .await
}

// ------------------------------------------------------------ regenerate

/// The language notes are written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum NotesLanguage {
    /// The meeting's own (dominant) language.
    Meeting,
    En,
    Vi,
}

/// The built-in notes templates, in menu order.
#[tauri::command]
#[specta::specta]
pub fn list_templates() -> Vec<TemplateInfo> {
    ghi_llm::template::builtin_ids()
        .filter_map(|id| ghi_llm::template::builtin(id).ok())
        .map(|t| TemplateInfo {
            sections: sections_of(Some(&t.id)),
            id: t.id,
            name: t.name,
        })
        .collect()
}

/// Rewrites the AI notes (template and language as chosen); what the user
/// wrote, edited, pinned or ticked off stays [RT-7]. Returns whether it waits
/// for the local model to be installed.
#[tauri::command]
#[specta::specta]
pub async fn regenerate_notes(
    core: CoreState<'_>,
    meeting: String,
    template: Option<String>,
    language: NotesLanguage,
) -> Result<bool, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let m = store.get_meeting(&meeting).map_err(err)?;
        match m.status.as_str() {
            "recording" => return Err("the meeting is still recording".into()),
            ghi_core::import::IMPORTING => return Err("the file is still being imported".into()),
            _ => {}
        }
        if let Some(t) = &template {
            ghi_llm::template::builtin(t).map_err(|e| e.to_string())?;
        }
        let kinds = [
            ghi_core::session::NOTES_LIVE_JOB,
            ghi_core::session::FINAL_PASS_JOB,
            ghi_core::notes_job::NOTES_FINAL_JOB,
        ];
        for k in kinds {
            if store.active_job(&meeting, k).map_err(err)?.is_some() {
                return Err(if k == ghi_core::session::FINAL_PASS_JOB
                    && !crate::core::speech_ready(&c.models())
                {
                    "the transcript waits for the speech models; the notes follow it"
                } else {
                    "the notes are already being written"
                }
                .into());
            }
        }
        if template.is_some() {
            store
                .set_meeting_template(&meeting, template.as_deref())
                .map_err(err)?;
        }
        let lang = match language {
            NotesLanguage::Meeting => "meeting",
            NotesLanguage::En => "en",
            NotesLanguage::Vi => "vi",
        };
        store
            .enqueue_job(
                Some(&meeting),
                ghi_core::notes_job::NOTES_FINAL_JOB,
                ghi_core::session::JOB_PAYLOAD_VERSION,
                &serde_json::json!({ "template": template, "lang": lang }),
            )
            .map_err(err)?;
        store
            .set_meeting_status(&meeting, "processing")
            .map_err(err)?;
        c.notify_jobs();
        Ok(!crate::core::llm_ready(&c.models()))
    })
    .await
}

// ---------------------------------------------------------------- search

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub text: String,
    /// `live` or `file`.
    pub source: Option<String>,
    pub template: Option<String>,
    /// Meeting start range, unix ms, inclusive.
    pub from_ms: Option<f64>,
    pub to_ms: Option<f64>,
    /// Only this meeting ("Find" inside a meeting uses the transcript instead).
    pub meeting: Option<String>,
    /// Only meetings in this folder (gid); `""` means meetings in no folder.
    #[serde(default)]
    #[specta(optional)]
    pub folder: Option<String>,
    /// Only meetings with any of these tags (gids; none: no restriction).
    #[serde(default)]
    #[specta(optional)]
    pub tags: Option<Vec<String>>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchHitView {
    /// `segment` or `note`.
    pub kind: String,
    pub meeting: String,
    pub meeting_title: String,
    pub meeting_started_at: f64,
    /// Segment or note block gid.
    pub item: String,
    pub speaker_gid: Option<String>,
    pub t0_ms: Option<f64>,
    pub t1_ms: Option<f64>,
    pub snippet: String,
    /// `[start, end)` in UTF-16 code units of `snippet` (JS string indexes).
    pub highlights: Vec<[u32; 2]>,
    /// The accented query matched exactly (ranked first).
    pub exact: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub hits: Vec<SearchHitView>,
    /// More matches exist than were considered.
    pub truncated: bool,
}

/// Char offsets into the full text → UTF-16 offsets into the snippet.
fn utf16_ranges(
    snippet: &str,
    snippet_start: usize,
    ranges: &[std::ops::Range<usize>],
) -> Vec<[u32; 2]> {
    // UTF-16 offset of each char boundary of the snippet.
    let mut at = Vec::with_capacity(snippet.len() + 1);
    let mut u = 0u32;
    for ch in snippet.chars() {
        at.push(u);
        u += ch.len_utf16() as u32;
    }
    at.push(u);
    let n = at.len() - 1;
    ranges
        .iter()
        .filter_map(|r| {
            let a = r.start.checked_sub(snippet_start)?;
            let b = r.end.saturating_sub(snippet_start).min(n);
            (a < b).then(|| [at[a], at[b]])
        })
        .collect()
}

/// Accent-insensitive search over transcripts and notes (VN-folded).
#[tauri::command]
#[specta::specta]
pub async fn search_meetings(
    core: CoreState<'_>,
    request: SearchRequest,
) -> Result<SearchResults, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let ms = |v: Option<f64>| v.filter(|x| x.is_finite()).map(|x| x as i64);
        let q = SearchQuery {
            text: request.text,
            filter: SearchFilter {
                person_gids: Vec::new(),
                source: request.source,
                template: request.template,
                from_ms: ms(request.from_ms),
                to_ms: ms(request.to_ms),
                meeting_gid: request.meeting,
                folder: request.folder,
                tags: request.tags.unwrap_or_default(),
                ..Default::default()
            },
            // Each hit is decrypted: keep pages small.
            limit: request.limit.clamp(1, 50) as usize,
            offset: request.offset as usize,
        };
        let page = store.search_page(&q).map_err(err)?;
        Ok(SearchResults {
            truncated: page.truncated,
            hits: page
                .hits
                .into_iter()
                .map(|h| SearchHitView {
                    kind: match h.kind {
                        HitKind::Segment => "segment",
                        HitKind::Note => "note",
                    }
                    .into(),
                    highlights: utf16_ranges(&h.snippet, h.snippet_start, &h.highlights),
                    meeting: h.meeting_gid,
                    meeting_title: h.meeting_title,
                    meeting_started_at: h.meeting_started_at as f64,
                    item: h.item_gid,
                    speaker_gid: h.speaker_gid,
                    t0_ms: h.t0_ms.map(|v| v as f64),
                    t1_ms: h.t1_ms.map(|v| v as f64),
                    snippet: h.snippet,
                    exact: h.exact,
                })
                .collect(),
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(t0: i64, t1: i64, text: &str) -> Segment {
        Segment {
            gid: format!("s{t0}"),
            version: 2,
            speaker_gid: Some("sp".into()),
            t0_ms: t0,
            t1_ms: t1,
            text: text.into(),
            lang: None,
            confidence: None,
            edited: false,
            overlap: false,
        }
    }

    fn anchor(t0: i64, t1: i64, v: i64) -> Anchor {
        Anchor {
            meeting_gid: "m".into(),
            t0_ms: t0,
            t1_ms: t1,
            transcript_version: v,
        }
    }

    #[test]
    fn citations_resolve_by_time_and_flag_stale_and_missing() {
        let segs = [seg(0, 1000, "xin chào"), seg(1000, 2000, "chốt lịch beta")];
        let c = citation(&anchor(500, 1500, 2), &segs, 2);
        assert_eq!(c.quote, "xin chào chốt lịch beta");
        assert!(!c.stale && !c.missing);
        assert_eq!(c.speaker_gid.as_deref(), Some("sp"));
        // A point anchor matches the segment containing it.
        assert_eq!(
            citation(&anchor(1200, 1200, 1), &segs, 2).quote,
            "chốt lịch beta"
        );
        assert!(citation(&anchor(1200, 1200, 1), &segs, 2).stale);
        assert!(citation(&anchor(5000, 6000, 2), &segs, 2).missing);
    }

    #[test]
    fn long_quotes_are_shortened_at_a_word() {
        let long = "lorem ipsum ".repeat(60);
        let q = shorten(&long, QUOTE_CHARS);
        assert!(q.chars().count() <= QUOTE_CHARS + 1 && q.ends_with('…'));
        assert!(!q.trim_end_matches('…').ends_with(' '));
    }

    #[test]
    fn highlights_become_utf16_offsets_into_the_snippet() {
        // "😀" is two UTF-16 units; "ồ" one.
        let snippet = "😀 họp tồi";
        // Full-text char offsets with the snippet starting at char 10.
        let r = utf16_ranges(snippet, 10, &[12..15, 16..19, 5..8]);
        assert_eq!(r, vec![[3, 6], [7, 10]]);
    }

    #[test]
    fn every_template_lists_its_sections() {
        let all = list_templates();
        assert_eq!(all.first().map(|t| t.id.as_str()), Some("general"));
        let client = all.iter().find(|t| t.id == "client").unwrap();
        assert_eq!(client.sections[0].id, "requests");
        assert!(!client.sections[0].title_vi.is_empty());
    }
}
