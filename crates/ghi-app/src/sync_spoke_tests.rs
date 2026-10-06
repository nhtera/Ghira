// SPDX-License-Identifier: Apache-2.0
//! The phone's side of the sync service against a real hub [`Core`] (fake
//! speech engines, a scripted notes model), joined by in-memory pipes under
//! Noise. The phone is a second `Core` with a [`SyncService`] in
//! [`Role::Spoke`] whose [`SpokeLink`] hands out pipes and a scripted scan.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use ghi_core::jobs::JobHandler;
use ghi_core::session::FINAL_PASS_JOB;
use ghi_store::store::Store;
use ghi_store::sync::records::Record;
use ghi_sync::lease::GrantorState;
use ghi_sync::mem::mem_pipe;
use ghi_sync::transport::ByteStream;

use super::spoke::*;
use super::tests::{
    Hub, handlers, handlers_when, hub, hub_with, hub_with_handlers, record_meeting, texts, wait_for,
};
use super::*;
use crate::core::{Core, CoreHooks};

/// A LAN address that is never dialled (the pipe ignores it).
fn fake_addr() -> SocketAddr {
    SocketAddr::new(Ipv4Addr::new(192, 168, 7, 7).into(), 7777)
}

/// The phone's platform: pipes to the hub's node, a scripted scan.
struct PipeLink {
    /// Who the phone's sockets reach (a test can point it elsewhere).
    hub: Mutex<Arc<SyncService>>,
    scan: Mutex<Option<Result<String, ScanError>>>,
    capable: AtomicBool,
    /// Hub-side sessions currently open.
    open: Arc<AtomicUsize>,
    connects: AtomicUsize,
    bg: AtomicUsize,
}

impl PipeLink {
    fn new(hub: &Hub) -> Arc<Self> {
        Arc::new(PipeLink {
            hub: Mutex::new(hub.svc.clone()),
            scan: Mutex::new(None),
            capable: AtomicBool::new(true),
            open: Arc::new(AtomicUsize::new(0)),
            connects: AtomicUsize::new(0),
            bg: AtomicUsize::new(0),
        })
    }
}

impl SpokeLink for PipeLink {
    fn connect(&self, _addr: SocketAddr) -> io::Result<Box<dyn ByteStream>> {
        self.connects.fetch_add(1, Ordering::SeqCst);
        let hub = lock(&self.hub).clone();
        let Some(node) = hub.node() else {
            return Err(io::ErrorKind::ConnectionRefused.into());
        };
        let (a, b) = mem_pipe();
        let (svc, open) = (hub, self.open.clone());
        open.fetch_add(1, Ordering::SeqCst);
        thread::spawn(move || {
            let _ = svc.serve_connection(&node, a, None, None);
            open.fetch_sub(1, Ordering::SeqCst);
        });
        Ok(Box::new(b))
    }

    fn discovered(&self) -> Vec<SocketAddr> {
        vec![fake_addr()]
    }

    fn browse(&self, _on: bool) {}

    fn scan(&self, _cancel: &AtomicBool) -> Result<String, ScanError> {
        self.scan
            .lock()
            .unwrap()
            .take()
            .unwrap_or(Err(ScanError::Cancelled))
    }

    fn begin_bg(&self) -> u64 {
        self.bg.fetch_add(1, Ordering::SeqCst) as u64 + 1
    }

    fn end_bg(&self, _token: u64) {
        self.bg.fetch_sub(1, Ordering::SeqCst);
    }

    fn capable(&self) -> bool {
        self.capable.load(Ordering::SeqCst)
    }
}

struct Phone {
    core: Arc<Core>,
    svc: Arc<SyncService>,
    link: Arc<PipeLink>,
    events: Arc<Mutex<Vec<SyncEvent>>>,
    hours: Arc<AtomicUsize>,
    _tmp: tempfile::TempDir,
}

impl Phone {
    fn store(&self) -> Arc<Store> {
        self.core.store().unwrap()
    }

    fn events(&self) -> Vec<SyncEvent> {
        lock(&self.events).clone()
    }
}

