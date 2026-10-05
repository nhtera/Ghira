// SPDX-License-Identifier: Apache-2.0
//! Voice profiles through the real job pipeline (phase 14c, slice C): room and
//! call meetings recorded from synthetic tones (one pitch = one "voice" for
//! `FakeVoice`), the final pass with its voice step, `voice_learn` and
//! enrollment. RT-13: with the third-party flag off, no voice but Me's is ever
//! stored.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use ghi_audio::Track;
use ghi_core::capture::{ReplayTrack, replay};
use ghi_core::engines::{FakeEngines, Script, SpeechEngines};
use ghi_core::events::bus;
use ghi_core::final_pass::FinalPassJob;
use ghi_core::jobs::{JobRunner, always_ready};
use ghi_core::live::Mode;
use ghi_core::profiles::{ANY_LANG, FakeVoice, VOICE_MODEL, VoiceEmbed, VoiceFactory, cosine};
use ghi_core::session::{RecordingHooks, Session, SessionConfig};
use ghi_core::voice_job::{
    EnrollError, VoiceLearnJob, enroll_from_pcm, queue_learn, requeue_index, self_consent,
};
use ghi_core::voice_step::{ThirdPartyGate, VoiceStep};
use ghi_speech::SpeakerSegment;
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::{NewSegment, NewSpeaker, Store};
use ghi_store::voice::{ThirdPartyApproved, VoiceConsent, VoiceExemplar};

// Pitches (Hz) of the three "voices": FakeVoice buckets are pitch / 100.
const A: f32 = 300.0;
const B: f32 = 700.0;
const C: f32 = 1100.0;

fn vector(hz: f32) -> Vec<f32> {
    FakeVoice::vector((hz / 100.0) as u64)
}

// ------------------------------------------------------------------- logs

struct Capture(Mutex<Vec<String>>);

static LOGS: Capture = Capture(Mutex::new(Vec::new()));
static INIT: Once = Once::new();

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }
    fn log(&self, r: &log::Record) {
        self.0.lock().unwrap().push(r.args().to_string());
    }
    fn flush(&self) {}
}

fn capture_logs() {
    INIT.call_once(|| {
        let _ = log::set_logger(&LOGS).map(|()| log::set_max_level(log::LevelFilter::Trace));
    });
}

// --------------------------------------------------------------- fixtures

fn open_store() -> (tempfile::TempDir, Arc<Store>) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    (tmp, store)
}

/// `secs` of 48 kHz audio: a tone of `hz` between `spans`, silence elsewhere.
fn tones(secs: f64, spans: &[(f64, f64, f32)]) -> Vec<f32> {
    let rate = 48_000.0;
    let mut out = vec![0.0f32; (secs * rate) as usize];
    for &(a, b, hz) in spans {
        for i in (a * rate) as usize..((b * rate) as usize).min(out.len()) {
            out[i] = (i as f32 * hz * std::f32::consts::TAU / rate as f32).sin() * 0.2;
        }
    }
    out
}

fn turn(speaker: u32, start: f64, end: f64) -> SpeakerSegment {
    SpeakerSegment {
        start,
        end,
        speaker,
    }
}

/// Three Vietnamese speakers taking turns in a room: A, B, C.
fn room_script() -> Script {
    Script {
        utterances: vec![
            (1.0, 3.0, "xin chào mọi người".into()),
            (4.0, 6.0, "hôm nay chốt lịch beta".into()),
            (9.0, 11.0, "anh Hoa nói về kế hoạch".into()),
            (12.0, 14.0, "chúng ta cần thêm thời gian".into()),
            (17.0, 19.0, "em đồng ý với anh".into()),
            (20.0, 22.0, "cảm ơn mọi người".into()),
        ],
        turns: vec![turn(1, 0.5, 7.0), turn(2, 8.0, 15.0), turn(3, 16.0, 23.0)],
    }
}

fn room_audio() -> Vec<ReplayTrack> {
    vec![ReplayTrack {
        track: Track::Mic,
        samples: tones(24.0, &[(0.5, 7.0, A), (8.0, 15.0, B), (16.0, 23.0, C)]),
        sample_rate: 48_000,
    }]
}

