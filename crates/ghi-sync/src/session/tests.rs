// SPDX-License-Identifier: Apache-2.0
//! Hub and spoke sessions over in-memory transports and fake stores.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use ciborium::value::Value;
use ghi_store::sync::devices::{DeviceRole, NewDevice};
use ghi_store::sync::records::{Bytes, MarkRec, MeetingRec, Record, TombCause};

use super::fake::FakeSyncStore;
use super::hub::HubSession;
use super::spoke::SpokeSession;
use super::{SessionReport, Timing};
use crate::SyncError;
use crate::clock::FakeClock;
use crate::mem::MemDuplex;
use crate::transport::Transport;
use crate::wire::{self, ErrorCode, Message, Proto};

pub(crate) const HUB_KEY: [u8; 32] = [0xA1; 32];
pub(crate) fn spoke_key(n: u8) -> [u8; 32] {
    [n; 32]
}

pub(crate) fn hub_store() -> Arc<FakeSyncStore> {
    Arc::new(FakeSyncStore::new("hub", true))
}

/// A spoke store, pinned to the hub and pinned by it.
pub(crate) fn paired_spoke(hub: &FakeSyncStore, name: &str, key_byte: u8) -> Arc<FakeSyncStore> {
    let spoke = Arc::new(FakeSyncStore::new(name, false));
    spoke_pins(hub, &spoke, name, key_byte);
    spoke
}

pub(crate) fn spoke_pins(hub: &FakeSyncStore, spoke: &FakeSyncStore, name: &str, key_byte: u8) {
    use crate::store::SyncStore;
    let psk = [9u8; 32];
    hub.pin_device(
        &NewDevice {
            gid: name.into(),
            name: name.into(),
            platform: "ios".into(),
            role: DeviceRole::Spoke,
            static_pub: spoke_key(key_byte),
        },
        &psk,
    )
    .unwrap();
    spoke
        .pin_device(
            &NewDevice {
                gid: "hub".into(),
                name: "desktop".into(),
                platform: "macos".into(),
                role: DeviceRole::Hub,
                static_pub: HUB_KEY,
            },
            &psk,
        )
        .unwrap();
}

pub(crate) fn meeting(gid: &str) -> Record {
    Record::Meeting(MeetingRec {
        gid: gid.into(),
        title_ct: Some(Bytes(vec![1, 2, 3])),
        started_at: Some(1_000),
        ..Default::default()
    })
}

pub(crate) fn mark(gid: &str, meeting_gid: &str, t: i64) -> Record {
    Record::Mark(MarkRec {
        gid: gid.into(),
        meeting_gid: meeting_gid.into(),
        t_ms: Some(t),
        ..Default::default()
    })
}

pub(crate) fn clock() -> Arc<FakeClock> {
    Arc::new(FakeClock::new(1_700_000_000_000))
}

/// One spoke pass against the hub on a thread; `bye` ends the session.
pub(crate) fn sync_once(
    hub: &Arc<FakeSyncStore>,
    spoke: &Arc<FakeSyncStore>,
    key_byte: u8,
) -> (crate::Result<SessionReport>, crate::Result<SessionReport>) {
    let (a, b) = MemDuplex::pair(spoke_key(key_byte), HUB_KEY);
    run_pair(hub, spoke, a, b)
}

pub(crate) fn run_pair<S: Transport + 'static, H: Transport + 'static>(
    hub: &Arc<FakeSyncStore>,
    spoke: &Arc<FakeSyncStore>,
    spoke_side: S,
    hub_side: H,
) -> (crate::Result<SessionReport>, crate::Result<SessionReport>) {
    run_with_clocks(hub, spoke, spoke_side, hub_side, clock(), clock())
}

