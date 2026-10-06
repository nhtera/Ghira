// SPDX-License-Identifier: Apache-2.0
//! Lease races (doc 07 §8 and §11 a/b/c; 15-L), on real stores with injected
//! sleep-inclusive clocks (one per device) and the real job runner: a leased
//! final pass runs at most once, the higher epoch wins when two results exist,
//! and a revoke racing a commit gets exactly one of `Revoked` / `AlreadyDone`.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use common::*;
use ghi_core::events::bus;
use ghi_core::jobs::{Fence, FenceAt, JobCtx, JobHandler, JobRunner, Outcome};
use ghi_core::session::{FINAL_PASS_JOB, JOB_PAYLOAD_VERSION};
use ghi_store::StoreError;
use ghi_store::jobs::Job;
use ghi_store::store::Store;
use ghi_store::sync::leases::{Lease, LeaseRole};
use ghi_store::sync::records::Record;
use ghi_sync::clock::FakeClock;
use ghi_sync::lease::{COMMIT_MARGIN_MS, DEFAULT_TTL_MS, GRACE_MS, Grantor, GrantorAction, Holder};
use ghi_sync::service::HubNode;
use ghi_sync::session::SessionReport;
use ghi_sync::wire::{LeaseQuery, Message};
use serde_json::json;

const WALL: i64 = 1_800_000_000_000;

/// A final pass that counts its executions and writes a one-line transcript
/// through the store's fenced commit (what `FinalPassJob` does at its end,
/// with the same fence calls).
struct Pass {
    execs: Arc<AtomicUsize>,
    text: &'static str,
}

