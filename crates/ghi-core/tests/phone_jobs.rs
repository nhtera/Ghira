// SPDX-License-Identifier: Apache-2.0
//! The phone's use of the job runner: explicit job kinds at stop, a final pass
//! that queues no `notes_final`, and the app-inactive preempt.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ghi_audio::Track;
use ghi_core::capture::{ReplayTrack, replay};
use ghi_core::engines::{FakeEngines, Script, SpeechEngines};
use ghi_core::events::{Event, bus};
use ghi_core::final_pass::{FinalPassJob, FinalPassNoNotes};
use ghi_core::jobs::{JobRunner, always_ready};
use ghi_core::live::Mode;
use ghi_core::notes_job::NOTES_FINAL_JOB;
use ghi_core::session::{FINAL_PASS_JOB, NOTES_LIVE_JOB, RecordingHooks, Session, SessionConfig};
use ghi_speech::SpeakerSegment;
use ghi_store::jobs::JobState;
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::{NewMeeting, NewSpeaker, Store};

fn script() -> Script {
    Script {
        utterances: vec![(1.0, 2.0, "xin chào mọi người".into())],
        turns: vec![SpeakerSegment {
            start: 0.5,
            end: 2.5,
            speaker: 1,
        }],
    }
}

#[test]
fn phone_queues_only_the_final_pass_and_it_adds_no_notes() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    let (tx, rx) = bus();
    let engines: Arc<dyn SpeechEngines> = FakeEngines::new(script());
    let final_engines = engines.clone();
    let runner = JobRunner::new(
        store.clone(),
        tx.clone(),
        vec![Arc::new(FinalPassNoNotes(FinalPassJob {
            engines: Arc::new(move || Ok(final_engines.clone())),
            chunk_s: 600.0,
            ready: always_ready(),
            voice: None,
        }))],
    );
    let capture = replay(
        vec![ReplayTrack {
            track: Track::Mic,
            samples: (0..48_000 * 4)
                .map(|i| (i as f32 * 0.03).sin() * 0.2)
                .collect(),
            sample_rate: 48_000,
        }],
        None,
    )
    .unwrap();
    let mut s = Session::start(
        store.clone(),
        Some(engines),
        capture,
        SessionConfig {
            sensitive: false,
            mode: Mode::Room,
            language: None,
            title: "phone".into(),
            queue_jobs: true,
            lossless: true,
            echo_cancellation: true,
        },
        tx,
        Some(runner.clone()),
    )
    .unwrap();
    s.set_job_kinds(&[FINAL_PASS_JOB]);
    let meeting = s.meeting().to_string();
    let t = Instant::now();
    loop {
        assert!(t.elapsed() < Duration::from_secs(20));
        if let Ok(env) = rx.recv_timeout(Duration::from_millis(50))
            && matches!(env.event, Event::TranscriptFinal { .. })
        {
            break;
        }
    }
    let report = s.stop().unwrap();
    assert_eq!(report.jobs.len(), 1);
    assert!(
        store
            .active_job(&meeting, NOTES_LIVE_JOB)
            .unwrap()
            .is_none(),
        "no notes_live on the phone"
    );

    // Inactive: nothing is claimed, even after a recording came and went.
    runner.app_inactive();
    runner.recording_started();
    runner.recording_stopped();
    assert!(runner.run_one().is_none());
    assert_eq!(store.job(report.jobs[0]).unwrap().state, JobState::Queued);
    runner.app_active();
    assert_eq!(runner.run_pending(), 1);
    assert_eq!(store.job(report.jobs[0]).unwrap().state, JobState::Done);
    assert!(
        store
            .active_job(&meeting, NOTES_FINAL_JOB)
            .unwrap()
            .is_none(),
        "no notes_final queued"
    );
    assert_eq!(store.get_meeting(&meeting).unwrap().status, "ready");
}

#[test]
fn a_pass_that_goes_inactive_mid_run_yields_uncommitted_then_commits_once() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    let (tx, rx) = bus();
    let engines: Arc<dyn SpeechEngines> = FakeEngines::new(script());
    let slot: Arc<std::sync::OnceLock<Arc<JobRunner>>> = Default::default();
    let once = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let (final_engines, s2, o2) = (engines.clone(), slot.clone(), once.clone());
    let runner = JobRunner::new(
        store.clone(),
        tx.clone(),
        vec![Arc::new(FinalPassNoNotes(FinalPassJob {
            // The app goes to the background while the engines load.
            engines: Arc::new(move || {
                if o2.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    s2.get().unwrap().app_inactive();
                }
                Ok(final_engines.clone())
            }),
            chunk_s: 600.0,
            ready: always_ready(),
            voice: None,
        }))],
    );
    slot.set(runner.clone()).ok();
    let capture = replay(
        vec![ReplayTrack {
            track: Track::Mic,
            samples: (0..48_000 * 4)
                .map(|i| (i as f32 * 0.03).sin() * 0.2)
                .collect(),
            sample_rate: 48_000,
        }],
        None,
    )
    .unwrap();
    let mut s = Session::start(
        store.clone(),
        Some(engines),
        capture,
        SessionConfig {
            sensitive: false,
            mode: Mode::Room,
            language: None,
            title: "phone".into(),
            queue_jobs: true,
            lossless: true,
            echo_cancellation: true,
        },
        tx,
        Some(runner.clone()),
    )
    .unwrap();
    s.set_job_kinds(&[FINAL_PASS_JOB]);
    let meeting = s.meeting().to_string();
    let t = Instant::now();
    loop {
        assert!(t.elapsed() < Duration::from_secs(20));
        if let Ok(env) = rx.recv_timeout(Duration::from_millis(50))
            && matches!(env.event, Event::TranscriptFinal { .. })
        {
            break;
        }
    }
    let job = s.stop().unwrap().jobs[0];
    let live = store.segments(&meeting).unwrap().len();
    let version = store.get_meeting(&meeting).unwrap().transcript_version;

    let (_, r) = runner.run_one().unwrap();
    assert!(matches!(r, Ok(ghi_core::jobs::Outcome::Yield(_))));
    let j = store.job(job).unwrap();
    assert_eq!((j.state, j.attempts), (JobState::Queued, 0));
    assert_eq!(store.segments(&meeting).unwrap().len(), live);
    assert_eq!(
        store.get_meeting(&meeting).unwrap().transcript_version,
        version,
        "nothing committed while inactive"
    );

    runner.app_active();
    assert_eq!(runner.run_pending(), 1);
    assert_eq!(store.job(job).unwrap().state, JobState::Done);
    let m = store.get_meeting(&meeting).unwrap();
    assert_eq!(m.transcript_version, version + 1, "committed exactly once");
    assert_eq!(
        store.segments(&meeting).unwrap().len(),
        live,
        "no duplicates"
    );
}

