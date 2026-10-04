// SPDX-License-Identifier: Apache-2.0
//! Speakers of a stored meeting ("Name your speakers" after processing, D5):
//! the list with a representative span for the 3 s sample, and renaming
//! outside a recording (the live session has its own commands).

use serde::Serialize;
use specta::Type;

use crate::core::Core;
use crate::{CoreState, blocking};

/// Error codes of the voice commands (the UI turns them into words).
pub const BUSY_RECORDING: &str = "busyRecording";
/// The meeting is the one being recorded: use the live speaker controls.
pub const LIVE_MEETING: &str = "liveMeeting";
pub const NOT_A_SPEAKER: &str = "notASpeaker";
/// In a call only the mic speaker is Me.
pub const FAR_SIDE: &str = "farSide";
pub const NOT_ME: &str = "notMe";
pub const NO_SUGGESTION: &str = "noSuggestion";
/// Other people's voice profiles are off in this build.
pub const THIRD_PARTY_OFF: &str = "thirdPartyOff";
pub const NOT_NAMED: &str = "notNamed";
pub const NO_VOICE: &str = "noVoice";
pub const INVALID_CONSENT: &str = "invalidConsent";
pub const STORAGE: &str = "storage";
/// Merging a speaker into itself.
pub const SAME_SPEAKER: &str = "sameSpeaker";
/// A split needs some of the speaker's lines, but not all of them.
pub const NOTHING_TO_SPLIT: &str = "nothingToSplit";
pub const WHOLE_SPEAKER: &str = "wholeSpeaker";
/// Me cannot be marked "not a person".
pub const IS_ME: &str = "isMe";

/// A store failure as a code (the detail goes to the log, never the UI).
pub fn storage(e: impl std::fmt::Display) -> String {
    log::warn!("voice: {e}");
    STORAGE.into()
}

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
    /// "Sounds like ..." from the final pass's voice matching, until the user
    /// names the speaker, accepts or dismisses it.
    pub suggestion: Option<VoiceSuggestion>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSuggestion {
    pub person_gid: String,
    /// The person's name (empty for Me: show "Me").
    pub name: String,
    pub is_me: bool,
    /// Cosine similarity, 0.5..1.
    pub score: f64,
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
                suggestion: s.suggestion.map(|g| VoiceSuggestion {
                    is_me: g.is_me,
                    person_gid: g.person_gid,
                    name: g.person_name,
                    score: f64::from(g.score),
                }),
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

// ------------------------------------------------------------ voice (14c)
//
// These work on stored meetings. The live session keeps its own speaker
// controls; "This is me" and suggestions apply once the meeting is processed,
// so there are no live variants.

/// A speaker of a stored meeting that is not being recorded now.
fn stored_speaker(
    c: &Core,
    store: &ghi_store::store::Store,
    meeting: &str,
    speaker: &str,
) -> Result<ghi_store::store::Speaker, String> {
    if c.with_session(|s| s.meeting() == meeting).unwrap_or(false) {
        return Err(LIVE_MEETING.into());
    }
    store
        .speakers(meeting)
        .map_err(storage)?
        .into_iter()
        .find(|s| s.gid == speaker && s.merged_into.is_none())
        .ok_or_else(|| NOT_A_SPEAKER.to_string())
}

/// In a call with a far-side track the mic speaker is Me by construction.
fn call_with_far_side(store: &ghi_store::store::Store, meeting: &str) -> Result<bool, String> {
    let m = store.get_meeting(meeting).map_err(storage)?;
    Ok(m.mode == "call"
        && store
            .tracks(meeting)
            .map_err(storage)?
            .iter()
            .any(|(k, _)| *k == ghi_store::store::TrackKind::System))
}

/// What taking a suggestion does.
#[derive(Debug, PartialEq)]
enum Accept {
    Me,
    Person(String),
}

/// What accepting the speaker's suggestion would do, or why not (no side
/// effects): someone else's needs the third-party proof, which is off.
fn accept_plan(
    store: &ghi_store::store::Store,
    sp: &ghi_store::store::Speaker,
) -> Result<Accept, String> {
    let sug = sp.suggestion.as_ref().ok_or(NO_SUGGESTION)?;
    if sug.is_me {
        return Ok(Accept::Me);
    }
    if crate::system::third_party_token(store).is_none() {
        return Err(THIRD_PARTY_OFF.into());
    }
    Ok(Accept::Person(sug.person_name.clone()))
}

/// "This is me" for a speaker (refused for a far-side speaker in a call).
fn mark_me(
    c: &Core,
    store: &ghi_store::store::Store,
    meeting: &str,
    sp: &ghi_store::store::Speaker,
) -> Result<(), String> {
    if call_with_far_side(store, meeting)? && !sp.is_me && sp.label_idx != -1 {
        return Err(FAR_SIDE.into());
    }
    ghi_core::voice_job::set_me(store, meeting, &sp.gid).map_err(storage)?;
    // The chunk text carries who spoke: it is stale now.
    ghi_core::voice_job::requeue_index(store, &[meeting.to_string()]).map_err(storage)?;
    c.notify_jobs();
    Ok(())
}

/// "This is me": the speaker becomes Me (another Me in the meeting stops
/// being Me, and Me's voice samples from this meeting go), and Me's profile
/// learns from the speaker's voice. Errors: `liveMeeting`, `notASpeaker`,
/// `farSide`, `storage`.
#[tauri::command]
#[specta::specta]
pub async fn set_speaker_me(
    core: CoreState<'_>,
    meeting: String,
    speaker: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let sp = stored_speaker(c, &store, &meeting, &speaker)?;
        mark_me(c, &store, &meeting, &sp)
    })
    .await
}

