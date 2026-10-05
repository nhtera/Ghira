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
//! - A resumable handler reports how much it has saved in its yield payload
//!   (`"done"`: a count that only grows while `"stamp"`, if given, is the same). After [`YieldBackoff::after`] yields
//!   in a row with no new `done`, the job is held (not failed) until the app
//!   has been quiet (no recording, active) for [`YieldBackoff::idle`], so a
//!   phone that records or backgrounds all the time doesn't spin on a long job.
//! - A leased job (payload `{lease: job_uuid, epoch}`, doc 07 §8: a phone's
//!   final pass or notes run here) is fenced: [`JobRunner::set_fence`] says
//!   whether its lease still holds at the claim, at every checkpoint
//!   ([`JobCtx::preempted`]) and before the commit ([`JobCtx::may_commit`]). A
//!   failed fence drops the job (completed as fenced, logged `Expired`) and
//!   never requeues it; the commit itself is also fenced by the store. Jobs
//!   without a lease are not affected.
//! - A handler that isn't [`JobHandler::ready`] (its models aren't installed)
//!   is skipped: its jobs wait in the queue without spending an attempt, and
//!   run once the models arrive ("record now, process later").

use std::collections::HashMap;
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

/// Where a leased job asks whether its lease still holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceAt {
    /// Before the job runs.
    Claim,
    /// A yield point (the check behind [`JobCtx::preempted`]).
    Checkpoint,
    /// Right before the result is committed: the holder keeps a margin
    /// (1 min) before its deadline.
    Commit,
}

/// Whether the lease of a leased job still holds at `FenceAt`. Never called
/// for a job without a lease.
pub type Fence = Arc<dyn Fn(&Job, FenceAt) -> bool + Send + Sync>;

/// The lease a job runs under: the grantor's job uuid and fencing epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobLease {
    pub job_uuid: String,
    pub epoch: i64,
}

impl JobLease {
    /// `{lease: job_uuid, epoch}` in a job payload; `None`: not leased.
    pub fn from_payload(payload: &Value) -> Option<JobLease> {
        let job_uuid = payload.get("lease")?.as_str()?.to_string();
        let epoch = payload.get("epoch").and_then(Value::as_i64).unwrap_or(0);
        Some(JobLease { job_uuid, epoch })
    }
}

/// `payload` with the lease keys of `job` (a resume payload built from
/// numbers alone would otherwise un-lease the job on its next run).
fn keep_lease(job: &Job, payload: &Value) -> Value {
    let mut out = payload.clone();
    if let (Some(o), Some(src)) = (out.as_object_mut(), job.payload.as_object()) {
        for k in ["lease", "epoch"] {
            if let (Some(v), false) = (src.get(k), o.contains_key(k)) {
                o.insert(k.to_string(), v.clone());
            }
        }
    }
    out
}

pub struct JobCtx<'a> {
    pub store: &'a Arc<Store>,
    pub events: &'a EventTx,
    pub job: &'a Job,
    preempt: &'a AtomicBool,
    inactive: &'a AtomicBool,
    refunded: AtomicBool,
    fence: Option<&'a Fence>,
    /// The handler gave up because the lease no longer holds.
    fenced: AtomicBool,
}

/// When a job that keeps yielding without saving anything new is held back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YieldBackoff {
    /// Yields in a row without progress (`"done"` unchanged) before holding.
    pub after: u32,
    /// How long the app must have been quiet (not recording, active) before a
    /// held job is claimed again.
    pub idle: Duration,
}

impl Default for YieldBackoff {
    fn default() -> Self {
        YieldBackoff {
            after: 3,
            idle: Duration::from_secs(90),
        }
    }
}

/// Yield accounting of one job.
#[derive(Debug, Clone, Copy, Default)]
struct YieldState {
    /// What `done` counts (the handler's `"stamp"`): another one starts over.
    stamp: u64,
    done: u64,
    streak: u32,
    held: bool,
}