/// Drives a final pass over a one-track meeting made with `engines`'s own
/// live script; returns the runner and meeting.
fn phone_meeting(
    store: &Arc<Store>,
    live: Script,
    final_script: Option<Script>,
    tx: ghi_core::events::EventTx,
    rx: &ghi_core::events::EventRx,
) -> (Arc<JobRunner>, String, i64) {
    let live_engines: Arc<dyn SpeechEngines> = FakeEngines::new(live);
    let final_engines: Option<Arc<dyn SpeechEngines>> =
        final_script.map(|f| FakeEngines::new(f) as _);
    let runner = JobRunner::new(
        store.clone(),
        tx.clone(),
        vec![Arc::new(FinalPassNoNotes(FinalPassJob {
            engines: Arc::new(move || final_engines.clone().ok_or_else(|| "no models".to_string())),
            chunk_s: 600.0,
            ready: always_ready(),
            voice: None,
        }))],
    );
    let capture = replay(
        vec![ReplayTrack {
            track: Track::Mic,
            samples: (0..48_000 * 4)
                .map(|i| (i as f32 * 0.03).sin() * 0.2)
                .collect(),
            sample_rate: 48_000,
        }],
        None,
    )
    .unwrap();
    let mut s = Session::start(
        store.clone(),
        Some(live_engines),
        capture,
        SessionConfig {
            sensitive: false,
            mode: Mode::Room,
            language: None,
            title: "phone".into(),
            queue_jobs: true,
            lossless: true,
            echo_cancellation: true,
        },
        tx,
        Some(runner.clone()),
    )
    .unwrap();
    s.set_job_kinds(&[FINAL_PASS_JOB]);
    let meeting = s.meeting().to_string();
    let t = Instant::now();
    loop {
        assert!(t.elapsed() < Duration::from_secs(20));
        if let Ok(env) = rx.recv_timeout(Duration::from_millis(50))
            && matches!(env.event, Event::TranscriptFinal { .. })
        {
            break;
        }
    }
    let job = s.stop().unwrap().jobs[0];
    (runner, meeting, job)
}

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

#[test]
fn speakers_a_yielded_pass_added_do_not_pile_up_on_rerun() {
    let (_tmp, store) = open_store();
    let (tx, rx) = bus();
    // Live heard one voice; the final pass hears a second one.
    let final_script = Script {
        utterances: vec![
            (1.0, 2.0, "xin chào mọi người".into()),
            (2.5, 3.5, "chào anh".into()),
        ],
        turns: vec![
            SpeakerSegment {
                start: 0.5,
                end: 2.2,
                speaker: 1,
            },
            SpeakerSegment {
                start: 2.3,
                end: 3.8,
                speaker: 2,
            },
        ],
    };
    let (runner, meeting, _) = phone_meeting(&store, script(), Some(final_script), tx, &rx);
    // What an earlier yielded run leaves: a new, unnamed speaker with no lines.
    store
        .add_speaker(
            &meeting,
            NewSpeaker {
                label_idx: 1,
                color_slot: 1,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(runner.run_pending(), 1);
    let speakers = store.speakers(&meeting).unwrap();
    let lines = store.segments(&meeting).unwrap();
    assert_eq!(speakers.len(), 2, "the ghost is gone, one new speaker");
    for sp in &speakers {
        assert!(
            lines
                .iter()
                .any(|l| l.speaker_gid.as_deref() == Some(sp.gid.as_str())),
            "every speaker has a line"
        );
    }
}

#[test]
fn an_empty_pass_and_a_failed_pass_settle_the_meeting_ready_with_an_event() {
    let (_tmp, store) = open_store();
    let (tx, rx) = bus();
    // The engines fail to load: the pass fails, the live transcript stays.
    let (runner, failed, _) = phone_meeting(&store, script(), None, tx, &rx);
    // A meeting with no audio at all: the pass has nothing to do.
    let empty = store.create_meeting(NewMeeting::default()).unwrap().gid;
    store.finish_meeting(&empty, 0).unwrap();
    store.set_meeting_status(&empty, "processing").unwrap();
    store
        .enqueue_job(Some(&empty), FINAL_PASS_JOB, 1, &serde_json::json!({}))
        .unwrap();
    assert_eq!(runner.run_pending(), 2);
    for m in [&failed, &empty] {
        assert_eq!(store.get_meeting(m).unwrap().status, "ready");
    }
    let told: Vec<String> = rx
        .try_iter()
        .filter_map(|e| match e.event {
            Event::StateChanged {
                meeting,
                state: ghi_core::events::SessionState::Ready,
            } => Some(meeting),
            _ => None,
        })
        .collect();
    assert!(
        told.contains(&failed) && told.contains(&empty),
        "the UI is told"
    );
}
