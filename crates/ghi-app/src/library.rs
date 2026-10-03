// SPDX-License-Identifier: Apache-2.0
//! Meetings for the UI: library rows (with job progress and "waiting for
//! models"), title and consent, the live notepad (user note blocks anchored to
//! meeting time), in-call tags, and what a discard would remove [RT-1].

use ghi_store::anchors::Anchor;
use ghi_store::store::{MarkTag, NewNoteBlock, Provenance};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::core::{Core, llm_ready, speech_ready};
use crate::{CoreState, blocking};

/// What the library shows about a meeting's processing.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MeetingJob {
    /// `notes_live`, `final_pass`, `notes_final`.
    pub kind: String,
    /// 0..1.
    pub progress: f64,
    /// Queued until its models are installed (record now, process later).
    pub waiting_for_models: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MeetingRow {
    pub gid: String,
    pub title: String,
    /// Unix ms.
    pub started_at: f64,
    pub duration_ms: f64,
    /// `live`, `import`, …
    pub source: String,
    /// `call` or `room`.
    pub mode: String,
    /// `recording`, `done`, `processing`, `ready`, …
    pub status: String,
    pub transcript_version: f64,
    pub cloud_used: bool,
    pub consent_confirmed: bool,
    /// Notes template id (`None`: the default).
    pub template: Option<String>,
    /// Named speakers, for the people column and filter.
    pub people: Vec<PersonChip>,
    /// The active job, if any.
    pub job: Option<MeetingJob>,
    /// The folder's gid (`list_folders` has the names); `None`: no folder.
    pub folder: Option<String>,
    pub tags: Vec<TagChip>,
    /// Where an imported file came from: `zoom`, `teams`, `meet`, `plaud`,
    /// `voice_memos`.
    pub source_app: Option<String>,
}

/// A tag on a meeting row.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TagChip {
    pub gid: String,
    pub name: String,
}

/// A named speaker as a chip: color + initial (never color alone).
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PersonChip {
    pub name: String,
    /// Palette slot 1..8 (0: Others).
    pub color_slot: u32,
}

const JOB_KINDS: [&str; 3] = [
    ghi_core::session::NOTES_LIVE_JOB,
    ghi_core::session::FINAL_PASS_JOB,
    ghi_core::notes_job::NOTES_FINAL_JOB,
];

/// The active job of each meeting that has one (the first in priority order).
pub fn active_jobs(
    core: &Core,
    store: &ghi_store::store::Store,
) -> Result<std::collections::HashMap<String, MeetingJob>, String> {
    let models = core.models();
    let (speech, llm) = (speech_ready(&models), llm_ready(&models));
    // All active jobs in one query, the first (in priority order) per meeting.
    let mut active: std::collections::HashMap<String, MeetingJob> =
        std::collections::HashMap::new();
    let jobs = store.active_jobs().map_err(|e| e.to_string())?;
    for kind in JOB_KINDS {
        for j in jobs.iter().filter(|j| j.kind == kind) {
            let Some(m) = &j.meeting_gid else { continue };
            let ready = if kind == ghi_core::session::FINAL_PASS_JOB {
                speech
            } else {
                llm
            };
            active.entry(m.clone()).or_insert(MeetingJob {
                kind: kind.to_string(),
                progress: j.progress,
                waiting_for_models: !ready,
            });
        }
    }
    Ok(active)
}