/// Records `tracks` (audio only; no live engines) and returns the meeting.
fn record(
    store: &Arc<Store>,
    runner: &Arc<JobRunner>,
    mode: Mode,
    tracks: Vec<ReplayTrack>,
    secs: f64,
) -> String {
    let (tx, _rx) = bus();
    let capture = replay(tracks, None).unwrap();
    let s = Session::start(
        store.clone(),
        None,
        capture,
        SessionConfig {
            sensitive: false,
            mode,
            language: None,
            title: "voices".into(),
            queue_jobs: true,
            lossless: true,
            echo_cancellation: true,
        },
        tx,
        Some(runner.clone()),
    )
    .unwrap();
    let t = Instant::now();
    while s.now_ms() < (secs * 1000.0) as i64 - 200 {
        assert!(
            t.elapsed() < Duration::from_secs(30),
            "the replay plays out"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let meeting = s.meeting().to_string();
    s.stop().unwrap();
    meeting
}

fn fake_factory() -> VoiceFactory {
    Arc::new(|| Ok(Box::new(FakeVoice) as Box<dyn VoiceEmbed>))
}

fn no_third_party() -> ThirdPartyGate {
    Arc::new(|| None)
}

/// Test only: the flag is "on".
fn third_party_on() -> ThirdPartyGate {
    Arc::new(|| Some(ThirdPartyApproved::assert_flag_checked()))
}

fn runner_with(
    store: &Arc<Store>,
    script: Script,
    voice: Option<VoiceStep>,
    learn: Option<VoiceLearnJob>,
) -> Arc<JobRunner> {
    let (tx, _rx) = bus();
    let engines: Arc<dyn SpeechEngines> = FakeEngines::new(script);
    let mut handlers: Vec<Arc<dyn ghi_core::jobs::JobHandler>> = vec![Arc::new(FinalPassJob {
        engines: Arc::new(move || Ok(engines.clone())),
        chunk_s: 600.0,
        ready: always_ready(),
        voice,
    })];
    if let Some(l) = learn {
        handlers.push(Arc::new(l));
    }
    JobRunner::new(store.clone(), tx, handlers)
}

fn step(third_party: ThirdPartyGate) -> VoiceStep {
    VoiceStep {
        embedder: fake_factory(),
        ready: always_ready(),
        third_party,
    }
}

/// A recorded 16 kHz passage of one voice (for enrollment).
fn passage(hz: f32) -> Vec<f32> {
    (0..16_000 * 20)
        .map(|i| (i as f32 * hz * std::f32::consts::TAU / 16_000.0).sin() * 0.2)
        .collect()
}

fn enroll_me(store: &Store, hz: f32) {
    enroll_from_pcm(
        store,
        &mut FakeVoice,
        &passage(hz),
        &self_consent("onboarding.voice.consent"),
    )
    .unwrap();
}

/// The speaker whose line starts at about `t_ms`.
fn speaker_at(store: &Store, meeting: &str, t_ms: i64) -> ghi_store::store::Speaker {
    let seg = store
        .segments(meeting)
        .unwrap()
        .into_iter()
        .find(|s| (s.t0_ms - t_ms).abs() < 800)
        .unwrap_or_else(|| panic!("no line near {t_ms}"));
    let gid = seg.speaker_gid.expect("the line has a speaker");
    store
        .speakers(meeting)
        .unwrap()
        .into_iter()
        .find(|s| s.gid == gid)
        .unwrap()
}

fn me_exemplars(store: &Store) -> Vec<VoiceExemplar> {
    store
        .me_voice_profile(VOICE_MODEL)
        .unwrap()
        .map(|p| p.sets.into_iter().flat_map(|s| s.exemplars).collect())
        .unwrap_or_default()
}

fn mix(a: &[f32], wa: f32, b: &[f32], wb: f32) -> Vec<f32> {
    let v: Vec<f32> = a.iter().zip(b).map(|(x, y)| x * wa + y * wb).collect();
    ghi_core::profiles::normalized(&v).unwrap()
}

// ------------------------------------------------------------------- tests

/// Me's profile decides: a close voice is Me, a middling one is only
/// suggested, a far one is left alone.
#[test]
fn room_me_is_applied_suggested_or_left_by_score() {
    let (_tmp, store) = open_store();
    // Me sounds 0.8 like B and 0.6 like A, and nothing like C.
    let me_vec = mix(&vector(B), 0.8, &vector(A), 0.6);
    let (sb, sa, sc) = (
        cosine(&me_vec, &vector(B)),
        cosine(&me_vec, &vector(A)),
        cosine(&me_vec, &vector(C)),
    );
    assert!(
        (0.70..0.95).contains(&sb) && (0.5..0.70).contains(&sa) && sc < 0.45,
        "{sb} {sa} {sc}"
    );
    let me = store.me_person().unwrap();
    store
        .put_voice_profile(
            &me,
            &self_consent("t"),
            None,
            VOICE_MODEL,
            vec![(
                ANY_LANG.into(),
                vec![VoiceExemplar {
                    vec: me_vec,
                    source: None,
                }],
            )],
            None,
        )
        .unwrap();
    let runner = runner_with(&store, room_script(), Some(step(no_third_party())), None);
    let meeting = record(&store, &runner, Mode::Room, room_audio(), 24.0);
    assert_eq!(runner.run_pending(), 1);

    let a = speaker_at(&store, &meeting, 1_000);
    let b = speaker_at(&store, &meeting, 9_000);
    let c = speaker_at(&store, &meeting, 17_000);
    assert!(
        b.is_me && b.person_gid.as_deref() == Some(me.as_str()),
        "{b:?}"
    );
    assert!(!a.is_me && a.display_name.is_none());
    let sug = a.suggestion.as_ref().expect("the band suggests Me");
    assert_eq!(sug.person_gid, me);
    assert!((sug.score - sa).abs() < 0.05, "{} vs {sa}", sug.score);
    assert!(!c.is_me && c.suggestion.is_none() && c.display_name.is_none());
    // The mine-and-only exemplar is still the enrolled one: B was only applied.
    assert_eq!(me_exemplars(&store).len(), 1);
}

/// What the user named is never touched, even if it sounds exactly like Me.
#[test]
fn named_speakers_are_never_touched() {
    let (_tmp, store) = open_store();
    enroll_me(&store, A);
    let runner = runner_with(&store, room_script(), Some(step(no_third_party())), None);
    let meeting = record(&store, &runner, Mode::Room, room_audio(), 24.0);
    // The live transcript had A's voice as "Lan".
    let lan = store
        .add_speaker(
            &meeting,
            NewSpeaker {
                label_idx: 0,
                display_name: Some("Lan".into()),
                color_slot: 1,
                ..Default::default()
            },
        )
        .unwrap();
    store
        .replace_transcript(
            &meeting,
            vec![NewSegment {
                gid: None,
                speaker_gid: Some(lan.clone()),
                t0_ms: 1_000,
                t1_ms: 3_000,
                text: "xin chào".into(),
                lang: Some("vi".into()),
                confidence: None,
                words: vec![],
                edited: false,
            }],
        )
        .unwrap();
    assert_eq!(runner.run_pending(), 1);
    let first = speaker_at(&store, &meeting, 1_000);
    assert_eq!(first.gid, lan);
    assert_eq!(first.display_name.as_deref(), Some("Lan"));
    assert!(!first.is_me && first.suggestion.is_none());
    assert!(store.speakers(&meeting).unwrap().iter().all(|s| !s.is_me));
}

/// RT-13: the flag is off, Me is enrolled, two other voices are in the
/// meeting. Afterwards only Me's profile exists, no other voice is stored
/// anywhere, and nothing carries a vector (job payloads, logs).
#[test]
fn rt13_purge_with_the_flag_off_stores_no_voice_but_mes() {
    capture_logs();
    let (_tmp, store) = open_store();
    enroll_me(&store, A);
    let runner = runner_with(&store, room_script(), Some(step(no_third_party())), None);
    let meeting = record(&store, &runner, Mode::Room, room_audio(), 24.0);
    assert_eq!(runner.run_pending(), 1);

    // Me was found; B and C are anonymous clusters.
    assert!(speaker_at(&store, &meeting, 1_000).is_me);
    let (b, c) = (
        speaker_at(&store, &meeting, 9_000),
        speaker_at(&store, &meeting, 17_000),
    );
    assert!(b.display_name.is_none() && b.suggestion.is_none() && !b.is_me);
    assert!(c.display_name.is_none() && c.suggestion.is_none() && !c.is_me);

    // No unnamed-cluster vector.
    assert!(store.speaker_voices(&meeting).unwrap().is_empty());
    for s in store.speakers(&meeting).unwrap() {
        assert!(store.speaker_voice(&s.gid).unwrap().is_none());
    }
    // One profile in the whole store, and it is Me's, with Me's one exemplar
    // (enrolled windows only: this meeting taught nothing it did not have to).
    let with_voice: Vec<_> = store
        .people_overview()
        .unwrap()
        .into_iter()
        .filter(|p| p.voice.is_some())
        .collect();
    assert_eq!(with_voice.len(), 1);
    assert!(with_voice[0].is_me);
    let others = store
        .third_party_voice_profiles(VOICE_MODEL, ThirdPartyApproved::assert_flag_checked())
        .unwrap();
    assert!(others.is_empty());
    for e in me_exemplars(&store) {
        // Every exemplar is Me's own voice (A), never B's or C's.
        assert!(
            cosine(&e.vec, &vector(A)) > 0.99,
            "an exemplar that is not Me"
        );
    }
    // Job payloads hold gids and numbers, no vectors.
    for j in store.jobs_for_meeting(&meeting).unwrap() {
        let p = j.payload.to_string();
        assert!(!p.contains('['), "{p}");
    }
    // Nothing logged holds a vector component, at full precision.
    let logs = LOGS.0.lock().unwrap().join("\n");
    for hz in [A, B, C] {
        for x in vector(hz) {
            for needle in [format!("{x}"), format!("{x:?}"), format!("{x:.7}")] {
                assert!(!logs.contains(&needle), "a vector component was logged");
            }
        }
    }
    // The tables themselves: one profile (Me's), nobody else's vectors, and
    // no unnamed-cluster voice.
    let c = store.raw_voice_counts().unwrap();
    assert_eq!((c.profiles, c.other_profiles), (1, 0));
    assert_eq!(c.other_embedding_rows, 0);
    assert_eq!(c.speaker_voices, 0);
}

/// Flag on: a same-language match names and links the speaker; another
/// language only suggests; clusters left unnamed keep their voice, sealed.
#[test]
fn flag_on_applies_same_language_suggests_across_and_seals_unnamed_voices() {
    let (_tmp, store) = open_store();
    let ok = ThirdPartyApproved::assert_flag_checked();
    let consent = VoiceConsent {
        method: "self_checkbox".into(),
        at_ms: 1,
        text_key: "t".into(),
        clip: None,
    };
    let ex = |hz: f32| {
        vec![VoiceExemplar {
            vec: vector(hz),
            source: None,
        }]
    };
    let hoa = store.add_person("Hoa", 2).unwrap();
    store
        .put_voice_profile(
            &hoa,
            &consent,
            None,
            VOICE_MODEL,
            vec![("vi".into(), ex(B))],
            Some(ok),
        )
        .unwrap();
    let binh = store.add_person("Binh", 3).unwrap();
    store
        .put_voice_profile(
            &binh,
            &consent,
            None,
            VOICE_MODEL,
            vec![("en".into(), ex(C))],
            Some(ok),
        )
        .unwrap();

    let runner = runner_with(&store, room_script(), Some(step(third_party_on())), None);
    let meeting = record(&store, &runner, Mode::Room, room_audio(), 24.0);
    assert_eq!(runner.run_pending(), 1);

    let (a, b, c) = (
        speaker_at(&store, &meeting, 1_000),
        speaker_at(&store, &meeting, 9_000),
        speaker_at(&store, &meeting, 17_000),
    );
    // B: Hoa, Vietnamese like her set: named and linked.
    assert_eq!(b.display_name.as_deref(), Some("Hoa"));
    assert_eq!(b.person_gid.as_deref(), Some(hoa.as_str()));
    // C: Binh's voice but an English-only set: a suggestion, no name.
    assert!(c.display_name.is_none() && c.person_gid.is_none());
    let sug = c.suggestion.as_ref().unwrap();
    assert_eq!(
        (sug.person_gid.as_str(), sug.person_name.as_str()),
        (binh.as_str(), "Binh")
    );
    assert!(sug.score > 0.99);
    // A: nobody; unnamed.
    assert!(a.display_name.is_none() && a.suggestion.is_none());
    // The voices of A and C (still unnamed) are kept; B's is not.
    let stored = store.speaker_voices(&meeting).unwrap();
    let who: HashSet<&str> = stored.iter().map(|(g, _)| g.as_str()).collect();
    assert_eq!(who, HashSet::from([a.gid.as_str(), c.gid.as_str()]));
    for (g, v) in &stored {
        let want = if *g == a.gid { A } else { C };
        assert!(cosine(&v.vec, &vector(want)) > 0.99);
        assert_eq!((v.model.as_str(), v.lang.as_str()), (VOICE_MODEL, "vi"));
    }
}

fn call_audio(mic_hz: f32) -> Vec<ReplayTrack> {
    vec![
        ReplayTrack {
            track: Track::Mic,
            samples: tones(24.0, &[(0.5, 7.0, mic_hz)]),
            sample_rate: 48_000,
        },
        ReplayTrack {
            track: Track::System,
            samples: tones(24.0, &[(12.0, 20.0, B)]),
            sample_rate: 48_000,
        },
    ]
}

fn call_script() -> Script {
    Script {
        utterances: vec![
            (1.0, 6.0, "xin chào các bạn nhé".into()),
            (13.0, 19.0, "mình nghe rõ rồi".into()),
        ],
        turns: vec![turn(1, 12.0, 20.0)],
    }
}

/// D7: in a call the mic lines are Me and teach Me's profile, but only if
/// they sound like Me (the speakerphone guard), and never without a profile.
#[test]
fn call_mode_me_learns_with_the_guard_and_never_without_a_profile() {
    // No profile: nothing is learned, nothing is created.
    let (_t1, s1) = open_store();
    let r1 = runner_with(&s1, call_script(), Some(step(no_third_party())), None);
    let m1 = record(&s1, &r1, Mode::Call, call_audio(A), 24.0);
    assert_eq!(r1.run_pending(), 1);
    assert!(s1.me_voice_profile(VOICE_MODEL).unwrap().is_none());
    assert!(s1.speakers(&m1).unwrap().iter().any(|s| s.is_me));

    // A profile and the mic sounds like Me: one exemplar from this meeting,
    // in the lines' language, with its source.
    let (_t2, s2) = open_store();
    enroll_me(&s2, A);
    assert_eq!(me_exemplars(&s2).len(), 3, "20 s: three 6.1 s windows");
    let r2 = runner_with(&s2, call_script(), Some(step(no_third_party())), None);
    let m2 = record(&s2, &r2, Mode::Call, call_audio(A), 24.0);
    assert_eq!(r2.run_pending(), 1);
    let me = s2.me_voice_profile(VOICE_MODEL).unwrap().unwrap();
    let vi = me.sets.iter().find(|s| s.lang == "vi").expect("a vi set");
    assert_eq!(vi.exemplars.len(), 1);
    let src = vi.exemplars[0].source.as_ref().unwrap();
    assert_eq!(src.meeting_gid, m2);
    // Only the part of Me's talk the far side did not overlap: 1-6 s.
    assert!(src.t0_ms >= 1_000 && src.t1_ms <= 7_000, "{src:?}");
    // The far cluster (B) is not Me and is not stored (flag off).
    assert!(s2.speaker_voices(&m2).unwrap().is_empty());

    // A speakerphone: the "mic" is somebody else's voice. Not learned.
    let (_t3, s3) = open_store();
    enroll_me(&s3, A);
    let r3 = runner_with(&s3, call_script(), Some(step(no_third_party())), None);
    let _m3 = record(&s3, &r3, Mode::Call, call_audio(C), 24.0);
    assert_eq!(r3.run_pending(), 1);
    assert_eq!(me_exemplars(&s3).len(), 3, "unchanged");
}

/// A missing model never blocks the pass.
#[test]
fn the_voice_step_never_blocks_the_final_pass() {
    let (_tmp, store) = open_store();
    enroll_me(&store, A);
    let broken = VoiceStep {
        embedder: Arc::new(|| Err("no model".into())),
        ready: always_ready(),
        third_party: no_third_party(),
    };
    let not_ready = VoiceStep {
        embedder: fake_factory(),
        ready: Arc::new(|| false),
        third_party: no_third_party(),
    };
    for voice in [broken, not_ready] {
        let runner = runner_with(&store, room_script(), Some(voice), None);
        let meeting = record(&store, &runner, Mode::Room, room_audio(), 24.0);
        assert_eq!(runner.run_pending(), 1);
        let m = store.get_meeting(&meeting).unwrap();
        assert_eq!(m.transcript_version, 2, "the transcript was stored");
        assert!(store.speakers(&meeting).unwrap().iter().all(|s| !s.is_me));
    }
}

/// `voice_learn`: after the user says who it is, the speaker's retained audio
/// teaches the profile; it waits for recording, checks consent when it runs,
/// and a meeting without audio is simply done.
#[test]
fn voice_learn_adds_exemplars_and_checks_consent_at_run_time() {
    let (_tmp, store) = open_store();
    enroll_me(&store, A);
    let learn = || VoiceLearnJob {
        embedder: fake_factory(),
        ready: always_ready(),
        third_party: no_third_party(),
    };
    // The final pass without voice: A is an unnamed cluster.
    let runner = runner_with(&store, room_script(), None, Some(learn()));
    let meeting = record(&store, &runner, Mode::Room, room_audio(), 24.0);
    assert_eq!(runner.run_pending(), 1);
    let a = speaker_at(&store, &meeting, 1_000);
    assert!(!a.is_me);
    // "This is me", then learn; queued twice, queued once.
    store.set_speaker_me(&a.gid).unwrap();
    queue_learn(&store, &meeting, &a.gid).unwrap();
    queue_learn(&store, &meeting, &a.gid).unwrap();
    let learn_jobs = || {
        store
            .jobs_for_meeting(&meeting)
            .unwrap()
            .into_iter()
            .filter(|j| j.kind == "voice_learn")
            .count()
    };
    assert_eq!(learn_jobs(), 1);
    // A recording preempts it.
    runner.recording_started();
    assert!(runner.run_one().is_none());
    runner.recording_stopped();
    let before = me_exemplars(&store).len();
    assert_eq!(runner.run_pending(), 1);
    let after = me_exemplars(&store);
    assert_eq!(after.len(), before + 1);
    let learned = after.last().unwrap();
    assert!(cosine(&learned.vec, &vector(A)) > 0.99);
    assert_eq!(learned.source.as_ref().unwrap().meeting_gid, meeting);
    // Run again for the same meeting: nothing more.
    queue_learn(&store, &meeting, &a.gid).unwrap();
    assert_eq!(runner.run_pending(), 1);
    assert_eq!(me_exemplars(&store).len(), before + 1);

    // Consent withdrawn (profile deleted) before it runs: nothing is created.
    let me = store
        .voice_profile(&store.me_person().unwrap())
        .unwrap()
        .unwrap();
    store.delete_voice_profile(&me.gid).unwrap();
    let other = store
        .create_meeting(ghi_store::store::NewMeeting {
            title: "x".into(),
            source: "live".into(),
            mode: "room".into(),
            ..Default::default()
        })
        .unwrap();
    let me_sp = store
        .add_speaker(
            &other.gid,
            NewSpeaker {
                label_idx: 0,
                is_me: true,
                color_slot: 1,
                ..Default::default()
            },
        )
        .unwrap();
    queue_learn(&store, &other.gid, &me_sp).unwrap();
    assert_eq!(runner.run_pending(), 1);
    assert!(store.me_voice_profile(VOICE_MODEL).unwrap().is_none());

    // With a profile again but no audio kept: done, unchanged.
    enroll_me(&store, A);
    let n = me_exemplars(&store).len();
    queue_learn(&store, &other.gid, &me_sp).unwrap();
    assert_eq!(runner.run_pending(), 1);
    assert_eq!(me_exemplars(&store).len(), n);
    let j = store
        .jobs_for_meeting(&other.gid)
        .unwrap()
        .into_iter()
        .next_back()
        .unwrap();
    assert_eq!(j.state, ghi_store::jobs::JobState::Done);
}

#[test]
fn a_third_party_speaker_is_not_learned_while_the_flag_is_off() {
    let (_tmp, store) = open_store();
    let ok = ThirdPartyApproved::assert_flag_checked();
    let hoa = store.add_person("Hoa", 2).unwrap();
    store
        .put_voice_profile(
            &hoa,
            &self_consent("t"),
            None,
            VOICE_MODEL,
            vec![(
                "vi".into(),
                vec![VoiceExemplar {
                    vec: vector(B),
                    source: None,
                }],
            )],
            Some(ok),
        )
        .unwrap();
    let learn = VoiceLearnJob {
        embedder: fake_factory(),
        ready: always_ready(),
        third_party: no_third_party(),
    };
    let runner = runner_with(&store, room_script(), None, Some(learn));
    let meeting = record(&store, &runner, Mode::Room, room_audio(), 24.0);
    assert_eq!(runner.run_pending(), 1);
    let b = speaker_at(&store, &meeting, 9_000);
    store.rename_speaker(&b.gid, Some("Hoa")).unwrap();
    queue_learn(&store, &meeting, &b.gid).unwrap();
    assert_eq!(runner.run_pending(), 1);
    let p = store.voice_profile(&hoa).unwrap().unwrap();
    assert_eq!(p.sets[0].exemplars.len(), 1, "nothing was added");
}

#[test]
fn enrollment_needs_consent_and_enough_speech() {
    let (_tmp, store) = open_store();
    let verbal = VoiceConsent {
        method: "verbal_clip".into(),
        ..self_consent("t")
    };
    assert!(enroll_from_pcm(&store, &mut FakeVoice, &passage(A), &verbal).is_err());
    assert!(
        enroll_from_pcm(
            &store,
            &mut FakeVoice,
            &passage(A)[..16_000],
            &self_consent("t")
        )
        .is_err(),
        "one second is not enough"
    );
    for secs in [8, 12] {
        let short = &passage(A)[..16_000 * secs];
        assert!(
            matches!(
                enroll_from_pcm(&store, &mut FakeVoice, short, &self_consent("t")),
                Err(EnrollError::TooLittleSpeech)
            ),
            "{secs} s: under 3 windows"
        );
    }
    assert!(
        enroll_from_pcm(
            &store,
            &mut FakeVoice,
            &vec![0.0; 16_000 * 10],
            &self_consent("t")
        )
        .is_err(),
        "silence is not a voice"
    );
    assert!(store.me_voice_profile(VOICE_MODEL).unwrap().is_none());
    enroll_me(&store, A);
    let p = store.me_voice_profile(VOICE_MODEL).unwrap().unwrap();
    assert_eq!(p.consent.method, "self_checkbox");
    assert_eq!(p.sets[0].lang, ANY_LANG);
    assert!(cosine(&p.sets[0].centroid, &vector(A)) > 0.99);
    // Enrolling again replaces the profile.
    enroll_me(&store, C);
    let p = store.me_voice_profile(VOICE_MODEL).unwrap().unwrap();
    assert!(cosine(&p.sets[0].centroid, &vector(C)) > 0.99);
    assert_eq!(
        store
            .people_overview()
            .unwrap()
            .iter()
            .filter(|p| p.voice.is_some())
            .count(),
        1
    );
}

#[test]
fn removing_a_name_requeues_the_meetings_for_indexing() {
    let (_tmp, store) = open_store();
    ghi_core::index_job::set_enabled(&store, true).unwrap();
    let m = store
        .create_meeting(ghi_store::store::NewMeeting {
            title: "x".into(),
            source: "live".into(),
            mode: "room".into(),
            ..Default::default()
        })
        .unwrap();
    requeue_index(&store, std::slice::from_ref(&m.gid)).unwrap();
    requeue_index(&store, std::slice::from_ref(&m.gid)).unwrap();
    let n = store
        .jobs_for_meeting(&m.gid)
        .unwrap()
        .into_iter()
        .filter(|j| j.kind == ghi_core::index_job::EMBED_INDEX_JOB)
        .count();
    assert_eq!(n, 1);
}

// ------------------------------------------------- review round: races etc.

/// An embedder that runs `hook` once, right after its first window: a user
/// edit landing between embedding and deciding.
struct Hooked {
    inner: FakeVoice,
    hook: Option<Box<dyn FnOnce() + Send>>,
}

impl VoiceEmbed for Hooked {
    fn embed(&mut self, pcm: &[f32]) -> Result<Option<Vec<f32>>, String> {
        let v = self.inner.embed(pcm);
        if let Some(h) = self.hook.take() {
            h();
        }
        v
    }
}

/// A factory whose embedder runs `hook` once (shared across runs).
fn hooked(hook: impl FnOnce() + Send + 'static) -> VoiceFactory {
    let hook = Mutex::new(Some(Box::new(hook) as Box<dyn FnOnce() + Send>));
    Arc::new(move || {
        Ok(Box::new(Hooked {
            inner: FakeVoice,
            hook: hook.lock().unwrap().take(),
        }) as Box<dyn VoiceEmbed>)
    })
}

fn with_factory(f: VoiceFactory, third_party: ThirdPartyGate) -> VoiceStep {
    VoiceStep {
        embedder: f,
        ready: always_ready(),
        third_party,
    }
}

type Slot = Arc<Mutex<Option<String>>>;

fn speaker_idx(store: &Store, meeting: &str, idx: i64) -> ghi_store::store::Speaker {
    store
        .speakers(meeting)
        .unwrap()
        .into_iter()
        .find(|s| s.label_idx == idx)
        .unwrap_or_else(|| panic!("no speaker {idx}"))
}

fn put_me(store: &Store, exemplars: Vec<Vec<f32>>) {
    let me = store.me_person().unwrap();
    let ex = exemplars
        .into_iter()
        .map(|vec| VoiceExemplar { vec, source: None })
        .collect();
    store
        .put_voice_profile(
            &me,
            &self_consent("t"),
            None,
            VOICE_MODEL,
            vec![(ANY_LANG.into(), ex)],
            None,
        )
        .unwrap();
}

/// H1: the user names a cluster after it was embedded but before the match
/// is written: the match is dropped, the user's name stays.
#[test]
fn a_rename_made_during_matching_is_not_overwritten() {
    let (_tmp, store) = open_store();
    let ok = ThirdPartyApproved::assert_flag_checked();
    let hoa = store.add_person("Hoa", 2).unwrap();
    store
        .put_voice_profile(
            &hoa,
            &self_consent("t"),
            None,
            VOICE_MODEL,
            vec![(
                "vi".into(),
                vec![VoiceExemplar {
                    vec: vector(B),
                    source: None,
                }],
            )],
            Some(ok),
        )
        .unwrap();
    let slot: Slot = Default::default();
    let hook = {
        let (store, slot) = (store.clone(), slot.clone());
        move || {
            let m = slot.lock().unwrap().clone().unwrap();
            let b = speaker_idx(&store, &m, 1);
            store.rename_speaker(&b.gid, Some("Minh")).unwrap();
        }
    };
    let runner = runner_with(
        &store,
        room_script(),
        Some(with_factory(hooked(hook), third_party_on())),
        None,
    );
    let meeting = record(&store, &runner, Mode::Room, room_audio(), 24.0);
    *slot.lock().unwrap() = Some(meeting.clone());
    assert_eq!(runner.run_pending(), 1);
    let b = speaker_at(&store, &meeting, 9_000);
    assert_eq!(b.label_idx, 1);
    assert_eq!(b.display_name.as_deref(), Some("Minh"));
    assert_ne!(b.person_gid.as_deref(), Some(hoa.as_str()));
    assert!(b.suggestion.is_none());
    assert!(
        store.speaker_voice(&b.gid).unwrap().is_none(),
        "a named speaker keeps no stored voice"
    );
}

/// H1: somebody else became Me while the match was computed.
#[test]
fn me_claimed_during_matching_is_not_moved() {
    let (_tmp, store) = open_store();
    enroll_me(&store, A);
    let slot: Slot = Default::default();
    let hook = {
        let (store, slot) = (store.clone(), slot.clone());
        move || {
            let m = slot.lock().unwrap().clone().unwrap();
            let c = speaker_idx(&store, &m, 2);
            store.set_speaker_me(&c.gid).unwrap();
        }
    };
    let runner = runner_with(
        &store,
        room_script(),
        Some(with_factory(hooked(hook), no_third_party())),
        None,
    );
    let meeting = record(&store, &runner, Mode::Room, room_audio(), 24.0);
    *slot.lock().unwrap() = Some(meeting.clone());
    assert_eq!(runner.run_pending(), 1);
    let (a, c) = (
        speaker_at(&store, &meeting, 1_000),
        speaker_at(&store, &meeting, 17_000),
    );
    assert!(c.is_me && !a.is_me, "the user's Me stays");
    assert!(a.suggestion.is_none());
}

/// H2: two clusters sound equally like Me: both only suggest.
#[test]
fn two_clusters_that_match_me_equally_only_suggest() {
    let (_tmp, store) = open_store();
    put_me(&store, vec![vector(A), vector(B)]);
    let runner = runner_with(&store, room_script(), Some(step(no_third_party())), None);
    let meeting = record(&store, &runner, Mode::Room, room_audio(), 24.0);
    assert_eq!(runner.run_pending(), 1);
    for t in [1_000, 9_000] {
        let s = speaker_at(&store, &meeting, t);
        assert!(!s.is_me, "{s:?}");
        assert!(s.suggestion.as_ref().is_some_and(|g| g.score > 0.99));
    }
    assert!(store.speakers(&meeting).unwrap().iter().all(|s| !s.is_me));
}

/// H2 (amended 2026-10-05, owner default with veto): a meeting with one
/// voice in a room only suggests a person, but the owner's own voice at
/// T_HIGH in the same language is applied as Me.
#[test]
fn a_lone_voice_in_a_room_is_me_when_it_matches() {
    let (_tmp, store) = open_store();
    enroll_me(&store, A);
    let script = Script {
        utterances: vec![
            (1.0, 3.0, "xin chào mọi người".into()),
            (4.0, 6.0, "hôm nay chốt lịch".into()),
        ],
        turns: vec![turn(1, 0.5, 7.0)],
    };
    let runner = runner_with(&store, script, Some(step(no_third_party())), None);
    let audio = vec![ReplayTrack {
        track: Track::Mic,
        samples: tones(8.0, &[(0.5, 7.0, A)]),
        sample_rate: 48_000,
    }];
    let meeting = record(&store, &runner, Mode::Room, audio, 8.0);
    assert_eq!(runner.run_pending(), 1);
    let a = speaker_at(&store, &meeting, 1_000);
    assert!(a.is_me && a.suggestion.is_none(), "{a:?}");
}

/// H2: under 2 windows and 6 s of speech is not enough to apply.
#[test]
fn a_cluster_with_little_speech_is_only_suggested() {
    let (_tmp, store) = open_store();
    enroll_me(&store, A);
    let script = Script {
        utterances: vec![
            (1.0, 2.5, "xin chào".into()),
            (9.0, 11.0, "anh Hoa nói về kế hoạch".into()),
        ],
        turns: vec![turn(1, 0.5, 3.0), turn(2, 8.0, 15.0)],
    };
    let runner = runner_with(&store, script, Some(step(no_third_party())), None);
    let audio = vec![ReplayTrack {
        track: Track::Mic,
        samples: tones(16.0, &[(0.5, 3.0, A), (8.0, 15.0, B)]),
        sample_rate: 48_000,
    }];
    let meeting = record(&store, &runner, Mode::Room, audio, 16.0);
    assert_eq!(runner.run_pending(), 1);
    let a = speaker_at(&store, &meeting, 1_000);
    assert!(!a.is_me && a.suggestion.is_some(), "2.5 s of speech: {a:?}");
}

/// H2: an imported file never has Me applied by itself.
#[test]
fn an_imported_file_only_suggests_me() {
    let (tmp, store) = open_store();
    enroll_me(&store, A);
    let path = tmp.path().join("meeting.wav");
    let mut w = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for i in 0..16_000 * 24 {
        let t = i as f32 / 16_000.0;
        let hz = if (0.5..7.0).contains(&t) {
            A
        } else if (8.0..15.0).contains(&t) {
            B
        } else if (16.0..23.0).contains(&t) {
            C
        } else {
            0.0
        };
        let x = (i as f32 * hz * std::f32::consts::TAU / 16_000.0).sin() * 0.2;
        w.write_sample((x * 32_767.0) as i16).unwrap();
    }
    w.finalize().unwrap();
    let runner = runner_with(&store, room_script(), Some(step(no_third_party())), None);
    let (tx, _rx) = bus();
    let report = ghi_core::import::import_file(
        &store,
        &path,
        &ghi_core::import::ImportOptions::default(),
        &tx,
    )
    .unwrap();
    assert_eq!(store.get_meeting(&report.meeting).unwrap().source, "file");
    assert_eq!(runner.run_pending(), 1);
    let a = speaker_at(&store, &report.meeting, 1_000);
    assert!(!a.is_me && a.suggestion.is_some(), "{a:?}");
}

/// M1: a profile with no vectors matches nothing.
#[test]
fn a_profile_without_vectors_matches_nothing() {
    let (_tmp, store) = open_store();
    let me = store.me_person().unwrap();
    store
        .put_voice_profile(&me, &self_consent("t"), None, VOICE_MODEL, vec![], None)
        .unwrap();
    let runner = runner_with(&store, room_script(), Some(step(no_third_party())), None);
    let meeting = record(&store, &runner, Mode::Room, room_audio(), 24.0);
    assert_eq!(runner.run_pending(), 1);
    for s in store.speakers(&meeting).unwrap() {
        assert!(!s.is_me && s.suggestion.is_none());
    }
}

/// M6: a person already in the meeting under a name is not applied again.
#[test]
fn a_person_already_in_the_meeting_is_only_suggested() {
    let (_tmp, store) = open_store();
    let ok = ThirdPartyApproved::assert_flag_checked();
    let hoa = store.add_person("Hoa", 2).unwrap();
    store
        .put_voice_profile(
            &hoa,
            &self_consent("t"),
            None,
            VOICE_MODEL,
            vec![(
                "vi".into(),
                vec![VoiceExemplar {
                    vec: vector(B),
                    source: None,
                }],
            )],
            Some(ok),
        )
        .unwrap();
    let runner = runner_with(&store, room_script(), Some(step(third_party_on())), None);
    let meeting = record(&store, &runner, Mode::Room, room_audio(), 24.0);
    // The live transcript named A's voice "Hoa" (a mistake the user made).
    let sp = store
        .add_speaker(
            &meeting,
            NewSpeaker {
                label_idx: 0,
                color_slot: 1,
                ..Default::default()
            },
        )
        .unwrap();
    store.rename_speaker(&sp, Some("Hoa")).unwrap();
    store
        .replace_transcript(
            &meeting,
            vec![NewSegment {
                speaker_gid: Some(sp.clone()),
                t0_ms: 1_000,
                t1_ms: 3_000,
                text: "xin chào".into(),
                lang: Some("vi".into()),
                ..Default::default()
            }],
        )
        .unwrap();
    assert_eq!(runner.run_pending(), 1);
    let b = speaker_at(&store, &meeting, 9_000);
    assert!(b.display_name.is_none(), "{b:?}");
    assert_eq!(
        b.suggestion.as_ref().map(|s| s.person_gid.as_str()),
        Some(hoa.as_str())
    );
}

/// M3: a recording arriving while Me's voice is embedded skips the learning
/// (the pass still ends) and queues it; it runs after the recording.
#[test]
fn me_learning_preempted_by_a_recording_is_queued() {
    let (_tmp, store) = open_store();
    enroll_me(&store, A);
    let runner_slot: Arc<std::sync::OnceLock<Arc<JobRunner>>> = Default::default();
    let hook = {
        let r = runner_slot.clone();
        move || r.get().unwrap().recording_started()
    };
    let script = Script {
        utterances: vec![
            (1.0, 10.0, "xin chào các bạn nhé".into()),
            (13.0, 19.0, "mình nghe rõ rồi".into()),
        ],
        turns: vec![turn(1, 12.0, 20.0)],
    };
    let runner = runner_with(
        &store,
        script,
        Some(with_factory(hooked(hook), no_third_party())),
        Some(VoiceLearnJob {
            embedder: fake_factory(),
            ready: always_ready(),
            third_party: no_third_party(),
        }),
    );
    runner_slot.set(runner.clone()).ok();
    let audio = vec![
        ReplayTrack {
            track: Track::Mic,
            samples: tones(24.0, &[(0.5, 11.5, A)]),
            sample_rate: 48_000,
        },
        ReplayTrack {
            track: Track::System,
            samples: tones(24.0, &[(12.0, 20.0, B)]),
            sample_rate: 48_000,
        },
    ];
    let meeting = record(&store, &runner, Mode::Call, audio, 24.0);
    let before = me_exemplars(&store).len();
    assert_eq!(
        runner.run_pending(),
        1,
        "the pass ends; nothing else runs while recording"
    );
    assert_eq!(store.get_meeting(&meeting).unwrap().transcript_version, 2);
    assert_eq!(me_exemplars(&store).len(), before, "learning was skipped");
    let queued: Vec<_> = store
        .jobs_for_meeting(&meeting)
        .unwrap()
        .into_iter()
        .filter(|j| j.kind == "voice_learn" && j.state == ghi_store::jobs::JobState::Queued)
        .collect();
    assert_eq!(queued.len(), 1);
    // The recording ends: it learns.
    runner.recording_stopped();
    assert_eq!(runner.run_pending(), 1);
    assert_eq!(me_exemplars(&store).len(), before + 1);
}

/// M2 + M4: Me moves to another speaker: the exemplars this meeting gave are
/// dropped and the new Me is learned from (or refused by the guard); one job
/// per meeting takes several speakers.
#[test]
fn moving_me_drops_what_the_meeting_taught_and_jobs_merge_per_meeting() {
    let (_tmp, store) = open_store();
    enroll_me(&store, A);
    let learn = VoiceLearnJob {
        embedder: fake_factory(),
        ready: always_ready(),
        third_party: no_third_party(),
    };
    let runner = runner_with(&store, room_script(), None, Some(learn));
    let meeting = record(&store, &runner, Mode::Room, room_audio(), 24.0);
    assert_eq!(runner.run_pending(), 1);
    let (a, b, c) = (
        speaker_at(&store, &meeting, 1_000),
        speaker_at(&store, &meeting, 9_000),
        speaker_at(&store, &meeting, 17_000),
    );
    let base = me_exemplars(&store).len();
    ghi_core::voice_job::set_me(&store, &meeting, &a.gid).unwrap();
    assert_eq!(runner.run_pending(), 1);
    assert_eq!(me_exemplars(&store).len(), base + 1);
    // "No, B is me": A's voice leaves Me's profile at once.
    ghi_core::voice_job::set_me(&store, &meeting, &b.gid).unwrap();
    assert_eq!(me_exemplars(&store).len(), base, "A's exemplar was dropped");
    assert!(
        store
            .speakers(&meeting)
            .unwrap()
            .iter()
            .any(|s| s.gid == b.gid && s.is_me)
    );
    assert_eq!(runner.run_pending(), 1);
    assert_eq!(
        me_exemplars(&store).len(),
        base,
        "B does not sound like Me: the guard holds"
    );
    // Several speakers, one queued job.
    ghi_core::voice_job::queue_learn(&store, &meeting, &a.gid).unwrap();
    ghi_core::voice_job::queue_learn(&store, &meeting, &c.gid).unwrap();
    ghi_core::voice_job::queue_learn(&store, &meeting, &c.gid).unwrap();
    let queued: Vec<_> = store
        .jobs_for_meeting(&meeting)
        .unwrap()
        .into_iter()
        .filter(|j| j.kind == "voice_learn" && j.state == ghi_store::jobs::JobState::Queued)
        .collect();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].payload["speakers"].as_array().unwrap().len(), 2);
    assert_eq!(queued[0].payload["meeting"], meeting.as_str());
    assert_eq!(
        runner.run_pending(),
        1,
        "neither is Me or named: nothing to learn, done"
    );
    assert_eq!(me_exemplars(&store).len(), base);
}

