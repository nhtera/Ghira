// SPDX-License-Identifier: Apache-2.0
//! The voice step of the final pass (phase 14c, D3, D5-D7): after the live
//! speakers are carried over to the final clusters and before the v2
//! transcript is stored, each unnamed cluster's voice is compared with Me's
//! profile (room mode) and, only when the third-party flag is on, with other
//! people's profiles.
//!
//! RT-13: cluster vectors exist only in this function's locals (wiped when
//! dropped). With the flag off nothing but Me's profile is ever written; with
//! it on, the vectors of clusters left unnamed are sealed in `speaker_voices`.
//! Nothing here logs or queues a vector or a name, only counts and error kinds.
//!
//! Order: embed everything first (a recording preempting restarts the pass
//! before anything is written), then write the decisions, each re-checked in
//! its own transaction so a rename the user made meanwhile is never undone.
//! Me's learning is embedded before the decisions; if it is preempted it is
//! skipped and queued as a `voice_learn` job instead.
//!
//! A missing or not-ready model skips the step; a failure inside it is logged
//! (its kind only) and skipped too: the transcript never waits for voices.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ghi_audio::Track;
use ghi_speech::SpeakerSegment;
use ghi_store::store::{NewSegment, Store};
use ghi_store::voice::{ExemplarSource, ThirdPartyApproved, VoiceExemplar, VoiceProfile};

use crate::jobs::{JobCtx, Ready};
use crate::profiles::{
    ANY_LANG, Candidate, ClusterVoice, Decision, Policy, Span, VOICE_MODEL, VoiceFactory, decide,
    embed_windows, has_exemplar_from, majority_lang, me_gate, pick_windows, range_ms, resolve,
    score_profile,
};

/// Hands out the proof that the third-party flag is on (`None`: off).
pub type ThirdPartyGate = Arc<dyn Fn() -> Option<ThirdPartyApproved> + Send + Sync>;

pub struct VoiceStep {
    pub embedder: VoiceFactory,
    /// The speaker model is installed (else the step is skipped).
    pub ready: Ready,
    pub third_party: ThirdPartyGate,
}

/// What the final pass knows when it reaches the voice step.
pub(crate) struct Input<'a> {
    pub meeting: &'a str,
    /// A call with a far-side track (mic = Me, far side diarized).
    pub call: bool,
    /// An imported file: Me is only suggested there.
    pub file_source: bool,
    pub pcm: &'a HashMap<Track, Vec<f32>>,
    pub diar_track: Track,
    pub segs: &'a [SpeakerSegment],
    /// Call mode: the time spans of the mic ("Me") lines.
    pub me_spans: &'a [(i64, i64)],
    /// Final cluster label to speaker gid.
    pub label_gid: &'a HashMap<u32, String>,
    /// The v2 lines about to be stored (for each cluster's language).
    pub v2: &'a [NewSegment],
}

/// `who` of Me's spans on the mic track.
const ME: u32 = u32::MAX;

/// Why the step stopped; never more than a kind.
type Fail = &'static str;

fn ms(s: f64) -> i64 {
    (s * 1000.0).round() as i64
}

/// Adds the cluster's voice to Me's profile (D7) unless the speakerphone
/// guard says it is not Me or this meeting already taught it. Returns whether
/// an exemplar was added.
pub fn learn_me(
    store: &Store,
    me: &VoiceProfile,
    meeting: &str,
    voice: &ClusterVoice,
    lang: Option<&str>,
) -> Result<bool, String> {
    if has_exemplar_from(me, meeting) || !me_gate(me, &voice.vec) {
        return Ok(false);
    }
    let (t0_ms, t1_ms) = range_ms(&voice.longest);
    store
        .add_voice_exemplars(
            &me.gid,
            VOICE_MODEL,
            lang.unwrap_or(ANY_LANG),
            vec![VoiceExemplar {
                vec: voice.vec.clone(),
                source: Some(ExemplarSource {
                    meeting_gid: meeting.to_string(),
                    t0_ms,
                    t1_ms,
                }),
            }],
            None,
        )
        .map_err(|e| e.to_string())?;
    Ok(true)
}

struct Cluster {
    gid: String,
    lang: Option<String>,
    voice: ClusterVoice,
    cands: Vec<Candidate>,
    decision: Decision,
}

impl VoiceStep {
    /// Runs the step. `Ok(false)`: a recording preempted it, start over.
    pub(crate) fn run(&self, ctx: &JobCtx, input: &Input) -> Result<bool, String> {
        if !(self.ready)() {
            log::info!("voice step skipped: the speaker model is not installed");
            return Ok(true);
        }
        match self.matching(ctx, input) {
            Ok(done) => Ok(done),
            Err(kind) => {
                log::warn!("voice step skipped: {kind} error");
                Ok(true)
            }
        }
    }