fn rows(core: &Core, limit: u32, offset: u32) -> Result<Vec<MeetingRow>, String> {
    let store = core.store()?;
    let mut active = active_jobs(core, &store)?;
    let meetings = store
        .list_meetings(limit.min(500) as usize, offset as usize)
        .map_err(|e| e.to_string())?;
    let gids: Vec<String> = meetings.iter().map(|m| m.gid.clone()).collect();
    let mut people = store.named_speakers(&gids).map_err(|e| e.to_string())?;
    let mut tags = store.meeting_tags(&gids).map_err(|e| e.to_string())?;
    Ok(meetings
        .into_iter()
        .map(|m| MeetingRow {
            people: people
                .remove(&m.gid)
                .unwrap_or_default()
                .into_iter()
                .map(|(name, slot)| PersonChip {
                    name,
                    color_slot: slot.clamp(0, 8) as u32,
                })
                .collect(),
            template: m.template,
            job: active.remove(&m.gid),
            folder: m.folder_gid,
            source_app: m.source_app,
            tags: tags
                .remove(&m.gid)
                .unwrap_or_default()
                .into_iter()
                .map(|t| TagChip {
                    gid: t.gid,
                    name: t.name,
                })
                .collect(),
            gid: m.gid,
            title: m.title,
            started_at: m.started_at as f64,
            duration_ms: m.duration_ms as f64,
            source: m.source,
            mode: m.mode,
            status: m.status,
            transcript_version: m.transcript_version as f64,
            cloud_used: m.cloud_used,
            consent_confirmed: m.consent_confirmed,
        })
        .collect())
}

/// Library rows, newest first.
#[tauri::command]
#[specta::specta]
pub async fn list_meetings(
    core: CoreState<'_>,
    limit: u32,
    offset: u32,
) -> Result<Vec<MeetingRow>, String> {
    blocking(&core, move |c| rows(c, limit, offset)).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_meeting_title(
    core: CoreState<'_>,
    meeting: String,
    title: String,
) -> Result<(), String> {
    let title = title.trim().chars().take(200).collect::<String>();
    blocking(&core, move |c| {
        c.store()?
            .set_meeting_title(&meeting, &title)
            .map_err(|e| e.to_string())
    })
    .await
}

/// A line of the live notepad (a user note block at a point in meeting time).
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NoteLine {
    pub gid: String,
    pub text: String,
    /// Meeting time the line was typed at (ms), if anchored.
    pub t_ms: Option<f64>,
    /// `note` (typed), `decision`, `action`, `question` (tagged).
    pub kind: String,
}

/// The kinds a user line can carry (in-call tags feed the notes, brief D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum NoteKind {
    Note,
    Decision,
    Action,
    Question,
}

impl NoteKind {
    fn as_str(self) -> &'static str {
        match self {
            NoteKind::Note => "note",
            NoteKind::Decision => "decision",
            NoteKind::Action => "action",
            NoteKind::Question => "question",
        }
    }
}

fn note_line(b: ghi_store::store::NoteBlock) -> NoteLine {
    NoteLine {
        t_ms: b.anchors.first().map(|a| a.t0_ms as f64),
        gid: b.gid,
        text: b.body,
        kind: b.kind,
    }
}

/// The user's own lines of a meeting, in time order.
#[tauri::command]
#[specta::specta]
pub async fn note_lines(core: CoreState<'_>, meeting: String) -> Result<Vec<NoteLine>, String> {
    blocking(&core, move |c| {
        let mut lines: Vec<NoteLine> = c
            .store()?
            .note_blocks(&meeting)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|b| b.provenance == Provenance::User)
            .map(note_line)
            .collect();
        lines.sort_by(|a, b| {
            a.t_ms
                .partial_cmp(&b.t_ms)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(lines)
    })
    .await
}

/// Adds a notepad line at meeting time `tMs` (invisible anchor: the line links
/// to that moment). A tagged kind also leaves a mark there.
#[tauri::command]
#[specta::specta]
pub async fn add_note_line(
    core: CoreState<'_>,
    meeting: String,
    text: String,
    t_ms: f64,
    kind: NoteKind,
) -> Result<NoteLine, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let t = t_ms.max(0.0) as i64;
        let anchor: Anchor = store
            .anchor_for_range(&meeting, t, t)
            .map_err(|e| e.to_string())?;
        let block = store
            .add_note_block(
                &meeting,
                NewNoteBlock {
                    kind: kind.as_str().into(),
                    provenance: Provenance::User,
                    body: cap(text),
                    anchors: vec![anchor],
                    pinned: false,
                },
            )
            .map_err(|e| e.to_string())?;
        let tag = match kind {
            NoteKind::Note => None,
            NoteKind::Decision => Some(MarkTag::Decision),
            NoteKind::Action => Some(MarkTag::Action),
            NoteKind::Question => Some(MarkTag::Question),
        };
        if let Some(tag) = tag {
            store
                .add_mark(&meeting, t, tag)
                .map_err(|e| e.to_string())?;
        }
        Ok(note_line(block))
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn update_note_line(
    core: CoreState<'_>,
    meeting: String,
    line: String,
    text: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        own_line(&store, &meeting, &line)?;
        store
            .update_note_block(&line, &cap(text))
            .map_err(|e| e.to_string())
    })
    .await
}