/// M5: in a call only the mic speaker can be Me.
#[test]
fn in_a_call_a_far_side_speaker_cannot_be_me() {
    let (_tmp, store) = open_store();
    let runner = runner_with(&store, call_script(), Some(step(no_third_party())), None);
    let meeting = record(&store, &runner, Mode::Call, call_audio(A), 24.0);
    assert_eq!(runner.run_pending(), 1);
    let speakers = store.speakers(&meeting).unwrap();
    let far = speakers
        .iter()
        .find(|s| !s.is_me && s.label_idx >= 0)
        .unwrap();
    assert!(store.set_speaker_me(&far.gid).is_err());
    assert!(!store.set_speaker_me_if_unclaimed(&far.gid).unwrap());
    let mic = speakers.iter().find(|s| s.is_me).unwrap();
    store.set_speaker_me(&mic.gid).unwrap();
}

/// L6: Me keeps at most 20 exemplars per language; the newest stay.
#[test]
fn me_keeps_twenty_exemplars_per_language() {
    use ghi_core::profiles::ClusterVoice;
    let (_tmp, store) = open_store();
    enroll_me(&store, A);
    for i in 0..25 {
        let me = store.me_voice_profile(VOICE_MODEL).unwrap().unwrap();
        let v = ClusterVoice {
            vec: vector(A),
            longest: 0..97_600,
            windows: 1,
            speech: 97_600,
        };
        assert!(
            ghi_core::voice_step::learn_me(&store, &me, &format!("m{i}"), &v, Some("en")).unwrap()
        );
    }
    let me = store.me_voice_profile(VOICE_MODEL).unwrap().unwrap();
    let en = me.sets.iter().find(|s| s.lang == "en").unwrap();
    assert_eq!(en.exemplars.len(), 20);
    let from: Vec<&str> = en
        .exemplars
        .iter()
        .map(|e| e.source.as_ref().unwrap().meeting_gid.as_str())
        .collect();
    assert_eq!((from[0], from[19]), ("m5", "m24"));
    assert_eq!(
        me.sets
            .iter()
            .find(|s| s.lang == ANY_LANG)
            .unwrap()
            .exemplars
            .len(),
        3
    );
}

