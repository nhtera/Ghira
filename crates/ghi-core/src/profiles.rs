// SPDX-License-Identifier: Apache-2.0
//! Voice matching math (phase 14c, D5-D7): L2-normalised 192-d vectors, a score
//! against a profile, the apply / suggest / nothing decision, and which audio
//! windows of a speaker are worth embedding. Pure functions, no I/O, so the
//! thresholds are testable without a model; [`FakeVoice`] stands in for it.
//!
//! The thresholds are provisional (`T_HIGH`, `T_LOW`: tune with the eval set).

use std::ops::Range;
use std::sync::Arc;

use ghi_store::voice::{VoiceProfile, VoiceSet};
use zeroize::Zeroize;

/// Registry id of the speaker model (`ghi-models/registry.toml`); also the
/// `model` the store keeps vectors under, so a new model starts clean.
pub const VOICE_MODEL: &str = "campplus-zh-en";
/// The language of a profile set that counts as the same language as any.
pub const ANY_LANG: &str = "any";

// Thresholds from the AMI cross-meeting eval (EN, 16 meetings, 63
// speaker-meetings; `tools/eval/reports/ami-speakerid-20261003.md`): EER 0%,
// highest different-speaker score 0.581, 3-6% of same-speaker pairs below
// 0.70. Re-check with Vietnamese and speakerphone audio (owner checklist).

/// At or above this, the same language and a clear winner: applied by itself.
pub const T_HIGH: f32 = 0.70;
/// From here up a match is only suggested ("sounds like ..."); also the
/// speakerphone guard for learning Me.
pub const T_LOW: f32 = 0.55;
/// The best match must beat the runner-up by this much to be applied.
pub const MARGIN: f32 = 0.10;

const RATE: usize = 16_000;
/// Shortest audio the embedder takes (200 fbank frames, 2.015 s).
pub const MIN_WINDOW: usize = 32_240;
/// Window sizes (6.1, 4.1, 2.1 s): the embedder crops from the start to 6, 4
/// or 2 s, so these keep a window's whole useful part.
const SIZES: [usize; 3] = [RATE * 61 / 10, RATE * 41 / 10, RATE * 21 / 10];
/// Windows embedded per speaker per meeting.
pub const MAX_WINDOWS: usize = 8;
/// Same-speaker turns closer than this are one turn.
const MERGE_GAP_MS: i64 = 300;

/// Turns audio into voice vectors; the model-free counterpart of
/// `ghi_speech::voice::VoiceEmbedder` (see `ghi_core::profiles::open_tract`
/// with the `voice` feature).
pub trait VoiceEmbed {
    /// The raw (not normalised) vector of one window of 16 kHz mono PCM;
    /// `None` when it is too short or near-silent.
    fn embed(&mut self, pcm16k: &[f32]) -> Result<Option<Vec<f32>>, String>;
}

/// Opens a [`VoiceEmbed`] (the model loads on first use of a job run).
pub type VoiceFactory = Arc<dyn Fn() -> Result<Box<dyn VoiceEmbed>, String> + Send + Sync>;

// ------------------------------------------------------------------ vectors

pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// `v` scaled to unit length; `None` for a zero or non-finite vector.
pub fn normalized(v: &[f32]) -> Option<Vec<f32>> {
    let n = dot(v, v).sqrt();
    if !n.is_finite() || n < 1e-9 || v.iter().any(|x| !x.is_finite()) {
        return None;
    }
    Some(v.iter().map(|x| x / n).collect())
}

/// Cosine similarity (0 for mismatched or zero vectors).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    match (normalized(a), normalized(b)) {
        (Some(a), Some(b)) => dot(&a, &b),
        _ => 0.0,
    }
}

/// The mean of unit vectors, normalised again (all one length, non-empty).
pub fn mean_normalized(vs: &[Vec<f32>]) -> Option<Vec<f32>> {
    let dim = vs.first()?.len();
    let mut sum = vec![0.0f32; dim];
    for v in vs {
        if v.len() != dim {
            return None;
        }
        for (s, x) in sum.iter_mut().zip(v) {
            *s += x;
        }
    }
    let out = normalized(&sum);
    sum.zeroize();
    out
}