/// Notepad lines are text the user typed: a sane upper bound.
const MAX_LINE_CHARS: usize = 10_000;

fn cap(text: String) -> String {
    if text.chars().count() > MAX_LINE_CHARS {
        text.chars().take(MAX_LINE_CHARS).collect()
    } else {
        text
    }
}

/// The notepad edits only the user's own lines of that meeting (never AI
/// note blocks, never another meeting's).
fn own_line(store: &ghi_store::store::Store, meeting: &str, line: &str) -> Result<(), String> {
    let ok = store
        .note_blocks(meeting)
        .map_err(|e| e.to_string())?
        .iter()
        .any(|b| b.gid == line && b.provenance == Provenance::User);
    if ok {
        Ok(())
    } else {
        Err("not a notepad line of this meeting".into())
    }
}

#[tauri::command]
#[specta::specta]
pub async fn delete_note_line(
    core: CoreState<'_>,
    meeting: String,
    line: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        own_line(&store, &meeting, &line)?;
        store.delete_note_block(&line).map_err(|e| e.to_string())
    })
    .await
}

/// What "discard the last N seconds" would remove, shown before confirming [RT-1].
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscardPreview {
    /// Where the cut lands (meeting ms).
    pub from_ms: f64,
    /// Transcript lines that end after the cut.
    pub lines: Vec<String>,
    /// Notepad lines typed after the cut.
    pub notes: Vec<String>,
    pub marks: u32,
}

#[tauri::command]
#[specta::specta]
pub async fn discard_preview(core: CoreState<'_>, seconds: f64) -> Result<DiscardPreview, String> {
    blocking(&core, move |c| {
        let (meeting, now) = c.with_session_unlocked(|s| (s.meeting().to_string(), s.now_ms()))?;
        let from = (now - (seconds.clamp(0.0, 24.0 * 3600.0) * 1000.0) as i64).max(0);
        let store = c.store()?;
        let lines = store
            .segments(&meeting)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|s| s.t1_ms > from)
            .map(|s| s.text)
            .collect();
        let notes = store
            .note_blocks(&meeting)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|b| {
                b.provenance == Provenance::User && b.anchors.iter().any(|a| a.t0_ms >= from)
            })
            .map(|b| b.body)
            .collect();
        let marks = store
            .marks(&meeting)
            .map_err(|e| e.to_string())?
            .iter()
            .filter(|m| m.t_ms >= from)
            .count() as u32;
        Ok(DiscardPreview {
            from_ms: from as f64,
            lines,
            notes,
            marks,
        })
    })
    .await
}

/// The "Consent confirmed" toggle of a meeting [RT-14].
#[tauri::command]
#[specta::specta]
pub async fn set_consent_confirmed(
    core: CoreState<'_>,
    meeting: String,
    confirmed: bool,
) -> Result<(), String> {
    blocking(&core, move |c| {
        c.store()?
            .set_consent_confirmed(&meeting, confirmed)
            .map_err(|e| e.to_string())
    })
    .await
}