/// "Not me": the speaker stops being Me. Errors: `liveMeeting`,
/// `notASpeaker`, `notMe`, `farSide` (in a call the mic speaker is always
/// Me), `storage`.
#[tauri::command]
#[specta::specta]
pub async fn clear_speaker_me(
    core: CoreState<'_>,
    meeting: String,
    speaker: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let sp = stored_speaker(c, &store, &meeting, &speaker)?;
        if !sp.is_me {
            return Err(NOT_ME.into());
        }
        if call_with_far_side(&store, &meeting)? {
            return Err(FAR_SIDE.into());
        }
        store.clear_speaker_me(&speaker).map_err(storage)?;
        ghi_core::voice_job::requeue_index(&store, std::slice::from_ref(&meeting))
            .map_err(storage)?;
        c.notify_jobs();
        Ok(())
    })
    .await
}

/// Takes the "sounds like ..." suggestion: for Me that is "This is me"; for
/// anyone else it names the speaker (and links the person) and is refused
/// while other people's voice profiles are off. Then the voice is learned.
/// Errors: as `set_speaker_me`, plus `noSuggestion`, `thirdPartyOff`.
#[tauri::command]
#[specta::specta]
pub async fn accept_voice_suggestion(
    core: CoreState<'_>,
    meeting: String,
    speaker: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let sp = stored_speaker(c, &store, &meeting, &speaker)?;
        let name = match accept_plan(&store, &sp)? {
            Accept::Me => return mark_me(c, &store, &meeting, &sp),
            Accept::Person(name) => name,
        };
        store
            .rename_speaker(&speaker, Some(&name))
            .map_err(storage)?;
        ghi_core::voice_job::queue_learn(&store, &meeting, &speaker).map_err(storage)?;
        c.notify_jobs();
        Ok(())
    })
    .await
}

/// Drops the suggestion without naming anyone. Errors: `liveMeeting`,
/// `notASpeaker`, `storage`.
#[tauri::command]
#[specta::specta]
pub async fn dismiss_voice_suggestion(
    core: CoreState<'_>,
    meeting: String,
    speaker: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        stored_speaker(c, &store, &meeting, &speaker)?;
        store
            .set_speaker_suggestion(&speaker, None)
            .map_err(storage)
    })
    .await
}

/// The evidence that a person agreed to a voice profile (D10). Only a spoken
/// agreement recorded in the meeting counts for someone else.
#[derive(Debug, Clone, serde::Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceConsentInput {
    /// `verbal_clip`.
    pub method: String,
    /// The locale key of the consent text that was shown.
    pub text_key: String,
    /// The span of this meeting where they said yes (at most 30 s).
    pub clip_t0_ms: Option<f64>,
    pub clip_t1_ms: Option<f64>,
}

/// Saving someone else's voice needs the third-party proof (off).
fn save_gate(
    store: &ghi_store::store::Store,
) -> Result<ghi_store::voice::ThirdPartyApproved, String> {
    crate::system::third_party_token(store).ok_or_else(|| THIRD_PARTY_OFF.to_string())
}

/// The consent's clip span in whole ms, or `invalidConsent`: a spoken
/// agreement, with a text key and a span that starts at 0 or later and runs
/// 30 s at most.
fn consent_span(c: &VoiceConsentInput) -> Result<(i64, i64), String> {
    let (Some(t0), Some(t1)) = (c.clip_t0_ms, c.clip_t1_ms) else {
        return Err(INVALID_CONSENT.into());
    };
    if c.method != "verbal_clip"
        || !t0.is_finite()
        || !t1.is_finite()
        || t0 < 0.0
        || t1 <= t0
        || t1 - t0 > 30_000.0
        || c.text_key.is_empty()
        || c.text_key.len() > 100
    {
        return Err(INVALID_CONSENT.into());
    }
    Ok((t0 as i64, t1 as i64))
}

