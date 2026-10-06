// SPDX-License-Identifier: Apache-2.0
//! Unpair and wipe (doc 07 §3.5; slice 15-F).
//!
//! `Unpair` deletes the pin and the pair PSK; data stays. `Wipe` shreds every
//! meeting exchanged with the sender, locally and without tombstones, then
//! unpairs, and is answered with `WipeDone`. A device in `wipe_pending` only
//! takes part in a session that delivers `Wipe`; so does one in
//! `unpair_pending` for `Unpair` (the desktop unpaired a phone that was away).

use ghi_store::StoreError;
use ghi_store::sync::devices::DeviceState;

use crate::store::SyncStore;
use crate::wire::Control;
use crate::{Result, SyncError};

/// What applying a command did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlOutcome {
    /// The pin is gone; close the session.
    Unpaired,
    /// Everything exchanged with the peer is gone and the pin too; answer
    /// `WipeDone`, then close.
    Wiped,
}

/// A pin that is already gone is what an unpair wants.
fn unpin(store: &dyn SyncStore, gid: &str) -> Result<()> {
    match store.unpin_device(gid) {
        Ok(()) | Err(StoreError::NotFound { .. }) => Ok(()),
        Err(e) => Err(SyncError::Store(e)),
    }
}

/// Applies a command received from `from_device`.
pub fn apply_control(
    store: &dyn SyncStore,
    from_device: &str,
    control: &Control,
) -> Result<ControlOutcome> {
    match control {
        Control::Unpair => {
            unpin(store, from_device)?;
            Ok(ControlOutcome::Unpaired)
        }
        Control::Wipe { .. } => {
            // Shred first: if it fails the pin stays and the command can be
            // delivered again.
            store.wipe_peer(from_device)?;
            unpin(store, from_device)?;
            Ok(ControlOutcome::Wiped)
        }
    }
}

/// The local half of a command this device *sent* and the peer confirmed:
/// only the pin goes. A wipe shreds what the *receiver* holds; the sender
/// keeps every copy of its own (doc 07 §3.5).
pub fn apply_sent(
    store: &dyn SyncStore,
    to_device: &str,
    control: &Control,
) -> Result<ControlOutcome> {
    unpin(store, to_device)?;
    Ok(match control {
        Control::Unpair => ControlOutcome::Unpaired,
        Control::Wipe { .. } => ControlOutcome::Wiped,
    })
}

/// The command a session must deliver first to `device_gid`
/// (`wipe_pending` or `unpair_pending`), if any.
pub fn pending_for(store: &dyn SyncStore, device_gid: &str) -> Result<Vec<Control>> {
    Ok(match store.device(device_gid)? {
        Some(d) if d.state == DeviceState::WipePending => vec![Control::Wipe {
            reason: "wipe".to_string(),
        }],
        Some(d) if d.state == DeviceState::UnpairPending => vec![Control::Unpair],
        _ => Vec::new(),
    })
}

