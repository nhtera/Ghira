// SPDX-License-Identifier: Apache-2.0
//! The job runner: the only consumer of the store's `jobs` table.
//!
//! - One job at a time, in handler priority order (notes first: the ≤3 min
//!   notes-after-stop target [RT-7]).
//! - Recording preempts [RT-10]: while a session records, no job is claimed,
//!   and the one running is asked to yield at its next checkpoint; it goes
//!   back to the queue with its resume point and without spending an attempt.
//! - A crash leaves the job `running`; the store requeues it at the next open
//!   (that one does count as an attempt).
//! - Payloads hold numbers and identifiers only; handlers fetch content by gid.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use ghi_store::jobs::Job;
use ghi_store::store::Store;
use serde_json::Value;

use crate::events::{ErrorKind, Event, EventTx, Stage};
use crate::session::{JOB_PAYLOAD_VERSION, RecordingHooks};

/// How a handler run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Done,
    /// Preempted: requeue with this resume payload.
    Yield(Value),
}

pub struct JobCtx<'a> {
    pub store: &'a Arc<Store>,
    pub events: &'a EventTx,
    pub job: &'a Job,
    preempt: &'a AtomicBool,
}

impl JobCtx<'_> {
    /// A recording started: return [`Outcome::Yield`] at the next safe point.
    pub fn preempted(&self) -> bool {
        self.preempt.load(Ordering::Acquire)
    }

    /// Saves progress and the resume point (numbers/identifiers only).
    pub fn checkpoint(&self, progress: f64, payload: &Value) -> Result<(), String> {
        self.store
            .checkpoint_job(self.job.id, progress, payload)
            .map_err(|e| e.to_string())
    }

    pub fn progress(&self, stage: Option<Stage>, progress: f32) {
        self.events.emit(Event::JobProgress {
            meeting: self.job.meeting_gid.clone(),
            job: self.job.id,
            kind: self.job.kind.clone(),
            stage,
            progress,
        });
    }

    pub fn meeting(&self) -> Result<&str, String> {
        self.job
            .meeting_gid
            .as_deref()
            .ok_or_else(|| format!("job {} has no meeting", self.job.id))
    }
}

pub trait JobHandler: Send + Sync {
    fn kind(&self) -> &'static str;
    fn run(&self, ctx: &JobCtx) -> Result<Outcome, String>;
    /// The run failed (the job is `failed`): settle the meeting so it is not
    /// left `processing` forever.
    fn failed(&self, _ctx: &JobCtx) {}
}

pub struct JobRunner {
    store: Arc<Store>,
    events: EventTx,
    handlers: Vec<Arc<dyn JobHandler>>,
    recording: AtomicUsize,
    preempt: AtomicBool,
    wake: (Mutex<bool>, Condvar),
    shutdown: AtomicBool,
}

impl JobRunner {
    /// `handlers` in priority order.
    pub fn new(
        store: Arc<Store>,
        events: EventTx,
        handlers: Vec<Arc<dyn JobHandler>>,
    ) -> Arc<JobRunner> {
        Arc::new(JobRunner {
            store,
            events,
            handlers,
            recording: AtomicUsize::new(0),
            preempt: AtomicBool::new(false),
            wake: (Mutex::new(false), Condvar::new()),
            shutdown: AtomicBool::new(false),
        })
    }

    /// Wakes the background loop (a job was queued).
    pub fn notify(&self) {
        let (m, cv) = &self.wake;
        *m.lock().unwrap_or_else(|e| e.into_inner()) = true;
        cv.notify_all();
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        self.preempt.store(true, Ordering::Release);
        self.notify();
    }

    fn recording(&self) -> bool {
        self.recording.load(Ordering::Acquire) > 0
    }

    /// Runs one claimable job. `None`: nothing to do (or recording).
    pub fn run_one(&self) -> Option<(Job, Result<Outcome, String>)> {
        if self.recording() || self.shutdown.load(Ordering::Acquire) {
            return None;
        }
        for h in &self.handlers {
            let job = match self.store.claim_next_job(h.kind(), JOB_PAYLOAD_VERSION) {
                Ok(Some(j)) => j,
                Ok(None) => continue,
                Err(e) => {
                    self.error(None, format!("claiming a job: {e}"));
                    continue;
                }
            };
            let ctx = JobCtx {
                store: &self.store,
                events: &self.events,
                job: &job,
                preempt: &self.preempt,
            };
            ctx.progress(None, 0.0);
            let r = h.run(&ctx);
            let settled = match &r {
                Ok(Outcome::Done) => self.store.complete_job(job.id),
                Ok(Outcome::Yield(payload)) => self.store.release_job(job.id, payload),
                Err(e) => {
                    self.error(job.meeting_gid.clone(), format!("{} failed: {e}", job.kind));
                    let r = self.store.fail_job(job.id);
                    h.failed(&ctx);
                    r
                }
            };
            if let Err(e) = settled {
                self.error(job.meeting_gid.clone(), format!("job {}: {e}", job.id));
            }
            if matches!(r, Ok(Outcome::Done)) {
                ctx.progress(None, 1.0);
            }
            return Some((job, r));
        }
        None
    }

