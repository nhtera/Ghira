// SPDX-License-Identifier: Apache-2.0
//! Sensitive meeting mode (doc 02, P1): no audio kept, no final pass, no voice
//! learning, the transcript stays; on at the start, turned on mid-recording
//! (one way) or set on a stored meeting.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ghi_audio::Track;
use ghi_core::capture::{ReplayTrack, replay};
use ghi_core::engines::{FakeEngines, Script, SpeechEngines};
use ghi_core::events::{Event, bus};
use ghi_core::jobs::{JobRunner, always_ready};
use ghi_core::live::Mode;
use ghi_core::profiles::{FakeVoice, VOICE_MODEL, VoiceEmbed, VoiceFactory};
use ghi_core::session::{Session, SessionConfig};
use ghi_core::voice_job::{VoiceLearnJob, enroll_from_pcm, queue_learn, self_consent};
use ghi_speech::SpeakerSegment;
use ghi_store::jobs::JobState;
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::{NewMeeting, NewSpeaker, Store, TrackKind};

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

fn script() -> Script {
    Script {
        utterances: vec![
            (1.0, 2.0, "xin chào mọi người".into()),
            (3.0, 4.0, "hôm nay chốt lịch beta".into()),
        ],
        turns: vec![SpeakerSegment {
            start: 0.5,
            end: 4.5,
            speaker: 1,
        }],
    }
}

fn capture() -> ghi_core::capture::Capture {
    replay(
        vec![ReplayTrack {
            track: Track::Mic,
            samples: (0..48_000 * 6)
                .map(|i| (i as f32 * 0.03).sin() * 0.2)
                .collect(),
            sample_rate: 48_000,
        }],
        None,
    )
    .unwrap()
}

fn engines() -> Option<Arc<dyn SpeechEngines>> {
    Some(FakeEngines::new(script()) as Arc<dyn SpeechEngines>)
}