impl JobHandler for Pass {
    fn kind(&self) -> &'static str {
        FINAL_PASS_JOB
    }

    fn run(&self, ctx: &JobCtx) -> Result<Outcome, String> {
        self.execs.fetch_add(1, Ordering::SeqCst);
        let meeting = ctx.meeting()?.to_string();
        let (lease, epoch) = match ctx.lease() {
            Some(l) => (Some(l.job_uuid), l.epoch),
            None => (None, ctx.epoch().unwrap_or(0)),
        };
        if !ctx.may_commit() {
            return Ok(ctx.abandon_fenced());
        }
        match ctx.store.replace_transcript_marked_epoch(
            &meeting,
            vec![seg(self.text, 0)],
            &[],
            epoch,
            lease.as_deref(),
        ) {
            Ok(_) => Ok(Outcome::Done),
            Err(StoreError::Fenced) => Ok(ctx.abandon_fenced()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// The app's fence (ghi-app `lease_fence`) on an injected clock.
fn fence(store: Arc<Store>, clock: Arc<FakeClock>) -> Fence {
    use ghi_sync::clock::Clock;
    Arc::new(move |job: &Job, at: FenceAt| {
        let Some(l) = ghi_core::jobs::JobLease::from_payload(&job.payload) else {
            return true;
        };
        let margin = match at {
            FenceAt::Commit => COMMIT_MARGIN_MS,
            FenceAt::Claim | FenceAt::Checkpoint => 0,
        };
        let now = i64::try_from(clock.now_cont_ns()).unwrap();
        store
            .lease_fence_ok(&l.job_uuid, now, &clock.boot_id(), margin)
            .unwrap_or(false)
    })
}

struct Device {
    node: Node,
    clock: Arc<FakeClock>,
    execs: Arc<AtomicUsize>,
    runner: Arc<JobRunner>,
}

fn device(text: &'static str) -> Device {
    let node = node();
    let clock = Arc::new(FakeClock::new(WALL));
    let execs = Arc::new(AtomicUsize::new(0));
    let (tx, _rx) = bus();
    let runner = JobRunner::new(
        node.store().clone(),
        tx,
        vec![Arc::new(Pass {
            execs: execs.clone(),
            text,
        })],
    );
    runner.set_fence(fence(node.store().clone(), clock.clone()));
    Device {
        node,
        clock,
        execs,
        runner,
    }
}

impl Device {
    fn store(&self) -> &Arc<Store> {
        self.node.store()
    }

    fn executions(&self) -> usize {
        self.execs.load(Ordering::SeqCst)
    }
}

struct Rig {
    hub: Device,
    phone: Device,
    node: Arc<HubNode>,
    meeting: String,
    job: String,
    epoch: i64,
}

impl Rig {
    fn sync(&self) -> (SessionReport, SessionReport) {
        sync_with_clock(&self.node, &self.phone.node, self.phone.clock.clone())
    }

    fn advance_phone(&self, by: Duration) {
        self.phone.clock.advance(by);
    }

    /// The hub's side of the job the lease queued.
    fn enqueue_on_hub(&self) {
        self.hub
            .store()
            .enqueue_job(
                Some(&self.meeting),
                FINAL_PASS_JOB,
                JOB_PAYLOAD_VERSION,
                &json!({"lease": self.job, "epoch": self.epoch}),
            )
            .unwrap();
    }

    fn lease_on(&self, d: &Device) -> Lease {
        d.store().lease_state(&self.job).unwrap().unwrap()
    }

    /// The phone takes the job back after `G` and runs it at `epoch + 1`.
    fn self_take_and_run(&self) {
        self.advance_phone(Duration::from_millis(
            u64::try_from(DEFAULT_TTL_MS + GRACE_MS).unwrap() + 60_000,
        ));
        let actions = Grantor
            .poll(
                self.phone.store().as_ref(),
                self.phone.clock.as_ref(),
                0,
                true,
            )
            .unwrap();
        let [GrantorAction::SelfTake(take)] = actions.as_slice() else {
            panic!("the phone did not take the job back: {actions:?}");
        };
        assert_eq!(take.epoch, self.epoch + 1);
        self.phone
            .store()
            .enqueue_job(
                Some(&self.meeting),
                FINAL_PASS_JOB,
                JOB_PAYLOAD_VERSION,
                &json!({"epoch": take.epoch}),
            )
            .unwrap();
        assert_eq!(self.phone.runner.run_pending(), 1);
    }
}

/// A paired hub and phone that exchanged a finished meeting; the phone
/// offered its final pass and the hub accepted (a granted holder lease).
fn rig() -> Rig {
    let (hub, phone) = (device("desktop v2"), device("phone v2"));
    let node = hub.node.hub_with_clock(hub.clock.clone());
    pair(&node, &hub.node, &phone.node);
    let meeting = meeting(phone.store(), "Leased meeting");
    sync_with_clock(&node, &phone.node, phone.clock.clone());
    let req = Grantor
        .offer(
            phone.store().as_ref(),
            hub.node.gid(),
            &meeting,
            &["final_pass".to_string()],
            DEFAULT_TTL_MS,
        )
        .unwrap();
    let rig = Rig {
        hub,
        phone,
        node,
        meeting,
        job: req.job_uuid.clone(),
        epoch: req.epoch,
    };
    let (_, theirs) = rig.sync();
    assert_eq!(
        theirs.new_leases,
        vec![req.job_uuid],
        "the hub opened the lease"
    );
    let held = rig.lease_on(&rig.hub);
    assert_eq!(
        (held.role, held.state.as_str()),
        (LeaseRole::Holder, "granted")
    );
    let given = rig.lease_on(&rig.phone);
    assert_eq!(
        (given.role, given.state.as_str()),
        (LeaseRole::Grantor, "granted")
    );
    rig
}

fn transcript_epoch(store: &Store, meeting: &str) -> i64 {
    store
        .changes_since(0, 10_000)
        .unwrap()
        .changes
        .into_iter()
        .rev()
        .find_map(|c| match c.record {
            Record::Meeting(m) if m.gid == meeting => m.transcript_epoch,
            _ => None,
        })
        .unwrap_or(0)
}

fn lines(s: &Store, meeting: &str) -> Vec<String> {
    texts(s, meeting)
}

fn segment_gids(s: &Store, meeting: &str) -> Vec<String> {
    s.segments(meeting)
        .unwrap()
        .into_iter()
        .map(|x| x.gid)
        .collect()
}

// -------------------------------------------------------------------- (a)

#[test]
fn a_holder_that_wakes_after_g_never_starts_and_the_phone_runs_it_once() {
    let rig = rig();
    rig.enqueue_on_hub();
    // The desktop sleeps through the lease; the phone's G (ttl + grace)
    // passes and it takes the job back.
    rig.hub.clock.advance(Duration::from_millis(
        u64::try_from(DEFAULT_TTL_MS + GRACE_MS).unwrap() + 60_000,
    ));
    rig.self_take_and_run();
    assert_eq!(rig.phone.executions(), 1);
    assert_eq!(lines(rig.phone.store(), &rig.meeting), ["phone v2"]);

    // The desktop wakes: its queued job meets a lease past H.
    assert!(
        !rig.hub
            .store()
            .lease_fence_ok(
                &rig.job,
                i64::try_from(ghi_sync::clock::Clock::now_cont_ns(rig.hub.clock.as_ref())).unwrap(),
                &ghi_sync::clock::Clock::boot_id(rig.hub.clock.as_ref()),
                0,
            )
            .unwrap(),
        "H has passed on the desktop"
    );
    assert_eq!(
        rig.hub.runner.run_pending(),
        1,
        "the job was claimed and dropped"
    );
    assert_eq!(rig.hub.executions(), 0, "the fenced job never started");

    // Exactly one execution in the world, and it is the phone's result
    // everywhere after a sync.
    rig.sync();
    let (m, t) = rig.sync();
    assert!(idle(&m) && idle(&t), "{m:?} {t:?}");
    assert_eq!(rig.hub.executions() + rig.phone.executions(), 1);
    assert_eq!(lines(rig.hub.store(), &rig.meeting), ["phone v2"]);
    assert_eq!(
        transcript_epoch(rig.hub.store(), &rig.meeting),
        rig.epoch + 1
    );
}

#[test]
fn a_holder_still_inside_h_runs_once_and_the_phone_does_not_take_it_back() {
    let rig = rig();
    rig.enqueue_on_hub();
    assert_eq!(rig.hub.runner.run_pending(), 1);
    assert_eq!(rig.hub.executions(), 1);
    // The next session reports the result; the phone does not self-take, even
    // long after G, because its lease is done.
    rig.sync();
    assert_eq!(rig.lease_on(&rig.phone).state, "done");
    rig.advance_phone(Duration::from_millis(
        u64::try_from(DEFAULT_TTL_MS + GRACE_MS).unwrap() * 2,
    ));
    let actions = Grantor
        .poll(
            rig.phone.store().as_ref(),
            rig.phone.clock.as_ref(),
            0,
            true,
        )
        .unwrap();
    assert!(actions.is_empty(), "{actions:?}");
    rig.sync();
    assert_eq!(lines(rig.phone.store(), &rig.meeting), ["desktop v2"]);
    assert_eq!(rig.phone.executions(), 0);
}

// -------------------------------------------------------------------- (b)

#[test]
fn two_results_leave_the_higher_epochs_transcript_everywhere() {
    let rig = rig();
    rig.enqueue_on_hub();
    // The desktop finishes inside H.
    assert_eq!(rig.hub.runner.run_pending(), 1);
    assert_eq!(lines(rig.hub.store(), &rig.meeting), ["desktop v2"]);
    let desktop_rows = segment_gids(rig.hub.store(), &rig.meeting);
    assert_eq!(transcript_epoch(rig.hub.store(), &rig.meeting), rig.epoch);

    // The phone never heard: after G it takes the job back and commits at
    // epoch + 1.
    rig.self_take_and_run();
    assert_eq!(lines(rig.phone.store(), &rig.meeting), ["phone v2"]);
    let phone_rows = segment_gids(rig.phone.store(), &rig.meeting);
    assert_eq!(rig.hub.executions() + rig.phone.executions(), 2, "both ran");

    // They meet.
    rig.sync();
    rig.sync();
    let (m, t) = rig.sync();
    assert!(idle(&m) && idle(&t), "{m:?} {t:?}");
    for d in [&rig.hub, &rig.phone] {
        assert_eq!(
            lines(d.store(), &rig.meeting),
            ["phone v2"],
            "the higher epoch's v2 stays"
        );
        assert_eq!(segment_gids(d.store(), &rig.meeting), phone_rows);
        assert_eq!(transcript_epoch(d.store(), &rig.meeting), rig.epoch + 1);
        for gid in &desktop_rows {
            assert!(
                d.store().is_tombstoned(gid).unwrap(),
                "the desktop's v2 row is tombstoned on every device"
            );
        }
        assert!(
            d.store().conflict_copies(&rig.meeting).unwrap().is_empty(),
            "a transcript replaced by a newer epoch is not a text conflict"
        );
    }
}

// -------------------------------------------------------------------- (c)

#[test]
fn a_revoke_that_arrives_before_the_commit_wins_and_the_commit_is_fenced() {
    let rig = rig();
    rig.enqueue_on_hub();
    // The phone presses "process on this phone now".
    Grantor
        .revoke(rig.phone.store().as_ref(), &rig.meeting)
        .unwrap();
    let (mine, _) = rig.sync();
    assert_eq!(
        mine.take_back.len(),
        1,
        "revoked: the phone has the job: {mine:?}"
    );
    assert_eq!(mine.take_back[0].epoch, rig.epoch + 1);
    assert_eq!(rig.lease_on(&rig.hub).state, "revoked");
    // The desktop's queued job now meets a revoked lease and never starts.
    assert_eq!(rig.hub.runner.run_pending(), 1);
    assert_eq!(rig.hub.executions(), 0);
    assert_eq!(
        rig.hub
            .store()
            .get_meeting(&rig.meeting)
            .unwrap()
            .transcript_version,
        1
    );
    assert_eq!(rig.lease_on(&rig.phone).state, "self_taken");
}

#[test]
fn a_commit_that_lands_before_the_revoke_wins_and_the_revoke_says_already_done() {
    let rig = rig();
    rig.enqueue_on_hub();
    assert_eq!(rig.hub.runner.run_pending(), 1);
    assert_eq!(rig.hub.executions(), 1);
    Grantor
        .revoke(rig.phone.store().as_ref(), &rig.meeting)
        .unwrap();
    let (mine, _) = rig.sync();
    assert!(
        mine.take_back.is_empty(),
        "AlreadyDone: nothing to take back: {mine:?}"
    );
    assert_eq!(rig.lease_on(&rig.phone).state, "done");
    assert_eq!(rig.lease_on(&rig.hub).state, "done");
    rig.sync();
    assert_eq!(lines(rig.phone.store(), &rig.meeting), ["desktop v2"]);
}

/// A holder lease on the hub's own store for a fresh meeting (no peer needed
/// to test the compare-and-set).
fn holder_lease(d: &Device, n: usize) -> (String, String) {
    let meeting = meeting(d.store(), &format!("race {n}"));
    let job = format!("race-job-{n}");
    let now = i64::try_from(ghi_sync::clock::Clock::now_cont_ns(d.clock.as_ref())).unwrap();
    d.store()
        .lease_open(&Lease {
            job_uuid: job.clone(),
            meeting_gid: meeting.clone(),
            role: LeaseRole::Holder,
            epoch: 3,
            kinds: vec!["final_pass".into()],
            state: "granted".into(),
            ttl_ms: DEFAULT_TTL_MS,
            deadline_cont_ns: Some(now + DEFAULT_TTL_MS * 1_000_000),
            boot_id: Some(ghi_sync::clock::Clock::boot_id(d.clock.as_ref())),
            wall_deadline_ms: Some(WALL + DEFAULT_TTL_MS),
            progress: 0.0,
            peer_gid: None,
        })
        .unwrap();
    (meeting, job)
}

#[test]
fn a_revoke_and_a_commit_in_the_same_instant_give_exactly_one_answer() {
    let d = device("raced v2");
    let (mut revoked, mut done) = (0, 0);
    for n in 0..60 {
        let (meeting, job) = holder_lease(&d, n);
        let barrier = Arc::new(Barrier::new(2));
        let (s1, b1, m1, j1) = (
            d.store().clone(),
            barrier.clone(),
            meeting.clone(),
            job.clone(),
        );
        let committer = thread::spawn(move || {
            b1.wait();
            for _ in 0..(n % 4) {
                thread::yield_now();
            }
            s1.replace_transcript_marked_epoch(&m1, vec![seg("raced v2", 0)], &[], 3, Some(&j1))
        });
        let (s2, b2, m2) = (d.store().clone(), barrier, meeting.clone());
        let revoker = thread::spawn(move || {
            b2.wait();
            for _ in 0..(n % 3) {
                thread::yield_now();
            }
            Holder.on_revoke(
                s2.as_ref(),
                &LeaseQuery {
                    meeting_gid: m2,
                    epoch: 3,
                },
            )
        });
        let commit = committer.join().unwrap();
        let answer = revoker.join().unwrap().unwrap();
        let version = d.store().get_meeting(&meeting).unwrap().transcript_version;
        match (&commit, &answer) {
            (Ok(_), Message::AlreadyDone { job_uuid }) => {
                assert_eq!(job_uuid, &job);
                assert_eq!(version, 2, "the committed v2 is kept");
                assert_eq!(lines(d.store(), &meeting), ["raced v2"]);
                done += 1;
            }
            (Err(StoreError::Fenced), Message::Revoked { epoch }) => {
                assert_eq!(*epoch, 3);
                assert_eq!(version, 1, "nothing was written by a fenced commit");
                assert_ne!(lines(d.store(), &meeting), ["raced v2"]);
                revoked += 1;
            }
            other => panic!("round {n}: both or neither won: {other:?}"),
        }
        let state = d.store().lease_state(&job).unwrap().unwrap().state;
        assert_eq!(
            state,
            if matches!(answer, Message::Revoked { .. }) {
                "revoked"
            } else {
                "done"
            }
        );
    }
    assert_eq!(revoked + done, 60);
}