/// L6: with the flag on, an accepted third-party speaker is learned from.
#[test]
fn flag_on_voice_learn_adds_to_a_third_partys_profile() {
    let (_tmp, store) = open_store();
    let ok = ThirdPartyApproved::assert_flag_checked();
    let hoa = store.add_person("Hoa", 2).unwrap();
    store
        .put_voice_profile(
            &hoa,
            &self_consent("t"),
            None,
            VOICE_MODEL,
            vec![(
                "vi".into(),
                vec![VoiceExemplar {
                    vec: vector(B),
                    source: None,
                }],
            )],
            Some(ok),
        )
        .unwrap();
    let learn = VoiceLearnJob {
        embedder: fake_factory(),
        ready: always_ready(),
        third_party: third_party_on(),
    };
    let runner = runner_with(&store, room_script(), None, Some(learn));
    let meeting = record(&store, &runner, Mode::Room, room_audio(), 24.0);
    assert_eq!(runner.run_pending(), 1);
    let b = speaker_at(&store, &meeting, 9_000);
    store.rename_speaker(&b.gid, Some("Hoa")).unwrap();
    queue_learn(&store, &meeting, &b.gid).unwrap();
    assert_eq!(runner.run_pending(), 1);
    let p = store.voice_profile(&hoa).unwrap().unwrap();
    let vi = p.sets.iter().find(|s| s.lang == "vi").unwrap();
    assert_eq!(vi.exemplars.len(), 2);
    let src = vi.exemplars[1].source.as_ref().unwrap();
    assert_eq!(src.meeting_gid, meeting);
    assert!(src.t0_ms >= 8_000 && src.t1_ms <= 15_000, "{src:?}");
}

/// L1: a vector never shows in `Debug`.
#[test]
fn debug_output_hides_vectors() {
    let (_tmp, store) = open_store();
    enroll_me(&store, A);
    let me = store.me_voice_profile(VOICE_MODEL).unwrap().unwrap();
    let cv = ghi_core::profiles::ClusterVoice {
        vec: vector(B),
        longest: 0..10,
        windows: 1,
        speech: 10,
    };
    let shown = format!("{me:?} {:?} {cv:?}", me.sets[0]);
    for x in vector(A).into_iter().chain(vector(B)).take(400) {
        assert!(!shown.contains(&format!("{x}")), "a component is in Debug");
    }
    assert!(shown.contains("hidden") || shown.contains("exemplars"));
}
