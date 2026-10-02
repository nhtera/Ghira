// SPDX-License-Identifier: Apache-2.0
//! Voice profile jobs (phase 14c): `voice_learn` adds speakers' voices from a
//! meeting to their profiles after the user confirmed who they are ("This is
//! me", an accepted suggestion, a saved voice); [`enroll_from_pcm`] makes Me's
//! profile from a recording, with consent.
//!
//! One job per meeting: payload `{meeting, speakers: [gid, ...]}`; asking again
//! while it is queued merges into it. Consent and the third-party flag are
//! checked again when the job runs: a profile deleted or a flag turned off
//! since it was queued means nothing is learned. The windows are chosen from
//! the stored lines first; no windows, or no stored audio (retention ended),
//! is a finished job, not an error, and nothing is decoded for it.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use ghi_store::jobs::JobState;
use ghi_store::store::{Segment, Speaker, Store, TrackKind};
use ghi_store::voice::{ExemplarSource, VoiceConsent, VoiceExemplar, VoiceProfile};
use serde_json::{Value, json};

use crate::jobs::{JobCtx, JobHandler, Outcome, Ready};
use crate::profiles::{
    ANY_LANG, LINE_GAP_MS, Span, VOICE_MODEL, VoiceEmbed, VoiceFactory, embed_windows,
    has_exemplar_from, majority_lang, pick_windows, pick_windows_merging, range_ms,
};
use crate::session::JOB_PAYLOAD_VERSION;
use crate::voice_step::{ThirdPartyGate, learn_me};

pub const VOICE_LEARN_JOB: &str = "voice_learn";

/// An enrolled passage needs this much speech (samples at 16 kHz) ...
const ENROLL_MIN_SPEECH: usize = 16_000 * 10;
/// ... in at least this many windows.
const ENROLL_MIN_WINDOWS: usize = 3;

pub struct VoiceLearnJob {
    pub embedder: VoiceFactory,
    /// The speaker model is installed (else the job waits).
    pub ready: Ready,
    pub third_party: ThirdPartyGate,
}

fn speakers_of(payload: &Value) -> Vec<String> {
    let mut out: Vec<String> = payload
        .get("speakers")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if let Some(s) = payload.get("speaker").and_then(Value::as_str) {
        out.push(s.to_string());
    }
    out
}

/// Queues learning from `speaker` of `meeting`. A job already waiting for the
/// meeting takes the speaker in (one decode per meeting).
pub fn queue_learn(store: &Store, meeting: &str, speaker: &str) -> Result<(), String> {
    let err = |e: ghi_store::StoreError| e.to_string();
    let queued = store.active_jobs().map_err(err)?.into_iter().find(|j| {
        j.kind == VOICE_LEARN_JOB
            && j.state == JobState::Queued
            && j.meeting_gid.as_deref() == Some(meeting)
    });
    let mut speakers = vec![];
    if let Some(j) = &queued {
        speakers = speakers_of(&j.payload);
        if speakers.iter().any(|s| s == speaker) {
            return Ok(());
        }
    }
    speakers.push(speaker.to_string());
    if let Some(j) = queued {
        store.cancel_job(j.id).map_err(err)?;
    }
    store
        .enqueue_job(
            Some(meeting),
            VOICE_LEARN_JOB,
            JOB_PAYLOAD_VERSION,
            &json!({"meeting": meeting, "speakers": speakers}),
        )
        .map_err(err)?;
    Ok(())
}

/// "This is me": marks the speaker as Me (the store refuses a far-side speaker
/// in a call, and drops Me's exemplars of this meeting if Me moved) and queues
/// learning from it.
pub fn set_me(store: &Store, meeting: &str, speaker: &str) -> Result<(), String> {
    store.set_speaker_me(speaker).map_err(|e| e.to_string())?;
    queue_learn(store, meeting, speaker)
}

/// Requeues semantic indexing for meetings whose speaker names changed
/// (`remove_person_name`, `merge_persons` return them).
pub fn requeue_index(store: &Store, meetings: &[String]) -> Result<(), String> {
    meetings
        .iter()
        .try_for_each(|m| crate::index_job::queue_one(store, m))
}

/// The wall-clock time as a consent record's `at_ms`.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Consent given by ticking the "learn my voice" box, shown with `text_key`
/// (the locale key of the checkbox text, e.g. `onboarding.voice.consent_mac`).
pub fn self_consent(text_key: &str) -> VoiceConsent {
    VoiceConsent {
        method: "self_checkbox".into(),
        at_ms: now_ms(),
        text_key: text_key.into(),
        clip: None,
    }
}

#[derive(Debug)]
pub enum EnrollError {
    /// Not the user's own consent.
    NoConsent,
    /// Under 10 s of speech or 3 windows.
    TooLittleSpeech,
    /// The speaker model failed.
    Model(String),
    Store(ghi_store::StoreError),
}