fn phone_with(hub: &Hub, handlers: super::tests::Handlers, reachable_within: Duration) -> Phone {
    let tmp = tempfile::tempdir().unwrap();
    let (core, _rx) = Core::for_test_with(
        tmp.path().join("data"),
        CoreHooks {
            handlers: Some(handlers),
            recover_kinds: Some(Vec::new()),
            ..Default::default()
        },
    );
    let link = PipeLink::new(hub);
    let events: Arc<Mutex<Vec<SyncEvent>>> = Arc::default();
    let hours = Arc::new(AtomicUsize::new(12));
    let (e2, h2) = (events.clone(), hours.clone());
    let mut cfg = SyncConfig::spoke(
        "iPhone",
        "ios",
        link.clone(),
        move |e| lock(&e2).push(e),
        || {},
    );
    cfg.reachable_within = reachable_within;
    cfg.offline_hours = Arc::new(move || h2.load(Ordering::SeqCst) as u32);
    let svc = SyncService::new(core.clone(), cfg);
    svc.start();
    svc.set_app_active(true);
    Phone {
        core,
        svc,
        link,
        events,
        hours,
        _tmp: tmp,
    }
}

fn phone(hub: &Hub) -> Phone {
    phone_with(hub, Arc::new(handlers), REACHABLE_WITHIN)
}

/// Opens the hub's pairing window and returns the code (what the scanner
/// would read) for the phone.
fn code_for(hub: &Hub) -> String {
    hub.svc.set_enabled(true).unwrap();
    lock(&hub.svc.state).pairing_until = Some(Instant::now() + PAIR_TTL);
    hub.svc.reconcile();
    let node = hub
        .svc
        .node()
        .expect("the pairing window brings the node up");
    node.open_pairing_code(&[fake_addr()]).unwrap()
}

/// Pairs `phone` with `hub` through the scan path.
fn pair(hub: &Hub, phone: &Phone) {
    phone.svc.set_enabled(true).unwrap();
    *phone.link.scan.lock().unwrap() = Some(Ok(code_for(hub)));
    phone.svc.pair_scan().unwrap();
    assert_eq!(phone.svc.devices().unwrap().len(), 1);
    // The hub's side of the pairing finishes on its own thread.
    wait_for("the hub to list the phone", 10, || {
        hub.svc.devices().unwrap().len() == 1
    });
}

/// What mobile's `finish` does for a Desktop-target recording.
fn stop_for_desktop(phone: &Phone, meeting: &str) {
    let store = phone.store();
    store.set_meeting_status(meeting, "processing").unwrap();
    desktop_pending_add(&store, meeting).unwrap();
    phone.svc.poke();
}

/// The epoch of a meeting's transcript, as it would go to a peer.
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

fn grantor_lease(store: &Store, meeting: &str) -> Option<ghi_store::sync::leases::Lease> {
    store
        .leases_for_meeting(meeting)
        .unwrap()
        .into_iter()
        .find(|l| l.role == LeaseRole::Grantor)
}

/// A hub whose final pass never gets ready: it cannot start the work it is given.
fn idle_hub() -> Hub {
    let never = |_: &Arc<Store>, _: &std::path::Path| -> Vec<Arc<dyn JobHandler>> {
        vec![Arc::new(ghi_core::final_pass::FinalPassJob {
            engines: Arc::new(|| Err("not now".into())),
            chunk_s: 600.0,
            ready: Arc::new(|| false),
            voice: None,
        })]
    };
    hub_with_handlers(Vec::new(), REACHABLE_WITHIN, Arc::new(never))
}

#[test]
fn a_scanned_code_pairs_and_the_phone_shows_the_computer() {
    let hub = hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    assert!(
        phone
            .events()
            .iter()
            .any(|e| matches!(e, SyncEvent::Paired { device } if device.name == "Mac")),
        "{:?}",
        phone.events()
    );
    // The hub emits its event from the connection thread, after the phone's
    // scan has already returned.
    wait_for("the hub's Paired event", 5, || {
        hub.events()
            .iter()
            .any(|e| matches!(e, SyncEvent::Paired { device } if device.name == "iPhone"))
    });
    // Already paired: no second scan.
    assert_eq!(phone.svc.pair_scan(), Err(ERR_ALREADY_PAIRED.to_string()));
}

