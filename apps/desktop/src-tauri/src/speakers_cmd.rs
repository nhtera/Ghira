// SPDX-License-Identifier: Apache-2.0
//! Speakers of a stored meeting ("Name your speakers" after processing, D5):
//! the list with a representative span for the 3 s sample, and renaming
//! outside a recording (the live session has its own commands).

use serde::Serialize;
use specta::Type;

use crate::core::Core;
use crate::{CoreState, blocking};

/// Error codes of the voice commands (the UI turns them into words).
pub(crate) const BUSY_RECORDING: &str = "busyRecording";
/// The meeting is the one being recorded: use the live speaker controls.
pub(crate) const LIVE_MEETING: &str = "liveMeeting";
pub(crate) const NOT_A_SPEAKER: &str = "notASpeaker";
/// In a call only the mic speaker is Me.
pub(crate) const FAR_SIDE: &str = "farSide";
pub(crate) const NOT_ME: &str = "notMe";
pub(crate) const NO_SUGGESTION: &str = "noSuggestion";
/// Other people's voice profiles are off in this build.
pub(crate) const THIRD_PARTY_OFF: &str = "thirdPartyOff";
pub(crate) const NOT_NAMED: &str = "notNamed";
pub(crate) const NO_VOICE: &str = "noVoice";
pub(crate) const INVALID_CONSENT: &str = "invalidConsent";
pub(crate) const STORAGE: &str = "storage";

/// A store failure as a code (the detail goes to the log, never the UI).
pub(crate) fn storage(e: impl std::fmt::Display) -> String {
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
                STORAGE
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
                "storage"
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
}