/// "Delete everything" with "also on paired devices": every pin moves to
/// `wipe_pending`, so each device gets `Wipe` the next time it connects (or
/// on its next `Ping` if it is connected now). Returns the devices queued.
pub fn queue_wipe_all(store: &dyn SyncStore) -> Result<Vec<String>> {
    let mut queued = Vec::new();
    for d in store.devices()? {
        store.set_wipe_pending(&d.gid)?;
        queued.push(d.gid);
    }
    Ok(queued)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::*;
    use crate::mem::MemDuplex;
    use crate::session::fake::FakeSyncStore;
    use crate::session::hub::HubSession;
    use crate::session::spoke::SpokeSession;
    use crate::session::tests::{
        HUB_KEY, clock, hub_store, mark, meeting, paired_spoke, spoke_key, sync_once,
    };
    use crate::wire::{self, ErrorCode, Message};

    /// A hub and phone that exchanged `m1` (phone's) and `h1` (hub's), plus
    /// `own` on the hub only and `phone-own` on the phone only.
    fn rig() -> (Arc<FakeSyncStore>, Arc<FakeSyncStore>) {
        let hub = hub_store();
        let phone = paired_spoke(&hub, "phone-a", 1);
        phone.put_local(meeting("m1"));
        phone.put_local(mark("k1", "m1", 1));
        phone.set_dek("m1", [7; 32]);
        phone.put_unsynced(meeting("phone-own"));
        hub.put_local(meeting("h1"));
        hub.set_dek("h1", [8; 32]);
        hub.put_unsynced(meeting("own"));
        sync_once(&hub, &phone, 1).0.unwrap();
        sync_once(&hub, &phone, 1).0.unwrap();
        (hub, phone)
    }

    fn connected(
        hub: &Arc<FakeSyncStore>,
        phone: &Arc<FakeSyncStore>,
    ) -> (
        SpokeSession<MemDuplex>,
        thread::JoinHandle<crate::Result<crate::session::SessionReport>>,
    ) {
        let (x, y) = MemDuplex::pair(spoke_key(1), HUB_KEY);
        let hs = hub.clone();
        let server = thread::spawn(move || HubSession::new(hs, clock(), y).serve());
        (
            SpokeSession::new(phone.clone(), clock(), x, "hub".into()),
            server,
        )
    }

    #[test]
    fn unpair_removes_both_pins_and_keeps_the_data() {
        let (hub, phone) = rig();
        let (mut s, server) = connected(&hub, &phone);
        assert_eq!(
            s.send_control(wire::Control::Unpair).unwrap(),
            ControlOutcome::Unpaired
        );
        drop(s);
        assert_eq!(
            server.join().unwrap().unwrap().closed_by,
            Some(ControlOutcome::Unpaired)
        );
        assert!(hub.device_ids().is_empty() && phone.device_ids().is_empty());
        assert!(hub.row("m1").is_some() && phone.row("h1").is_some());
    }

    #[test]
    fn a_phone_that_wipes_the_desktop_keeps_all_its_own_copies() {
        let (hub, phone) = rig();
        let (mut s, server) = connected(&hub, &phone);
        assert_eq!(
            s.send_control(wire::Control::Wipe {
                reason: "lost".into()
            })
            .unwrap(),
            ControlOutcome::Wiped
        );
        drop(s);
        server.join().unwrap().unwrap();
        // The computer shreds everything it shares with the phone, no
        // tombstones; the phone loses only its pin.
        assert!(hub.row("m1").is_none() && hub.row("h1").is_none());
        assert!(hub.dek("m1").is_none());
        assert!(hub.row("own").is_some());
        assert!(phone.row("m1").is_some() && phone.row("h1").is_some());
        assert!(phone.dek("m1").is_some(), "the sender keeps its keys");
        assert!(phone.row("phone-own").is_some());
        assert!(!hub.is_tombstoned("m1"), "a wipe never writes tombstones");
        assert!(!phone.is_tombstoned("m1") && !phone.is_tombstoned("h1"));
        assert!(hub.device_ids().is_empty() && phone.device_ids().is_empty());
    }

    #[test]
    fn a_wipe_is_delivered_on_reconnect_then_the_pin_goes() {
        let (hub, phone) = rig();
        // The user hits "Unpair and wipe" on the desktop; the phone is away.
        use crate::store::SyncStore;
        hub.set_wipe_pending("phone-a").unwrap();
        assert_eq!(pending_for(hub.as_ref(), "phone-a").unwrap().len(), 1);
        assert!(pending_for(hub.as_ref(), "nobody").unwrap().is_empty());
        // It reconnects: HelloOk carries the Wipe, nothing else happens.
        let (mut s, server) = connected(&hub, &phone);
        phone.put_local(meeting("late"));
        let rep = s.run_once().unwrap();
        assert_eq!(rep.closed_by, Some(ControlOutcome::Wiped));
        assert_eq!(
            (rep.rows_pushed, rep.rows_pulled),
            (0, 0),
            "no sync in a wipe session"
        );
        drop(s);
        let hub_rep = server.join().unwrap().unwrap();
        assert_eq!(hub_rep.closed_by, Some(ControlOutcome::Wiped));
        assert!(hub.device_ids().is_empty(), "pin removed after WipeDone");
        assert!(phone.device_ids().is_empty());
        assert!(phone.row("h1").is_none() && phone.row("m1").is_none());
        assert!(phone.row("phone-own").is_some(), "never exchanged");
        assert!(hub.row("late").is_none(), "nothing was pushed");
    }

    #[test]
    fn a_wipe_pending_session_does_nothing_but_deliver_the_wipe() {
        let (hub, _phone) = rig();
        use crate::store::SyncStore;
        hub.set_wipe_pending("phone-a").unwrap();
        let (mut x, y) = MemDuplex::pair(spoke_key(1), HUB_KEY);
        let hs = hub.clone();
        let server = thread::spawn(move || HubSession::new(hs, clock(), y).serve());
        let mut rpc = crate::session::Rpc::default();
        let hello = Message::Hello(wire::Hello {
            proto: vec![wire::PROTO],
            app_version: "1".into(),
            device_gid: "phone-a".into(),
            feed_id: "f".into(),
            pull_cursor: 0,
            caps: vec![],
        });
        let Message::HelloOk(ok) = rpc.call(&mut x, &hello).unwrap() else {
            panic!("hello")
        };
        assert!(matches!(
            ok.pending.as_slice(),
            [wire::Control::Wipe { .. }]
        ));
        let rows = Message::PushRows(wire::PushRows {
            rows: vec![wire::Record(mark("sneaky", "m1", 1))],
            upto_seq: 5,
        });
        assert!(matches!(
            rpc.call(&mut x, &rows),
            Err(crate::SyncError::Peer(ErrorCode::Busy))
        ));
        assert!(server.join().unwrap().is_ok());
        assert!(hub.row("sneaky").is_none());
        assert_eq!(hub.device_ids().len(), 1, "no WipeDone yet: the pin stays");
    }

    #[test]
    fn a_wipe_reaches_a_phone_that_is_already_connected_on_its_next_ping() {
        let (hub, phone) = rig();
        let (mut s, server) = connected(&hub, &phone);
        s.run_once().unwrap();
        assert!(!s.ping().unwrap());
        use crate::store::SyncStore;
        hub.set_wipe_pending("phone-a").unwrap();
        assert!(!s.ping().unwrap());
        assert_eq!(s.report().closed_by, Some(ControlOutcome::Wiped));
        assert!(phone.device_ids().is_empty() && phone.row("h1").is_none());
        drop(s);
        server.join().unwrap().unwrap();
        assert!(hub.device_ids().is_empty());
    }

    #[test]
    fn an_unpair_is_delivered_on_reconnect_then_the_pin_goes_on_both_sides() {
        use crate::store::SyncStore;
        let (hub, phone) = rig();
        hub.set_unpair_pending("phone-a").unwrap();
        assert!(matches!(
            pending_for(hub.as_ref(), "phone-a").unwrap().as_slice(),
            [wire::Control::Unpair]
        ));
        let (mut s, server) = connected(&hub, &phone);
        phone.put_local(meeting("late"));
        let rep = s.run_once().unwrap();
        assert_eq!(rep.closed_by, Some(ControlOutcome::Unpaired));
        assert_eq!((rep.rows_pushed, rep.rows_pulled), (0, 0));
        drop(s);
        let hub_rep = server.join().unwrap().unwrap();
        assert_eq!(hub_rep.closed_by, Some(ControlOutcome::Unpaired));
        assert!(hub.device_ids().is_empty() && phone.device_ids().is_empty());
        assert!(phone.row("h1").is_some(), "an unpair keeps the data");
        assert!(hub.row("late").is_none());
    }

    #[test]
    fn an_unpair_reaches_a_phone_that_is_already_connected_on_its_next_ping() {
        use crate::store::SyncStore;
        let (hub, phone) = rig();
        let (mut s, server) = connected(&hub, &phone);
        s.run_once().unwrap();
        assert!(!s.ping().unwrap());
        hub.set_unpair_pending("phone-a").unwrap();
        assert!(!s.ping().unwrap());
        assert_eq!(s.report().closed_by, Some(ControlOutcome::Unpaired));
        assert!(phone.device_ids().is_empty());
        drop(s);
        server.join().unwrap().unwrap();
        assert!(hub.device_ids().is_empty());
    }

    #[test]
    fn delete_everything_queues_a_wipe_for_every_paired_device() {
        let hub = hub_store();
        let a = paired_spoke(&hub, "phone-a", 1);
        let b = paired_spoke(&hub, "phone-b", 2);
        a.put_local(meeting("ma"));
        b.put_local(meeting("mb"));
        sync_once(&hub, &a, 1).0.unwrap();
        sync_once(&hub, &b, 2).0.unwrap();
        let mut queued = queue_wipe_all(hub.as_ref()).unwrap();
        queued.sort();
        assert_eq!(queued, ["phone-a", "phone-b"]);
        // Each wipes on its own reconnect; the hub keeps a pin until then.
        let r = sync_once(&hub, &a, 1);
        assert_eq!(r.0.unwrap().closed_by, Some(ControlOutcome::Wiped));
        assert_eq!(hub.device_ids(), ["phone-b"]);
        assert!(a.row("ma").is_none());
        let r = sync_once(&hub, &b, 2);
        assert_eq!(r.0.unwrap().closed_by, Some(ControlOutcome::Wiped));
        assert!(hub.device_ids().is_empty());
    }

    #[test]
    fn applying_the_same_command_twice_is_harmless() {
        let (hub, _phone) = rig();
        let c = wire::Control::Wipe { reason: "x".into() };
        assert_eq!(
            apply_control(hub.as_ref(), "phone-a", &c).unwrap(),
            ControlOutcome::Wiped
        );
        assert_eq!(
            apply_control(hub.as_ref(), "phone-a", &c).unwrap(),
            ControlOutcome::Wiped
        );
        assert_eq!(
            apply_control(hub.as_ref(), "phone-a", &wire::Control::Unpair).unwrap(),
            ControlOutcome::Unpaired
        );
    }
}