#[test]
fn scan_failures_come_back_as_codes() {
    let hub = hub();
    let phone = phone(&hub);
    phone.svc.set_enabled(true).unwrap();
    for (scan, want) in [
        (Err(ScanError::Denied), SCAN_CAMERA_OFF),
        (Err(ScanError::Unavailable), SCAN_CAMERA_OFF),
        (Ok("hello".to_string()), SCAN_INVALID),
        (Ok("GHI1:NOT-BASE45!".to_string()), SCAN_INVALID),
    ] {
        *phone.link.scan.lock().unwrap() = Some(scan);
        assert_eq!(phone.svc.pair_scan(), Err(want.to_string()));
    }
    // Closed by the user: not an error.
    *phone.link.scan.lock().unwrap() = Some(Err(ScanError::Cancelled));
    assert_eq!(phone.svc.pair_scan(), Ok(()));
    // A code whose window has closed: the hub closes the handshake.
    let text = code_for(&hub);
    hub.svc.node().unwrap().close_pairing();
    *phone.link.scan.lock().unwrap() = Some(Ok(text));
    assert_eq!(phone.svc.pair_scan(), Err(SCAN_EXPIRED.to_string()));
    // Nothing answering at all: an event, not a code.
    hub.svc.set_enabled(false).unwrap();
    assert!(hub.svc.node().is_none());
    let text = {
        hub.svc.set_enabled(true).unwrap();
        let t = code_for(&hub);
        hub.svc.stop();
        t
    };
    *phone.link.scan.lock().unwrap() = Some(Ok(text));
    assert_eq!(phone.svc.pair_scan(), Ok(()));
    assert!(
        phone.events().iter().any(|e| matches!(
            e,
            SyncEvent::Error {
                code: SyncErrorCode::Unreachable
            }
        )),
        "{:?}",
        phone.events()
    );
    assert!(phone.svc.devices().unwrap().is_empty());
}

#[test]
fn desktop_target_meeting_goes_through_the_hub_pass_under_the_lease_and_comes_back_as_v2() {
    let hub = hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    let meeting = record_meeting(&phone.store());
    let live = phone
        .store()
        .get_meeting(&meeting)
        .unwrap()
        .transcript_version;
    stop_for_desktop(&phone, &meeting);

    // Rows, key and audio go first; then the lease is offered and granted.
    wait_for("the lease to be granted", 30, || {
        grantor_lease(&phone.store(), &meeting).is_some_and(|l| l.state != "offered")
    });
    assert!(desktop_pending(&phone.store()).is_empty());
    let view = meeting_sync_view(&phone.store(), &meeting);
    assert_eq!(view.device.as_deref(), Some("Mac"));
    assert!(view.lease.is_some(), "{view:?}");

    // The hub runs the pass under the lease; the phone pulls the result.
    wait_for("the final pass on the hub", 60, || {
        hub.store()
            .get_meeting(&meeting)
            .unwrap()
            .transcript_version
            > live
    });
    wait_for("v2 on the phone", 40, || {
        phone
            .store()
            .get_meeting(&meeting)
            .unwrap()
            .transcript_version
            == hub
                .store()
                .get_meeting(&meeting)
                .unwrap()
                .transcript_version
    });
    assert_eq!(
        texts(&phone.store(), &meeting),
        texts(&hub.store(), &meeting)
    );
    wait_for("the lease to close", 30, || {
        grantor_lease(&phone.store(), &meeting).is_some_and(|l| l.state == "done")
    });
    let view = meeting_sync_view(&phone.store(), &meeting);
    assert_eq!(view.lease.map(|l| l.state), Some(GrantorState::Done));
    assert!(view.synced, "{view:?}");
    assert!(view.lease_open().is_none());
}

#[test]
fn a_revoke_before_the_hub_starts_makes_the_phone_take_the_job_at_the_next_epoch() {
    let hub = idle_hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    let meeting = record_meeting(&phone.store());
    let live = phone
        .store()
        .get_meeting(&meeting)
        .unwrap()
        .transcript_version;
    stop_for_desktop(&phone, &meeting);
    wait_for("the lease to be granted", 30, || {
        grantor_lease(&phone.store(), &meeting).is_some_and(|l| l.state == "granted")
    });
    let lease = grantor_lease(&phone.store(), &meeting).unwrap();
    assert_eq!(lease.epoch, 1);
    assert!(
        meeting_sync_view(&phone.store(), &meeting)
            .lease_open()
            .is_some(),
        "read-only while the lease is open"
    );

    phone.svc.lease_revoke(&meeting).unwrap();
    // Revoked: the phone runs the pass itself, at epoch + 1.
    wait_for("the phone's own pass", 60, || {
        phone
            .store()
            .get_meeting(&meeting)
            .unwrap()
            .transcript_version
            > live
    });
    let own = grantor_lease(&phone.store(), &meeting).unwrap();
    assert_eq!(own.state, "self_taken");
    assert_eq!(
        transcript_epoch(&phone.store(), &meeting),
        2,
        "written at epoch + 1"
    );
    // The hub never committed and its lease is closed.
    let theirs = hub.store().lease_state(&lease.job_uuid).unwrap().unwrap();
    assert_eq!(theirs.state, "revoked");
    assert_eq!(
        hub.store()
            .get_meeting(&meeting)
            .unwrap()
            .transcript_version,
        live
    );
}