/// Like [`run_pair`], with the clocks each side's session reads.
pub(crate) fn run_with_clocks<S: Transport + 'static, H: Transport + 'static>(
    hub: &Arc<FakeSyncStore>,
    spoke: &Arc<FakeSyncStore>,
    spoke_side: S,
    hub_side: H,
    hub_clock: Arc<FakeClock>,
    spoke_clock: Arc<FakeClock>,
) -> (crate::Result<SessionReport>, crate::Result<SessionReport>) {
    let hub_store = hub.clone();
    let server = thread::spawn(move || HubSession::new(hub_store, hub_clock, hub_side).serve());
    let mut s = SpokeSession::new(spoke.clone(), spoke_clock, spoke_side, "hub".into());
    let mine = s.run_once();
    let _ = s.bye();
    drop(s);
    (mine, server.join().unwrap())
}

#[test]
fn a_hub_and_two_spokes_converge() {
    let hub = hub_store();
    let a = paired_spoke(&hub, "phone-a", 1);
    let b = paired_spoke(&hub, "phone-b", 2);
    a.put_local(meeting("m1"));
    for i in 0..3 {
        a.put_local(mark(&format!("k{i}"), "m1", i));
    }
    a.set_dek("m1", [7; 32]);
    b.put_local(meeting("m2"));
    b.set_dek("m2", [8; 32]);

    let (ra, rh) = sync_once(&hub, &a, 1);
    let ra = ra.unwrap();
    assert_eq!((ra.rows_pushed, ra.tombs_pushed), (4, 0));
    assert_eq!(rh.unwrap().rows_pushed, 4);
    // The hub got rows and the key; the spoke is clean and the key marked.
    assert_eq!(hub.row_gids(), ["k0", "k1", "k2", "m1"]);
    assert_eq!(hub.dek("m1"), Some([7; 32]));
    assert!(a.key_sent("hub", "m1"));
    assert!(!a.is_dirty("m1") && !a.is_dirty("k2"));

    sync_once(&hub, &b, 2).0.unwrap();
    assert_eq!(
        b.row_gids(),
        ["k0", "k1", "k2", "m1", "m2"],
        "b pulled a's meeting"
    );
    assert_eq!(b.dek("m1"), Some([7; 32]), "the key came with the record");
    assert!(hub.key_sent("phone-b", "m1"), "the ack-pull marked the key");
    sync_once(&hub, &a, 1).0.unwrap();
    assert_eq!(a.row_gids(), b.row_gids());
    assert_eq!(a.dek("m2"), Some([8; 32]));

    // A delete travels: tombstone from a reaches b through the hub.
    a.delete_local("k1", "mark", Some(TombCause::User));
    let r = sync_once(&hub, &a, 1).0.unwrap();
    assert_eq!(r.tombs_pushed, 1);
    let r = sync_once(&hub, &b, 2).0.unwrap();
    assert_eq!(r.tombs_pulled, 1);
    assert!(b.is_tombstoned("k1") && b.row("k1").is_none());

    // Nothing left to do: a quiet pass changes nothing.
    let before = (hub.applied_rows(), a.applied_rows(), b.applied_rows());
    let r = sync_once(&hub, &a, 1).0.unwrap();
    assert_eq!((r.rows_pushed, r.rows_pulled, r.tombs_pushed), (0, 0, 0));
    assert_eq!(
        before,
        (hub.applied_rows(), a.applied_rows(), b.applied_rows())
    );
}

#[test]
fn cursors_move_only_with_the_ack() {
    let hub = hub_store();
    let a = paired_spoke(&hub, "phone-a", 1);
    for i in 0..5 {
        a.put_local(meeting(&format!("m{i}")));
    }
    sync_once(&hub, &a, 1).0.unwrap();
    let (push, feed, pull) = a.cursors("hub").unwrap();
    assert_eq!(push, a.head());
    assert_eq!(feed.as_deref(), Some("feed-hub"));
    assert_eq!(pull, hub.head());
}

/// Fails the n-th message (counting sends and receives) and everything
/// after it, like a connection that dropped there.
pub(crate) struct Flaky {
    pub(crate) inner: Option<MemDuplex>,
    pub(crate) budget: usize,
}