// ----------------------------------------------------------------- scoring

/// How a cluster's voice compares with one profile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score {
    pub value: f32,
    /// Compared against a set of the cluster's language (or an `any` set).
    pub same_lang: bool,
}

/// A set counts as the cluster's language when they are equal or the set is
/// `any` (a passage enrolled without a language).
fn lang_matches(set: &VoiceSet, lang: Option<&str>) -> bool {
    set.lang == ANY_LANG || lang == Some(set.lang.as_str())
}

/// max(cos centroid, max cos exemplars) over one set.
fn set_score(set: &VoiceSet, v: &[f32]) -> f32 {
    std::iter::once(&set.centroid[..])
        .chain(set.exemplars.iter().map(|e| &e.vec[..]))
        .filter(|c| c.len() == v.len())
        .map(|c| dot(c, v))
        .fold(f32::NEG_INFINITY, f32::max)
}

/// Score of the unit vector `v` against `profile`: the best over the sets of
/// the cluster's language; with none, the best over all sets, flagged
/// cross-language. `None` when the profile has nothing of this dimension.
pub fn score_profile(profile: &VoiceProfile, v: &[f32], lang: Option<&str>) -> Option<Score> {
    let best = |same: bool| {
        profile
            .sets
            .iter()
            .filter(|s| lang_matches(s, lang) == same || !same)
            .map(|s| set_score(s, v))
            .filter(|x| x.is_finite())
            .fold(None, |a: Option<f32>, x| Some(a.map_or(x, |a| a.max(x))))
    };
    match best(true) {
        Some(value) => Some(Score {
            value,
            same_lang: true,
        }),
        None => best(false).map(|value| Score {
            value,
            same_lang: false,
        }),
    }
}

/// Whether Me may learn from `v` (D7): its centroid is close enough to what
/// Me already is (the speakerphone guard). A profile with no vector of this
/// dimension for the model (nothing to compare) never learns: it counts as no
/// profile, like no consent.
pub fn me_gate(me: &VoiceProfile, v: &[f32]) -> bool {
    me.sets
        .iter()
        .filter(|s| s.centroid.len() == v.len())
        .any(|s| dot(&s.centroid, v) >= T_LOW)
}

/// Me already has an exemplar from this meeting (a rerun adds nothing).
pub fn has_exemplar_from(profile: &VoiceProfile, meeting_gid: &str) -> bool {
    profile.sets.iter().any(|s| {
        s.exemplars.iter().any(|e| {
            e.source
                .as_ref()
                .is_some_and(|x| x.meeting_gid == meeting_gid)
        })
    })
}

// ---------------------------------------------------------------- decisions

/// One person a cluster was compared with.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub person_gid: String,
    pub score: f32,
    pub same_lang: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Name the speaker (or mark it Me) by itself.
    Apply(Candidate),
    /// Only offer "sounds like ...".
    Suggest(Candidate),
    Nothing,
}

/// D5. Apply: the same language, `score >= T_HIGH`, and the best is `MARGIN`
/// ahead of the next person. A score in `[T_LOW, T_HIGH)`, another language,
/// or a close runner-up only suggests. Below `T_LOW`: nothing.
pub fn decide(cands: &[Candidate]) -> Decision {
    let mut sorted: Vec<&Candidate> = cands.iter().filter(|c| c.score.is_finite()).collect();
    sorted.sort_by(|a, b| b.score.total_cmp(&a.score));
    let Some(best) = sorted.first() else {
        return Decision::Nothing;
    };
    if best.score < T_LOW {
        return Decision::Nothing;
    }
    let clear = sorted.get(1).is_none_or(|n| best.score - n.score >= MARGIN);
    if best.same_lang && best.score >= T_HIGH && clear {
        Decision::Apply((*best).clone())
    } else {
        Decision::Suggest((*best).clone())
    }
}