#[test]
fn a_revoke_that_loses_to_the_finished_pass_does_nothing() {
    let hub = hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    let meeting = record_meeting(&phone.store());
    stop_for_desktop(&phone, &meeting);
    wait_for("the hub's result", 90, || {
        hub.store()
            .lease_state(
                &grantor_lease(&phone.store(), &meeting)
                    .map(|l| l.job_uuid)
                    .unwrap_or_default(),
            )
            .ok()
            .flatten()
            .is_some_and(|l| l.state == "done")
    });
    // The phone has not heard yet: its lease is still granted when it revokes.
    let lease = grantor_lease(&phone.store(), &meeting).unwrap();
    if lease.state == "granted" || lease.state == "running" {
        phone.svc.lease_revoke(&meeting).unwrap();
    }
    wait_for("the lease to close", 30, || {
        grantor_lease(&phone.store(), &meeting).is_some_and(|l| l.state == "done")
    });
    assert!(
        phone
            .store()
            .jobs_for_meeting(&meeting)
            .unwrap()
            .iter()
            .all(|j| j.kind != FINAL_PASS_JOB),
        "nothing was taken back"
    );
}

#[test]
fn a_phone_below_the_tier_cannot_revoke() {
    let hub = hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    phone.link.capable.store(false, Ordering::SeqCst);
    assert_eq!(
        phone.svc.lease_revoke("m"),
        Err(ERR_NOT_CAPABLE.to_string())
    );
}

#[test]
fn a_hub_that_stays_away_is_replaced_by_the_phone_after_the_hours() {
    let hub = idle_hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    let meeting = record_meeting(&phone.store());
    let live = phone
        .store()
        .get_meeting(&meeting)
        .unwrap()
        .transcript_version;
    stop_for_desktop(&phone, &meeting);
    let store = phone.store();
    wait_for("the lease to be granted", 30, || {
        grantor_lease(&store, &meeting).is_some_and(|l| l.state == "granted")
    });
    // The computer goes away; its lease's deadline (and the grace) passes.
    hub.svc.stop();
    let lease = grantor_lease(&store, &meeting).unwrap();
    SyncStore::lease_renew(
        store.as_ref(),
        &lease.job_uuid,
        3_600_000,
        0,
        &SystemClock.boot_id(),
    )
    .unwrap();
    phone.svc.poke();
    wait_for("the self-take", 120, || {
        store.get_meeting(&meeting).unwrap().transcript_version > live
    });
    assert_eq!(grantor_lease(&store, &meeting).unwrap().state, "self_taken");
}

#[test]
fn a_phone_below_the_tier_closes_a_lease_the_computer_never_answers_and_does_not_run_the_pass() {
    let hub = idle_hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    phone.link.capable.store(false, Ordering::SeqCst);
    let meeting = record_meeting(&phone.store());
    let store = phone.store();
    stop_for_desktop(&phone, &meeting);
    wait_for("the lease to be granted", 30, || {
        grantor_lease(&store, &meeting).is_some_and(|l| l.state == "granted")
    });
    hub.svc.stop();
    let lease = grantor_lease(&store, &meeting).unwrap();
    SyncStore::lease_renew(
        store.as_ref(),
        &lease.job_uuid,
        3_600_000,
        0,
        &SystemClock.boot_id(),
    )
    .unwrap();
    phone.svc.poke();
    // It cannot take the job back: the lease is closed and the meeting
    // shown as waiting for the computer (not for Wi-Fi), never run here.
    wait_for("the lease to expire", 30, || {
        grantor_lease(&store, &meeting).is_some_and(|l| l.state == "expired")
    });
    assert_eq!(
        meeting_sync_view(&store, &meeting).lease.map(|l| l.state),
        Some(GrantorState::Expired)
    );
    assert!(
        store
            .jobs_for_meeting(&meeting)
            .unwrap()
            .iter()
            .all(|j| j.kind != FINAL_PASS_JOB)
    );
}