    fn matching(&self, ctx: &JobCtx, input: &Input) -> Result<bool, Fail> {
        let store: &Store = ctx.store;
        let st = |_: ghi_store::StoreError| "store";
        let token = (self.third_party)();
        // A profile with no vectors has nothing to match or learn from.
        let me_profile = store
            .me_voice_profile(VOICE_MODEL)
            .map_err(st)?
            .filter(|p| !p.sets.is_empty());
        let others = match token {
            Some(t) => store
                .third_party_voice_profiles(VOICE_MODEL, t)
                .map_err(st)?,
            None => Vec::new(),
        };
        let speakers = store.speakers(input.meeting).map_err(st)?;
        let me_speaker = speakers
            .iter()
            .find(|s| s.is_me && s.merged_into.is_none())
            .map(|s| s.gid.clone());
        // Me can be matched in room mode, while nobody is Me yet.
        let match_me = !input.call && me_speaker.is_none() && me_profile.is_some();
        let lang_of = |gid: &str| {
            majority_lang(
                input
                    .v2
                    .iter()
                    .filter(|s| s.speaker_gid.as_deref() == Some(gid))
                    .map(|s| s.lang.as_deref()),
            )
        };
        let diar_pcm = input.pcm.get(&input.diar_track).map_or(&[][..], |v| &v[..]);
        let spans: Vec<Span> = input
            .segs
            .iter()
            .map(|s| Span {
                who: s.speaker,
                t0_ms: ms(s.start),
                t1_ms: ms(s.end),
            })
            .collect();

        // Which work there is: matching, storing unnamed voices, learning Me.
        let mut labels: Vec<(u32, &String)> =
            input.label_gid.iter().map(|(l, g)| (*l, g)).collect();
        labels.sort_by_key(|(l, _)| *l);
        let learn_room = me_profile.is_some()
            && !input.call
            && labels
                .iter()
                .any(|(_, g)| me_speaker.as_deref() == Some(g.as_str()));
        let learn_call = me_profile.is_some() && input.call && !input.me_spans.is_empty();
        let matching = match_me || !others.is_empty() || token.is_some();
        if !(matching || learn_room || learn_call) {
            return Ok(true);
        }
        let mut embedder = (self.embedder)().map_err(|_| "model")?;

        // Each unnamed cluster: its voice and what it matches.
        let mut clusters: Vec<Cluster> = Vec::new();
        if matching {
            for (label, gid) in &labels {
                let Some(sp) = speakers.iter().find(|s| &s.gid == *gid) else {
                    continue;
                };
                let named = sp
                    .display_name
                    .as_deref()
                    .is_some_and(|n| !n.trim().is_empty());
                if sp.merged_into.is_some() || sp.is_me || sp.not_person || named {
                    continue;
                }
                let windows = pick_windows(&spans, *label, diar_pcm.len());
                let voice =
                    embed_windows(embedder.as_mut(), diar_pcm, &windows, &|| ctx.preempted())
                        .map_err(|_| "embed")?;
                if ctx.preempted() {
                    return Ok(false);
                }
                let Some(voice) = voice else { continue };
                let lang = lang_of(gid);
                let cands: Vec<Candidate> = me_profile
                    .iter()
                    .filter(|_| match_me)
                    .chain(others.iter())
                    .filter_map(|p| {
                        let s = score_profile(p, &voice.vec, lang.as_deref())?;
                        Some(Candidate {
                            person_gid: p.person_gid.clone(),
                            score: s.value,
                            same_lang: s.same_lang,
                        })
                    })
                    .collect();
                let decision = decide(&cands);
                clusters.push(Cluster {
                    gid: (*gid).clone(),
                    lang,
                    voice,
                    cands,
                    decision,
                });
            }
            let cands: Vec<Vec<Candidate>> = clusters.iter().map(|c| c.cands.clone()).collect();
            let enough: Vec<bool> = clusters.iter().map(|c| c.voice.enough_to_apply()).collect();
            let mut ds: Vec<Decision> = clusters.iter().map(|c| c.decision.clone()).collect();
            let policy = Policy {
                single_cluster: !input.call && labels.len() == 1,
                me_person: me_profile.as_ref().map(|p| p.person_gid.clone()),
                file_source: input.file_source,
                linked: speakers
                    .iter()
                    .filter(|s| s.merged_into.is_none())
                    .filter_map(|s| s.person_gid.clone())
                    .collect::<HashSet<_>>(),
            };
            resolve(&mut ds, &cands, &enough, &policy);
            for (c, d) in clusters.iter_mut().zip(ds) {
                c.decision = d;
            }
        }

        // D7: Me's voice from the mic lines (call) or a confirmed Me (room),
        // embedded now, written after the decisions. Preempted: queue it.
        let mut me_voice: Option<(ClusterVoice, Option<String>)> = None;
        if let (Some(_), Some(me_gid)) = (&me_profile, &me_speaker) {
            let (pcm, windows) = if input.call {
                let mut sp: Vec<Span> = input
                    .me_spans
                    .iter()
                    .map(|&(t0_ms, t1_ms)| Span {
                        who: ME,
                        t0_ms,
                        t1_ms,
                    })
                    .collect();
                // The far side bleeding into the mic is not Me.
                sp.extend(spans.iter().copied());
                let pcm = input.pcm.get(&Track::Mic).map_or(&[][..], |v| &v[..]);
                (pcm, pick_windows(&sp, ME, pcm.len()))
            } else {
                let label = labels
                    .iter()
                    .find(|(_, g)| g.as_str() == me_gid.as_str())
                    .map(|(l, _)| *l);
                (
                    diar_pcm,
                    label.map_or_else(Vec::new, |l| pick_windows(&spans, l, diar_pcm.len())),
                )
            };
            if !windows.is_empty() {
                let voice = embed_windows(embedder.as_mut(), pcm, &windows, &|| ctx.preempted())
                    .map_err(|_| "embed")?;
                match voice {
                    Some(v) if !ctx.preempted() => me_voice = Some((v, lang_of(me_gid))),
                    _ if ctx.preempted() => {
                        let _ = crate::voice_job::queue_learn(store, input.meeting, me_gid);
                    }
                    _ => {}
                }
            }
        }
        drop(embedder);

        // Decisions: the last thing a recording can preempt. Each write is
        // checked again inside its transaction; one that no longer applies
        // (the user got there first) leaves the cluster alone.
        let (mut applied, mut suggested, mut stored, mut learned) = (0, 0, 0, 0);
        let me_person = me_profile.as_ref().map(|p| p.person_gid.clone());
        for c in &clusters {
            let had_suggestion = speakers
                .iter()
                .any(|s| s.gid == c.gid && s.suggestion.is_some());
            let mut named = false;
            let suggest = |p: &Candidate| {
                store
                    .set_speaker_suggestion_if_unnamed(&c.gid, Some((&p.person_gid, p.score)))
                    .map_err(st)
            };
            match &c.decision {
                Decision::Apply(p) if Some(&p.person_gid) == me_person.as_ref() => {
                    if !store.set_speaker_me_if_unclaimed(&c.gid).map_err(st)? {
                        continue;
                    }
                    named = true;
                    applied += 1;
                }
                Decision::Apply(p) => {
                    let name = store.person(&p.person_gid).map_err(st)?.name;
                    if name.trim().is_empty() {
                        if !suggest(p)? {
                            continue;
                        }
                        suggested += 1;
                    } else {
                        if !store.rename_speaker_if_unnamed(&c.gid, &name).map_err(st)? {
                            continue;
                        }
                        named = true;
                        applied += 1;
                    }
                }
                Decision::Suggest(p) => {
                    if !suggest(p)? {
                        continue;
                    }
                    suggested += 1;
                }
                Decision::Nothing if had_suggestion => {
                    store
                        .set_speaker_suggestion_if_unnamed(&c.gid, None)
                        .map_err(st)?;
                }
                Decision::Nothing => {}
            }
            // Flag on: what stays unnamed keeps its voice, sealed, so the
            // user can name it once and have it remembered.
            if let (Some(t), false) = (token, named) {
                let still_unnamed = store
                    .speakers(input.meeting)
                    .map_err(st)?
                    .iter()
                    .any(|s| s.gid == c.gid && s.display_name.is_none() && !s.is_me);
                if still_unnamed {
                    store
                        .put_speaker_voice(
                            &c.gid,
                            VOICE_MODEL,
                            c.lang.as_deref().unwrap_or(ANY_LANG),
                            &c.voice.vec,
                            t,
                        )
                        .map_err(st)?;
                    stored += 1;
                }
            }
        }
        if let (Some(me), Some((v, lang))) = (&me_profile, &me_voice) {
            learned += usize::from(
                learn_me(store, me, input.meeting, v, lang.as_deref()).map_err(|_| "store")?,
            );
        }
        log::info!(
            "voice step clusters={} applied={applied} suggested={suggested} stored={stored} learned={learned}",
            clusters.len()
        );
        Ok(true)
    }
}