fn config(sensitive: bool) -> SessionConfig {
    SessionConfig {
        mode: Mode::Room,
        language: None,
        title: "nhạy cảm".into(),
        queue_jobs: true,
        lossless: true,
        echo_cancellation: true,
        sensitive,
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let t = Instant::now();
    while !done() {
        assert!(t.elapsed() < Duration::from_secs(20), "timed out: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn job_kinds(store: &Store, meeting: &str) -> Vec<String> {
    store
        .jobs_for_meeting(meeting)
        .unwrap()
        .into_iter()
        .map(|j| j.kind)
        .collect()
}

fn fake_factory() -> VoiceFactory {
    Arc::new(|| Ok(Box::new(FakeVoice) as Box<dyn VoiceEmbed>))
}

fn me_exemplar_count(store: &Store) -> usize {
    store
        .me_voice_profile(VOICE_MODEL)
        .unwrap()
        .map(|p| p.sets.iter().map(|s| s.exemplars.len()).sum())
        .unwrap_or(0)
}

#[test]
fn a_sensitive_recording_writes_no_audio_and_keeps_the_transcript() {
    let (tmp, store) = open_store();
    let (tx, rx) = bus();
    let s = Session::start(store.clone(), engines(), capture(), config(true), tx, None).unwrap();
    let meeting = s.meeting().to_string();
    assert!(s.sensitive());
    // Not even a bundle file is created.
    assert!(store.tracks(&meeting).unwrap().is_empty());
    assert!(!tmp.path().join("bundles").join(&meeting).exists());
    wait_for("the replay plays out", || {
        s.source_ended() && s.now_ms() >= 5_900
    });
    let snap = s.snapshot();
    assert!(snap.sensitive);
    assert_eq!(snap.lines.len(), 2, "the transcript is kept");
    s.stop().unwrap();
    let events: Vec<Event> = rx.try_iter().map(|e| e.event).collect();
    assert!(events.iter().any(
        |e| matches!(e, Event::SensitiveChanged { meeting: m, sensitive: true } if *m == meeting)
    ));

    let m = store.get_meeting(&meeting).unwrap();
    assert!(m.sensitive && m.status != "recording");
    assert!(!store.audio_available(&meeting).unwrap());
    assert!(!tmp.path().join("bundles").join(&meeting).exists());
    assert_eq!(store.segments(&meeting).unwrap().len(), 2);
    // Notes from the live transcript, but no final pass (it needs the audio).
    assert_eq!(job_kinds(&store, &meeting), ["notes_final"]);
}

#[test]
fn turning_it_on_mid_recording_drops_the_audio_so_far_at_stop() {
    let (tmp, store) = open_store();
    let (tx, rx) = bus();
    let s = Session::start(store.clone(), engines(), capture(), config(false), tx, None).unwrap();
    let meeting = s.meeting().to_string();
    assert!(!s.sensitive());
    assert!(
        s.set_sensitive(false).is_ok(),
        "off while off is nothing to do"
    );
    wait_for("some audio is written", || s.now_ms() >= 2_500);
    assert!(store.audio_available(&meeting).unwrap(), "audio so far");

    s.set_sensitive(true).unwrap();
    assert!(s.sensitive() && s.snapshot().sensitive);
    assert!(store.get_meeting(&meeting).unwrap().sensitive);
    s.set_sensitive(true).unwrap();
    // One way: part of the audio is gone, so it cannot go back to normal.
    assert!(s.set_sensitive(false).is_err());
    assert!(s.sensitive());
    wait_for("the replay plays out", || {
        s.source_ended() && s.now_ms() >= 5_900
    });
    s.stop().unwrap();
    let on_events = rx
        .try_iter()
        .filter(|e| {
            matches!(
                e.event,
                Event::SensitiveChanged {
                    sensitive: true,
                    ..
                }
            )
        })
        .count();
    assert_eq!(on_events, 1, "announced once");

    assert!(!store.audio_available(&meeting).unwrap());
    assert!(!tmp.path().join("bundles").join(&meeting).exists());
    assert_eq!(store.segments(&meeting).unwrap().len(), 2);
    assert_eq!(job_kinds(&store, &meeting), ["notes_final"]);
}

#[test]
fn a_sensitive_recording_can_still_discard_its_last_seconds() {
    let (_tmp, store) = open_store();
    let (tx, _rx) = bus();
    let s = Session::start(store.clone(), engines(), capture(), config(true), tx, None).unwrap();
    let meeting = s.meeting().to_string();
    wait_for("the replay plays out", || {
        s.source_ended() && s.now_ms() >= 5_900 && s.snapshot().lines.len() == 2
    });
    // The second line (3-4 s) goes; there is no audio to cut, only text.
    s.discard(3.0).unwrap();
    s.stop().unwrap();
    let texts: Vec<String> = store
        .segments(&meeting)
        .unwrap()
        .into_iter()
        .map(|x| x.text)
        .collect();
    assert_eq!(texts, ["xin chào mọi người"]);
    assert!(store.pending_discards().unwrap().is_empty());
    assert!(!store.audio_available(&meeting).unwrap());
}

#[test]
fn a_normal_recording_still_keeps_its_audio_and_queues_the_final_pass() {
    let (_tmp, store) = open_store();
    let (tx, _rx) = bus();
    let s = Session::start(store.clone(), engines(), capture(), config(false), tx, None).unwrap();
    let meeting = s.meeting().to_string();
    wait_for("the replay plays out", || {
        s.source_ended() && s.now_ms() >= 5_900
    });
    s.stop().unwrap();
    assert!(!store.get_meeting(&meeting).unwrap().sensitive);
    assert_eq!(store.tracks(&meeting).unwrap()[0].0, TrackKind::Mic);
    assert_eq!(job_kinds(&store, &meeting), ["notes_live", "final_pass"]);
}

#[test]
fn sensitive_mode_needs_a_live_transcript() {
    let (_tmp, store) = open_store();
    let (tx, _rx) = bus();
    let r = Session::start(store.clone(), None, capture(), config(true), tx, None);
    let e = r.err().expect("refused without speech engines");
    assert!(e.0.contains("sensitive"), "{e}");
    assert!(
        store.list_meetings(10, 0).unwrap().is_empty(),
        "no half-made meeting"
    );
}

#[test]
fn a_stored_meeting_loses_its_audio_when_marked_sensitive_but_not_while_its_pass_is_pending() {
    let (tmp, store) = open_store();
    let m = store
        .create_meeting(NewMeeting {
            title: "đã ghi".into(),
            source: "live".into(),
            mode: "room".into(),
            ..Default::default()
        })
        .unwrap()
        .gid;
    let mut w = store.open_track(&m, TrackKind::Mic).unwrap();
    w.append(b"pcm").unwrap();
    store.finish_track(&m, TrackKind::Mic, w).unwrap();
    store.set_waveform(&m, &[1, 2, 3]).unwrap();
    store.set_meeting_status(&m, "processing").unwrap();

    // No transcript: nothing would be kept, so nothing is deleted.
    assert_eq!(
        ghi_core::sensitive::set_stored(&store, &m, true).unwrap_err(),
        ghi_core::sensitive::ERR_NO_TRANSCRIPT
    );
    assert!(!store.get_meeting(&m).unwrap().sensitive);
    assert!(store.audio_available(&m).unwrap());

    store
        .add_segment(
            &m,
            ghi_store::store::NewSegment {
                t0_ms: 0,
                t1_ms: 1000,
                text: "chữ ở lại".into(),
                ..Default::default()
            },
        )
        .unwrap();
    // The final pass (queued, or waiting for models) still needs the audio and
    // its result is the transcript: refused, and the flag is put back.
    let job = store
        .enqueue_job(Some(&m), "final_pass", 1, &serde_json::json!({}))
        .unwrap();
    assert_eq!(
        ghi_core::sensitive::set_stored(&store, &m, true).unwrap_err(),
        ghi_core::sensitive::ERR_PENDING
    );
    assert!(!store.get_meeting(&m).unwrap().sensitive);
    assert!(store.audio_available(&m).unwrap());
    store.cancel_job(job).unwrap();

    let applied = ghi_core::sensitive::set_stored(&store, &m, true).unwrap();
    assert_eq!(applied.tracks, 1);
    assert!(store.get_meeting(&m).unwrap().sensitive);
    assert!(!store.audio_available(&m).unwrap());
    assert!(!tmp.path().join("bundles").join(&m).exists());
    assert_eq!(store.waveform(&m).unwrap(), None);
    assert_eq!(store.segments(&m).unwrap()[0].text, "chữ ở lại");

    // Off: the flag only; the audio stays gone.
    ghi_core::sensitive::set_stored(&store, &m, false).unwrap();
    assert!(!store.get_meeting(&m).unwrap().sensitive);
    assert!(!store.audio_available(&m).unwrap());

    // A recording meeting is changed through its session.
    let rec = store
        .create_meeting(NewMeeting {
            title: "đang ghi".into(),
            ..Default::default()
        })
        .unwrap()
        .gid;
    assert!(ghi_core::sensitive::set_stored(&store, &rec, true).is_err());
}

#[test]
fn a_recording_without_a_live_transcript_cannot_turn_sensitive_on() {
    let (_tmp, store) = open_store();
    let (tx, _rx) = bus();
    let s = Session::start(store.clone(), None, capture(), config(false), tx, None).unwrap();
    let meeting = s.meeting().to_string();
    let e = s.set_sensitive(true).unwrap_err();
    assert!(
        e.0.contains("sensitive mode needs the speech models"),
        "{e}"
    );
    assert!(!s.sensitive());
    assert!(!store.get_meeting(&meeting).unwrap().sensitive);
    // Off while off is nothing to do.
    s.set_sensitive(false).unwrap();
    s.stop().unwrap();
    assert!(store.audio_available(&meeting).unwrap());
}

#[test]
fn a_final_pass_that_meets_a_sensitive_meeting_settles_it() {
    use ghi_core::final_pass::{FinalPassJob, FinalPassNoNotes};
    let (_tmp, store) = open_store();
    let make = |notes: bool| -> Arc<JobRunner> {
        let (tx, _rx) = bus();
        let engines: Arc<dyn SpeechEngines> = FakeEngines::new(script());
        let job = FinalPassJob {
            engines: Arc::new(move || Ok(engines.clone())),
            chunk_s: 600.0,
            ready: always_ready(),
            voice: None,
        };
        let handler: Arc<dyn ghi_core::jobs::JobHandler> = if notes {
            Arc::new(job)
        } else {
            Arc::new(FinalPassNoNotes(job))
        };
        JobRunner::new(store.clone(), tx, vec![handler])
    };
    let sensitive_meeting = |title: &str| {
        let m = store
            .create_meeting(NewMeeting {
                title: title.into(),
                source: "live".into(),
                mode: "room".into(),
                sensitive: true,
                ..Default::default()
            })
            .unwrap()
            .gid;
        store.set_meeting_status(&m, "processing").unwrap();
        store
            .enqueue_job(Some(&m), "final_pass", 1, &serde_json::json!({}))
            .unwrap();
        m
    };
    // The phone: no notes job, so the pass itself must settle the meeting.
    let phone = sensitive_meeting("phone");
    assert_eq!(make(false).run_pending(), 1);
    assert_eq!(store.get_meeting(&phone).unwrap().status, "ready");
    // The desktop: the notes follow, as for a meeting without audio.
    let desktop = sensitive_meeting("desktop");
    assert_eq!(make(true).run_pending(), 1);
    assert!(store.active_job(&desktop, "notes_final").unwrap().is_some());
}

#[test]
fn a_sensitive_meeting_never_teaches_a_voice() {
    let (_tmp, store) = open_store();
    enroll_from_pcm(
        &store,
        &mut FakeVoice,
        &(0..16_000 * 20)
            .map(|i| (i as f32 * 300.0 * std::f32::consts::TAU / 16_000.0).sin() * 0.2)
            .collect::<Vec<_>>(),
        &self_consent("onboarding.voice.consent"),
    )
    .unwrap();
    let before = me_exemplar_count(&store);
    assert!(before > 0);

    // A meeting that still has its audio (flag set, audio not yet deleted).
    let m = store
        .create_meeting(NewMeeting {
            title: "nhạy cảm".into(),
            source: "live".into(),
            mode: "room".into(),
            sensitive: true,
            ..Default::default()
        })
        .unwrap()
        .gid;
    let me = store
        .add_speaker(
            &m,
            NewSpeaker {
                label_idx: 0,
                is_me: true,
                color_slot: 1,
                ..Default::default()
            },
        )
        .unwrap();
    store
        .add_segment(
            &m,
            ghi_store::store::NewSegment {
                speaker_gid: Some(me.clone()),
                t0_ms: 0,
                t1_ms: 10_000,
                text: "tôi đang nói rất lâu".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let mut w = store.open_track(&m, TrackKind::Mic).unwrap();
    w.append(b"pcm").unwrap();
    store.finish_track(&m, TrackKind::Mic, w).unwrap();

    // "This is me" queues nothing.
    queue_learn(&store, &m, &me).unwrap();
    assert!(job_kinds(&store, &m).is_empty());
    // A job queued some other way does nothing when it runs.
    store
        .enqueue_job(
            Some(&m),
            "voice_learn",
            1,
            &serde_json::json!({"meeting": m, "speakers": [me]}),
        )
        .unwrap();
    let (tx, _rx) = bus();
    let runner = JobRunner::new(
        store.clone(),
        tx,
        vec![Arc::new(VoiceLearnJob {
            embedder: fake_factory(),
            ready: always_ready(),
            third_party: Arc::new(|| None),
        })],
    );
    assert_eq!(runner.run_pending(), 1);
    assert_eq!(me_exemplar_count(&store), before, "nothing was learned");
    let j = store.jobs_for_meeting(&m).unwrap().pop().unwrap();
    assert_eq!(j.state, JobState::Done);
}

#[test]
fn marking_a_meeting_sensitive_drops_the_voices_learned_from_it() {
    let (_tmp, store) = open_store();
    let m = store
        .create_meeting(NewMeeting {
            title: "đã học".into(),
            source: "live".into(),
            mode: "room".into(),
            ..Default::default()
        })
        .unwrap()
        .gid;
    enroll_from_pcm(
        &store,
        &mut FakeVoice,
        &(0..16_000 * 20)
            .map(|i| (i as f32 * 300.0 * std::f32::consts::TAU / 16_000.0).sin() * 0.2)
            .collect::<Vec<_>>(),
        &self_consent("onboarding.voice.consent"),
    )
    .unwrap();
    let me = store.me_voice_profile(VOICE_MODEL).unwrap().unwrap();
    store
        .add_voice_exemplars(
            &me.gid,
            VOICE_MODEL,
            ghi_core::profiles::ANY_LANG,
            vec![ghi_store::voice::VoiceExemplar {
                vec: FakeVoice::vector(7),
                source: Some(ghi_store::voice::ExemplarSource {
                    meeting_gid: m.clone(),
                    t0_ms: 0,
                    t1_ms: 1000,
                }),
            }],
            None,
        )
        .unwrap();
    let with = me_exemplar_count(&store);
    store.set_meeting_status(&m, "done").unwrap();
    store
        .add_segment(
            &m,
            ghi_store::store::NewSegment {
                t0_ms: 0,
                t1_ms: 1000,
                text: "có chữ".into(),
                ..Default::default()
            },
        )
        .unwrap();
    ghi_core::sensitive::set_stored(&store, &m, true).unwrap();
    assert_eq!(me_exemplar_count(&store), with - 1);
}

// ------------------------------------------------ notes after a sensitive stop

struct OneLiner;

impl ghi_llm::Llm for OneLiner {
    fn engine(&self) -> ghi_llm::EngineInfo {
        ghi_llm::EngineInfo {
            name: "scripted".into(),
            version: "1".into(),
        }
    }
    fn context_tokens(&self) -> u32 {
        16_384
    }
    fn complete(&mut self, _req: &ghi_llm::Request) -> ghi_llm::Result<ghi_llm::Completion> {
        Ok(ghi_llm::Completion {
            text: r#"{"tldr":[{"text":"Chốt lịch beta","cite":[0]}],"decisions":[],"action_items":[],"open_questions":[],"key_quotes":[],"topics":[]}"#.into(),
            tokens_in: 10,
            tokens_out: 5,
            truncated: false,
        })
    }
}

fn notes_runner(
    store: &Arc<Store>,
    llm: ghi_core::notes_job::LlmFactory,
    ready: bool,
) -> Arc<JobRunner> {
    use ghi_core::notes_job::{NOTES_FINAL_JOB, NotesJob};
    let (tx, _rx) = bus();
    JobRunner::new(
        store.clone(),
        tx,
        vec![Arc::new(NotesJob {
            kind: NOTES_FINAL_JOB,
            version: 2,
            template: ghi_llm::template::builtin("general").unwrap(),
            llm,
            ready: if ready {
                always_ready()
            } else {
                Arc::new(|| false)
            },
        })],
    )
}

/// A sensitive recording, stopped, with its two lines.
fn stopped_sensitive(store: &Arc<Store>) -> String {
    let (tx, _rx) = bus();
    let s = Session::start(store.clone(), engines(), capture(), config(true), tx, None).unwrap();
    let meeting = s.meeting().to_string();
    wait_for("the replay plays out", || {
        s.source_ended() && s.now_ms() >= 5_900 && s.snapshot().lines.len() == 2
    });
    s.stop().unwrap();
    meeting
}

#[test]
fn a_sensitive_recording_is_ready_at_stop_and_its_notes_follow_with_a_model() {
    let (_tmp, store) = open_store();
    let meeting = stopped_sensitive(&store);
    // Ready as recorded: no pass will refine the transcript.
    assert_eq!(store.get_meeting(&meeting).unwrap().status, "ready");
    assert!(store.note_blocks(&meeting).unwrap().is_empty());
    let llm: ghi_core::notes_job::LlmFactory =
        Arc::new(|_| Ok(Box::new(OneLiner) as Box<dyn ghi_llm::Llm + Send>));
    assert_eq!(notes_runner(&store, llm, true).run_pending(), 1);
    assert_eq!(store.get_meeting(&meeting).unwrap().status, "ready");
    assert!(
        !store.note_blocks(&meeting).unwrap().is_empty(),
        "notes were written"
    );
    assert!(!store.audio_available(&meeting).unwrap());
}

#[test]
fn a_sensitive_recording_stays_ready_without_a_model_or_when_the_model_fails() {
    let (_tmp, store) = open_store();
    let meeting = stopped_sensitive(&store);
    // No model installed: the notes job waits, the meeting is ready anyway.
    let missing: ghi_core::notes_job::LlmFactory = Arc::new(|_| Err("no model".into()));
    assert_eq!(
        notes_runner(&store, missing.clone(), false).run_pending(),
        0
    );
    assert_eq!(store.get_meeting(&meeting).unwrap().status, "ready");
    // The model fails: the job fails, the meeting is still ready, no notes.
    notes_runner(&store, missing, true).run_pending();
    assert_eq!(store.get_meeting(&meeting).unwrap().status, "ready");
    assert!(store.note_blocks(&meeting).unwrap().is_empty());
    assert_eq!(
        store.segments(&meeting).unwrap().len(),
        2,
        "the transcript is kept"
    );
}

// ------------------------------------------------ conditional store writes

#[test]
fn sensitive_mode_is_set_in_one_step_unless_a_final_pass_is_active() {
    let (_tmp, store) = open_store();
    let m = store
        .create_meeting(NewMeeting {
            title: "x".into(),
            ..Default::default()
        })
        .unwrap()
        .gid;
    let job = store
        .enqueue_job(Some(&m), "final_pass", 1, &serde_json::json!({}))
        .unwrap();
    assert!(!store.set_sensitive_unless_final_pass_active(&m).unwrap());
    assert!(
        !store.get_meeting(&m).unwrap().sensitive,
        "nothing half-way"
    );
    store.cancel_job(job).unwrap();
    assert!(store.set_sensitive_unless_final_pass_active(&m).unwrap());
    assert!(store.get_meeting(&m).unwrap().sensitive);
    assert!(
        store
            .set_sensitive_unless_final_pass_active(&ghi_store::new_gid())
            .is_err()
    );
}

#[test]
fn a_sensitive_meeting_gets_no_waveform_and_no_stored_voices_from_the_store() {
    let (_tmp, store) = open_store();
    let m = store
        .create_meeting(NewMeeting {
            title: "x".into(),
            ..Default::default()
        })
        .unwrap()
        .gid;
    let mut w = store.open_track(&m, TrackKind::Mic).unwrap();
    w.append(b"pcm").unwrap();
    store.finish_track(&m, TrackKind::Mic, w).unwrap();
    let sp = store
        .add_speaker(
            &m,
            NewSpeaker {
                label_idx: 0,
                color_slot: 1,
                ..Default::default()
            },
        )
        .unwrap();
    let token = ghi_store::voice::ThirdPartyApproved::assert_flag_checked();
    store.set_sensitive(&m, true).unwrap();
    // The audio is still there (the flag came first): neither is written.
    store.set_waveform(&m, &[1, 2]).unwrap();
    assert_eq!(store.waveform(&m).unwrap(), None);
    store
        .put_speaker_voice(&sp, VOICE_MODEL, "any", &FakeVoice::vector(3), token)
        .unwrap();
    assert!(store.speaker_voices(&m).unwrap().is_empty());
    // Not sensitive: both are written.
    store.set_sensitive(&m, false).unwrap();
    store.set_waveform(&m, &[1, 2]).unwrap();
    assert_eq!(store.waveform(&m).unwrap(), Some(vec![1, 2]));
    store
        .put_speaker_voice(&sp, VOICE_MODEL, "any", &FakeVoice::vector(3), token)
        .unwrap();
    assert_eq!(store.speaker_voices(&m).unwrap().len(), 1);
}

#[test]
fn a_discard_the_caller_gave_up_on_is_dropped_not_committed_late() {
    use ghi_core::live::{DiscardGate, PersistMsg};
    let (_tmp, store) = open_store();
    let m = store
        .create_meeting(NewMeeting {
            title: "x".into(),
            ..Default::default()
        })
        .unwrap()
        .gid;
    let (events, _rx) = bus();
    let persist = ghi_core::persist::Persist::new(store.clone(), m.clone(), events);
    let (msgs, rx) = crossbeam_channel::unbounded();
    let thread = std::thread::spawn(move || persist.run(rx));
    let discard = |gate: Arc<DiscardGate>| {
        let (reply, answer) = crossbeam_channel::bounded(1);
        msgs.send(PersistMsg::Discard {
            t_cut_ms: 0,
            now_ms: 1000,
            keep: Vec::new(),
            gate,
            reply,
        })
        .unwrap();
        answer
    };
    // Timed out before the thread got to it: abandoned, and it never runs.
    let gate = Arc::new(DiscardGate::default());
    let (_unused_reply, never) = crossbeam_channel::bounded::<Result<i64, String>>(1);
    assert!(gate.settle(&never).is_none());
    let answer = discard(gate);
    assert_eq!(answer.recv().unwrap().unwrap_err(), "abandoned");
    assert!(store.pending_discards().unwrap().is_empty());
    // A live request is committed, and a late settle finds its answer.
    let gate = Arc::new(DiscardGate::default());
    let answer = discard(gate.clone());
    std::thread::sleep(Duration::from_millis(300));
    let late = gate.settle(&answer).expect("it ran meanwhile");
    assert!(late.is_ok());
    assert_eq!(store.pending_discards().unwrap().len(), 1);
    drop(msgs);
    thread.join().unwrap();
}
