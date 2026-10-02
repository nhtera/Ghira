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
//! - A handler that isn't [`JobHandler::ready`] (its models aren't installed)
//!   is skipped: its jobs wait in the queue without spending an attempt, and
//!   run once the models arrive ("record now, process later").

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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

/// Whether what a handler needs (its models) is installed. Cheap: called
/// before every claim.
pub type Ready = Arc<dyn Fn() -> bool + Send + Sync>;

/// A [`Ready`] that always is (tests, explicit CLI model paths).
pub fn always_ready() -> Ready {
    Arc::new(|| true)
}

pub trait JobHandler: Send + Sync {
    fn kind(&self) -> &'static str;
    /// False while the handler can't run (models missing): its jobs stay
    /// queued. The runner looks again when woken and at least every 30 s.
    fn ready(&self) -> bool {
        true
    }
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
    /// Callers of `run_one` past the recording check (a job claimed or being
    /// looked for). Raised before the check, so a recording that starts
    /// meanwhile either sees it (`wait_idle` waits) or stops the claim.
    running: AtomicUsize,
    /// Id of the claimed job being run.
    current: Mutex<Option<i64>>,
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
            running: AtomicUsize::new(0),
            current: Mutex::new(None),
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

    /// Clean quit: asks the running job to yield, waits up to `wait`, and if
    /// it is still running puts its claim back in the queue without counting
    /// the attempt (its last checkpoint is kept), so quitting never burns
    /// attempts. Returns whether the runner went idle by itself. The caller
    /// then exits (and calls `ghi_llm::local::kill_workers()`); a job that
    /// still finishes before that settles against a released claim, which is
    /// harmless.
    pub fn shutdown_and_release(&self, wait: Duration) -> bool {
        self.shutdown();
        if self.wait_idle(wait) {
            return true;
        }
        let id = *self.current.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(id) = id
            && let Ok(job) = self.store.job(id)
            && let Err(e) = self.store.release_job(id, &job.payload)
        {
            self.error(job.meeting_gid, format!("releasing job {id}: {e}"));
        }
        false
    }

    fn recording(&self) -> bool {
        self.recording.load(Ordering::SeqCst) > 0
    }

    /// Runs one claimable job. `None`: nothing to do (or recording).
    pub fn run_one(&self) -> Option<(Job, Result<Outcome, String>)> {
        if self.shutdown.load(Ordering::SeqCst) {
            return None;
        }
        // Counted before the recording check (SeqCst with `recording_started`
        // and `wait_idle`): no window where a claim is under way unseen.
        struct Running<'a>(&'a AtomicUsize);
        impl Drop for Running<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        self.running.fetch_add(1, Ordering::SeqCst);
        let _running = Running(&self.running);
        if self.recording() {
            return None;
        }
        for h in &self.handlers {
            if !h.ready() {
                continue;
            }
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
            *self.current.lock().unwrap_or_else(|e| e.into_inner()) = Some(job.id);
            log::info!(
                "job start kind={} id={} attempt={}",
                job.kind,
                job.id,
                job.attempts
            );
            let began = std::time::Instant::now();
            let r = h.run(&ctx);
            let ms = began.elapsed().as_millis();
            match &r {
                Ok(Outcome::Done) => log::info!("job done kind={} id={} ms={ms}", job.kind, job.id),
                Ok(Outcome::Yield(_)) => {
                    log::info!("job yielded kind={} id={} ms={ms}", job.kind, job.id)
                }
                // The error text may quote content; only the kind is logged.
                Err(_) => log::warn!("job failed kind={} id={} ms={ms}", job.kind, job.id),
            }
            *self.current.lock().unwrap_or_else(|e| e.into_inner()) = None;
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
        self.recording.fetch_add(1, Ordering::SeqCst);
        self.preempt.store(true, Ordering::Release);
    }

    fn wait_idle(&self, max: Duration) -> bool {
        let end = Instant::now() + max;
        while self.running.load(Ordering::SeqCst) > 0 {
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
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
    use ghi_store::jobs::JobState;
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

    /// Ignores the preempt flag until told to stop (a model call in progress).
    struct Stuck {
        started: AtomicBool,
        stop: AtomicBool,
    }
    impl JobHandler for Stuck {
        fn kind(&self) -> &'static str {
            "stuck"
        }
        fn run(&self, _: &JobCtx) -> Result<Outcome, String> {
            self.started.store(true, Ordering::SeqCst);
            while !self.stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(Outcome::Done)
        }
    }

    #[test]
    fn wait_idle_sees_a_running_job_and_quit_releases_it_without_an_attempt() {
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
        let (tx, _rx) = bus();
        let stuck = Arc::new(Stuck {
            started: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        let runner = JobRunner::new(store.clone(), tx, vec![stuck.clone()]);
        let id = store.enqueue_job(Some(&m), "stuck", 1, &json!({})).unwrap();
        let r = runner.clone();
        let t = std::thread::spawn(move || r.run_one());
        while !stuck.started.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        // A recording starts: the job is busy, so waiting times out.
        runner.recording_started();
        assert!(!runner.wait_idle(Duration::from_millis(100)));
        assert_eq!(store.job(id).unwrap().attempts, 1);
        // Quit: the claim goes back without counting the attempt.
        assert!(!runner.shutdown_and_release(Duration::from_millis(100)));
        let j = store.job(id).unwrap();
        assert_eq!((j.state, j.attempts), (JobState::Queued, 0));
        stuck.stop.store(true, Ordering::SeqCst);
        t.join().unwrap();
        assert!(runner.wait_idle(Duration::from_secs(1)));
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
