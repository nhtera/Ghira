// SPDX-License-Identifier: Apache-2.0
//! The phone's notes engine is stopped mid-answer when a recording starts or
//! the app leaves the screen: the job yields (queued again, no attempt used,
//! nothing written) instead of running on for minutes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ghi_core::events::bus;
use ghi_core::jobs::JobHandler;
use ghi_core::jobs::{JobRunner, Outcome, always_ready};
use ghi_core::notes_job::{NOTES_FINAL_JOB, NotesJob, PhoneNotesJob};
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

type Setup = (
    Arc<Store>,
    Arc<JobRunner>,
    i64,
    Arc<AtomicBool>,
    Arc<AtomicBool>,
    tempfile::TempDir,
);

fn setup() -> Setup {
    setup_with(None)
}

/// With `(hot, critical)`, the phone's job (compact notes; starts only while
/// not hot, stops only when critical).
fn setup_with(heat: Option<(Arc<AtomicBool>, Arc<AtomicBool>)>) -> Setup {
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
        vec![{
            let job = NotesJob {
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
            };
            match heat {
                Some((hot, critical)) => Arc::new(PhoneNotesJob {
                    job,
                    hot: Arc::new(move || hot.load(Ordering::SeqCst)),
                    too_hot: Arc::new(move || critical.load(Ordering::SeqCst)),
                }) as Arc<dyn JobHandler>,
                None => Arc::new(job),
            }
        }],
    );
    (store, runner, id, stop, asked, tmp)
}

fn stopped_by(interrupt: impl Fn(&JobRunner)) {
    stopped_in(setup(), interrupt);
}