impl Flaky {
    fn tick(&mut self) -> crate::Result<&mut MemDuplex> {
        if self.budget == 0 {
            self.inner = None;
            return Err(SyncError::Closed);
        }
        self.budget -= 1;
        self.inner.as_mut().ok_or(SyncError::Closed)
    }
}

impl Transport for Flaky {
    fn send(&mut self, msg: &[u8]) -> crate::Result<()> {
        self.tick()?.send(msg)
    }
    fn recv(&mut self) -> crate::Result<Vec<u8>> {
        self.tick()?.recv()
    }
    fn peer_static(&self) -> [u8; 32] {
        self.inner.as_ref().map_or([0; 32], |i| i.peer_static())
    }
    fn set_recv_timeout(&mut self, t: Option<Duration>) -> crate::Result<()> {
        self.inner
            .as_mut()
            .map_or(Ok(()), |i| i.set_recv_timeout(t))
    }
}

#[test]
fn a_drop_at_every_message_boundary_resumes_without_loss_or_duplicates() {
    let hub = hub_store();
    let a = paired_spoke(&hub, "phone-a", 1);
    let b = paired_spoke(&hub, "phone-b", 2);
    // Enough rows for three push batches and tombstones in two.
    a.put_local(meeting("m1"));
    a.set_dek("m1", [7; 32]);
    for i in 0..600 {
        a.put_local(mark(&format!("k{i:04}"), "m1", i));
    }
    for i in 0..3 {
        a.delete_local(&format!("k{i:04}"), "mark", Some(TombCause::User));
    }
    b.put_local(meeting("m2"));
    b.set_dek("m2", [8; 32]);
    sync_once(&hub, &b, 2).0.unwrap();

    let mut budget = 0;
    let mut failures = 0;
    loop {
        let (x, y) = MemDuplex::pair(spoke_key(1), HUB_KEY);
        let r = run_pair(
            &hub,
            &a,
            Flaky {
                inner: Some(x),
                budget,
            },
            y,
        )
        .0;
        match r {
            Ok(_) => break,
            Err(SyncError::Closed) => failures += 1,
            Err(e) => panic!("budget {budget}: {e}"),
        }
        budget += 1;
        assert!(budget < 200, "never completes");
    }
    assert!(failures >= 10, "dropped at {failures} boundaries");
    // Converged: the hub has a's meeting, its live marks, its tombstones.
    assert_eq!(hub.row_gids().len(), 1 + 597 + 1);
    assert!(hub.is_tombstoned("k0001") && hub.row("k0001").is_none());
    assert_eq!(hub.dek("m1"), Some([7; 32]));
    assert!(!a.is_dirty("m1") && !a.is_dirty("k0500"));
    // Re-sent batches were no-ops: every row was applied once.
    assert_eq!(hub.applied_rows(), 1 + 597 + 1);
    // And a pulls b's meeting.
    assert!(a.row("m2").is_some());
}

#[test]
fn a_drop_while_pulling_does_not_lose_the_key() {
    let hub = hub_store();
    let a = paired_spoke(&hub, "phone-a", 1);
    let b = paired_spoke(&hub, "phone-b", 2);
    a.put_local(meeting("m1"));
    a.set_dek("m1", [7; 32]);
    sync_once(&hub, &a, 1).0.unwrap();
    // b drops after the Rows reply but before applying everything.
    let mut budget = 0;
    loop {
        let (x, y) = MemDuplex::pair(spoke_key(2), HUB_KEY);
        let r = run_pair(
            &hub,
            &b,
            Flaky {
                inner: Some(x),
                budget,
            },
            y,
        )
        .0;
        if r.is_ok() {
            break;
        }
        // Whatever happened, b either has the row with its key or has neither.
        assert_eq!(b.row("m1").is_some(), b.dek("m1").is_some());
        budget += 1;
        assert!(budget < 100);
    }
    assert_eq!(b.dek("m1"), Some([7; 32]));
    assert!(hub.key_sent("phone-b", "m1"));
}