    /// Runs jobs until none is claimable (CLI, tests). Returns how many ran.
    pub fn run_pending(&self) -> usize {
        let mut n = 0;
        while let Some((_, r)) = self.run_one() {
            n += 1;
            if matches!(r, Ok(Outcome::Yield(_))) {
                break;
            }
        }
        n
    }

    /// The background loop (desktop): waits for work, never while recording.
    pub fn spawn(self: &Arc<Self>) -> std::io::Result<JoinHandle<()>> {
        let me = self.clone();
        std::thread::Builder::new()
            .name("ghi-jobs".into())
            .spawn(move || {
                while !me.shutdown.load(Ordering::Acquire) {
                    if me.run_one().is_some() {
                        continue;
                    }
                    let (m, cv) = &me.wake;
                    let mut woken = m.lock().unwrap_or_else(|e| e.into_inner());
                    if !*woken {
                        woken = cv
                            .wait_timeout(woken, Duration::from_secs(30))
                            .unwrap_or_else(|e| e.into_inner())
                            .0;
                    }
                    *woken = false;
                }
            })
    }

    fn error(&self, meeting: Option<String>, message: String) {
        self.events.emit(Event::Error {
            meeting,
            kind: ErrorKind::Job,
            message,
        });
    }
}

impl RecordingHooks for JobRunner {
    fn recording_started(&self) {
        self.recording.fetch_add(1, Ordering::AcqRel);
        self.preempt.store(true, Ordering::Release);
    }

    fn recording_stopped(&self) {
        let before = self
            .recording
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                Some(n.saturating_sub(1))
            })
            .unwrap_or(0);
        if before <= 1 {
            self.preempt.store(false, Ordering::Release);
        }
        self.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::bus;
    use ghi_store::keys::{MemoryKeyStore, Protection};
    use ghi_store::store::NewMeeting;
    use serde_json::json;

    /// Counts to 3 in steps, yielding when preempted.
    struct Steps {
        runs: AtomicUsize,
    }

    impl JobHandler for Steps {
        fn kind(&self) -> &'static str {
            "steps"
        }
        fn run(&self, ctx: &JobCtx) -> Result<Outcome, String> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            let mut step = ctx.job.payload["step"].as_u64().unwrap_or(0);
            while step < 3 {
                if ctx.preempted() {
                    return Ok(Outcome::Yield(json!({ "step": step })));
                }
                step += 1;
                ctx.checkpoint(step as f64 / 3.0, &json!({ "step": step }))?;
            }
            Ok(Outcome::Done)
        }
    }

    struct Fails;
    impl JobHandler for Fails {
        fn kind(&self) -> &'static str {
            "fails"
        }
        fn run(&self, _: &JobCtx) -> Result<Outcome, String> {
            Err("nope".into())
        }
    }

    #[test]
    fn runs_in_priority_order_yields_while_recording_and_resumes() {
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
        let (tx, rx) = bus();
        let steps = Arc::new(Steps {
            runs: AtomicUsize::new(0),
        });
        let runner = JobRunner::new(store.clone(), tx, vec![Arc::new(Fails), steps.clone()]);
        let a = store.enqueue_job(Some(&m), "steps", 1, &json!({})).unwrap();
        let f = store.enqueue_job(Some(&m), "fails", 1, &json!({})).unwrap();

        // A recording is running: nothing is claimed.
        runner.recording_started();
        assert!(runner.run_one().is_none());
        runner.recording_stopped();
        // A recording starting mid-run (simulated: the preempt flag alone):
        // the handler yields at its next checkpoint, attempt not spent.
        runner.preempt.store(true, Ordering::SeqCst);
        let (job, r) = runner.run_one().unwrap();
        assert_eq!(job.id, f, "priority order");
        assert!(r.is_err());
        let (job, r) = runner.run_one().unwrap();
        assert_eq!((job.id, r), (a, Ok(Outcome::Yield(json!({"step": 0})))));
        let j = store.job(a).unwrap();
        assert_eq!(
            (j.attempts, j.state),
            (0, ghi_store::jobs::JobState::Queued)
        );
        // The recording stops: the job resumes from its payload and finishes.
        runner.preempt.store(false, Ordering::SeqCst);
        assert_eq!(runner.run_pending(), 1);
        assert_eq!(store.job(a).unwrap().state, ghi_store::jobs::JobState::Done);
        assert_eq!(steps.runs.load(Ordering::SeqCst), 2);
        let errors = rx
            .try_iter()
            .filter(|e| matches!(e.event, Event::Error { .. }))
            .count();
        assert_eq!(errors, 1);
    }
}
