// SPDX-License-Identifier: Apache-2.0
//! Leased jobs (phase 15, doc 07 §8): the fence at the claim, at checkpoints
//! and before the commit; jobs without a lease are not affected.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ghi_audio::Track;
use ghi_core::capture::{ReplayTrack, replay};
use ghi_core::engines::{FakeEngines, Script, SpeechEngines};
use ghi_core::events::bus;
use ghi_core::final_pass::FinalPassJob;
use ghi_core::jobs::{FenceAt, JobRunner, always_ready};
use ghi_core::live::Mode;
use ghi_core::notes_job::{NOTES_FINAL_JOB, NotesJob};
use ghi_core::session::{FINAL_PASS_JOB, JOB_PAYLOAD_VERSION, Session, SessionConfig};
use ghi_llm::{Completion, EngineInfo, Llm, Request};
use ghi_speech::SpeakerSegment;
use ghi_store::jobs::JobState;
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::{NewSegment, Store};
use serde_json::json;

struct OneLiner;

impl Llm for OneLiner {
    fn engine(&self) -> EngineInfo {
        EngineInfo {
            name: "scripted".into(),
            version: "1".into(),
        }
    }
    fn context_tokens(&self) -> u32 {
        16_384
    }
    fn complete(&mut self, _req: &Request) -> ghi_llm::Result<Completion> {
        Ok(Completion {
            text: r#"{"tldr":[{"text":"Chốt lịch beta","cite":[0]}],"decisions":[],"action_items":[],"open_questions":[],"key_quotes":[],"topics":[]}"#.into(),
            tokens_in: 10,
            tokens_out: 5,
            truncated: false,
        })
    }
}

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

struct Rig {
    store: Arc<Store>,
    runner: Arc<JobRunner>,
    meeting: String,
    /// Engine loads (a run that got past the claim).
    loads: Arc<AtomicUsize>,
    _tmp: tempfile::TempDir,
}