fn raw(id: u32, t: u8, body: Value) -> Vec<u8> {
    let env = Value::Map(vec![
        (Value::Text("t".into()), Value::Integer(t.into())),
        (Value::Text("id".into()), Value::Integer(id.into())),
        (Value::Text("b".into()), body),
        // A later minor may add envelope keys too.
        (Value::Text("future".into()), Value::Bool(true)),
    ]);
    let mut out = Vec::new();
    ciborium::into_writer(&env, &mut out).unwrap();
    out
}

fn text(s: &str) -> Value {
    Value::Text(s.into())
}

fn proto(major: u8, minor: u8) -> Value {
    Value::Map(vec![
        (text("major"), Value::Integer(major.into())),
        (text("minor"), Value::Integer(minor.into())),
    ])
}

#[test]
fn a_newer_minor_peer_with_extra_fields_and_messages_is_served() {
    let hub = hub_store();
    let _a = paired_spoke(&hub, "phone-a", 1);
    let (mut spoke_side, hub_side) = MemDuplex::pair(spoke_key(1), HUB_KEY);
    let (hs, clk) = (hub.clone(), clock());
    let server = thread::spawn(move || HubSession::new(hs, clk, hub_side).serve());
    // Hello from protocol 1.1 with a field this build has never heard of.
    let hello = Value::Map(vec![
        (text("proto"), Value::Array(vec![proto(1, 1)])),
        (text("app_version"), text("9.9")),
        (text("device_gid"), text("phone-a")),
        (text("feed_id"), text("feed-phone-a")),
        (text("pull_cursor"), Value::Integer(0.into())),
        (text("caps"), Value::Array(vec![])),
        (text("new_in_minor_1"), text("x")),
    ]);
    spoke_side.send(&raw(1, 1, hello)).unwrap();
    let (id, m) = wire::decode(&spoke_side.recv().unwrap()).unwrap();
    assert_eq!(id, 1);
    let Message::HelloOk(ok) = m else {
        panic!("{m:?}")
    };
    assert_eq!(
        ok.proto,
        Proto { major: 1, minor: 0 },
        "the older minor wins"
    );
    // A message type from the future: answered, ignored, the session goes on.
    spoke_side.send(&raw(2, 200, Value::Null)).unwrap();
    let (id, m) = wire::decode(&spoke_side.recv().unwrap()).unwrap();
    assert_eq!(id, 2);
    assert!(matches!(m, Message::Error(e) if e.code == ErrorCode::Unsupported));
    spoke_side
        .send(&wire::encode(3, &Message::Ping).unwrap())
        .unwrap();
    let (id, m) = wire::decode(&spoke_side.recv().unwrap()).unwrap();
    assert_eq!((id, m), (3, Message::Pong { dirty: false }));
    spoke_side
        .send(&wire::encode(4, &Message::Bye).unwrap())
        .unwrap();
    server.join().unwrap().unwrap();
}

#[test]
fn no_common_major_exchanges_nothing() {
    let hub = hub_store();
    let a = paired_spoke(&hub, "phone-a", 1);
    a.put_local(meeting("m1"));
    let (x, y) = MemDuplex::pair(spoke_key(1), HUB_KEY);
    let (hs, clk) = (hub.clone(), clock());
    let server = thread::spawn(move || {
        HubSession::new(hs, clk, y)
            .with_protos(vec![Proto { major: 2, minor: 0 }])
            .serve()
    });
    let mut s = SpokeSession::new(a.clone(), clock(), x, "hub".into());
    let err = s.run_once().unwrap_err();
    assert!(
        matches!(err, SyncError::Peer(ErrorCode::UpgradeRequired)),
        "{err}"
    );
    // The hub (newer) says which side must update.
    assert_eq!(
        s.last_peer_error().unwrap().detail.as_deref(),
        Some("spoke")
    );
    drop(s);
    let rep = server.join().unwrap().unwrap();
    assert_eq!(rep, SessionReport::default());
    assert!(hub.row_gids().is_empty());
    assert!(a.is_dirty("m1"), "nothing was pushed");
}