/// What the pass knows beyond one cluster's scores.
#[derive(Debug, Default)]
pub struct Policy {
    /// Room mode with one cluster in the whole meeting: a lone voice is
    /// suggested, never applied.
    pub single_cluster: bool,
    /// The person who is Me.
    pub me_person: Option<String>,
    /// An imported file: Me is never applied by itself there.
    pub file_source: bool,
    /// Persons already linked to a speaker of this meeting.
    pub linked: std::collections::HashSet<String>,
}

/// Turns an apply into a suggestion unless it has the evidence (`enough[i]`:
/// 2 or more windows, or 6 s of speech), is not alone in the meeting, and the
/// person's score on this cluster beats their score on every other cluster by
/// [`MARGIN`] (else both clusters only suggest). `cands[i]` are cluster i's
/// scores for every person.
pub fn resolve(
    decisions: &mut [Decision],
    cands: &[Vec<Candidate>],
    enough: &[bool],
    policy: &Policy,
) {
    let before: Vec<Decision> = decisions.to_vec();
    for (i, d) in decisions.iter_mut().enumerate() {
        let Decision::Apply(c) = &before[i] else {
            continue;
        };
        let rival = cands.iter().enumerate().any(|(j, cs)| {
            j != i
                && cs
                    .iter()
                    .any(|o| o.person_gid == c.person_gid && c.score - o.score < MARGIN)
        });
        let is_me = policy.me_person.as_deref() == Some(c.person_gid.as_str());
        if policy.single_cluster
            || !enough.get(i).copied().unwrap_or(false)
            || rival
            || (policy.file_source && is_me)
            || policy.linked.contains(&c.person_gid)
        {
            *d = Decision::Suggest(c.clone());
        }
    }
}

/// The language most of `langs` agree on (ties: the first seen).
pub fn majority_lang<'a>(langs: impl Iterator<Item = Option<&'a str>>) -> Option<String> {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for l in langs.flatten() {
        match counts.iter_mut().find(|(k, _)| *k == l) {
            Some(c) => c.1 += 1,
            None => counts.push((l, 1)),
        }
    }
    let mut best: Option<(&str, usize)> = None;
    for c in counts {
        if best.is_none_or(|b| c.1 > b.1) {
            best = Some(c);
        }
    }
    best.map(|b| b.0.to_string())
}

// ------------------------------------------------------------------ windows

/// Who spoke when, on the timeline of one track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub who: u32,
    pub t0_ms: i64,
    pub t1_ms: i64,
}

/// `iv` minus the (sorted, possibly overlapping) `others`.
fn subtract(iv: (i64, i64), others: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let mut at = iv.0;
    for &(a, b) in others {
        if b <= at {
            continue;
        }
        if a >= iv.1 {
            break;
        }
        if a > at {
            out.push((at, a));
        }
        at = at.max(b);
    }
    if at < iv.1 {
        out.push((at, iv.1));
    }
    out
}

/// D6: sample ranges of `pcm` (of `pcm_len` samples) where only `who` speaks
/// for at least [`MIN_WINDOW`], cut into 6.1 / 4.1 / 2.1 s windows, the
/// longest first, at most [`MAX_WINDOWS`]; ties are spread over the meeting.
pub fn pick_windows(spans: &[Span], who: u32, pcm_len: usize) -> Vec<Range<usize>> {
    pick_windows_merging(spans, who, pcm_len, MERGE_GAP_MS)
}

/// Pauses within one speaker's talk when the spans are transcript lines
/// (diarization turns are already whole).
pub const LINE_GAP_MS: i64 = 1_500;