/// Saves a named speaker's voice as a profile, with the consent evidence;
/// afterwards the profile also learns from this meeting. Refused unless other
/// people's voice profiles are on. Errors: `liveMeeting`, `notASpeaker`,
/// `thirdPartyOff`, `notNamed`, `noVoice` (no stored voice for the speaker),
/// `invalidConsent`, `storage`.
#[tauri::command]
#[specta::specta]
pub async fn save_voice_profile(
    core: CoreState<'_>,
    meeting: String,
    speaker: String,
    consent: VoiceConsentInput,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let token = save_gate(&store)?;
        let sp = stored_speaker(c, &store, &meeting, &speaker)?;
        let person = match (sp.person_gid.as_deref(), sp.is_me) {
            (Some(p), false) => p.to_string(),
            _ => return Err(NOT_NAMED.into()),
        };
        let (t0, t1) = consent_span(&consent)?;
        let voice = store
            .speaker_voice(&speaker)
            .map_err(storage)?
            .ok_or(NO_VOICE)?;
        let clip = crate::audio_protocol::span_wav(&store, &meeting, t0, t1)
            .map_err(|_| INVALID_CONSENT.to_string())?;
        let record = ghi_store::voice::VoiceConsent {
            method: consent.method,
            at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as i64),
            text_key: consent.text_key,
            clip: Some(ghi_store::voice::ExemplarSource {
                meeting_gid: meeting.clone(),
                t0_ms: t0,
                t1_ms: t1,
            }),
        };
        let exemplar = ghi_store::voice::VoiceExemplar {
            vec: voice.vec.clone(),
            source: None,
        };
        match store.voice_profile(&person).map_err(storage)? {
            // Already has one: this voice is one more sample of it.
            Some(p) => store
                .add_voice_exemplars(
                    &p.gid,
                    &voice.model,
                    &voice.lang,
                    vec![exemplar],
                    Some(token),
                )
                .map_err(storage)?,
            None => {
                store
                    .put_voice_profile(
                        &person,
                        &record,
                        Some(&clip),
                        &voice.model,
                        vec![(voice.lang.clone(), vec![exemplar])],
                        Some(token),
                    )
                    .map_err(storage)?;
            }
        }
        store.clear_speaker_voice(&speaker).map_err(storage)?;
        ghi_core::voice_job::queue_learn(&store, &meeting, &speaker).map_err(storage)?;
        c.notify_jobs();
        Ok(())
    })
    .await
}

// ------------------------------------------------- merge, split, not a person
//
// Edits of a stored meeting's speakers (the side panel, design 7c). The store
// does the work in one transaction each and bumps the meeting's `index_gen`;
// here the chunk text (it carries who spoke) is queued for a re-index. Lines
// keep their segments, so notes citations (time anchors) still resolve, and
// the transcript version does not change. Nothing is emitted: the panel
// reloads the meeting when the command returns, like the other stored-speaker
// commands (the speaker events belong to the live session).

/// The first palette slot (1..8) no speaker of the meeting uses, else Others (0).
fn free_color_slot(speakers: &[ghi_store::store::Speaker]) -> i64 {
    (1..=8)
        .find(|n| {
            !speakers
                .iter()
                .any(|s| s.merged_into.is_none() && s.color_slot == *n)
        })
        .unwrap_or(0)
}

fn reindex(c: &Core, store: &ghi_store::store::Store, meeting: &str) -> Result<(), String> {
    ghi_core::voice_job::requeue_index(store, &[meeting.to_string()]).map_err(storage)?;
    c.notify_jobs();
    Ok(())
}

/// Merges `from` into `into`: all of `from`'s lines and action items move to
/// `into`, and `from` is hidden. Me carries over (`into` becomes Me, and the
/// person Me links to, replacing `into`'s own; `into` stops being "not a
/// person"); in a call Me is
/// the mic speaker only, so Me and a far-side speaker cannot be merged either
/// way. A named `from` loses its name with it; its stored voice is dropped.
/// Errors: `liveMeeting`, `notASpeaker`, `sameSpeaker`, `farSide`, `storage`.
fn merge_speakers_in(c: &Core, meeting: &str, from: &str, into: &str) -> Result<(), String> {
    let store = c.store()?;
    let f = stored_speaker(c, &store, meeting, from)?;
    let t = stored_speaker(c, &store, meeting, into)?;
    if from == into {
        return Err(SAME_SPEAKER.into());
    }
    if (f.is_me || (t.is_me && f.label_idx != -1)) && call_with_far_side(&store, meeting)? {
        return Err(FAR_SIDE.into());
    }
    store.merge_speakers(from, into).map_err(storage)?;
    // The merge has committed: a voice left behind on the hidden speaker is
    // never read again, so a failure here is logged, not reported.
    if let Err(e) = store.clear_speaker_voice(from) {
        log::warn!("merge: clearing the merged speaker's voice: {e}");
    }
    reindex(c, &store, meeting)
}

/// Merges `from` into `into` (lines, action items and Me move; `from` is hidden).
/// Errors: `liveMeeting`, `notASpeaker`, `sameSpeaker`, `farSide`, `storage`.
#[tauri::command]
#[specta::specta]
pub async fn merge_meeting_speakers(
    core: CoreState<'_>,
    meeting: String,
    from: String,
    into: String,
) -> Result<(), String> {
    blocking(&core, move |c| merge_speakers_in(c, &meeting, &from, &into)).await
}