#[test]
fn a_mass_delete_waits_for_the_users_confirmation() {
    let hub = hub_store();
    let a = paired_spoke(&hub, "phone-a", 1);
    for i in 0..12 {
        a.put_local(meeting(&format!("m{i:02}")));
    }
    sync_once(&hub, &a, 1).0.unwrap();
    assert_eq!(hub.row_gids().len(), 12);
    for i in 0..11 {
        a.delete_local(&format!("m{i:02}"), "meeting", Some(TombCause::Meeting));
    }
    let (mine, theirs) = sync_once(&hub, &a, 1);
    assert!(matches!(
        mine,
        Err(SyncError::Peer(ErrorCode::NeedsConfirm))
    ));
    assert_eq!(theirs.unwrap().needs_confirm, 11);
    assert_eq!(hub.row_gids().len(), 12, "not applied");
    let (push, ..) = a.cursors("hub").unwrap();
    assert!(push < a.head(), "not acked, so the cursor stays");
    // The user confirms on the hub; the resent batch applies, once.
    hub.confirm_mass_delete("phone-a");
    sync_once(&hub, &a, 1).0.unwrap();
    assert_eq!(hub.row_gids(), ["m11"]);
    use crate::store::SyncStore;
    assert!(!hub.mass_delete_confirmed("phone-a").unwrap(), "spent");
}

#[test]
fn ten_meeting_deletes_and_any_number_of_retention_deletes_need_no_confirmation() {
    let hub = hub_store();
    let a = paired_spoke(&hub, "phone-a", 1);
    for i in 0..30 {
        a.put_local(meeting(&format!("m{i:02}")));
    }
    sync_once(&hub, &a, 1).0.unwrap();
    for i in 0..10 {
        a.delete_local(&format!("m{i:02}"), "meeting", Some(TombCause::Meeting));
    }
    for i in 10..30 {
        a.delete_local(&format!("m{i:02}"), "meeting", Some(TombCause::Retention));
    }
    sync_once(&hub, &a, 1).0.unwrap();
    assert!(hub.row_gids().is_empty());
}

#[test]
fn the_hub_holds_a_mass_delete_it_would_send_too() {
    // The spoke guards its own pull the same way.
    let hub = hub_store();
    let a = paired_spoke(&hub, "phone-a", 1);
    let b = paired_spoke(&hub, "phone-b", 2);
    for i in 0..12 {
        a.put_local(meeting(&format!("m{i:02}")));
    }
    sync_once(&hub, &a, 1).0.unwrap();
    sync_once(&hub, &b, 2).0.unwrap();
    assert_eq!(b.row_gids().len(), 12);
    for i in 0..11 {
        a.delete_local(&format!("m{i:02}"), "meeting", Some(TombCause::Meeting));
    }
    hub.confirm_mass_delete("phone-a");
    sync_once(&hub, &a, 1).0.unwrap();
    let r = sync_once(&hub, &b, 2).0.unwrap();
    assert_eq!(r.needs_confirm, 11);
    assert_eq!(b.row_gids().len(), 12);
    b.confirm_mass_delete("hub");
    sync_once(&hub, &b, 2).0.unwrap();
    assert_eq!(b.row_gids(), ["m11"]);
}

