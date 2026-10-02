// SPDX-License-Identifier: Apache-2.0
//! People (phase 14c, D8): the list, one person's detail, merging two people,
//! deleting voice data, and "remove name from notes". Errors are codes the UI
//! turns into words; names never go to logs.
//!
//! Deleting voice data and removing a name are separate on purpose: each
//! changes only its own side (the voice profile, or the name in meetings).

use std::collections::HashSet;

use serde::Serialize;
use specta::Type;

use crate::core::Core;
use crate::speakers_cmd::{BUSY_RECORDING, storage};
use crate::{CoreState, blocking};

/// The person does not exist (any more).
pub(crate) const NOT_FOUND: &str = "notFound";
/// Me can't be merged away or have a name removed.
pub(crate) const IS_ME: &str = "isMe";
/// Both sides of a merge are the same person.
pub(crate) const SAME_PERSON: &str = "samePerson";
/// There is no voice profile to delete.
pub(crate) const NO_PROFILE: &str = "noProfile";
/// Other people's voice profiles are off in this build.
pub(crate) const THIRD_PARTY_OFF: &str = crate::speakers_cmd::THIRD_PARTY_OFF;

const MEETINGS_SHOWN: usize = 50;
const SAMPLES_SHOWN: usize = 10;

/// Whose voice is kept, and how they agreed.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PersonVoice {
    /// `self` (Me, own consent), `agreed` (someone else, their consent) or
    /// `none` (no voice profile).
    pub kind: String,
    /// When they agreed (unix ms); `None` for `none`.
    pub at_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PersonRow {
    pub gid: String,
    /// Empty for Me: show "Me".
    pub name: String,
    pub is_me: bool,
    /// Palette slot 0..8.
    pub color_slot: u32,
    pub meetings: u32,
    pub open_actions: u32,
    pub last_met_ms: Option<f64>,
    pub voice: PersonVoice,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PeopleList {
    /// Me first, then by most recent meeting.
    pub people: Vec<PersonRow>,
    /// Other people's voice profiles are on (always off in this build): the
    /// unknown-voices queue and "Save a voice profile" stay hidden without it.
    pub third_party: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PersonMeeting {
    pub gid: String,
    pub title: String,
    pub started_at: f64,
    pub duration_ms: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PersonAction {
    pub gid: String,
    pub meeting_gid: String,
    pub meeting_title: String,
    pub text: String,
    pub due: Option<f64>,
    pub due_text: Option<String>,
}

/// A span of the meeting where this voice was heard and learned from (its
/// audio is still kept). Play it with `issue_audio_sample(meetingGid, t0Ms,
/// t1Ms, null)`.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSample {
    pub meeting_gid: String,
    pub meeting_title: String,
    pub t0_ms: f64,
    pub t1_ms: f64,
    /// The track the span is on: `0` (mic) for Me's samples, `null` for the
    /// meeting's diarized track. Pass it to `issue_audio_sample`.
    pub track: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PersonDetail {
    pub person: PersonRow,
    /// Newest first, at most 50.
    pub meetings: Vec<PersonMeeting>,
    pub open_actions: Vec<PersonAction>,
    /// Up to 10, newest first; only those whose audio is still kept.
    pub samples: Vec<VoiceSample>,
}

fn row(p: ghi_store::people::PersonOverview) -> PersonRow {
    let voice = match &p.voice {
        Some(v) => PersonVoice {
            kind: if p.is_me { "self" } else { "agreed" }.into(),
            at_ms: Some(v.consent_at_ms as f64),
        },
        None => PersonVoice {
            kind: "none".into(),
            at_ms: None,
        },
    };
    PersonRow {
        gid: p.gid,
        name: p.name,
        is_me: p.is_me,
        color_slot: p.color_slot.clamp(0, 8) as u32,
        meetings: p.meetings.max(0) as u32,
        open_actions: p.open_actions.max(0) as u32,
        last_met_ms: p.last_met_ms.map(|t| t as f64),
        voice,
    }
}

/// The content is only for an unlocked app: checked again just before it is
/// returned, in case the lock came on while the store was read.
fn still_unlocked(c: &Core) -> Result<(), String> {
    if c.locked() {
        return Err("the app is locked".into());
    }
    Ok(())
}

fn find(store: &ghi_store::store::Store, gid: &str) -> Result<PersonRow, String> {
    match store.person(gid) {
        Ok(p) => Ok(row(p)),
        Err(ghi_store::StoreError::NotFound { .. }) => Err(NOT_FOUND.into()),
        Err(e) => Err(storage(e)),
    }
}

/// Everyone with a name in a meeting, plus Me. Errors: `storage`.
#[tauri::command]
#[specta::specta]
pub async fn list_people(core: CoreState<'_>) -> Result<PeopleList, String> {
    blocking(&core, |c| {
        let store = c.store()?;
        let mut people: Vec<PersonRow> = store
            .people_overview()
            .map_err(storage)?
            .into_iter()
            .map(row)
            .collect();
        people.sort_by(|a, b| {
            b.is_me
                .cmp(&a.is_me)
                .then(
                    b.last_met_ms
                        .partial_cmp(&a.last_met_ms)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        still_unlocked(c)?;
        Ok(PeopleList {
            people,
            third_party: crate::system::third_party_token(&store).is_some(),
        })
    })
    .await
}

/// One person: their meetings, open actions and the voice samples. Errors:
/// `notFound`, `storage`.
#[tauri::command]
#[specta::specta]
pub async fn person_detail(core: CoreState<'_>, gid: String) -> Result<PersonDetail, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let person = find(&store, &gid)?;
        let meetings = store
            .person_meetings(&gid, MEETINGS_SHOWN)
            .map_err(storage)?
            .into_iter()
            .map(|m| PersonMeeting {
                gid: m.gid,
                title: m.title,
                started_at: m.started_at as f64,
                duration_ms: m.duration_ms as f64,
            })
            .collect();
        let open_actions = store
            .person_open_actions(&gid)
            .map_err(storage)?
            .into_iter()
            .map(|a| PersonAction {
                gid: a.gid,
                meeting_gid: a.meeting_gid,
                meeting_title: a.meeting_title,
                text: a.text,
                due: a.due.map(|d| d as f64),
                due_text: a.due_text,
            })
            .collect();
        // Someone else's samples only while their profiles are allowed.
        let samples = if person.is_me || crate::system::third_party_token(&store).is_some() {
            samples_of(&store, &gid, person.is_me)?
        } else {
            Vec::new()
        };
        still_unlocked(c)?;
        Ok(PersonDetail {
            person,
            meetings,
            open_actions,
            samples,
        })
    })
    .await
}

/// Where the profile's exemplars came from, for the meetings whose audio is
/// still kept.
fn samples_of(
    store: &ghi_store::store::Store,
    person: &str,
    is_me: bool,
) -> Result<Vec<VoiceSample>, String> {
    let Some(profile) = store.voice_profile(person).map_err(storage)? else {
        return Ok(Vec::new());
    };
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for src in profile
        .sets
        .iter()
        .flat_map(|s| s.exemplars.iter().rev())
        .filter_map(|e| e.source.as_ref())
    {
        if !seen.insert((src.meeting_gid.clone(), src.t0_ms)) {
            continue;
        }
        if !store.audio_available(&src.meeting_gid).unwrap_or(false) {
            continue;
        }
        let Ok(m) = store.get_meeting(&src.meeting_gid) else {
            continue;
        };
        out.push(VoiceSample {
            meeting_gid: m.gid,
            meeting_title: m.title,
            t0_ms: src.t0_ms as f64,
            t1_ms: src.t1_ms as f64,
            // Me is heard on the mic, in a call too.
            track: is_me.then_some(0),
        });
        if out.len() == SAMPLES_SHOWN {
            break;
        }
    }
    Ok(out)
}

/// Merges `from` into `into`: their meetings and voice data become one
/// person's, `from` is gone. Not while recording. Errors: `busyRecording`,
/// `notFound`, `isMe`, `samePerson`, `storage`.
#[tauri::command]
#[specta::specta]
pub async fn merge_people(core: CoreState<'_>, from: String, into: String) -> Result<(), String> {
    blocking(&core, move |c| {
        if c.busy() {
            return Err(BUSY_RECORDING.into());
        }
        let store = c.store()?;
        let token = check_merge(&store, &from, &into)?;
        let affected = store.merge_persons(&from, &into, token).map_err(storage)?;
        ghi_core::voice_job::requeue_index(&store, &affected).map_err(storage)?;
        c.notify_jobs();
        Ok(())
    })
    .await
}

/// The rules of a merge, before anything changes: two different people, not
/// Me; and when either has a voice profile (so it is someone else's) the
/// third-party proof, which is `None` while those are off.
pub(crate) fn check_merge(
    store: &ghi_store::store::Store,
    from: &str,
    into: &str,
) -> Result<Option<ghi_store::voice::ThirdPartyApproved>, String> {
    if from == into {
        return Err(SAME_PERSON.into());
    }
    let (a, b) = (find_overview(store, from)?, find_overview(store, into)?);
    if a.is_me || b.is_me {
        return Err(IS_ME.into());
    }
    if a.voice.is_none() && b.voice.is_none() {
        return Ok(None);
    }
    crate::system::third_party_token(store)
        .map(Some)
        .ok_or_else(|| THIRD_PARTY_OFF.to_string())
}

fn find_overview(
    store: &ghi_store::store::Store,
    gid: &str,
) -> Result<ghi_store::people::PersonOverview, String> {
    match store.person(gid) {
        Ok(p) => Ok(p),
        Err(ghi_store::StoreError::NotFound { .. }) => Err(NOT_FOUND.into()),
        Err(e) => Err(storage(e)),
    }
}

/// Deletes the person's voice profile (a crypto-shred). Names in meetings
/// stay. Errors: `notFound`, `noProfile`, `storage`.
#[tauri::command]
#[specta::specta]
pub async fn delete_voice_data(core: CoreState<'_>, gid: String) -> Result<(), String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let p = store.person(&gid).map_err(|e| match e {
            ghi_store::StoreError::NotFound { .. } => NOT_FOUND.to_string(),
            e => storage(e),
        })?;
        let profile = p.voice.ok_or(NO_PROFILE)?;
        store
            .delete_voice_profile(&profile.profile_gid)
            .map_err(storage)
    })
    .await
}