impl std::fmt::Display for EnrollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnrollError::NoConsent => {
                f.write_str("a voice profile for Me needs the user's own consent")
            }
            EnrollError::TooLittleSpeech => {
                f.write_str("too little speech to learn a voice from: read for 10-25 s")
            }
            EnrollError::Model(e) => write!(f, "speaker model: {e}"),
            EnrollError::Store(e) => write!(f, "store: {e}"),
        }
    }
}

impl std::error::Error for EnrollError {}

/// Creates (or replaces) Me's voice profile from `pcm` (16 kHz mono, a spoken
/// passage of at least 10 s of speech in 3 windows): each 2/4/6 s window is an
/// exemplar, in language `any`. `consent` is the evidence the user agreed; no
/// consent, nothing is stored. Returns the profile gid.
pub fn enroll_from_pcm(
    store: &Store,
    embedder: &mut dyn VoiceEmbed,
    pcm: &[f32],
    consent: &VoiceConsent,
) -> Result<String, EnrollError> {
    if consent.method != "self_checkbox" {
        return Err(EnrollError::NoConsent);
    }
    let whole = [Span {
        who: 0,
        t0_ms: 0,
        t1_ms: (pcm.len() * 1000 / 16_000) as i64,
    }];
    let mut exemplars = Vec::new();
    let mut speech = 0;
    for w in pick_windows(&whole, 0, pcm.len()) {
        let Some(v) = embed_windows(embedder, pcm, std::slice::from_ref(&w), &|| false)
            .map_err(EnrollError::Model)?
        else {
            continue;
        };
        speech += v.speech;
        // A recording is not a meeting: no source to replay.
        exemplars.push(VoiceExemplar {
            vec: v.vec.clone(),
            source: None,
        });
    }
    if exemplars.len() < ENROLL_MIN_WINDOWS || speech < ENROLL_MIN_SPEECH {
        return Err(EnrollError::TooLittleSpeech);
    }
    let me = store.me_person().map_err(EnrollError::Store)?;
    store
        .put_voice_profile(
            &me,
            consent,
            None,
            VOICE_MODEL,
            vec![(ANY_LANG.to_string(), exemplars)],
            None,
        )
        .map_err(EnrollError::Store)
}

/// One speaker to learn: who, their profile, and where their voice is.
struct Work {
    speaker: String,
    profile: VoiceProfile,
    is_me: bool,
    lang: Option<String>,
    kind: TrackKind,
    windows: Vec<Range<usize>>,
}

/// The track a speaker's voice is on, and whether the meeting is a call whose
/// sides sit on separate tracks.
fn track_of(
    tracks: &[(TrackKind, u32)],
    call_mode: bool,
    is_me: bool,
) -> (Option<TrackKind>, bool) {
    let has = |k: TrackKind| tracks.iter().any(|(t, _)| *t == k);
    let two_sided = call_mode && has(TrackKind::System);
    let kind = if two_sided && !is_me {
        Some(TrackKind::System)
    } else if has(TrackKind::Mic) {
        Some(TrackKind::Mic)
    } else if has(TrackKind::File) {
        Some(TrackKind::File)
    } else {
        None
    };
    (kind, two_sided)
}

/// Where only `sp` talks, from the stored lines. In a two-sided call a far-side
/// speaker is on the system track, so Me's lines are not "others" there; for
/// Me the far side's lines are (their voice bleeds into the mic).
fn windows_from_lines(
    speakers: &[Speaker],
    segments: &[Segment],
    sp: &Speaker,
    two_sided: bool,
) -> Vec<Range<usize>> {
    let idx: HashMap<&str, (u32, bool)> = speakers
        .iter()
        .enumerate()
        .map(|(i, s)| (s.gid.as_str(), (i as u32, s.is_me)))
        .collect();
    let target = idx[sp.gid.as_str()].0;
    let spans: Vec<Span> = segments
        .iter()
        .filter_map(|s| {
            let &(who, is_me) = idx.get(s.speaker_gid.as_deref()?)?;
            if two_sided && !sp.is_me && is_me {
                return None;
            }
            Some(Span {
                who,
                t0_ms: s.t0_ms,
                t1_ms: s.t1_ms,
            })
        })
        .collect();
    let end = spans.iter().map(|s| s.t1_ms).max().unwrap_or(0).max(0) as usize * 16;
    pick_windows_merging(&spans, target, end, LINE_GAP_MS)
}