#[test]
fn a_computer_that_slept_past_the_lease_gets_the_meeting_again_and_makes_one_v2() {
    // The computer is not ready to work (its lid is shut) until `awake`.
    let awake = Arc::new(AtomicBool::new(false));
    let gate = awake.clone();
    let gated = move |_: &Arc<Store>, _: &std::path::Path| -> Vec<Arc<dyn JobHandler>> {
        let gate = gate.clone();
        handlers_when(Arc::new(move || gate.load(Ordering::SeqCst)))
    };
    let hub = hub_with_handlers(Vec::new(), REACHABLE_WITHIN, Arc::new(gated));
    let phone = phone(&hub);
    pair(&hub, &phone);
    phone.link.capable.store(false, Ordering::SeqCst);
    let meeting = record_meeting(&phone.store());
    let live = phone
        .store()
        .get_meeting(&meeting)
        .unwrap()
        .transcript_version;
    let store = phone.store();
    stop_for_desktop(&phone, &meeting);
    wait_for("the lease to be granted", 30, || {
        grantor_lease(&store, &meeting).is_some_and(|l| l.state == "granted")
    });
    let first = grantor_lease(&store, &meeting).unwrap();

    // More than the lease's time passes with the lid shut: on the computer
    // the lease is over (its job is fenced when it wakes).
    assert!(
        hub.store()
            .lease_transition(&first.job_uuid, &["granted", "running"], "expired")
            .unwrap()
    );
    phone.svc.poke();

    // The phone cannot run the pass, so it offers the meeting again, at the
    // next epoch, and the computer takes it.
    wait_for("the second lease", 60, || {
        store.leases_for_meeting(&meeting).unwrap().iter().any(|l| {
            l.role == LeaseRole::Grantor && l.epoch == first.epoch + 1 && l.state == "granted"
        })
    });
    assert_eq!(
        store
            .leases_for_meeting(&meeting)
            .unwrap()
            .iter()
            .filter(|l| l.role == LeaseRole::Grantor && l.state == "expired")
            .count(),
        1
    );
    assert!(desktop_pending(&store).is_empty());

    awake.store(true, Ordering::SeqCst);
    hub.core.notify_jobs();
    wait_for("v2 on the computer", 60, || {
        hub.store()
            .get_meeting(&meeting)
            .unwrap()
            .transcript_version
            > live
    });
    wait_for("v2 on the phone", 60, || {
        store.get_meeting(&meeting).unwrap().transcript_version
            == hub
                .store()
                .get_meeting(&meeting)
                .unwrap()
                .transcript_version
    });
    wait_for("the lease to close", 30, || {
        store.leases_for_meeting(&meeting).unwrap().iter().any(|l| {
            l.role == LeaseRole::Grantor && l.epoch == first.epoch + 1 && l.state == "done"
        })
    });
    // Exactly one result: the version moved once, under the second epoch.
    assert_eq!(
        hub.store()
            .get_meeting(&meeting)
            .unwrap()
            .transcript_version,
        live + 1
    );
    assert_eq!(transcript_epoch(&hub.store(), &meeting), first.epoch + 1);
    assert_eq!(texts(&store, &meeting), texts(&hub.store(), &meeting));
    assert!(store.get_meeting(&meeting).unwrap().status == "ready");
}

/// Waits until the phone has dialled `n` more times than `since`.
fn wait_for_connects(phone: &Phone, since: usize, n: usize) {
    wait_for("more connection attempts", 40, || {
        phone.link.connects.load(Ordering::SeqCst) >= since + n
    });
}

#[test]
fn a_refusing_hub_never_costs_the_phone_its_pin() {
    let hub = hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    let spoke_gid = phone.store().sync_device_gid().unwrap();
    // The session that pairing opens ends with an "away" report once the hub
    // forgets the phone, and a down state is reported once: let it settle
    // first (and end), so the refusal below is the first thing reported.
    wait_for("a session", 20, || {
        phone.link.open.load(Ordering::SeqCst) == 1
    });
    phone.svc.set_app_active(false);
    wait_for("the session to end", 20, || {
        phone.link.open.load(Ordering::SeqCst) == 0
    });
    // The desktop forgets the phone without telling it. Whatever closes the
    // handshake (this, a second Ghira computer, a full hub) is only "can't
    // reach": the pin and its Wipe scope stay.
    hub.store().unpin_device(&spoke_gid).unwrap();
    let _ = code_for(&hub);
    let before = phone.link.connects.load(Ordering::SeqCst);
    phone.svc.set_app_active(true);
    wait_for_connects(&phone, before, 3);
    wait_for("the refusal to be reported", 20, || {
        phone.events().iter().any(|e| {
            matches!(
                e,
                SyncEvent::Error {
                    code: SyncErrorCode::Refused
                }
            )
        })
    });
    assert_eq!(phone.svc.devices().unwrap().len(), 1, "the pin stays");
    let events = phone.events();
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SyncEvent::Unpaired { .. })),
        "{events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            SyncEvent::Error {
                code: SyncErrorCode::Refused
            }
        )),
        "{events:?}"
    );
}

