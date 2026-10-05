// SPDX-License-Identifier: Apache-2.0
//! Pairing (doc 07 §3.4; slice 15-F): `PairHello` / `PairAccept` / `PairDone`.
//!
//! The hub opens a [`PairingWindow`] (QR PSK, 120 s, single use, burned on
//! `PairHello`; a new QR after 3 failed handshakes). The pin is staged until
//! `PairDone` arrives (10 s) and only then committed, so a crash leaves no
//! half pin. The listener exists only while the window is open.

use std::time::Duration;

use ghi_store::StoreError;
use ghi_store::sync::devices::{DeviceRole, DeviceState, NewDevice};
use ghi_store::sync::records::Bytes;
use zeroize::Zeroize;

use crate::clock::Clock;
use crate::identity::{Identity, Psk};
use crate::session::{Rpc, error_msg, recv_msg, send_msg};
use crate::store::SyncStore;
use crate::transport::Transport;
use crate::wire::{self, Decoded, ErrorCode, Message, PairAccept, PairHello};
use crate::{Result, SyncError};

/// How long a QR PSK lives.
pub const QR_TTL: Duration = Duration::from_secs(120);
/// How long a staged pin waits for `PairDone`.
pub const PAIR_DONE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the hub waits for `PairHello` after the handshake.
pub const PAIR_HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// Failed handshakes with the QR PSK before a new QR is needed.
pub const MAX_QR_FAILURES: u32 = 3;
/// Bounds on what a peer may name itself.
const MAX_NAME: usize = 128;
const MAX_PLATFORM: usize = 32;

/// The hub's open pairing window.
#[derive(Debug)]
pub struct PairingWindow {
    psk: Psk,
    expires_cont_ns: u64,
    burned: bool,
    failures: u32,
}

impl PairingWindow {
    /// Opens a window: a fresh QR PSK valid for [`QR_TTL`] on `clock`.
    pub fn open(clock: &dyn Clock) -> Result<Self> {
        let ttl = u64::try_from(QR_TTL.as_nanos()).unwrap_or(u64::MAX);
        Ok(Self {
            psk: Psk::random()?,
            expires_cont_ns: clock.now_cont_ns().saturating_add(ttl),
            burned: false,
            failures: 0,
        })
    }

    /// The PSK to put in the QR (and to resolve an unknown initiator to).
    pub fn psk(&self) -> &Psk {
        &self.psk
    }

    fn expired(&self, clock: &dyn Clock) -> bool {
        clock.now_cont_ns() >= self.expires_cont_ns
    }

    /// Whether the PSK may still be used: not burned, not expired, not too
    /// many failures.
    pub fn usable(&self, clock: &dyn Clock) -> bool {
        !self.burned && self.failures < MAX_QR_FAILURES && !self.expired(clock)
    }

    /// Burns the PSK (on `PairHello`).
    pub fn burn(&mut self) {
        self.burned = true;
    }

    pub fn record_failure(&mut self) {
        self.failures += 1;
    }

    /// The QR must be replaced: it is spent, expired or attacked.
    pub fn needs_new_qr(&self, clock: &dyn Clock) -> bool {
        !self.usable(clock)
    }
}

/// The result of a pairing, for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paired {
    pub device_gid: String,
    pub name: String,
    /// The hub's listener port (the spoke keeps it with its addresses).
    pub port: u16,
}

fn bad(what: &str) -> SyncError {
    SyncError::Wire(what.to_string())
}

fn valid_gid(s: &str) -> bool {
    uuid::Uuid::parse_str(s).is_ok_and(|u| u.hyphenated().to_string() == s)
}

fn valid_peer(gid: &str, name: &str) -> bool {
    valid_gid(gid) && !name.is_empty() && name.len() <= MAX_NAME
}