fn stopped_in(setup: Setup, interrupt: impl Fn(&JobRunner)) {
    let (store, runner, id, stop, asked, _tmp) = setup;
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

#[test]
fn a_phone_that_heats_up_keeps_writing_and_stops_only_when_critical() {
    let hot = Arc::new(AtomicBool::new(false));
    let critical = Arc::new(AtomicBool::new(false));
    let setup = setup_with(Some((hot.clone(), critical.clone())));
    let stop = setup.3.clone();
    stopped_in(setup, |_| {
        // Serious: the run goes on (one prompt outlasts the cool spells).
        hot.store(true, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(500));
        assert!(!stop.load(Ordering::SeqCst), "not stopped while only hot");
        // Critical: stopped, queued again.
        critical.store(true, Ordering::SeqCst);
    });
}

#[test]
fn a_hot_phone_starts_no_notes_until_it_cools() {
    let hot = Arc::new(AtomicBool::new(true));
    let (_store, runner, _id, stop, _asked, _tmp) =
        setup_with(Some((hot.clone(), Arc::new(AtomicBool::new(false)))));
    assert!(runner.run_one().is_none(), "a hot phone starts no notes");
    hot.store(false, Ordering::SeqCst);
    stop.store(true, Ordering::SeqCst); // the scripted model answers at once
    assert!(runner.run_one().is_some(), "cool again: the notes run");
}

/// The phone asks for compact notes: no quotes or topics in its schema.
#[test]
fn the_phone_writes_compact_notes() {
    struct Schema(Arc<std::sync::Mutex<Option<serde_json::Value>>>);
    impl Llm for Schema {
        fn engine(&self) -> EngineInfo {
            EngineInfo {
                name: "schema".into(),
                version: "1".into(),
            }
        }
        fn context_tokens(&self) -> u32 {
            16_384
        }
        fn complete(&mut self, req: &Request) -> ghi_llm::Result<Completion> {
            *self.0.lock().unwrap() = req.schema.clone();
            Ok(Completion {
                text: r#"{"tldr":[{"text":"Chốt lịch beta","cite":[0]}],"decisions":[],"action_items":[],"open_questions":[],"key_quotes":[],"topics":[]}"#.into(),
                tokens_in: 10,
                tokens_out: 5,
                truncated: false,
            })
        }
    }
    let (store, _runner, _id, _stop, _asked, _tmp) = setup();
    let seen = Arc::new(std::sync::Mutex::new(None));
    let s = seen.clone();
    let (tx, _rx) = bus();
    let runner = JobRunner::new(
        store.clone(),
        tx,
        vec![Arc::new(PhoneNotesJob {
            job: NotesJob {
                kind: NOTES_FINAL_JOB,
                version: 2,
                template: ghi_llm::template::builtin("general").unwrap(),
                llm: Arc::new(move |_| Ok(Box::new(Schema(s.clone())) as Box<dyn Llm + Send>)),
                ready: always_ready(),
            },
            hot: Arc::new(|| false),
            too_hot: Arc::new(|| false),
        })],
    );
    assert_eq!(runner.run_pending(), 1);
    let schema = seen.lock().unwrap().clone().expect("asked with a schema");
    assert_eq!(schema["properties"]["key_quotes"]["maxItems"], 0);
    assert_eq!(schema["properties"]["topics"]["maxItems"], 0);
}

/// A 20-minute meeting on the phone: notes in 5-minute parts. Stopped (the app
/// left the screen) while on part 3, the next run goes on from part 3: parts 1
/// and 2 are not read again.
#[test]
fn the_phone_resumes_its_notes_from_the_last_part() {
    use std::sync::atomic::AtomicUsize;
    /// Answers every part with no facts and the reduce with empty notes;
    /// the call numbered `block` waits until stopped.
    struct Parts {
        calls: Arc<AtomicUsize>,
        block: usize,
        stop: Arc<AtomicBool>,
        waiting: Arc<AtomicBool>,
    }
    impl Llm for Parts {
        fn engine(&self) -> EngineInfo {
            EngineInfo {
                name: "parts".into(),
                version: "1".into(),
            }
        }
        fn context_tokens(&self) -> u32 {
            12_288
        }
        fn complete(&mut self, req: &Request) -> ghi_llm::Result<Completion> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if n == self.block {
                self.waiting.store(true, Ordering::SeqCst);
                let t = Instant::now();
                while !self.stop.load(Ordering::SeqCst) {
                    assert!(t.elapsed() < Duration::from_secs(10), "never stopped");
                    std::thread::sleep(Duration::from_millis(10));
                }
                return Err(LlmError::Worker("stopped".into()));
            }
            let reduce = req.messages[1].content.contains("- [")
                || !req
                    .schema
                    .as_ref()
                    .unwrap()
                    .to_string()
                    .contains("\"facts\"");
            Ok(Completion {
                text: if reduce {
                    r#"{"tldr":[],"decisions":[],"action_items":[],"open_questions":[],"key_quotes":[],"topics":[]}"#.into()
                } else {
                    r#"{"facts":[]}"#.into()
                },
                tokens_in: 10,
                tokens_out: 5,
                truncated: false,
            })
        }
        fn stopper(&self) -> Option<Arc<dyn Fn() + Send + Sync>> {
            let stop = self.stop.clone();
            Some(Arc::new(move || stop.store(true, Ordering::SeqCst)))
        }
    }
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
            (0..40i64)
                .map(|i| NewSegment {
                    t0_ms: i * 30_000,
                    t1_ms: i * 30_000 + 25_000,
                    text: "we talk about the beta plan".into(),
                    lang: Some("en".into()),
                    ..Default::default()
                })
                .collect(),
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
    let calls = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let waiting = Arc::new(AtomicBool::new(false));
    let block = Arc::new(AtomicUsize::new(3));
    let (c, s, w, b) = (calls.clone(), stop.clone(), waiting.clone(), block.clone());
    let (tx, _rx) = bus();
    let runner = JobRunner::new(
        store.clone(),
        tx,
        vec![Arc::new(PhoneNotesJob {
            job: NotesJob {
                kind: NOTES_FINAL_JOB,
                version: 2,
                template: ghi_llm::template::builtin("general").unwrap(),
                llm: Arc::new(move |_| {
                    Ok(Box::new(Parts {
                        calls: c.clone(),
                        block: b.load(Ordering::SeqCst),
                        stop: s.clone(),
                        waiting: w.clone(),
                    }) as Box<dyn Llm + Send>)
                }),
                ready: always_ready(),
            },
            hot: Arc::new(|| false),
            too_hot: Arc::new(|| false),
        })],
    );
    let worker = {
        let runner = runner.clone();
        std::thread::spawn(move || runner.run_one())
    };
    let t = Instant::now();
    while !waiting.load(Ordering::SeqCst) {
        assert!(t.elapsed() < Duration::from_secs(10), "part 3 never asked");
        std::thread::sleep(Duration::from_millis(10));
    }
    runner.app_inactive();
    let (_, outcome) = worker.join().unwrap().expect("a job ran");
    let Ok(Outcome::Yield(payload)) = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(payload["done"], 2, "two parts kept");
    assert_eq!(store.job(id).unwrap().state, JobState::Queued);
    // Back on screen: parts 3 and 4, then the notes from all the parts.
    runner.app_active();
    block.store(usize::MAX, Ordering::SeqCst);
    calls.store(0, Ordering::SeqCst);
    assert_eq!(runner.run_pending(), 1);
    assert_eq!(store.job(id).unwrap().state, JobState::Done);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "parts 3-4 and the reduce, not 1-2 again"
    );
}