#[test]
fn another_hub_on_the_lan_that_cannot_read_the_handshake_keeps_the_pin() {
    let hub = hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    wait_for("a session", 20, || {
        phone.link.open.load(Ordering::SeqCst) == 1
    });
    // The phone's sockets now reach a different computer (another static key).
    let stranger = super::tests::hub();
    let _ = code_for(&stranger);
    *lock(&phone.link.hub) = stranger.svc.clone();
    phone.svc.set_app_active(false);
    wait_for("the session to end", 20, || {
        phone.link.open.load(Ordering::SeqCst) == 0
    });
    let before = phone.link.connects.load(Ordering::SeqCst);
    phone.svc.set_app_active(true);
    wait_for_connects(&phone, before, 3);
    assert_eq!(phone.svc.devices().unwrap().len(), 1);
    assert!(
        !phone
            .events()
            .iter()
            .any(|e| matches!(e, SyncEvent::Unpaired { .. }))
    );
    // The real hub is back: the session resumes with the same pin.
    *lock(&phone.link.hub) = hub.svc.clone();
    wait_for("a session again", 30, || {
        phone.link.open.load(Ordering::SeqCst) == 1
    });
}

#[test]
fn the_app_lock_ends_the_session_and_unlocking_brings_it_back() {
    let hub = hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    wait_for("a session", 20, || {
        phone.link.open.load(Ordering::SeqCst) == 1
    });
    phone.core.set_locked(true);
    wait_for("the session to end", 20, || {
        phone.link.open.load(Ordering::SeqCst) == 0
    });
    let connects = phone.link.connects.load(Ordering::SeqCst);
    thread::sleep(Duration::from_millis(2200));
    assert_eq!(
        phone.link.connects.load(Ordering::SeqCst),
        connects,
        "no connection while locked"
    );
    phone.core.set_locked(false);
    wait_for("a session again", 20, || {
        phone.link.open.load(Ordering::SeqCst) == 1
    });
}

#[test]
fn leaving_the_foreground_says_bye_inside_a_background_task() {
    let hub = hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    wait_for("a session", 20, || {
        phone.link.open.load(Ordering::SeqCst) == 1
    });
    phone.svc.set_app_active(false);
    wait_for("the session to end", 20, || {
        phone.link.open.load(Ordering::SeqCst) == 0
    });
    wait_for("the background task to end", 20, || {
        phone.link.bg.load(Ordering::SeqCst) == 0
    });
    phone.svc.set_app_active(true);
    wait_for("a session again", 20, || {
        phone.link.open.load(Ordering::SeqCst) == 1
    });
}

#[test]
fn a_change_on_the_phone_reaches_the_hub_without_waiting_for_the_timer() {
    let hub = hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    wait_for("a session", 20, || {
        phone.link.open.load(Ordering::SeqCst) == 1
    });
    let store = phone.store();
    let m = store
        .create_meeting(ghi_store::store::NewMeeting {
            title: "Typed on the phone".into(),
            ..Default::default()
        })
        .unwrap()
        .gid;
    store.set_meeting_status(&m, "done").unwrap();
    // Well under the 30 s timer.
    wait_for("the meeting on the hub", 10, || {
        hub.store().get_meeting(&m).is_ok()
    });
}

#[test]
fn a_pong_that_says_dirty_makes_the_phone_pull() {
    let hub = hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    wait_for("a session", 20, || {
        phone.link.open.load(Ordering::SeqCst) == 1
    });
    // Let the first pass settle, then change something on the hub only.
    thread::sleep(Duration::from_millis(1500));
    let m = hub
        .store()
        .create_meeting(ghi_store::store::NewMeeting {
            title: "From the computer".into(),
            ..Default::default()
        })
        .unwrap()
        .gid;
    hub.store().set_meeting_status(&m, "done").unwrap();
    // The next ping (15 s) is answered `dirty`; the 30 s timer is later.
    wait_for("the meeting on the phone", 25, || {
        phone.store().get_meeting(&m).is_ok()
    });
}