/// Splits lines off `speaker` into a new speaker ("Speaker N", the next free
/// color, never Me, no name): either the given `segment_gids`, or every line of
/// the speaker from `from_segment` on. Returns the new speaker's gid. Errors:
/// `liveMeeting`, `notASpeaker`, `nothingToSplit` (no lines, a line that is
/// not theirs, or both ways given), `wholeSpeaker` (it would take all their
/// lines), `storage`.
fn split_speaker_in(
    c: &Core,
    meeting: &str,
    speaker: &str,
    segment_gids: Vec<String>,
    from_segment: Option<String>,
) -> Result<String, String> {
    let store = c.store()?;
    stored_speaker(c, &store, meeting, speaker)?;
    let segs = store.segments(meeting).map_err(storage)?;
    let own: Vec<_> = segs
        .iter()
        .filter(|g| g.speaker_gid.as_deref() == Some(speaker))
        .collect();
    let moving: Vec<String> = match (segment_gids.is_empty(), from_segment) {
        (false, None) => segment_gids,
        (true, Some(from)) => {
            let t0 = own
                .iter()
                .find(|g| g.gid == from)
                .map(|g| g.t0_ms)
                .ok_or(NOTHING_TO_SPLIT)?;
            own.iter()
                .filter(|g| g.t0_ms >= t0)
                .map(|g| g.gid.clone())
                .collect()
        }
        _ => return Err(NOTHING_TO_SPLIT.into()),
    };
    let mut seen = std::collections::HashSet::new();
    if moving
        .iter()
        .any(|g| !seen.insert(g.as_str()) || !own.iter().any(|o| &o.gid == g))
    {
        return Err(NOTHING_TO_SPLIT.into());
    }
    if moving.len() >= own.len() {
        return Err(WHOLE_SPEAKER.into());
    }
    let all = store.speakers(meeting).map_err(storage)?;
    let not_person = all.iter().any(|s| s.gid == speaker && s.not_person);
    let gid = store
        .split_speaker(speaker, &moving, free_color_slot(&all))
        .map_err(storage)?;
    // Lines split off a TV are still the TV's kind of lines.
    if not_person && let Err(e) = store.set_speaker_not_person(&gid, true) {
        log::warn!("split: carrying not a person: {e}");
    }
    reindex(c, &store, meeting)?;
    Ok(gid)
}

/// Splits the given lines (or all from `from_segment` on) off a speaker into a
/// new one; returns its gid. Errors: `liveMeeting`, `notASpeaker`,
/// `nothingToSplit`, `wholeSpeaker`, `storage`.
#[tauri::command]
#[specta::specta]
pub async fn split_meeting_speaker(
    core: CoreState<'_>,
    meeting: String,
    speaker: String,
    segment_gids: Vec<String>,
    from_segment: Option<String>,
) -> Result<String, String> {
    blocking(&core, move |c| {
        split_speaker_in(c, &meeting, &speaker, segment_gids, from_segment)
    })
    .await
}

/// Marks a speaker "not a person" (a TV, a notification sound) or back: a
/// not-a-person speaker is unlinked from its person, loses any voice
/// suggestion and is not counted among the unnamed voices; turning it off
/// links it again by its name. Me cannot be marked. Errors: `liveMeeting`,
/// `notASpeaker`, `isMe`, `storage`.
fn set_not_person_in(
    c: &Core,
    meeting: &str,
    speaker: &str,
    not_person: bool,
) -> Result<(), String> {
    let store = c.store()?;
    let sp = stored_speaker(c, &store, meeting, speaker)?;
    if not_person && sp.is_me {
        return Err(IS_ME.into());
    }
    store
        .set_speaker_not_person(speaker, not_person)
        .map_err(storage)?;
    reindex(c, &store, meeting)
}