/// Hub side, after the Noise handshake with the QR PSK: reads `PairHello`,
/// stages the pin, answers `PairAccept`, commits on `PairDone`.
pub fn accept_pairing(
    store: &dyn SyncStore,
    clock: &dyn Clock,
    identity: &Identity,
    window: &mut PairingWindow,
    transport: &mut dyn Transport,
    own_name: &str,
    port: u16,
) -> Result<Paired> {
    if !window.usable(clock) {
        return Err(bad("pairing window closed"));
    }
    transport.set_recv_timeout(Some(PAIR_HELLO_TIMEOUT))?;
    let (id, decoded) = recv_msg(transport)?;
    let Decoded::Known(Message::PairHello(hello)) = decoded else {
        return Err(bad("expected PairHello"));
    };
    // Single use: the QR PSK is spent by the first PairHello, valid or not.
    window.burn();
    if window.expired(clock) {
        return Err(bad("pairing window expired"));
    }
    let refuse = |t: &mut dyn Transport, code: ErrorCode| {
        let _ = send_msg(t, id, &error_msg(code, None));
    };
    let PairHello {
        device_gid,
        name,
        platform,
        proto,
    } = hello;
    if wire::negotiate(&crate::session::default_protos(), &proto).is_none() {
        refuse(transport, ErrorCode::UpgradeRequired);
        return Err(SyncError::Peer(ErrorCode::UpgradeRequired));
    }
    if !valid_peer(&device_gid, &name) || platform.len() > MAX_PLATFORM {
        refuse(transport, ErrorCode::BadRecord);
        return Err(bad("invalid PairHello"));
    }
    let existing = store.device(&device_gid)?;
    if existing
        .as_ref()
        .is_some_and(|d| d.state == DeviceState::WipePending)
    {
        // A wipe is still owed to that device: it can't pair again first.
        refuse(transport, ErrorCode::Busy);
        return Err(SyncError::Peer(ErrorCode::Busy));
    }
    // Staged in memory only: nothing is written until PairDone.
    let staged = NewDevice {
        gid: device_gid.clone(),
        name: name.clone(),
        platform,
        role: DeviceRole::Spoke,
        static_pub: transport.peer_static(),
    };
    let pair_psk = Psk::random()?;
    let mut accept = Message::PairAccept(PairAccept {
        device_gid: identity.device_gid.clone(),
        name: own_name.to_string(),
        pair_psk: Bytes(pair_psk.as_bytes().to_vec()),
        port,
    });
    let sent = send_msg(transport, id, &accept);
    if let Message::PairAccept(a) = &mut accept {
        a.pair_psk.0.zeroize();
    }
    sent?;
    transport.set_recv_timeout(Some(PAIR_DONE_TIMEOUT))?;
    let (_, decoded) = recv_msg(transport)?;
    if !matches!(decoded, Decoded::Known(Message::PairDone)) {
        return Err(bad("expected PairDone"));
    }
    if existing.is_some() {
        // The same device pairing again replaces its pin.
        match store.unpin_device(&device_gid) {
            Ok(()) | Err(StoreError::NotFound { .. }) => {}
            Err(e) => return Err(e.into()),
        }
    }
    store.pin_device(&staged, pair_psk.as_bytes())?;
    Ok(Paired {
        device_gid,
        name,
        port,
    })
}