/// A recorded meeting (one mic track, no queued jobs) and a runner with the
/// final pass and the final notes.
fn rig() -> Rig {
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
    let loads = Arc::new(AtomicUsize::new(0));
    let (final_engines, l2) = (engines.clone(), loads.clone());
    let llm: ghi_core::notes_job::LlmFactory =
        Arc::new(|_| Ok(Box::new(OneLiner) as Box<dyn Llm + Send>));
    let runner = JobRunner::new(
        store.clone(),
        tx.clone(),
        vec![
            Arc::new(FinalPassJob {
                engines: Arc::new(move || {
                    l2.fetch_add(1, Ordering::SeqCst);
                    Ok(final_engines.clone())
                }),
                chunk_s: 600.0,
                ready: always_ready(),
                voice: None,
            }),
            Arc::new(NotesJob {
                kind: NOTES_FINAL_JOB,
                version: 2,
                template: ghi_llm::template::builtin("general").unwrap(),
                llm,
                ready: always_ready(),
            }),
        ],
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
    let s = Session::start(
        store.clone(),
        Some(engines),
        capture,
        SessionConfig {
            sensitive: false,
            mode: Mode::Room,
            language: None,
            title: "leased".into(),
            queue_jobs: false,
            lossless: true,
            echo_cancellation: true,
        },
        tx,
        Some(runner.clone()),
    )
    .unwrap();
    let t = std::time::Instant::now();
    while store.segments(s.meeting()).unwrap().is_empty() {
        assert!(t.elapsed() < std::time::Duration::from_secs(20));
        let _ = rx.recv_timeout(std::time::Duration::from_millis(50));
    }
    let meeting = s.meeting().to_string();
    s.stop().unwrap();
    Rig {
        store,
        runner,
        meeting,
        loads,
        _tmp: tmp,
    }
}

fn version(r: &Rig) -> i64 {
    r.store.get_meeting(&r.meeting).unwrap().transcript_version
}

fn queue_leased(r: &Rig, kind: &str, uuid: &str) -> i64 {
    r.store
        .enqueue_job(
            Some(&r.meeting),
            kind,
            JOB_PAYLOAD_VERSION,
            &json!({"lease": uuid, "epoch": 7}),
        )
        .unwrap()
}

#[test]
fn a_failed_fence_before_the_claim_means_zero_executions() {
    let r = rig();
    let v = version(&r);
    r.runner.set_fence(Arc::new(|_, at| at != FenceAt::Claim));
    r.store
        .insert_lease_for_tests("u1", &r.meeting, 7, "granted")
        .unwrap();
    let id = queue_leased(&r, FINAL_PASS_JOB, "u1");
    assert_eq!(r.runner.run_pending(), 1);
    assert_eq!(r.loads.load(Ordering::SeqCst), 0, "the pass never ran");
    assert_eq!(r.store.job(id).unwrap().state, JobState::Done);
    assert_eq!(version(&r), v);
}

#[test]
fn a_fence_that_fails_after_the_voice_step_commits_nothing() {
    let r = rig();
    let (v, live) = (version(&r), r.store.segments(&r.meeting).unwrap());
    // Everything holds until the commit's own look.
    r.runner.set_fence(Arc::new(|_, at| at != FenceAt::Commit));
    r.store
        .insert_lease_for_tests("u2", &r.meeting, 7, "granted")
        .unwrap();
    let id = queue_leased(&r, FINAL_PASS_JOB, "u2");
    assert_eq!(r.runner.run_pending(), 1);
    assert_eq!(r.loads.load(Ordering::SeqCst), 1, "it did run");
    assert_eq!(r.store.job(id).unwrap().state, JobState::Done);
    assert_eq!(version(&r), v, "no replace_transcript");
    let now = r.store.segments(&r.meeting).unwrap();
    assert_eq!(
        now.iter().map(|s| &s.gid).collect::<Vec<_>>(),
        live.iter().map(|s| &s.gid).collect::<Vec<_>>()
    );
    assert!(
        r.store
            .active_job(&r.meeting, NOTES_FINAL_JOB)
            .unwrap()
            .is_none(),
        "no notes queued by an abandoned pass"
    );
}

#[test]
fn a_checkpoint_fence_failure_drops_the_job_instead_of_requeueing() {
    let r = rig();
    let v = version(&r);
    r.runner.set_fence(Arc::new(|_, at| at == FenceAt::Claim));
    r.store
        .insert_lease_for_tests("u3", &r.meeting, 7, "granted")
        .unwrap();
    let id = queue_leased(&r, FINAL_PASS_JOB, "u3");
    assert_eq!(r.runner.run_pending(), 1);
    let j = r.store.job(id).unwrap();
    assert_eq!(j.state, JobState::Done, "dropped, not queued again");
    assert_eq!(version(&r), v);
    assert_eq!(r.runner.run_pending(), 0);
}

#[test]
fn a_revoked_lease_at_the_commit_is_fenced_by_the_store_and_writes_nothing() {
    let r = rig();
    let v = version(&r);
    // The runner's fence still says yes (the revoke raced it): the store's
    // compare-and-set is what stops the commit.
    r.runner.set_fence(Arc::new(|_, _| true));
    r.store
        .insert_lease_for_tests("u4", &r.meeting, 7, "revoked")
        .unwrap();
    let id = queue_leased(&r, FINAL_PASS_JOB, "u4");
    assert_eq!(r.runner.run_pending(), 1);
    assert_eq!(r.store.job(id).unwrap().state, JobState::Done);
    assert_eq!(version(&r), v, "nothing written");
    assert!(
        r.store
            .active_job(&r.meeting, NOTES_FINAL_JOB)
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_granted_lease_commits_at_its_epoch() {
    let r = rig();
    let v = version(&r);
    r.runner.set_fence(Arc::new(|_, _| true));
    r.store
        .insert_lease_for_tests("u5", &r.meeting, 7, "granted")
        .unwrap();
    let id = queue_leased(&r, FINAL_PASS_JOB, "u5");
    // The pass, then the notes it queues (their own lease is the sync layer's).
    assert_eq!(r.runner.run_pending(), 2);
    assert_eq!(r.store.job(id).unwrap().state, JobState::Done);
    assert_eq!(version(&r), v + 1);
    // The lease closed in the commit's transaction: a second commit under it
    // is fenced.
    let again = r.store.replace_transcript_epoch(
        &r.meeting,
        vec![NewSegment {
            text: "x".into(),
            ..Default::default()
        }],
        7,
        Some("u5"),
    );
    assert!(matches!(again, Err(ghi_store::StoreError::Fenced)));
}

#[test]
fn leased_notes_are_fenced_before_and_at_the_commit() {
    let r = rig();
    // The live line is the transcript; no final pass needed for notes.
    r.runner.set_fence(Arc::new(|_, at| at != FenceAt::Commit));
    r.store
        .insert_lease_for_tests("n1", &r.meeting, 7, "granted")
        .unwrap();
    let id = queue_leased(&r, NOTES_FINAL_JOB, "n1");
    assert_eq!(r.runner.run_pending(), 1);
    assert_eq!(r.store.job(id).unwrap().state, JobState::Done);
    assert!(r.store.note_blocks(&r.meeting).unwrap().is_empty());

    // A revoked lease: the store stops the commit.
    r.runner.set_fence(Arc::new(|_, _| true));
    r.store
        .insert_lease_for_tests("n2", &r.meeting, 8, "revoked")
        .unwrap();
    let id = queue_leased(&r, NOTES_FINAL_JOB, "n2");
    assert_eq!(r.runner.run_pending(), 1);
    assert_eq!(r.store.job(id).unwrap().state, JobState::Done);
    assert!(r.store.note_blocks(&r.meeting).unwrap().is_empty());

    // Granted: the notes land at the lease epoch.
    r.store
        .insert_lease_for_tests("n3", &r.meeting, 9, "granted")
        .unwrap();
    queue_leased(&r, NOTES_FINAL_JOB, "n3");
    assert_eq!(r.runner.run_pending(), 1);
    assert!(!r.store.note_blocks(&r.meeting).unwrap().is_empty());
}

#[test]
fn jobs_without_a_lease_ignore_the_fence() {
    let r = rig();
    let v = version(&r);
    r.runner.set_fence(Arc::new(|_, _| false));
    let id = r
        .store
        .enqueue_job(
            Some(&r.meeting),
            FINAL_PASS_JOB,
            JOB_PAYLOAD_VERSION,
            &json!({}),
        )
        .unwrap();
    assert_eq!(r.runner.run_pending(), 2, "the pass, then its notes");
    assert_eq!(r.store.job(id).unwrap().state, JobState::Done);
    assert_eq!(version(&r), v + 1);
}
