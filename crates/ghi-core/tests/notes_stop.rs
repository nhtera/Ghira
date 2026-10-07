// SPDX-License-Identifier: Apache-2.0
//! The phone's notes engine is stopped mid-answer when a recording starts or
//! the app leaves the screen: the job yields (queued again, no attempt used,
//! nothing written) instead of running on for minutes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ghi_core::events::bus;
use ghi_core::jobs::{JobRunner, Outcome, always_ready};
use ghi_core::notes_job::{NOTES_FINAL_JOB, NotesJob};
use ghi_core::session::{JOB_PAYLOAD_VERSION, RecordingHooks};
use ghi_llm::{Completion, EngineInfo, Llm, LlmError, Request};
use ghi_store::jobs::JobState;
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::{NewMeeting, NewSegment, Store};

/// Answers only once stopped (then fails), like the in-process engine.
struct Stoppable {
    stop: Arc<AtomicBool>,
    asked: Arc<AtomicBool>,
}

impl Llm for Stoppable {
    fn engine(&self) -> EngineInfo {
        EngineInfo {
            name: "stoppable".into(),
            version: "1".into(),
        }
    }
    fn context_tokens(&self) -> u32 {
        16_384
    }
    fn complete(&mut self, _req: &Request) -> ghi_llm::Result<Completion> {
        self.asked.store(true, Ordering::SeqCst);
        let t = Instant::now();
        while !self.stop.load(Ordering::SeqCst) {
            assert!(t.elapsed() < Duration::from_secs(10), "never stopped");
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(LlmError::Worker("stopped".into()))
    }
    fn stopper(&self) -> Option<Arc<dyn Fn() + Send + Sync>> {
        let stop = self.stop.clone();
        Some(Arc::new(move || stop.store(true, Ordering::SeqCst)))
    }
}

fn setup() -> (
    Arc<Store>,
    Arc<JobRunner>,
    i64,
    Arc<AtomicBool>,
    Arc<AtomicBool>,
    tempfile::TempDir,
) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    let m = store.create_meeting(NewMeeting::default()).unwrap().gid;
    store
        .add_segments(
            &m,
            vec![NewSegment {
                t0_ms: 0,
                t1_ms: 2000,
                text: "chốt lịch beta vào thứ sáu".into(),
                lang: Some("vi".into()),
                ..Default::default()
            }],
        )
        .unwrap();
    let id = store
        .enqueue_job(
            Some(&m),
            NOTES_FINAL_JOB,
            JOB_PAYLOAD_VERSION,
            &serde_json::json!({}),
        )
        .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let asked = Arc::new(AtomicBool::new(false));
    let (s, a) = (stop.clone(), asked.clone());
    let (tx, _rx) = bus();
    let runner = JobRunner::new(
        store.clone(),
        tx,
        vec![Arc::new(NotesJob {
            kind: NOTES_FINAL_JOB,
            version: 2,
            template: ghi_llm::template::builtin("general").unwrap(),
            llm: Arc::new(move |_| {
                Ok(Box::new(Stoppable {
                    stop: s.clone(),
                    asked: a.clone(),
                }) as Box<dyn Llm + Send>)
            }),
            ready: always_ready(),
        })],
    );
    (store, runner, id, stop, asked, tmp)
}

fn stopped_by(interrupt: impl Fn(&JobRunner)) {
    let (store, runner, id, stop, asked, _tmp) = setup();
    let worker = {
        let runner = runner.clone();
        std::thread::spawn(move || runner.run_one())
    };
    let t = Instant::now();
    while !asked.load(Ordering::SeqCst) {
        assert!(
            t.elapsed() < Duration::from_secs(10),
            "the model was never asked"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    interrupt(&runner);
    let (_, outcome) = worker.join().unwrap().expect("a job ran");
    assert!(stop.load(Ordering::SeqCst), "the engine was told to stop");
    assert!(matches!(outcome, Ok(Outcome::Yield(_))), "{outcome:?}");
    assert!(t.elapsed() < Duration::from_secs(5));
    let job = store.job(id).unwrap();
    assert_eq!(job.state, JobState::Queued, "it runs again later");
    assert!(
        store
            .note_blocks(&job.meeting_gid.unwrap())
            .unwrap()
            .is_empty(),
        "nothing written"
    );
}

#[test]
fn a_recording_stops_the_notes_engine_and_the_job_waits() {
    stopped_by(|r| r.recording_started());
}

#[test]
fn leaving_the_screen_stops_the_notes_engine_and_the_job_waits() {
    stopped_by(|r| r.app_inactive());
}