/// [`pick_windows`], joining the speaker's spans closer than `gap_ms`.
pub fn pick_windows_merging(
    spans: &[Span],
    who: u32,
    pcm_len: usize,
    gap_ms: i64,
) -> Vec<Range<usize>> {
    let mut mine: Vec<(i64, i64)> = spans
        .iter()
        .filter(|s| s.who == who && s.t1_ms > s.t0_ms)
        .map(|s| (s.t0_ms, s.t1_ms))
        .collect();
    mine.sort_unstable();
    let mut turns: Vec<(i64, i64)> = Vec::new();
    for (a, b) in mine {
        match turns.last_mut() {
            Some(t) if a - t.1 <= gap_ms => t.1 = t.1.max(b),
            _ => turns.push((a, b)),
        }
    }
    let mut others: Vec<(i64, i64)> = spans
        .iter()
        .filter(|s| s.who != who)
        .map(|s| (s.t0_ms, s.t1_ms))
        .collect();
    others.sort_unstable();

    let mut windows: Vec<Range<usize>> = Vec::new();
    for turn in turns {
        for (a, b) in subtract(turn, &others) {
            let mut start = (a.max(0) as usize) * RATE / 1000;
            let end = ((b.max(0) as usize) * RATE / 1000).min(pcm_len);
            for size in SIZES {
                while end.saturating_sub(start) >= size {
                    windows.push(start..start + size);
                    start += size;
                }
            }
            if end.saturating_sub(start) >= MIN_WINDOW {
                windows.push(start..end);
            }
        }
    }
    windows.sort_by(|a, b| b.len().cmp(&a.len()).then(a.start.cmp(&b.start)));
    if windows.len() > MAX_WINDOWS {
        let cut = windows[MAX_WINDOWS - 1].len();
        let longer = windows.iter().filter(|w| w.len() > cut).count();
        let tier: Vec<Range<usize>> = windows.iter().filter(|w| w.len() == cut).cloned().collect();
        let need = MAX_WINDOWS - longer;
        let mut picked: Vec<Range<usize>> = windows[..longer].to_vec();
        picked.extend((0..need).map(|i| tier[i * tier.len() / need].clone()));
        windows = picked;
    }
    windows
}

/// A speaker's voice in one meeting. The vector is wiped when this drops, and
/// `Debug` never shows it.
#[derive(Clone, PartialEq)]
pub struct ClusterVoice {
    /// Unit vector: the normalised mean of the windows.
    pub vec: Vec<f32>,
    /// The longest window, where the voice can be heard again.
    pub longest: Range<usize>,
    pub windows: usize,
    /// Samples of speech behind it (the embedded windows).
    pub speech: usize,
}

impl ClusterVoice {
    /// Enough to apply a name by itself: 2 windows or 6 s of speech.
    pub fn enough_to_apply(&self) -> bool {
        self.windows >= 2 || self.speech >= 6 * RATE
    }
}

impl Drop for ClusterVoice {
    fn drop(&mut self) {
        self.vec.zeroize();
    }
}

impl std::fmt::Debug for ClusterVoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusterVoice")
            .field("windows", &self.windows)
            .field("speech", &self.speech)
            .finish_non_exhaustive()
    }
}

/// Embeds `windows` of `pcm` and averages them (D6). Windows the model skips
/// (silence) are left out; `None` when none is left. `stop` is asked between
/// windows; when it is true the work is abandoned (`None`).
pub fn embed_windows(
    embedder: &mut dyn VoiceEmbed,
    pcm: &[f32],
    windows: &[Range<usize>],
    stop: &dyn Fn() -> bool,
) -> Result<Option<ClusterVoice>, String> {
    let mut vecs: Vec<Vec<f32>> = Vec::new();
    let mut longest: Option<Range<usize>> = None;
    let mut speech = 0;
    for w in windows {
        if stop() {
            wipe(&mut vecs);
            return Ok(None);
        }
        let Some(raw) = pcm.get(w.clone()) else {
            continue;
        };
        let mut v = embedder.embed(raw)?;
        let unit = v.as_deref().and_then(normalized);
        if let Some(v) = v.as_mut() {
            v.zeroize();
        }
        if let Some(u) = unit {
            if longest.as_ref().is_none_or(|l| w.len() > l.len()) {
                longest = Some(w.clone());
            }
            speech += w.len();
            vecs.push(u);
        }
    }
    let mean = mean_normalized(&vecs);
    let n = vecs.len();
    wipe(&mut vecs);
    let (Some(vec), Some(longest)) = (mean, longest) else {
        return Ok(None);
    };
    Ok(Some(ClusterVoice {
        vec,
        longest,
        windows: n,
        speech,
    }))
}