#[test]
fn delete_everything_on_the_phone_wipes_the_hub_when_it_is_reachable() {
    let hub = hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    let meeting = record_meeting(&phone.store());
    stop_for_desktop(&phone, &meeting);
    wait_for("the meeting on the hub", 30, || {
        hub.store().get_meeting(&meeting).is_ok()
    });
    phone.svc.delete_everywhere_prepare(true).unwrap();
    // The hub shredded what it got from the phone and forgot it.
    assert!(hub.store().get_meeting(&meeting).is_err());
    assert!(hub.svc.devices().unwrap().is_empty());
    assert!(phone.svc.devices().unwrap().is_empty());
    assert_eq!(
        phone.svc.delete_everywhere_status().state,
        DeleteEverywhereState::Done
    );
}

#[test]
fn delete_everything_does_not_wait_for_a_hub_that_was_not_seen_lately() {
    let hub = hub();
    let phone = phone_with(&hub, Arc::new(handlers), Duration::ZERO);
    pair(&hub, &phone);
    hub.svc.stop();
    // A zero window means "seen in an earlier millisecond than now" (the
    // service compares `now - last_seen <= 0`): let the pairing's own stamp age.
    thread::sleep(Duration::from_millis(50));
    let t = Instant::now();
    phone.svc.delete_everywhere_prepare(true).unwrap();
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
    // Not reached: the pin is still there (the computer finds itself
    // refused when this phone is gone).
    assert_eq!(phone.svc.devices().unwrap().len(), 1);
}

#[test]
fn a_failed_delete_puts_the_queued_devices_back_to_paired() {
    let hub = hub_with(Vec::new(), Duration::ZERO);
    let phone = phone(&hub);
    pair(&hub, &phone);
    // The prepare queues the wipe; the delete then fails.
    let r = hub.core.delete_everything_with(&|| {
        hub.svc.delete_everywhere_prepare(true)?;
        assert!(
            hub.svc
                .devices()
                .unwrap()
                .iter()
                .all(|d| d.state == DeviceState::WipePending)
        );
        Err("the data is still in use".into())
    });
    assert_eq!(r, Err("the data is still in use".to_string()));
    assert!(
        hub.svc
            .devices()
            .unwrap()
            .iter()
            .all(|d| d.state == DeviceState::Paired),
        "{:?}",
        hub.svc.devices()
    );
}

#[test]
fn a_device_that_was_already_queued_stays_queued_after_a_failed_delete() {
    let hub = hub_with(Vec::new(), Duration::ZERO);
    let phone = phone(&hub);
    pair(&hub, &phone);
    let gid = hub.svc.devices().unwrap()[0].gid.clone();
    hub.svc.unpair_and_wipe(&gid).unwrap();
    let r = hub.core.delete_everything_with(&|| {
        hub.svc.delete_everywhere_prepare(true)?;
        Err("boom".into())
    });
    assert!(r.is_err());
    assert_eq!(
        hub.svc.devices().unwrap()[0].state,
        DeviceState::WipePending,
        "the user's own wipe request is not undone"
    );
}

#[test]
fn the_setting_in_hours_sets_the_lease_time() {
    let hub = hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    phone.hours.store(3, Ordering::SeqCst);
    let meeting = record_meeting(&phone.store());
    stop_for_desktop(&phone, &meeting);
    wait_for("the offer", 30, || {
        grantor_lease(&phone.store(), &meeting).is_some()
    });
    assert_eq!(
        grantor_lease(&phone.store(), &meeting).unwrap().ttl_ms,
        3 * 3_600_000
    );
}

#[test]
fn restored_pins_are_dropped_with_the_identity() {
    let hub = hub();
    let phone = phone(&hub);
    pair(&hub, &phone);
    let secrets = phone.core.secrets().unwrap();
    drop_restored_pins(&phone.store(), secrets.as_ref()).unwrap();
    assert!(phone.svc.devices().unwrap().is_empty());
    assert!(
        Identity::load(secrets.as_ref()).unwrap().is_none(),
        "the old identity is gone"
    );
}