impl JobCtx<'_> {
    /// A recording started (or the app left the foreground): return
    /// [`Outcome::Yield`] at the next safe point. Reads the inactive flag too,
    /// so a clear of `preempt` racing a lifecycle change can't hide it.
    /// A leased job whose fence fails is asked to stop too (the runner then
    /// drops it instead of requeueing).
    pub fn preempted(&self) -> bool {
        self.preempt.load(Ordering::SeqCst)
            || self.inactive.load(Ordering::SeqCst)
            || !self.fence_ok(FenceAt::Checkpoint)
    }

    /// The lease this job runs under, if any.
    pub fn lease(&self) -> Option<JobLease> {
        JobLease::from_payload(&self.job.payload)
    }

    /// The fencing epoch of a job its device took back from a lease (doc 07
    /// §8: a self-taken pass writes at `epoch + 1`); `None` for an ordinary
    /// job. Leased jobs get theirs from [`JobCtx::lease`].
    pub fn epoch(&self) -> Option<i64> {
        self.job.payload.get("epoch").and_then(Value::as_i64)
    }

    fn fence_ok(&self, at: FenceAt) -> bool {
        match (self.fence, self.lease()) {
            (Some(f), Some(_)) => f(self.job, at),
            _ => true,
        }
    }

    /// Whether the result may be committed now: always for a job without a
    /// lease; otherwise the lease must still hold with its commit margin.
    /// An early exit only: the store's commit is fenced atomically too.
    pub fn may_commit(&self) -> bool {
        self.fence_ok(FenceAt::Commit)
    }

    /// The lease is gone (a failed [`JobCtx::may_commit`], or the store's
    /// `Fenced`): nothing was written. Return its outcome from the handler;
    /// the runner completes the job as fenced.
    pub fn abandon_fenced(&self) -> Outcome {
        self.fenced.store(true, Ordering::SeqCst);
        Outcome::Done
    }

    /// Saves progress and the resume point (numbers/identifiers only).
    pub fn checkpoint(&self, progress: f64, payload: &Value) -> Result<(), String> {
        self.store
            .checkpoint_job(self.job.id, progress, &keep_lease(self.job, payload))
            .map_err(|e| e.to_string())
    }

    /// The run saved something a later run will skip (a checkpoint): the claim
    /// no longer counts as an attempt, so repeated kills of a long job that
    /// keeps advancing never wear its attempts out. Once per run.
    pub fn made_progress(&self) {
        if !self.refunded.swap(true, Ordering::SeqCst)
            && let Err(e) = self.store.refund_job_attempt(self.job.id)
        {
            log::warn!("refunding job {}: {e}", self.job.id);
        }
    }

    pub fn progress(&self, stage: Option<Stage>, progress: f32) {
        // The phone's chip reads "Final pass on <device> · %" from the lease.
        if self.job.kind == crate::session::FINAL_PASS_JOB
            && let Some(l) = self.lease()
            && let Err(e) = self
                .store
                .lease_set_progress(&l.job_uuid, f64::from(progress.clamp(0.0, 1.0)))
        {
            log::debug!("lease progress not saved: {e}");
        }
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
    /// The app is not active (mobile lifecycle): no claims, running job yields.
    inactive: AtomicBool,
    preempt: AtomicBool,
    fence: Mutex<Option<Fence>>,
    backoff: Mutex<YieldBackoff>,
    yields: Mutex<HashMap<i64, YieldState>>,
    /// When the app last stopped being busy (recording stopped, active again);
    /// held jobs wait for it to age.
    quiet_since: Mutex<Instant>,
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
            inactive: AtomicBool::new(false),
            preempt: AtomicBool::new(false),
            fence: Mutex::new(None),
            backoff: Mutex::new(YieldBackoff::default()),
            yields: Mutex::new(HashMap::new()),
            quiet_since: Mutex::new(Instant::now()),
            running: AtomicUsize::new(0),
            current: Mutex::new(None),
            wake: (Mutex::new(false), Condvar::new()),
            shutdown: AtomicBool::new(false),
        })
    }

    /// Fences leased jobs (see [`FenceAt`]). Without one, leases are not
    /// checked here (the store still fences the commit).
    pub fn set_fence(&self, fence: Fence) {
        *self.fence.lock().unwrap_or_else(|e| e.into_inner()) = Some(fence);
    }

    /// Changes when a job that yields without progress is held.
    pub fn set_yield_backoff(&self, b: YieldBackoff) {
        *self.backoff.lock().unwrap_or_else(|e| e.into_inner()) = b;
    }

    /// Ids of the jobs held back by the yield backoff (still queued).
    pub fn held_jobs(&self) -> Vec<i64> {
        let mut v: Vec<i64> = self
            .yields
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(_, s)| s.held)
            .map(|(id, _)| *id)
            .collect();
        v.sort_unstable();
        v
    }

    fn touch_quiet(&self) {
        *self.quiet_since.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    /// Notes how a run ended for the yield backoff: a `Done` or an error
    /// forgets the job; a yield whose `"done"` grew restarts the count, one
    /// that didn't extends it and, at the limit, holds the job.
    fn account(&self, id: i64, r: &Result<Outcome, String>) {
        let mut map = self.yields.lock().unwrap_or_else(|e| e.into_inner());
        match r {
            Ok(Outcome::Yield(p)) => {
                let Some(done) = p.get("done").and_then(Value::as_u64) else {
                    return;
                };
                let after = self.backoff.lock().unwrap_or_else(|e| e.into_inner()).after;
                let st = map.entry(id).or_default();
                // The saved work was thrown away (a discard, another engine):
                // the old count means nothing, and that is not spinning.
                let stamp = p.get("stamp").and_then(Value::as_u64).unwrap_or(0);
                if stamp != st.stamp {
                    *st = YieldState {
                        stamp,
                        ..Default::default()
                    };
                }
                if done > st.done {
                    st.streak = 0;
                    st.done = done;
                } else {
                    st.streak += 1;
                }
                if after > 0 && st.streak >= after {
                    st.held = true;
                    log::info!(
                        "job held id={id} after {} yields without progress",
                        st.streak
                    );
                }
            }
            _ => {
                map.remove(&id);
            }
        }
    }

    /// Jobs the claim must pass over now. Held ones are released (their count
    /// starts again) once the app has been quiet long enough.
    fn held_now(&self) -> Vec<i64> {
        let idle = self.backoff.lock().unwrap_or_else(|e| e.into_inner()).idle;
        let quiet = self
            .quiet_since
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed()
            >= idle;
        let mut map = self.yields.lock().unwrap_or_else(|e| e.into_inner());
        let mut skip = Vec::new();
        for (id, st) in map.iter_mut().filter(|(_, s)| s.held) {
            if quiet {
                st.held = false;
                st.streak = 0;
            } else {
                skip.push(*id);
            }
        }
        skip
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

    /// The app left the foreground (iOS lifecycle): the running job is asked
    /// to yield and nothing is claimed until [`JobRunner::app_active`]. Sticky:
    /// a recording that stops meanwhile does not clear the request.
    ///
    /// Contract for the phone: call it on `didEnterBackground` (not
    /// `willResignActive`: a Control Center pull or an alert does not end
    /// GPU work) and `app_active` on `willEnterForeground`/`didBecomeActive`;
    /// a launch in the background calls `app_inactive` before `spawn`.
    /// Checkpoints are the only yield points, so an uninterruptible engine
    /// call (diarizer finish, a long ASR block) must run inside the iOS
    /// engine gate that holds the background task.
    pub fn app_inactive(&self) {
        self.inactive.store(true, Ordering::SeqCst);
        self.preempt.store(true, Ordering::SeqCst);
        self.touch_quiet();
    }

    /// The app is active again: jobs may be claimed (unless recording).
    pub fn app_active(&self) {
        self.inactive.store(false, Ordering::SeqCst);
        self.touch_quiet();
        self.refresh_preempt();
        self.notify();
    }

    /// Whether anything still asks running jobs to yield.
    fn must_yield(&self) -> bool {
        self.recording()
            || self.inactive.load(Ordering::SeqCst)
            || self.shutdown.load(Ordering::SeqCst)
    }

    /// Clears the preempt flag if nothing asks for it, then looks again: a
    /// recording, lifecycle change or shutdown that raced the clear raises it
    /// back (it would otherwise be lost).
    fn refresh_preempt(&self) {
        let want = self.must_yield();
        self.preempt.store(want, Ordering::SeqCst);
        if !want && self.must_yield() {
            self.preempt.store(true, Ordering::SeqCst);
        }
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
        if self.recording() || self.inactive.load(Ordering::SeqCst) {
            return None;
        }
        let held = self.held_now();
        let fence = self.fence.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for h in &self.handlers {
            if !h.ready() {
                continue;
            }
            let job = match self
                .store
                .claim_next_job_except(h.kind(), JOB_PAYLOAD_VERSION, &held)
            {
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
                inactive: &self.inactive,
                refunded: AtomicBool::new(false),
                fence: fence.as_ref(),
                fenced: AtomicBool::new(false),
            };
            if !ctx.fence_ok(FenceAt::Claim) {
                // Never runs: the lease is gone before it started.
                self.drop_fenced(&job);
                return Some((job, Ok(Outcome::Done)));
            }
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
            // A leased job that gave up, or yielded with its lease gone, is
            // dropped: requeueing it would only run it under a dead lease.
            let fenced = ctx.fenced.load(Ordering::SeqCst)
                || (matches!(r, Ok(Outcome::Yield(_))) && !ctx.fence_ok(FenceAt::Checkpoint));
            if fenced {
                self.yields
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&job.id);
                self.drop_fenced(&job);
                return Some((job, Ok(Outcome::Done)));
            }
            self.account(job.id, &r);
            let settled = match &r {
                Ok(Outcome::Done) => self.store.complete_job(job.id),
                Ok(Outcome::Yield(payload)) => {
                    self.store.release_job(job.id, &keep_lease(&job, payload))
                }
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

    /// Completes a job whose lease is gone (`Expired`): it wrote nothing.
    fn drop_fenced(&self, job: &Job) {
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = None;
        log::info!("job fenced code=Expired kind={} id={}", job.kind, job.id);
        if let Err(e) = self.store.complete_job(job.id) {
            self.error(job.meeting_gid.clone(), format!("job {}: {e}", job.id));
        }
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
        self.touch_quiet();
        self.preempt.store(true, Ordering::SeqCst);
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
        self.touch_quiet();
        let before = self
            .recording
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                Some(n.saturating_sub(1))
            })
            .unwrap_or(0);
        if before <= 1 {
            self.refresh_preempt();
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

    #[test]
    fn app_inactive_is_sticky_and_blocks_claims_until_active() {
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
        let steps = Arc::new(Steps {
            runs: AtomicUsize::new(0),
        });
        let runner = JobRunner::new(store.clone(), tx, vec![steps]);
        let a = store.enqueue_job(Some(&m), "steps", 1, &json!({})).unwrap();
        runner.app_inactive();
        // A recording that stops while inactive does not clear the request.
        runner.recording_started();
        runner.recording_stopped();
        assert!(runner.preempt.load(Ordering::SeqCst));
        assert!(runner.run_one().is_none());
        runner.app_active();
        assert!(!runner.preempt.load(Ordering::SeqCst));
        assert_eq!(runner.run_pending(), 1);
        assert_eq!(store.job(a).unwrap().state, JobState::Done);
        // Active again but recording: the preempt stays up.
        runner.recording_started();
        runner.app_inactive();
        runner.app_active();
        assert!(runner.preempt.load(Ordering::SeqCst));
    }

    #[test]
    fn a_stop_racing_an_inactive_change_never_hides_the_yield_request() {
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
        let runner = JobRunner::new(store.clone(), tx, vec![]);
        let id = store.enqueue_job(Some(&m), "steps", 1, &json!({})).unwrap();
        let job = store.job(id).unwrap();
        let ctx = || JobCtx {
            store: &runner.store,
            events: &runner.events,
            job: &job,
            preempt: &runner.preempt,
            inactive: &runner.inactive,
            refunded: AtomicBool::new(false),
            fence: None,
            fenced: AtomicBool::new(false),
        };
        // Stop, then inactive.
        runner.recording_started();
        runner.recording_stopped();
        runner.app_inactive();
        assert!(ctx().preempted());
        // Inactive, then the recording stops after: still asked to yield.
        runner.app_active();
        runner.recording_started();
        runner.app_inactive();
        runner.recording_stopped();
        assert!(ctx().preempted() && runner.preempt.load(Ordering::SeqCst));
        runner.app_active();
        assert!(!ctx().preempted());
        // A flag set while the clear ran is put back.
        runner.inactive.store(true, Ordering::SeqCst);
        runner.refresh_preempt();
        assert!(runner.preempt.load(Ordering::SeqCst));
    }

    /// Yields at once with a `done` count that grows only when told to.
    struct Spin {
        done: std::sync::atomic::AtomicU64,
        stamp: std::sync::atomic::AtomicU64,
        grow: AtomicBool,
    }

    impl JobHandler for Spin {
        fn kind(&self) -> &'static str {
            "spin"
        }
        fn run(&self, ctx: &JobCtx) -> Result<Outcome, String> {
            if self.grow.load(Ordering::SeqCst) {
                self.done.fetch_add(1, Ordering::SeqCst);
                ctx.made_progress();
            }
            Ok(Outcome::Yield(json!({
                "done": self.done.load(Ordering::SeqCst),
                "stamp": self.stamp.load(Ordering::SeqCst),
            })))
        }
    }

    fn spin_runner(
        grow: bool,
        backoff: YieldBackoff,
    ) -> (tempfile::TempDir, Arc<Store>, Arc<JobRunner>, String) {
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
        let spin = Arc::new(Spin {
            done: Default::default(),
            stamp: Default::default(),
            grow: AtomicBool::new(grow),
        });
        let runner = JobRunner::new(store.clone(), tx, vec![spin]);
        runner.set_yield_backoff(backoff);
        (tmp, store, runner, m)
    }

    #[test]
    fn a_job_that_yields_without_progress_is_held_until_the_app_is_quiet() {
        let idle = Duration::from_millis(250);
        let (_t, store, runner, m) = spin_runner(false, YieldBackoff { after: 2, idle });
        let a = store.enqueue_job(Some(&m), "spin", 1, &json!({})).unwrap();
        let b = store.enqueue_job(Some(&m), "spin", 1, &json!({})).unwrap();
        // Two yields in a row with nothing new: a is held (not failed, and a
        // yield never costs an attempt). b is a different job and still runs.
        assert_eq!(runner.run_one().unwrap().0.id, a);
        assert!(runner.held_jobs().is_empty());
        assert_eq!(runner.run_one().unwrap().0.id, a);
        assert_eq!(runner.held_jobs(), [a]);
        let j = store.job(a).unwrap();
        assert_eq!((j.state, j.attempts), (JobState::Queued, 0));
        assert_eq!(
            runner.run_one().unwrap().0.id,
            b,
            "the held one is passed over"
        );
        // A recording just ended: not quiet yet.
        runner.recording_started();
        runner.recording_stopped();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(runner.run_one().unwrap().0.id, b);
        assert_eq!(runner.held_jobs(), [a, b], "b yielded twice too");
        // Quiet for `idle`: a runs again (and its count starts over).
        std::thread::sleep(idle);
        assert_eq!(runner.run_one().unwrap().0.id, a);
        assert!(
            runner.held_jobs().is_empty(),
            "both released, counts start over"
        );
    }

    #[test]
    fn progress_between_yields_never_holds_the_job_and_refunds_the_attempt() {
        let (_t, store, runner, m) = spin_runner(
            true,
            YieldBackoff {
                after: 2,
                idle: Duration::from_secs(3600),
            },
        );
        let a = store.enqueue_job(Some(&m), "spin", 1, &json!({})).unwrap();
        for _ in 0..6 {
            assert_eq!(runner.run_one().unwrap().0.id, a);
            assert!(runner.held_jobs().is_empty());
        }
        assert_eq!(store.job(a).unwrap().attempts, 0);
        // A run that made progress and was then killed (left running, requeued
        // at the next open) costs nothing: its attempt was given back.
        let claimed = store.claim_next_job("spin", 1).unwrap().unwrap();
        assert_eq!(claimed.attempts, 1);
        store.refund_job_attempt(a).unwrap();
        assert_eq!(store.job(a).unwrap().attempts, 0);
    }

    #[test]
    fn a_new_stamp_starts_the_yield_count_over() {
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
        let spin = Arc::new(Spin {
            done: Default::default(),
            stamp: Default::default(),
            grow: AtomicBool::new(false),
        });
        let runner = JobRunner::new(store.clone(), tx, vec![spin.clone()]);
        runner.set_yield_backoff(YieldBackoff {
            after: 2,
            idle: Duration::from_secs(3600),
        });
        let a = store.enqueue_job(Some(&m), "spin", 1, &json!({})).unwrap();
        spin.stamp.store(7, Ordering::SeqCst);
        spin.done.store(5, Ordering::SeqCst);
        drop(runner.run_one().unwrap()); // first sight of stamp 7: counts as new
        drop(runner.run_one().unwrap()); // nothing new: streak 1
        // The audio changed (a discard): the saved work is gone, `done` is
        // lower. That is not spinning, so the job is not held.
        spin.stamp.store(8, Ordering::SeqCst);
        spin.done.store(1, Ordering::SeqCst);
        drop(runner.run_one().unwrap());
        assert!(runner.held_jobs().is_empty());
        drop(runner.run_one().unwrap()); // streak 1 under stamp 8
        assert!(runner.held_jobs().is_empty());
        drop(runner.run_one().unwrap()); // streak 2: now held
        assert_eq!(runner.held_jobs(), [a]);
    }
}