/// Spoke side: sends `PairHello`, stores the pin and the pair PSK from
/// `PairAccept`, answers `PairDone`.
pub fn pair_with(
    store: &dyn SyncStore,
    identity: &Identity,
    transport: &mut dyn Transport,
    own_name: &str,
    platform: &str,
) -> Result<Paired> {
    // A spoke pairs with exactly one hub; a new one needs an unpair first.
    if store.devices()?.iter().any(|d| d.role == DeviceRole::Hub) {
        return Err(SyncError::Store(StoreError::Invalid(
            "already paired with a hub".into(),
        )));
    }
    transport.set_recv_timeout(Some(PAIR_HELLO_TIMEOUT))?;
    let mut rpc = Rpc::default();
    let reply = rpc.call(
        transport,
        &Message::PairHello(PairHello {
            device_gid: identity.device_gid.clone(),
            name: own_name.to_string(),
            platform: platform.to_string(),
            proto: crate::session::default_protos(),
        }),
    )?;
    let Message::PairAccept(mut accept) = reply else {
        return Err(bad("expected PairAccept"));
    };
    let psk: Result<[u8; 32]> = accept
        .pair_psk
        .0
        .as_slice()
        .try_into()
        .map_err(|_| bad("bad pair key"));
    accept.pair_psk.0.zeroize();
    let mut psk = psk?;
    if !valid_peer(&accept.device_gid, &accept.name) {
        psk.zeroize();
        return Err(bad("invalid PairAccept"));
    }
    let hub = NewDevice {
        gid: accept.device_gid.clone(),
        name: accept.name.clone(),
        platform: "desktop".to_string(),
        role: DeviceRole::Hub,
        static_pub: transport.peer_static(),
    };
    let pinned = store.pin_device(&hub, &psk);
    psk.zeroize();
    pinned?;
    if let Err(e) = send_msg(transport, 0, &Message::PairDone) {
        // The hub never heard: it keeps no pin, so neither do we.
        let _ = store.unpin_device(&accept.device_gid);
        return Err(e);
    }
    Ok(Paired {
        device_gid: accept.device_gid,
        name: accept.name,
        port: accept.port,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::*;
    use crate::clock::FakeClock;
    use crate::mem::MemDuplex;
    use crate::session::fake::FakeSyncStore;

    struct Rig {
        hub_id: Identity,
        phone_id: Identity,
        hub: Arc<FakeSyncStore>,
        phone: Arc<FakeSyncStore>,
        clock: Arc<FakeClock>,
    }

    fn copy_identity(i: &Identity) -> Identity {
        Identity {
            device_gid: i.device_gid.clone(),
            secret: crate::identity::StaticSecret::from_bytes(*i.secret.as_bytes()),
            public: i.public,
        }
    }

    fn rig() -> Rig {
        let (hub_id, phone_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        Rig {
            hub: Arc::new(FakeSyncStore::new(&hub_id.device_gid, true)),
            phone: Arc::new(FakeSyncStore::new(&phone_id.device_gid, false)),
            hub_id,
            phone_id,
            clock: Arc::new(FakeClock::new(1_700_000_000_000)),
        }
    }

    /// Runs both sides; the spoke's transport is returned to the caller's
    /// closure so a test can misbehave.
    fn run(r: &Rig, window: &mut PairingWindow) -> (Result<Paired>, Result<Paired>) {
        let (mut phone_t, mut hub_t) = MemDuplex::pair(r.phone_id.public, r.hub_id.public);
        let (phone, phone_id) = (r.phone.clone(), copy_identity(&r.phone_id));
        let t = thread::spawn(move || {
            pair_with(phone.as_ref(), &phone_id, &mut phone_t, "iPhone", "ios")
        });
        let hub = accept_pairing(
            r.hub.as_ref(),
            r.clock.as_ref(),
            &r.hub_id,
            window,
            &mut hub_t,
            "Mac",
            4455,
        );
        drop(hub_t);
        (t.join().unwrap(), hub)
    }

    #[test]
    fn a_pairing_pins_both_sides_with_the_same_psk() {
        use crate::store::SyncStore;
        let r = rig();
        let mut w = PairingWindow::open(r.clock.as_ref()).unwrap();
        let (phone, hub) = run(&r, &mut w);
        let (phone, hub) = (phone.unwrap(), hub.unwrap());
        assert_eq!(phone.device_gid, r.hub_id.device_gid);
        assert_eq!((phone.name.as_str(), phone.port), ("Mac", 4455));
        assert_eq!(hub.device_gid, r.phone_id.device_gid);
        assert_eq!(hub.name, "iPhone");
        let on_hub = r.hub.device(&r.phone_id.device_gid).unwrap().unwrap();
        let on_phone = r.phone.device(&r.hub_id.device_gid).unwrap().unwrap();
        assert_eq!(on_hub.role, DeviceRole::Spoke);
        assert_eq!(on_phone.role, DeviceRole::Hub);
        assert_eq!(on_hub.static_pub, r.phone_id.public);
        assert_eq!(on_phone.static_pub, r.hub_id.public);
        let (a, b) = (
            r.hub.pair_psk(&r.phone_id.device_gid).unwrap(),
            r.phone.pair_psk(&r.hub_id.device_gid).unwrap(),
        );
        assert_eq!(*a, *b);
        assert_ne!(*a, *w.psk().as_bytes(), "the pair PSK is not the QR PSK");
    }

    #[test]
    fn the_qr_psk_is_single_use() {
        let r = rig();
        let mut w = PairingWindow::open(r.clock.as_ref()).unwrap();
        run(&r, &mut w).1.unwrap();
        assert!(!w.usable(r.clock.as_ref()), "burned by the first PairHello");
        // A second phone using the same QR is turned away before any pin.
        let second = rig();
        let (phone, hub) = {
            let (mut p, mut h) = MemDuplex::pair(second.phone_id.public, r.hub_id.public);
            let phone = second.phone.clone();
            let id = copy_identity(&second.phone_id);
            let t = thread::spawn(move || pair_with(phone.as_ref(), &id, &mut p, "x", "ios"));
            let hub = accept_pairing(
                r.hub.as_ref(),
                r.clock.as_ref(),
                &r.hub_id,
                &mut w,
                &mut h,
                "Mac",
                1,
            );
            drop(h);
            (t.join().unwrap(), hub)
        };
        assert!(hub.is_err() && phone.is_err());
        assert_eq!(r.hub.device_ids().len(), 1);
        assert!(second.phone.device_ids().is_empty());
    }

    #[test]
    fn a_window_expires_after_120_seconds_and_after_three_failures() {
        let r = rig();
        let w = PairingWindow::open(r.clock.as_ref()).unwrap();
        r.clock.advance(Duration::from_secs(119));
        assert!(w.usable(r.clock.as_ref()));
        r.clock.advance(Duration::from_secs(2));
        assert!(!w.usable(r.clock.as_ref()), "121 s");
        assert!(w.needs_new_qr(r.clock.as_ref()));

        let mut w = PairingWindow::open(r.clock.as_ref()).unwrap();
        w.record_failure();
        w.record_failure();
        assert!(w.usable(r.clock.as_ref()));
        w.record_failure();
        assert!(!w.usable(r.clock.as_ref()), "a new QR after 3 failures");

        // An expired window refuses the pairing and pins nothing.
        let mut w = PairingWindow::open(r.clock.as_ref()).unwrap();
        r.clock.advance(Duration::from_secs(121));
        let (phone, hub) = run(&r, &mut w);
        assert!(hub.is_err() && phone.is_err());
        assert!(r.hub.device_ids().is_empty() && r.phone.device_ids().is_empty());
    }

    #[test]
    fn expiry_between_the_handshake_and_pair_hello_is_caught() {
        use crate::transport::Transport;
        let r = rig();
        let mut w = PairingWindow::open(r.clock.as_ref()).unwrap();
        let (mut phone_t, mut hub_t) = MemDuplex::pair(r.phone_id.public, r.hub_id.public);
        let (hub, clock, hub_id) = (r.hub.clone(), r.clock.clone(), copy_identity(&r.hub_id));
        // The hub is waiting for PairHello (the window was usable on entry).
        let t = thread::spawn(move || {
            accept_pairing(
                hub.as_ref(),
                clock.as_ref(),
                &hub_id,
                &mut w,
                &mut hub_t,
                "Mac",
                1,
            )
        });
        thread::sleep(Duration::from_millis(100));
        r.clock.advance(Duration::from_secs(121));
        let hello = wire::encode(
            1,
            &Message::PairHello(PairHello {
                device_gid: r.phone_id.device_gid.clone(),
                name: "iPhone".into(),
                platform: "ios".into(),
                proto: vec![wire::PROTO],
            }),
        )
        .unwrap();
        phone_t.send(&hello).unwrap();
        assert!(t.join().unwrap().is_err());
        // Refused before PairAccept: the phone gets nothing but the close.
        assert!(matches!(phone_t.recv(), Err(SyncError::Closed)));
        assert!(r.hub.device_ids().is_empty());
    }

    #[test]
    fn a_crash_before_pair_done_leaves_no_pin() {
        use crate::transport::Transport;
        let r = rig();
        let mut w = PairingWindow::open(r.clock.as_ref()).unwrap();
        let (mut phone_t, mut hub_t) = MemDuplex::pair(r.phone_id.public, r.hub_id.public);
        let gid = r.phone_id.device_gid.clone();
        let t = thread::spawn(move || {
            let mut rpc = Rpc::default();
            let reply = rpc
                .call(
                    &mut phone_t,
                    &Message::PairHello(PairHello {
                        device_gid: gid,
                        name: "iPhone".into(),
                        platform: "ios".into(),
                        proto: vec![wire::PROTO],
                    }),
                )
                .unwrap();
            assert!(matches!(reply, Message::PairAccept(_)));
            // The phone dies here: PairDone is never sent.
            drop(phone_t);
        });
        let res = accept_pairing(
            r.hub.as_ref(),
            r.clock.as_ref(),
            &r.hub_id,
            &mut w,
            &mut hub_t,
            "Mac",
            1,
        );
        t.join().unwrap();
        assert!(matches!(res, Err(SyncError::Closed)), "{res:?}");
        assert!(r.hub.device_ids().is_empty());
        assert!(!w.usable(r.clock.as_ref()), "the QR was spent anyway");
        let _ = hub_t.peer_static();
    }

    #[test]
    fn a_spoke_pairs_with_exactly_one_hub() {
        let r = rig();
        let mut w = PairingWindow::open(r.clock.as_ref()).unwrap();
        run(&r, &mut w).0.unwrap();
        // A second hub: refused before anything is sent.
        let other = Identity::generate().unwrap();
        let (mut p, _h) = MemDuplex::pair(r.phone_id.public, other.public);
        let id = copy_identity(&r.phone_id);
        let err = pair_with(r.phone.as_ref(), &id, &mut p, "iPhone", "ios").unwrap_err();
        assert!(
            matches!(err, SyncError::Store(StoreError::Invalid(_))),
            "{err}"
        );
        assert_eq!(r.phone.device_ids().len(), 1);
    }

    #[test]
    fn a_malformed_pair_hello_is_refused_without_a_pin() {
        use crate::transport::Transport;
        let r = rig();
        let mut w = PairingWindow::open(r.clock.as_ref()).unwrap();
        let (mut phone_t, mut hub_t) = MemDuplex::pair(r.phone_id.public, r.hub_id.public);
        let hello = wire::encode(
            1,
            &Message::PairHello(PairHello {
                device_gid: "not-a-uuid".into(),
                name: "iPhone".into(),
                platform: "ios".into(),
                proto: vec![wire::PROTO],
            }),
        )
        .unwrap();
        phone_t.send(&hello).unwrap();
        let res = accept_pairing(
            r.hub.as_ref(),
            r.clock.as_ref(),
            &r.hub_id,
            &mut w,
            &mut hub_t,
            "Mac",
            1,
        );
        assert!(res.is_err());
        let (_, m) = wire::decode(&phone_t.recv().unwrap()).unwrap();
        assert!(matches!(m, Message::Error(e) if e.code == ErrorCode::BadRecord));
        assert!(r.hub.device_ids().is_empty());
    }

    #[test]
    fn a_device_with_a_wipe_owed_cannot_pair_again() {
        use crate::store::SyncStore;
        let r = rig();
        let mut w = PairingWindow::open(r.clock.as_ref()).unwrap();
        run(&r, &mut w).1.unwrap();
        r.hub.set_wipe_pending(&r.phone_id.device_gid).unwrap();
        let mut w2 = PairingWindow::open(r.clock.as_ref()).unwrap();
        let fresh_phone = Arc::new(FakeSyncStore::new(&r.phone_id.device_gid, false));
        let (mut p, mut h) = MemDuplex::pair(r.phone_id.public, r.hub_id.public);
        let id = copy_identity(&r.phone_id);
        let t =
            thread::spawn(move || pair_with(fresh_phone.as_ref(), &id, &mut p, "iPhone", "ios"));
        let res = accept_pairing(
            r.hub.as_ref(),
            r.clock.as_ref(),
            &r.hub_id,
            &mut w2,
            &mut h,
            "Mac",
            1,
        );
        drop(h);
        assert!(matches!(res, Err(SyncError::Peer(ErrorCode::Busy))));
        assert!(t.join().unwrap().is_err());
    }
}