/// Removes the person's name from every meeting: speaker names go back to
/// "Speaker N", and the name is replaced in AI-written notes (what the user
/// wrote is left). The voice profile stays. Returns how many meetings
/// changed. Not while recording. Errors: `busyRecording`, `notFound`, `isMe`,
/// `storage`.
#[tauri::command]
#[specta::specta]
pub async fn remove_person_name(core: CoreState<'_>, gid: String) -> Result<u32, String> {
    blocking(&core, move |c| {
        if c.busy() {
            return Err(BUSY_RECORDING.into());
        }
        let store = c.store()?;
        if find(&store, &gid)?.is_me {
            return Err(IS_ME.into());
        }
        let affected = store.remove_person_name(&gid).map_err(storage)?;
        ghi_core::voice_job::requeue_index(&store, &affected).map_err(storage)?;
        c.notify_jobs();
        Ok(affected.len() as u32)
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
    fn rows_say_whose_voice_and_how_they_agreed() {
        let (_t, s) = open();
        let m = s
            .create_meeting(NewMeeting {
                title: "Planning".into(),
                ..Default::default()
            })
            .unwrap()
            .gid;
        let sp = s
            .add_speaker(
                &m,
                NewSpeaker {
                    label_idx: 0,
                    color_slot: 2,
                    ..Default::default()
                },
            )
            .unwrap();
        s.rename_speaker(&sp, Some("Lan")).unwrap();
        let me = s.me_person().unwrap();
        s.put_voice_profile(
            &me,
            &ghi_core::voice_job::self_consent("onboarding.voice.consent_mac"),
            None,
            ghi_core::profiles::VOICE_MODEL,
            vec![(
                "any".into(),
                vec![ghi_store::voice::VoiceExemplar {
                    vec: vec![1.0, 0.0],
                    source: None,
                }],
            )],
            None,
        )
        .unwrap();
        let rows: Vec<PersonRow> = s.people_overview().unwrap().into_iter().map(row).collect();
        let lan = rows.iter().find(|r| r.name == "Lan").unwrap();
        assert_eq!(
            (lan.meetings, lan.color_slot, lan.voice.kind.as_str()),
            (1, 2, "none")
        );
        assert!(lan.voice.at_ms.is_none());
        let me = rows.iter().find(|r| r.is_me).unwrap();
        assert_eq!(me.voice.kind, "self");
        assert!(me.voice.at_ms.is_some());
        assert_eq!(me.name, "");
        // The detail's error mapping.
        assert_eq!(find(&s, "nobody").unwrap_err(), NOT_FOUND);
    }

    #[test]
    fn merging_people_with_a_voice_profile_is_refused_while_third_party_is_off() {
        let (_t, s) = open();
        let ok = ghi_store::voice::ThirdPartyApproved::assert_flag_checked();
        let a = s.add_person("Lan", 1).unwrap();
        let b = s.add_person("Lan B", 2).unwrap();
        let me = s.me_person().unwrap();
        assert_eq!(check_merge(&s, &a, &a).unwrap_err(), SAME_PERSON);
        assert_eq!(check_merge(&s, &me, &a).unwrap_err(), IS_ME);
        assert_eq!(check_merge(&s, &a, "nobody").unwrap_err(), NOT_FOUND);
        assert!(
            check_merge(&s, &a, &b).unwrap().is_none(),
            "no profile: no proof"
        );
        s.put_voice_profile(
            &a,
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
        assert_eq!(check_merge(&s, &a, &b).unwrap_err(), THIRD_PARTY_OFF);
        assert_eq!(check_merge(&s, &b, &a).unwrap_err(), THIRD_PARTY_OFF);
        // The store itself refuses too.
        assert!(s.merge_persons(&b, &a, None).is_err());
    }

    #[test]
    fn samples_need_audio_that_is_still_kept() {
        let (_t, s) = open();
        let m = s
            .create_meeting(NewMeeting {
                title: "x".into(),
                ..Default::default()
            })
            .unwrap()
            .gid;
        let me = s.me_person().unwrap();
        s.put_voice_profile(
            &me,
            &ghi_core::voice_job::self_consent("k"),
            None,
            ghi_core::profiles::VOICE_MODEL,
            vec![(
                "any".into(),
                vec![ghi_store::voice::VoiceExemplar {
                    vec: vec![1.0, 0.0],
                    source: Some(ghi_store::voice::ExemplarSource {
                        meeting_gid: m,
                        t0_ms: 0,
                        t1_ms: 1000,
                    }),
                }],
            )],
            None,
        )
        .unwrap();
        // No audio was recorded for it: nothing to play.
        assert!(samples_of(&s, &me, true).unwrap().is_empty());
    }
}