impl JobHandler for VoiceLearnJob {
    fn kind(&self) -> &'static str {
        VOICE_LEARN_JOB
    }

    fn ready(&self) -> bool {
        (self.ready)()
    }

    fn run(&self, ctx: &JobCtx) -> Result<Outcome, String> {
        let err = |e: ghi_store::StoreError| e.to_string();
        let store: &Arc<Store> = ctx.store;
        let meeting = ctx.meeting()?.to_string();
        let wanted = speakers_of(&ctx.job.payload);
        let speakers = store.speakers(&meeting).map_err(err)?;
        let segments = store.segments(&meeting).map_err(err)?;
        let m = store.get_meeting(&meeting).map_err(err)?;
        let tracks = store.tracks(&meeting).map_err(err)?;
        let token = (self.third_party)();
        let me_profile = store.me_voice_profile(VOICE_MODEL).map_err(err)?;
        let others = match token {
            Some(t) => store
                .third_party_voice_profiles(VOICE_MODEL, t)
                .map_err(err)?,
            None => Vec::new(),
        };

        // Who can be learned, and where their voice is: no audio is read yet.
        let mut work: Vec<Work> = Vec::new();
        for gid in &wanted {
            let Some(sp) = speakers
                .iter()
                .find(|s| &s.gid == gid && s.merged_into.is_none())
            else {
                continue;
            };
            // Consent and the flag, as they are now.
            let profile = if sp.is_me {
                me_profile.clone()
            } else {
                sp.person_gid
                    .as_deref()
                    .and_then(|p| others.iter().find(|o| o.person_gid == p).cloned())
            };
            let Some(profile) = profile.filter(|p| !has_exemplar_from(p, &meeting)) else {
                continue;
            };
            let (kind, two_sided) = track_of(&tracks, m.mode == "call", sp.is_me);
            let Some(kind) = kind else { continue };
            let windows = windows_from_lines(&speakers, &segments, sp, two_sided);
            if windows.is_empty() {
                continue;
            }
            let lang = majority_lang(
                segments
                    .iter()
                    .filter(|s| s.speaker_gid.as_deref() == Some(gid.as_str()))
                    .map(|s| s.lang.as_deref()),
            );
            work.push(Work {
                speaker: gid.clone(),
                profile,
                is_me: sp.is_me,
                lang,
                kind,
                windows,
            });
        }
        if work.is_empty() {
            return Ok(Outcome::Done);
        }
        let remaining = |from: &[Work]| {
            Outcome::Yield(json!({
                "meeting": meeting,
                "speakers": from.iter().map(|w| w.speaker.clone()).collect::<Vec<_>>(),
            }))
        };
        if ctx.preempted() {
            return Ok(remaining(&work));
        }

        // Decode each track once and keep only the windows' samples.
        let mut samples: HashMap<String, (Vec<f32>, Vec<Range<usize>>)> = HashMap::new();
        for kind in [TrackKind::Mic, TrackKind::System, TrackKind::File] {
            if !work.iter().any(|w| w.kind == kind) {
                continue;
            }
            let Ok(bundle) = store.open_bundle(&meeting, kind) else {
                continue;
            };
            let ogg = bundle.read_all().map_err(err)?;
            let pcm = ghi_audio::encoder::read_ogg_opus(&ogg[..]).map_err(|e| e.to_string())?;
            drop(ogg);
            for w in work.iter().filter(|w| w.kind == kind) {
                let mut cat = Vec::new();
                let mut rebased = Vec::new();
                for r in &w.windows {
                    let Some(part) = pcm.get(r.clone()) else {
                        continue;
                    };
                    rebased.push(cat.len()..cat.len() + part.len());
                    cat.extend_from_slice(part);
                }
                samples.insert(w.speaker.clone(), (cat, rebased));
            }
        }

        let mut embedder = (self.embedder)()?;
        for (i, w) in work.iter().enumerate() {
            let Some((cat, rebased)) = samples.get(&w.speaker) else {
                continue;
            };
            let voice = embed_windows(embedder.as_mut(), cat, rebased, &|| ctx.preempted())?;
            if ctx.preempted() {
                return Ok(remaining(&work[i..]));
            }
            let Some(voice) = voice else { continue };
            // Back to the meeting's timeline for the source of the exemplar.
            let source_window = rebased
                .iter()
                .position(|r| r.start == voice.longest.start)
                .and_then(|k| w.windows.get(k))
                .unwrap_or(&voice.longest);
            if w.is_me {
                let mut v = voice.clone();
                v.longest = source_window.clone();
                learn_me(store, &w.profile, &meeting, &v, w.lang.as_deref())?;
            } else {
                let (t0_ms, t1_ms) = range_ms(source_window);
                store
                    .add_voice_exemplars(
                        &w.profile.gid,
                        VOICE_MODEL,
                        w.lang.as_deref().unwrap_or(ANY_LANG),
                        vec![VoiceExemplar {
                            vec: voice.vec.clone(),
                            source: Some(ExemplarSource {
                                meeting_gid: meeting.clone(),
                                t0_ms,
                                t1_ms,
                            }),
                        }],
                        token,
                    )
                    .map_err(err)?;
            }
        }
        Ok(Outcome::Done)
    }
}