fn wipe(vs: &mut [Vec<f32>]) {
    vs.iter_mut().for_each(|v| v.zeroize());
}

/// Sample range to milliseconds.
pub fn range_ms(r: &Range<usize>) -> (i64, i64) {
    ((r.start * 1000 / RATE) as i64, (r.end * 1000 / RATE) as i64)
}

// ------------------------------------------------------------------- tract

/// Opens the real speaker model (CAM++ over tract) after checking the file
/// against the registry (RT-12).
#[cfg(feature = "voice")]
pub fn open_tract(path: &std::path::Path) -> Result<Box<dyn VoiceEmbed>, String> {
    use ghi_speech::voice::{TractVoice, VoiceEmbedder};

    struct Real(TractVoice);
    impl VoiceEmbed for Real {
        fn embed(&mut self, pcm: &[f32]) -> Result<Option<Vec<f32>>, String> {
            self.0.embed(pcm).map_err(|e| e.to_string())
        }
    }
    let m = ghi_models::find(VOICE_MODEL).ok_or("the voice model is not in the registry")?;
    ghi_models::verify::verify_for_load(path, &m).map_err(|e| e.to_string())?;
    Ok(Box::new(Real(
        TractVoice::open(path).map_err(|e| e.to_string())?,
    )))
}

// -------------------------------------------------------------------- fake

/// A model-free embedder for tests: a window's vector depends on its dominant
/// pitch (zero crossings, in 100 Hz steps), so one tone is one "voice" and
/// two tones are unrelated random directions. Short or quiet audio is `None`,
/// like the real one.
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeVoice;

impl FakeVoice {
    /// The "voice" (pitch bucket) of a window.
    pub fn bucket(pcm: &[f32]) -> u64 {
        let crossings = pcm
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        let hz = crossings as f64 / 2.0 / (pcm.len() as f64 / RATE as f64);
        (hz / 100.0).round() as u64
    }

    /// The vector of a voice (unit length, 192 values).
    pub fn vector(bucket: u64) -> Vec<f32> {
        let mut state = bucket.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
        let mut next = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        let v: Vec<f32> = (0..192).map(|_| next() as f32).collect();
        normalized(&v).expect("random vector is not zero")
    }
}