/// Marks a speaker "not a person" or back. Errors: `liveMeeting`, `notASpeaker`,
/// `isMe`, `storage`.
#[tauri::command]
#[specta::specta]
pub async fn set_speaker_not_person(
    core: CoreState<'_>,
    meeting: String,
    speaker: String,
    not_person: bool,
) -> Result<(), String> {
    blocking(&core, move |c| {
        set_not_person_in(c, &meeting, &speaker, not_person)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_store::keys::{MemoryKeyStore, Protection};
    use ghi_store::store::{NewMeeting, NewSpeaker, Store};
    use std::sync::Arc;

    fn open() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        (tmp, s)
    }

    #[test]
    fn error_codes_are_stable_words_not_english() {
        assert_eq!(
            [
                BUSY_RECORDING,
                LIVE_MEETING,
                NOT_A_SPEAKER,
                FAR_SIDE,
                NOT_ME,
                NO_SUGGESTION,
                THIRD_PARTY_OFF,
                NOT_NAMED,
                NO_VOICE,
                INVALID_CONSENT,
                STORAGE,
                SAME_SPEAKER,
                NOTHING_TO_SPLIT,
                WHOLE_SPEAKER,
                IS_ME
            ],
            [
                "busyRecording",
                "liveMeeting",
                "notASpeaker",
                "farSide",
                "notMe",
                "noSuggestion",
                "thirdPartyOff",
                "notNamed",
                "noVoice",
                "invalidConsent",
                "storage",
                "sameSpeaker",
                "nothingToSplit",
                "wholeSpeaker",
                "isMe"
            ]
        );
        // A store failure never leaks its text to the UI.
        assert_eq!(storage("secret detail"), STORAGE);
    }

    #[test]
    fn third_party_actions_are_refused_while_the_flag_is_off() {
        let (_t, s) = open();
        let m = s
            .create_meeting(NewMeeting {
                title: "x".into(),
                ..Default::default()
            })
            .unwrap()
            .gid;
        let sp = s
            .add_speaker(
                &m,
                NewSpeaker {
                    label_idx: 0,
                    ..Default::default()
                },
            )
            .unwrap();
        let get = |s: &Store| s.speakers(&m).unwrap().remove(0);
        assert_eq!(accept_plan(&s, &get(&s)).unwrap_err(), NO_SUGGESTION);
        let ok = ghi_store::voice::ThirdPartyApproved::assert_flag_checked();
        let hoa = s.add_person("Hoa", 1).unwrap();
        s.put_voice_profile(
            &hoa,
            &ghi_core::voice_job::self_consent("k"),
            None,
            ghi_core::profiles::VOICE_MODEL,
            vec![(
                "vi".into(),
                vec![ghi_store::voice::VoiceExemplar {
                    vec: vec![1.0, 0.0],
                    source: None,
                }],
            )],
            Some(ok),
        )
        .unwrap();
        s.set_speaker_suggestion(&sp, Some((&hoa, 0.8))).unwrap();
        assert_eq!(accept_plan(&s, &get(&s)).unwrap_err(), THIRD_PARTY_OFF);
        assert_eq!(save_gate(&s).unwrap_err(), THIRD_PARTY_OFF);
        // Me's suggestion needs no proof.
        let me = s.me_person().unwrap();
        s.set_speaker_suggestion(&sp, Some((&me, 0.8))).unwrap();
        assert_eq!(accept_plan(&s, &get(&s)).unwrap(), Accept::Me);
    }

    #[test]
    fn a_consent_clip_must_be_a_valid_span() {
        let c = |t0, t1| VoiceConsentInput {
            method: "verbal_clip".into(),
            text_key: "k".into(),
            clip_t0_ms: t0,
            clip_t1_ms: t1,
        };
        assert_eq!(
            consent_span(&c(Some(1000.0), Some(4000.0))),
            Ok((1000, 4000))
        );
        for bad in [
            c(None, Some(4000.0)),
            c(Some(1000.0), None),
            c(Some(-1.0), Some(4000.0)),
            c(Some(5000.0), Some(4000.0)),
            c(Some(0.0), Some(31_000.0)),
            c(Some(f64::NAN), Some(4000.0)),
        ] {
            assert_eq!(consent_span(&bad), Err(INVALID_CONSENT.into()));
        }
        let mut v = c(Some(0.0), Some(1000.0));
        v.method = "self_checkbox".into();
        assert_eq!(consent_span(&v), Err(INVALID_CONSENT.into()));
    }

    #[test]
    fn a_suggestion_reaches_the_speaker_list_and_a_named_speaker_drops_it() {
        let (_t, s) = open();
        let m = s
            .create_meeting(NewMeeting {
                title: "x".into(),
                ..Default::default()
            })
            .unwrap()
            .gid;
        let sp = s
            .add_speaker(
                &m,
                NewSpeaker {
                    label_idx: 0,
                    ..Default::default()
                },
            )
            .unwrap();
        let me = s.me_person().unwrap();
        s.set_speaker_suggestion(&sp, Some((&me, 0.62))).unwrap();
        let list = speakers_of(&s, &m).unwrap();
        let g = list[0].suggestion.as_ref().unwrap();
        assert!(g.is_me && g.name.is_empty() && g.person_gid == me);
        assert!((g.score - 0.62).abs() < 1e-6);
        s.rename_speaker(&sp, Some("Lan")).unwrap();
        assert!(speakers_of(&s, &m).unwrap()[0].suggestion.is_none());
    }

    #[test]
    fn me_in_a_call_is_judged_by_the_far_side_track() {
        use ghi_store::store::TrackKind;
        let (_t, s) = open();
        let call = s
            .create_meeting(NewMeeting {
                title: "c".into(),
                mode: "call".into(),
                ..Default::default()
            })
            .unwrap()
            .gid;
        assert!(
            !call_with_far_side(&s, &call).unwrap(),
            "no system track yet"
        );
        s.open_track(&call, TrackKind::Mic).unwrap();
        assert!(!call_with_far_side(&s, &call).unwrap(), "mic only");
        s.open_track(&call, TrackKind::System).unwrap();
        assert!(call_with_far_side(&s, &call).unwrap());
        // The store enforces the same: only the mic speaker (-1) can be Me.
        let far = s
            .add_speaker(
                &call,
                NewSpeaker {
                    label_idx: 0,
                    ..Default::default()
                },
            )
            .unwrap();
        let mic = s
            .add_speaker(
                &call,
                NewSpeaker {
                    label_idx: -1,
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(s.set_speaker_me(&far).is_err());
        s.set_speaker_me(&mic).unwrap();
        // ... and "not me" is refused there too.
        assert!(s.clear_speaker_me(&mic).is_err());
        let room = s
            .create_meeting(NewMeeting {
                title: "r".into(),
                mode: "room".into(),
                ..Default::default()
            })
            .unwrap()
            .gid;
        assert!(!call_with_far_side(&s, &room).unwrap());
    }

    // ---- merge, split, not a person (over a real Core with a temp store)

    use ghi_store::anchors::Anchor;
    use ghi_store::store::{NewActionItem, NewNoteBlock, NewSegment};

    struct Fix {
        _tmp: tempfile::TempDir,
        core: Arc<Core>,
        rx: ghi_core::events::EventRx,
        meeting: String,
        /// Speakers a, b, c (gids); a is Me when `with_me`.
        sp: Vec<String>,
        /// Segments: a0, b0, b1, b2, c0.
        seg: Vec<String>,
    }

    fn fix(mode: &str, me_is_a: bool) -> Fix {
        let tmp = tempfile::tempdir().unwrap();
        let (core, rx) = Core::for_test(tmp.path().join("data"));
        let store = core.store().unwrap();
        let meeting = store
            .create_meeting(NewMeeting {
                title: "x".into(),
                mode: mode.into(),
                ..Default::default()
            })
            .unwrap()
            .gid;
        let add = |idx: i64, slot: i64, me: bool| {
            store
                .add_speaker(
                    &meeting,
                    NewSpeaker {
                        label_idx: idx,
                        color_slot: slot,
                        is_me: me,
                        ..Default::default()
                    },
                )
                .unwrap()
        };
        let sp = vec![
            add(if me_is_a { -1 } else { 0 }, 1, me_is_a),
            add(1, 2, false),
            add(2, 3, false),
        ];
        let line = |i: usize, n: i64| NewSegment {
            speaker_gid: Some(sp[i].clone()),
            t0_ms: n * 1000,
            t1_ms: n * 1000 + 800,
            text: format!("line {n}"),
            ..Default::default()
        };
        let seg = store
            .add_segments(
                &meeting,
                vec![line(0, 0), line(1, 1), line(1, 2), line(1, 3), line(2, 4)],
            )
            .unwrap()
            .into_iter()
            .map(|g| g.gid)
            .collect();
        Fix {
            _tmp: tmp,
            core,
            rx,
            meeting,
            sp,
            seg,
        }
    }

    fn lines_of(store: &Store, m: &str, sp: &str) -> Vec<String> {
        store
            .segments(m)
            .unwrap()
            .into_iter()
            .filter(|g| g.speaker_gid.as_deref() == Some(sp))
            .map(|g| g.gid)
            .collect()
    }

    #[test]
    fn merge_moves_lines_and_keeps_citations_and_the_transcript_version() {
        let f = fix("room", false);
        let store = f.core.store().unwrap();
        let version = store.get_meeting(&f.meeting).unwrap().transcript_version;
        let anchor = Anchor {
            meeting_gid: f.meeting.clone(),
            t0_ms: 1000,
            t1_ms: 1800,
            transcript_version: version,
        };
        store
            .add_note_block(
                &f.meeting,
                NewNoteBlock {
                    kind: "decision".into(),
                    body: "b".into(),
                    provenance: ghi_store::store::Provenance::Ai,
                    anchors: vec![anchor.clone()],
                    pinned: false,
                },
            )
            .unwrap();
        store
            .add_action_item(
                &f.meeting,
                NewActionItem {
                    text: "do".into(),
                    owner_speaker_gid: Some(f.sp[1].clone()),
                    ..Default::default()
                },
            )
            .unwrap();
        store.rename_speaker(&f.sp[1], Some("Lan")).unwrap();
        let version_of = |s: &Store| s.get_meeting(&f.meeting).unwrap().transcript_version;
        merge_speakers_in(&f.core, &f.meeting, &f.sp[1], &f.sp[2]).unwrap();
        assert_eq!(lines_of(&store, &f.meeting, &f.sp[2]).len(), 4);
        assert!(lines_of(&store, &f.meeting, &f.sp[1]).is_empty());
        assert_eq!(version_of(&store), version);
        let list = speakers_of(&store, &f.meeting).unwrap();
        assert_eq!(list.len(), 2, "the merged speaker is hidden");
        assert_eq!(list.iter().find(|s| s.gid == f.sp[2]).unwrap().lines, 4);
        // The name goes with the merged speaker; the survivor keeps its own.
        assert!(list.iter().all(|s| s.name.is_none()));
        assert_eq!(
            store.action_items(&f.meeting).unwrap()[0]
                .owner_speaker_gid
                .as_deref(),
            Some(f.sp[2].as_str())
        );
        // The citation still resolves to the same lines.
        let note = &store.note_blocks(&f.meeting).unwrap()[0];
        assert_eq!(note.anchors, vec![anchor.clone()]);
        let r = store.resolve_anchor(&anchor).unwrap();
        assert_eq!(r.segments.len(), 1);
        assert_eq!(r.segments[0].speaker_gid.as_deref(), Some(f.sp[2].as_str()));
        // Errors.
        let e = |from: &str, into: &str| merge_speakers_in(&f.core, &f.meeting, from, into);
        assert_eq!(e(&f.sp[2], &f.sp[2]).unwrap_err(), SAME_SPEAKER);
        assert_eq!(
            e(&f.sp[1], &f.sp[0]).unwrap_err(),
            NOT_A_SPEAKER,
            "already merged"
        );
        assert_eq!(e("nope", &f.sp[0]).unwrap_err(), NOT_A_SPEAKER);
    }

    #[test]
    fn merging_me_carries_me_unless_a_call_pins_it_to_the_mic() {
        // A room: Me merged into another speaker makes that speaker Me.
        let f = fix("room", true);
        let store = f.core.store().unwrap();
        merge_speakers_in(&f.core, &f.meeting, &f.sp[0], &f.sp[1]).unwrap();
        let list = speakers_of(&store, &f.meeting).unwrap();
        assert!(list.iter().find(|s| s.gid == f.sp[1]).unwrap().is_me);
        assert_eq!(list.iter().filter(|s| s.is_me).count(), 1);
        // Someone merged into Me leaves Me as it was.
        merge_speakers_in(&f.core, &f.meeting, &f.sp[2], &f.sp[1]).unwrap();
        assert!(speakers_of(&store, &f.meeting).unwrap()[0].is_me);
        // A call with a far side: Me stays on the mic speaker.
        let c = fix("call", true);
        let cs = c.core.store().unwrap();
        cs.open_track(&c.meeting, ghi_store::store::TrackKind::Mic)
            .unwrap();
        cs.open_track(&c.meeting, ghi_store::store::TrackKind::System)
            .unwrap();
        assert_eq!(
            merge_speakers_in(&c.core, &c.meeting, &c.sp[0], &c.sp[1]).unwrap_err(),
            FAR_SIDE
        );
        // Nor are far-side lines credited to Me.
        assert_eq!(
            merge_speakers_in(&c.core, &c.meeting, &c.sp[1], &c.sp[0]).unwrap_err(),
            FAR_SIDE
        );
        assert!(speakers_of(&cs, &c.meeting).unwrap()[0].is_me);
    }

    #[test]
    fn split_moves_exactly_the_chosen_lines_to_a_new_unnamed_speaker() {
        let f = fix("room", false);
        let store = f.core.store().unwrap();
        // By gids.
        let new =
            split_speaker_in(&f.core, &f.meeting, &f.sp[1], vec![f.seg[2].clone()], None).unwrap();
        assert_eq!(lines_of(&store, &f.meeting, &new), vec![f.seg[2].clone()]);
        assert_eq!(
            lines_of(&store, &f.meeting, &f.sp[1]),
            vec![f.seg[1].clone(), f.seg[3].clone()]
        );
        let list = speakers_of(&store, &f.meeting).unwrap();
        let n = list.iter().find(|s| s.gid == new).unwrap();
        assert!(n.name.is_none() && !n.is_me && n.lines == 1);
        assert_eq!(n.number, 4, "Speaker 4");
        assert_eq!(n.color_slot, 4, "the first free color");
        // From a line on.
        let tail = split_speaker_in(
            &f.core,
            &f.meeting,
            &f.sp[1],
            vec![],
            Some(f.seg[3].clone()),
        )
        .unwrap();
        assert_eq!(lines_of(&store, &f.meeting, &tail), vec![f.seg[3].clone()]);
        assert_eq!(
            lines_of(&store, &f.meeting, &f.sp[1]),
            vec![f.seg[1].clone()]
        );
        // Errors leave things alone.
        let sp = |segs: Vec<String>, from: Option<String>| {
            split_speaker_in(&f.core, &f.meeting, &f.sp[0], segs, from)
        };
        assert_eq!(sp(vec![], None).unwrap_err(), NOTHING_TO_SPLIT);
        assert_eq!(
            sp(vec![f.seg[1].clone()], None).unwrap_err(),
            NOTHING_TO_SPLIT,
            "not theirs"
        );
        assert_eq!(
            sp(vec![f.seg[0].clone()], Some(f.seg[0].clone())).unwrap_err(),
            NOTHING_TO_SPLIT,
            "both ways"
        );
        assert_eq!(sp(vec![f.seg[0].clone()], None).unwrap_err(), WHOLE_SPEAKER);
        assert_eq!(
            sp(vec![], Some(f.seg[0].clone())).unwrap_err(),
            WHOLE_SPEAKER
        );
        assert_eq!(speakers_of(&store, &f.meeting).unwrap().len(), 5);
    }

    #[test]
    fn splitting_me_leaves_me_with_the_original() {
        let f = fix("room", true);
        let store = f.core.store().unwrap();
        // Give Me a second line to split off.
        store
            .set_segment_speaker(&f.seg[1], Some(&f.sp[0]))
            .unwrap();
        let new =
            split_speaker_in(&f.core, &f.meeting, &f.sp[0], vec![f.seg[1].clone()], None).unwrap();
        let list = speakers_of(&store, &f.meeting).unwrap();
        assert!(list.iter().find(|s| s.gid == f.sp[0]).unwrap().is_me);
        assert!(!list.iter().find(|s| s.gid == new).unwrap().is_me);
    }

    #[test]
    fn not_a_person_leaves_the_people_and_the_unnamed_counts() {
        let f = fix("room", true);
        let store = f.core.store().unwrap();
        let unnamed = |s: &Store| {
            s.unnamed_voice_counts(std::slice::from_ref(&f.meeting))
                .unwrap()
                .get(&f.meeting)
                .copied()
                .unwrap_or(0)
        };
        assert_eq!(unnamed(&store), 2);
        set_not_person_in(&f.core, &f.meeting, &f.sp[2], true).unwrap();
        assert_eq!(unnamed(&store), 1);
        assert!(speakers_of(&store, &f.meeting).unwrap()[2].not_person);
        // A named one: its person goes with it, and comes back by the name.
        store.rename_speaker(&f.sp[1], Some("Lan")).unwrap();
        let has_lan = |s: &Store| s.people_overview().unwrap().iter().any(|p| p.name == "Lan");
        assert!(has_lan(&store));
        set_not_person_in(&f.core, &f.meeting, &f.sp[1], true).unwrap();
        assert!(!has_lan(&store));
        set_not_person_in(&f.core, &f.meeting, &f.sp[1], false).unwrap();
        assert!(has_lan(&store));
        set_not_person_in(&f.core, &f.meeting, &f.sp[2], false).unwrap();
        assert_eq!(unnamed(&store), 1, "Lan is named, the other is back");
        // Me is a person.
        assert_eq!(
            set_not_person_in(&f.core, &f.meeting, &f.sp[0], true).unwrap_err(),
            IS_ME
        );
        assert_eq!(
            set_not_person_in(&f.core, &f.meeting, "nope", true).unwrap_err(),
            NOT_A_SPEAKER
        );
    }

    #[test]
    fn edits_are_refused_while_locked_and_change_nothing() {
        let f = fix("room", false);
        f.core.set_locked(true);
        let locked = "the app is locked";
        assert_eq!(
            merge_speakers_in(&f.core, &f.meeting, &f.sp[1], &f.sp[2]).unwrap_err(),
            locked
        );
        assert_eq!(
            split_speaker_in(&f.core, &f.meeting, &f.sp[1], vec![f.seg[1].clone()], None)
                .unwrap_err(),
            locked
        );
        assert_eq!(
            set_not_person_in(&f.core, &f.meeting, &f.sp[1], true).unwrap_err(),
            locked
        );
        f.core.set_locked(false);
        let store = f.core.store().unwrap();
        assert_eq!(speakers_of(&store, &f.meeting).unwrap().len(), 3);
        assert_eq!(lines_of(&store, &f.meeting, &f.sp[1]).len(), 3);
    }

    #[test]
    fn stored_edits_emit_no_live_events() {
        // The panel reloads the meeting when the command returns; the speaker
        // events (with the session's numeric ids) are for the live session.
        let f = fix("room", false);
        merge_speakers_in(&f.core, &f.meeting, &f.sp[1], &f.sp[2]).unwrap();
        set_not_person_in(&f.core, &f.meeting, &f.sp[2], true).unwrap();
        assert!(f.rx.try_recv().is_err());
    }

    #[test]
    fn merging_me_into_a_not_person_makes_it_a_person_and_split_keeps_the_kind() {
        let f = fix("room", true);
        let store = f.core.store().unwrap();
        let np = |gid: &str| {
            store
                .speakers(&f.meeting)
                .unwrap()
                .into_iter()
                .find(|s| s.gid == gid)
                .unwrap()
                .not_person
        };
        // Lines split off a "not a person" speaker are not a person either.
        set_not_person_in(&f.core, &f.meeting, &f.sp[1], true).unwrap();
        let new =
            split_speaker_in(&f.core, &f.meeting, &f.sp[1], vec![f.seg[3].clone()], None).unwrap();
        assert!(np(&new));
        // Me merged into it: Me is a person.
        merge_speakers_in(&f.core, &f.meeting, &f.sp[0], &f.sp[1]).unwrap();
        assert!(!np(&f.sp[1]));
        assert_eq!(
            set_not_person_in(&f.core, &f.meeting, &f.sp[1], true).unwrap_err(),
            IS_ME
        );
    }
}