#[test]
fn a_silent_spoke_is_dropped_and_a_ping_gets_a_pong_with_the_dirty_hint() {
    let hub = hub_store();
    let a = paired_spoke(&hub, "phone-a", 1);
    // Silence.
    let (x, y) = MemDuplex::pair(spoke_key(1), HUB_KEY);
    let (hs, clk) = (hub.clone(), clock());
    let server = thread::spawn(move || {
        HubSession::new(hs, clk, y)
            .with_timing(Timing {
                silence: Duration::from_millis(60),
                ..Timing::default()
            })
            .serve()
    });
    assert!(matches!(server.join().unwrap(), Err(SyncError::Timeout)));
    drop(x);

    // The hub has a change the spoke hasn't pulled: Pong{dirty}.
    let (x, y) = MemDuplex::pair(spoke_key(1), HUB_KEY);
    let (hs, clk) = (hub.clone(), clock());
    let server = thread::spawn(move || HubSession::new(hs, clk, y).serve());
    let mut s = SpokeSession::new(a.clone(), clock(), x, "hub".into());
    s.run_once().unwrap();
    assert!(!s.ping().unwrap());
    hub.put_local(meeting("hub-m"));
    assert!(s.ping().unwrap());
    s.run_once().unwrap();
    assert!(a.row("hub-m").is_some());
    assert!(!s.ping().unwrap());
    s.bye().unwrap();
    server.join().unwrap().unwrap();
}

#[test]
fn a_session_with_only_pings_ends_after_the_idle_limit() {
    let hub = hub_store();
    let a = paired_spoke(&hub, "phone-a", 1);
    let (x, y) = MemDuplex::pair(spoke_key(1), HUB_KEY);
    let hub_clock = clock();
    let (hs, hc) = (hub.clone(), hub_clock.clone());
    let server = thread::spawn(move || HubSession::new(hs, hc, y).serve());
    let mut s = SpokeSession::new(a, clock(), x, "hub".into());
    s.run_once().unwrap();
    hub_clock.advance(Duration::from_secs(4 * 60));
    assert!(s.ping().is_ok());
    hub_clock.advance(Duration::from_secs(61));
    // Past five minutes without work: the hub hangs up instead of answering.
    assert!(matches!(s.ping(), Err(SyncError::Closed)));
    server.join().unwrap().unwrap();
}

#[test]
fn a_request_before_hello_and_an_unpinned_key_are_refused() {
    let hub = hub_store();
    let _a = paired_spoke(&hub, "phone-a", 1);
    let (mut x, y) = MemDuplex::pair(spoke_key(1), HUB_KEY);
    let (hs, clk) = (hub.clone(), clock());
    let server = thread::spawn(move || HubSession::new(hs, clk, y).serve());
    x.send(&wire::encode(1, &Message::Ping).unwrap()).unwrap();
    assert!(server.join().unwrap().is_err());
    // A key the hub has no pin for.
    let (mut x, y) = MemDuplex::pair(spoke_key(77), HUB_KEY);
    let (hs, clk) = (hub.clone(), clock());
    let server = thread::spawn(move || HubSession::new(hs, clk, y).serve());
    let hello = Message::Hello(wire::Hello {
        proto: vec![wire::PROTO],
        app_version: "1".into(),
        device_gid: "phone-a".into(),
        feed_id: "f".into(),
        pull_cursor: 0,
        caps: vec![],
    });
    x.send(&wire::encode(1, &hello).unwrap()).unwrap();
    assert!(server.join().unwrap().is_err());
    let _ = x.recv();
}

#[test]
fn a_new_hub_feed_id_restarts_the_pull_from_zero() {
    let hub = hub_store();
    let a = paired_spoke(&hub, "phone-a", 1);
    hub.put_local(meeting("m1"));
    sync_once(&hub, &a, 1).0.unwrap();
    use crate::store::SyncStore;
    hub.regen_feed_id().unwrap();
    // The restored hub has a fresh log: a resets and re-pulls everything.
    let before = a.applied_rows();
    sync_once(&hub, &a, 1).0.unwrap();
    let (_, feed, _) = a.cursors("hub").unwrap();
    assert_eq!(feed.as_deref(), Some(hub.feed_id().unwrap().as_str()));
    assert_eq!(a.applied_rows(), before, "the same version is a no-op");
}