impl VoiceEmbed for FakeVoice {
    fn embed(&mut self, pcm: &[f32]) -> Result<Option<Vec<f32>>, String> {
        if pcm.len() < MIN_WINDOW {
            return Ok(None);
        }
        let rms =
            (pcm.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>() / pcm.len() as f64).sqrt();
        if rms < 1e-3 {
            return Ok(None);
        }
        Ok(Some(FakeVoice::vector(FakeVoice::bucket(pcm))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_store::voice::{ExemplarSource, VoiceConsent, VoiceExemplar};

    fn cand(p: &str, score: f32, same_lang: bool) -> Candidate {
        Candidate {
            person_gid: p.into(),
            score,
            same_lang,
        }
    }

    fn set(lang: &str, vecs: &[Vec<f32>]) -> VoiceSet {
        VoiceSet {
            model: VOICE_MODEL.into(),
            lang: lang.into(),
            centroid: mean_normalized(vecs).unwrap(),
            exemplars: vecs
                .iter()
                .map(|v| VoiceExemplar {
                    vec: v.clone(),
                    source: None,
                })
                .collect(),
        }
    }

    fn profile(sets: Vec<VoiceSet>) -> VoiceProfile {
        VoiceProfile {
            gid: "p".into(),
            person_gid: "me".into(),
            is_me: true,
            consent: VoiceConsent {
                method: "self_checkbox".into(),
                at_ms: 0,
                text_key: "k".into(),
                clip: None,
            },
            created_at: 0,
            updated_at: 0,
            sets,
        }
    }

    #[test]
    fn decide_thresholds_margin_and_language() {
        let apply = decide(&[cand("a", 0.82, true), cand("b", 0.40, true)]);
        assert!(matches!(apply, Decision::Apply(c) if c.person_gid == "a"));
        // Exactly T_HIGH applies, just under only suggests.
        assert!(matches!(
            decide(&[cand("a", T_HIGH, true)]),
            Decision::Apply(_)
        ));
        assert!(matches!(
            decide(&[cand("a", T_HIGH - 0.01, true)]),
            Decision::Suggest(_)
        ));
        // Another language never applies.
        assert!(matches!(
            decide(&[cand("a", 0.95, false)]),
            Decision::Suggest(_)
        ));
        // The runner-up is too close: no apply, the best is suggested.
        let d = decide(&[cand("a", 0.80, true), cand("b", 0.75, true)]);
        assert!(matches!(d, Decision::Suggest(c) if c.person_gid == "a"));
        // The band and below.
        assert!(matches!(
            decide(&[cand("a", T_LOW, true)]),
            Decision::Suggest(_)
        ));
        assert_eq!(decide(&[cand("a", T_LOW - 0.01, true)]), Decision::Nothing);
        assert_eq!(decide(&[]), Decision::Nothing);
        assert_eq!(decide(&[cand("a", f32::NAN, true)]), Decision::Nothing);
    }

    fn applied(ds: &[Decision]) -> Vec<bool> {
        ds.iter().map(|d| matches!(d, Decision::Apply(_))).collect()
    }

    #[test]
    fn an_apply_needs_a_margin_over_the_persons_other_clusters() {
        let cands = vec![
            vec![cand("a", 0.90, true)],
            vec![cand("a", 0.85, true)],
            vec![cand("a", 0.40, true), cand("b", 0.80, true)],
        ];
        let mut ds: Vec<Decision> = cands.iter().map(|c| decide(c)).collect();
        assert_eq!(applied(&ds), [true, true, true]);
        resolve(&mut ds, &cands, &[true; 3], &Policy::default());
        // 0.90 vs 0.85: too close, both only suggest; b is alone with 0.80.
        assert_eq!(applied(&ds), [false, false, true]);
        assert!(matches!(ds[0], Decision::Suggest(_)) && matches!(ds[1], Decision::Suggest(_)));
        // A clear winner keeps it.
        let cands = vec![vec![cand("a", 0.95, true)], vec![cand("a", 0.55, true)]];
        let mut ds: Vec<Decision> = cands.iter().map(|c| decide(c)).collect();
        resolve(&mut ds, &cands, &[true; 2], &Policy::default());
        assert_eq!(applied(&ds), [true, false]);
    }

    #[test]
    fn an_apply_also_needs_evidence_company_and_a_live_source() {
        let cands = vec![vec![cand("me", 0.9, true)], vec![cand("x", 0.2, true)]];
        let base = |f: &dyn Fn(&mut Policy)| {
            let mut p = Policy {
                me_person: Some("me".into()),
                ..Default::default()
            };
            f(&mut p);
            let mut ds: Vec<Decision> = cands.iter().map(|c| decide(c)).collect();
            resolve(&mut ds, &cands, &[true, true], &p);
            applied(&ds)[0]
        };
        assert!(base(&|_| {}));
        assert!(!base(&|p| p.single_cluster = true));
        assert!(!base(&|p| p.file_source = true));
        assert!(!base(&|p| {
            p.linked.insert("me".into());
        }));
        let mut ds = vec![decide(&cands[0])];
        resolve(&mut ds, &cands[..1], &[false], &Policy::default());
        assert!(!applied(&ds)[0], "too little speech");
        // A file does not stop a third party from being applied.
        let mut p = Policy {
            file_source: true,
            me_person: Some("me".into()),
            ..Default::default()
        };
        p.linked.clear();
        let c = vec![vec![cand("hoa", 0.9, true)]];
        let mut ds = vec![decide(&c[0])];
        resolve(&mut ds, &c, &[true], &p);
        assert!(applied(&ds)[0]);
    }

    #[test]
    fn scores_use_the_clusters_language_then_fall_back_across() {
        let a = FakeVoice::vector(3);
        let b = FakeVoice::vector(7);
        let p = profile(vec![
            set("en", std::slice::from_ref(&a)),
            set("vi", std::slice::from_ref(&b)),
        ]);
        let s = score_profile(&p, &a, Some("en")).unwrap();
        assert!(s.same_lang && s.value > 0.99);
        // Cluster is Vietnamese: only the vi set counts, a is not a match.
        let s = score_profile(&p, &a, Some("vi")).unwrap();
        assert!(s.same_lang && s.value < 0.5, "{s:?}");
        // No set of that language: cross-language, best of all.
        let s = score_profile(&p, &a, Some("fr")).unwrap();
        assert!(!s.same_lang && s.value > 0.99);
        // `any` is every language; an unknown cluster language is cross.
        let any = profile(vec![set(ANY_LANG, std::slice::from_ref(&a))]);
        assert!(score_profile(&any, &a, Some("vi")).unwrap().same_lang);
        let en = profile(vec![set("en", std::slice::from_ref(&a))]);
        assert!(!score_profile(&en, &a, None).unwrap().same_lang);
        // Nothing of this dimension.
        assert!(score_profile(&en, &[1.0, 0.0], Some("en")).is_none());
    }

    #[test]
    fn score_takes_the_better_of_centroid_and_exemplars() {
        let a = FakeVoice::vector(1);
        let b = FakeVoice::vector(2);
        let c = FakeVoice::vector(3);
        let p = profile(vec![set("en", &[a.clone(), b.clone(), c.clone()])]);
        // The centroid of three random voices is ~0.58 from each; an exemplar
        // matches its own voice exactly.
        let s = score_profile(&p, &b, Some("en")).unwrap();
        assert!(s.value > 0.99);
        let centroid = cosine(&p.sets[0].centroid, &b);
        assert!(centroid < 0.9 && centroid > 0.3, "{centroid}");
    }

    #[test]
    fn me_gate_guards_against_speakerphone_contamination() {
        let a = FakeVoice::vector(1);
        let other = FakeVoice::vector(9);
        let me = profile(vec![set("any", std::slice::from_ref(&a))]);
        assert!(me_gate(&me, &a));
        assert!(!me_gate(&me, &other));
        assert!(
            !me_gate(&profile(vec![]), &other),
            "nothing to compare: no learning"
        );
        assert!(
            !me_gate(&profile(vec![set("any", &[vec![1.0, 0.0]])]), &a),
            "other dimension"
        );
    }

    #[test]
    fn exemplar_from_a_meeting_is_remembered() {
        let a = FakeVoice::vector(1);
        let mut s = set("en", std::slice::from_ref(&a));
        assert!(!has_exemplar_from(&profile(vec![s.clone()]), "m1"));
        s.exemplars[0].source = Some(ExemplarSource {
            meeting_gid: "m1".into(),
            t0_ms: 0,
            t1_ms: 1,
        });
        let p = profile(vec![s]);
        assert!(has_exemplar_from(&p, "m1") && !has_exemplar_from(&p, "m2"));
    }

    #[test]
    fn majority_language() {
        assert_eq!(
            majority_lang([Some("vi"), Some("en"), Some("vi"), None].into_iter()),
            Some("vi".into())
        );
        assert_eq!(majority_lang([None, None].into_iter()), None);
        assert_eq!(
            majority_lang([Some("en"), Some("vi")].into_iter()),
            Some("en".into()),
            "ties go to the first"
        );
    }

    fn span(who: u32, t0: i64, t1: i64) -> Span {
        Span {
            who,
            t0_ms: t0,
            t1_ms: t1,
        }
    }

    #[test]
    fn windows_skip_overlap_short_turns_and_crop_to_buckets() {
        let len = 16_000 * 120;
        // Speaker 1: 0-7 s (alone), 10-13 s (the other talks 11-12 s), 20-21 s (too short).
        let spans = [
            span(1, 0, 7_000),
            span(1, 10_000, 13_000),
            span(2, 11_000, 12_000),
            span(1, 20_000, 21_000),
        ];
        let w = pick_windows(&spans, 1, len);
        // 7 s -> a 6.1 s window; 10-11 s and 12-13 s are too short alone.
        assert_eq!(w, vec![0..97_600], "{w:?}");
        // A 4.5 s turn is a 4.1 s window; 2.5 s a 2.1 s one.
        let w = pick_windows(&[span(1, 0, 4_500), span(1, 8_000, 10_500)], 1, len);
        assert_eq!(w, vec![0..65_600, 128_000..161_600]);
        // 2.015 s is the least that counts, and is used whole.
        let w = pick_windows(&[span(1, 0, 2_015)], 1, len);
        assert_eq!(w, vec![0..32_240]);
        assert!(pick_windows(&[span(1, 0, 2_000)], 1, len).is_empty());
        // Close turns of one speaker are one turn.
        let w = pick_windows(&[span(1, 0, 3_000), span(1, 3_200, 6_500)], 1, len);
        assert_eq!(w, vec![0..97_600]);
        // Never past the end of the audio.
        assert!(pick_windows(&[span(1, 0, 10_000)], 1, 16_000).is_empty());
    }

    #[test]
    fn at_most_eight_windows_longest_first_and_spread_out() {
        let len = 16_000 * 4_000;
        // Twelve 6.2 s turns, spread over an hour.
        let spans: Vec<Span> = (0..12)
            .map(|i| span(1, i * 300_000, i * 300_000 + 6_200))
            .collect();
        let w = pick_windows(&spans, 1, len);
        assert_eq!(w.len(), 8);
        assert!(w.iter().all(|r| r.len() == 97_600));
        // Not just the first eight: the last turn's window is in.
        assert!(w.last().unwrap().start > 16_000 * 300 * 6);
        // A longer window outranks the shorter ones.
        let mut spans = spans;
        spans.push(span(1, 3_000_000, 3_004_500));
        let w = pick_windows(&spans, 1, len);
        assert_eq!(w.len(), 8);
    }

    #[allow(clippy::single_range_in_vec_init)]
    #[test]
    fn embedding_averages_windows_and_skips_silence() {
        let tone = |hz: f32, secs: f32| -> Vec<f32> {
            (0..(16_000.0 * secs) as usize)
                .map(|i| (i as f32 * hz * std::f32::consts::TAU / 16_000.0).sin() * 0.3)
                .collect()
        };
        let mut pcm = tone(300.0, 7.0);
        pcm.extend(vec![0.0; 16_000 * 3]);
        pcm.extend(tone(300.0, 3.0));
        let windows = [0..97_600, 112_000..160_000, 160_000..200_000];
        let v = embed_windows(&mut FakeVoice, &pcm, &windows, &|| false)
            .unwrap()
            .unwrap();
        // The silent window is skipped; two tone windows remain, one voice.
        assert_eq!(v.windows, 2);
        assert_eq!(v.longest, 0..97_600);
        assert!(cosine(&v.vec, &FakeVoice::vector(3)) > 0.999);
        assert!((dot(&v.vec, &v.vec) - 1.0).abs() < 1e-4, "unit length");
        // Stopped: nothing.
        assert!(
            embed_windows(&mut FakeVoice, &pcm, &windows, &|| true)
                .unwrap()
                .is_none()
        );
        // Short audio: nothing.
        assert!(FakeVoice.embed(&pcm[..1_000]).unwrap().is_none());
        assert!(
            embed_windows(&mut FakeVoice, &pcm, &[100_000..110_000], &|| false)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn fake_voices_are_stable_per_pitch_and_distinct() {
        assert_eq!(FakeVoice::vector(3), FakeVoice::vector(3));
        assert!(cosine(&FakeVoice::vector(3), &FakeVoice::vector(4)).abs() < 0.4);
        let tone: Vec<f32> = (0..64_000)
            .map(|i| (i as f32 * 700.0 * std::f32::consts::TAU / 16_000.0).sin() * 0.2)
            .collect();
        assert_eq!(FakeVoice::bucket(&tone), 7);
    }
}