/// The recording in progress as it stands now (a reloaded webview, a second
/// window): apply events with a greater `seq` after it. `None` when idle.
#[tauri::command]
#[specta::specta]
pub async fn session_snapshot(
    core: CoreState<'_>,
) -> Result<Option<ghi_core::events::SessionSnapshot>, String> {
    blocking(&core, |c| {
        // The transcript and speakers are content: re-read after unlocking.
        if c.locked() {
            return Err("the app is locked".into());
        }
        Ok(c.with_session(|s| s.snapshot()).ok())
    })
    .await
}

/// Deletes a meeting for good (crypto-shred: its key goes, so audio and text
/// become unreadable). Not the one being recorded.
#[tauri::command]
#[specta::specta]
pub async fn delete_meeting(
    core: CoreState<'_>,
    tokens: tauri::State<'_, std::sync::Arc<crate::audio_protocol::AudioTokens>>,
    meeting: String,
) -> Result<(), String> {
    let tokens = tokens.inner().clone();
    blocking(&core, move |c| {
        if c.with_session(|s| s.meeting() == meeting).unwrap_or(false) {
            return Err("stop the recording first".into());
        }
        let store = c.store()?;
        // A job reading its audio would lose it mid-run.
        let running = store
            .active_jobs()
            .map_err(|e| e.to_string())?
            .iter()
            .any(|j| {
                j.meeting_gid.as_deref() == Some(meeting.as_str())
                    && j.state == ghi_store::jobs::JobState::Running
            });
        if running {
            return Err("this meeting is being processed; try again in a moment".into());
        }
        store.delete_meeting(&meeting).map_err(|e| e.to_string())?;
        tokens.revoke_meeting(&meeting);
        Ok(())
    })
    .await
}

/// Runs a meeting's failed jobs again (the library's "Failed · Retry").
#[tauri::command]
#[specta::specta]
pub async fn retry_meeting(core: CoreState<'_>, meeting: String) -> Result<u32, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let err = |e: ghi_store::StoreError| e.to_string();
        let mut n = 0;
        for j in store.jobs_for_meeting(&meeting).map_err(err)? {
            if j.state == ghi_store::jobs::JobState::Failed {
                store.retry_job(j.id).map_err(err)?;
                n += 1;
            }
        }
        if n > 0 {
            store
                .set_meeting_status(&meeting, "processing")
                .map_err(err)?;
            c.notify_jobs();
        }
        Ok(n)
    })
    .await
}

/// A meeting closed by crash recovery at this launch.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RecoveredMeeting {
    pub gid: String,
    pub title: String,
    /// What was saved (ms).
    pub duration_ms: f64,
}

/// Meetings recovered after a crash at this launch (D12 "recovered"; the
/// notice shows once: the list is cleared when read).
#[tauri::command]
#[specta::specta]
pub async fn take_recovered_meetings(core: CoreState<'_>) -> Result<Vec<RecoveredMeeting>, String> {
    blocking(&core, |c| {
        let store = c.store()?;
        Ok(c.take_recovered()
            .into_iter()
            .filter_map(|gid| store.get_meeting(&gid).ok())
            .map(|m| RecoveredMeeting {
                gid: m.gid,
                title: m.title,
                duration_ms: m.duration_ms as f64,
            })
            .collect())
    })
    .await
}

/// Names of the people the user has named (not Me), most recently met first
/// (rename autocomplete).
#[tauri::command]
#[specta::specta]
pub async fn known_speaker_names(core: CoreState<'_>) -> Result<Vec<String>, String> {
    blocking(&core, |c| {
        // The people the user has named (Me has no name to suggest), the
        // most recently met first.
        let store = c.store()?;
        let mut people: Vec<_> = store
            .people_overview()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|p| !p.is_me && !p.name.is_empty())
            .collect();
        people.sort_by_key(|p| std::cmp::Reverse(p.last_met_ms));
        // Names are content: re-checked just before they are returned.
        if c.locked() {
            return Err("the app is locked".into());
        }
        Ok(people.into_iter().map(|p| p.name).take(100).collect())
    })
    .await
}
