// SPDX-License-Identifier: Apache-2.0
//! Speakers of a stored meeting ("Name your speakers" after processing, D5):
//! the list with a representative span for the 3 s sample, and renaming
//! outside a recording (the live session has its own commands).

use serde::Serialize;
use specta::Type;

use crate::{CoreState, blocking};

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSpeaker {
    pub gid: String,
    /// The user's name for them, if any.
    pub name: Option<String>,
    /// 1-based number shown as "Speaker N" while unnamed.
    pub number: u32,
    /// Palette slot 1..8 (0: Others).
    pub color_slot: u32,
    pub is_me: bool,
    pub not_person: bool,
    pub lines: u32,
    /// A span of their speech for the sample (meeting ms, at most 3 s).
    pub sample_t0_ms: Option<f64>,
    pub sample_t1_ms: Option<f64>,
}

const SAMPLE_MS: i64 = 3_000;

/// The meeting's speakers (merged ones left out), with the middle 3 s of each
/// one's longest line as the sample.
#[tauri::command]
#[specta::specta]
pub async fn meeting_speakers(
    core: CoreState<'_>,
    meeting: String,
) -> Result<Vec<MeetingSpeaker>, String> {
    blocking(&core, move |c| speakers_of(&*c.store()?, &meeting)).await
}

/// The meeting's speakers (merged ones left out), with their samples.
pub fn speakers_of(
    store: &ghi_store::store::Store,
    meeting: &str,
) -> Result<Vec<MeetingSpeaker>, String> {
    let err = |e: ghi_store::StoreError| e.to_string();
    let segs = store.segments(meeting).map_err(err)?;
    speakers_with(store, meeting, &segs)
}

/// [`speakers_of`] with the segments already read.
pub fn speakers_with(
    store: &ghi_store::store::Store,
    meeting: &str,
    segs: &[ghi_store::store::Segment],
) -> Result<Vec<MeetingSpeaker>, String> {
    let err = |e: ghi_store::StoreError| e.to_string();
    Ok(store
        .speakers(meeting)
        .map_err(err)?
        .into_iter()
        .filter(|s| s.merged_into.is_none())
        .map(|s| {
            let own: Vec<_> = segs
                .iter()
                .filter(|g| g.speaker_gid.as_deref() == Some(&s.gid))
                .collect();
            let longest = own.iter().max_by_key(|g| g.t1_ms - g.t0_ms);
            let (t0, t1) = match longest {
                Some(g) => {
                    let len = (g.t1_ms - g.t0_ms).min(SAMPLE_MS);
                    let start = g.t0_ms + ((g.t1_ms - g.t0_ms) - len) / 2;
                    (Some(start as f64), Some((start + len) as f64))
                }
                None => (None, None),
            };
            MeetingSpeaker {
                gid: s.gid,
                name: s.display_name,
                number: (s.label_idx.max(0) + 1) as u32,
                color_slot: s.color_slot.clamp(0, 8) as u32,
                is_me: s.is_me,
                not_person: s.not_person,
                lines: own.len() as u32,
                sample_t0_ms: t0,
                sample_t1_ms: t1,
            }
        })
        .collect())
}

/// Names a speaker of a stored meeting (empty name: back to "Speaker N").
#[tauri::command]
#[specta::specta]
pub async fn rename_meeting_speaker(
    core: CoreState<'_>,
    meeting: String,
    speaker: String,
    name: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        if c.with_session(|s| s.meeting() == meeting).unwrap_or(false) {
            return Err("use the live speaker controls while recording".into());
        }
        let store = c.store()?;
        let err = |e: ghi_store::StoreError| e.to_string();
        // Only a speaker of this meeting (gids come from the UI).
        if !store
            .speakers(&meeting)
            .map_err(err)?
            .iter()
            .any(|s| s.gid == speaker)
        {
            return Err("not a speaker of this meeting".into());
        }
        let name = name.trim().chars().take(100).collect::<String>();
        store
            .rename_speaker(&speaker, (!name.is_empty()).then_some(name.as_str()))
            .map_err(err)
    })
    .await
}
