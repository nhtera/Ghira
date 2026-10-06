// SPDX-License-Identifier: Apache-2.0
//! Connections end to end (slice 15-T): what a hub does with an accepted
//! stream and what a spoke does to reach its hub. The Noise handshake, the PSK
//! choice, pairing and the session are all driven from here, over any
//! [`ByteStream`] (sockets in the apps and the CLI, a memory pipe in tests).
//!
//! The hub picks the PSK from the initiator's static key: a pinned device gets
//! its pair PSK; an unknown key gets the QR PSK only while the pairing window
//! is open; otherwise the connection closes without a reply.

use std::cell::Cell;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use ghi_net::lan::{self, Limits};
use ghi_store::sync::devices::{Device, DeviceRole};

use crate::clock::Clock;
use crate::identity::{Identity, Psk};
use crate::pair::{self, Paired, PairingWindow};
use crate::qr::QrPayload;
use crate::session::SessionReport;
use crate::session::hub::HubSession;
use crate::session::spoke::SpokeSession;
use crate::store::SyncStore;
use crate::transport::{ByteStream, NoiseTransport, PskResolver, Transport};
use crate::wire::MAJORS;
use crate::{Result, SyncError};

/// What a served connection turned out to be.
#[derive(Debug, Clone, PartialEq)]
pub enum Served {
    Paired(Paired),
    Session(SessionReport),
}

/// The hub's side of every connection.
pub struct HubNode {
    store: Arc<dyn SyncStore>,
    identity: Identity,
    clock: Arc<dyn Clock>,
    name: String,
    port: u16,
    window: Mutex<Option<PairingWindow>>,
}

struct Resolver<'a> {
    store: &'a dyn SyncStore,
    /// The QR PSK, while the window is usable.
    qr: Option<Psk>,
    used_qr: Cell<bool>,
}

impl PskResolver for Resolver<'_> {
    fn psk_for(&self, remote: &[u8; 32]) -> Option<Psk> {
        match self.store.device_by_key(remote) {
            Ok(Some(d)) => self
                .store
                .pair_psk(&d.gid)
                .ok()
                .map(|z| Psk::from_bytes(*z)),
            Ok(None) => {
                let psk = self.qr.clone()?;
                self.used_qr.set(true);
                Some(psk)
            }
            Err(_) => None,
        }
    }
}

impl HubNode {
    /// `port` is the listener's port (told to a pairing phone).
    pub fn new(
        store: Arc<dyn SyncStore>,
        identity: Identity,
        clock: Arc<dyn Clock>,
        name: &str,
        port: u16,
    ) -> Self {
        Self {
            store,
            identity,
            clock,
            name: name.to_string(),
            port,
            window: Mutex::new(None),
        }
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    fn window(&self) -> std::sync::MutexGuard<'_, Option<PairingWindow>> {
        self.window.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Opens a pairing window and returns its QR PSK (for the QR text).
    pub fn open_pairing(&self) -> Result<Psk> {
        let w = PairingWindow::open(self.clock.as_ref())?;
        let psk = w.psk().clone();
        *self.window() = Some(w);
        Ok(psk)
    }

    /// Opens a pairing window and returns the QR text for it: this hub's
    /// identity, the one-time PSK and up to [`crate::qr::MAX_QR_ADDRS`] of
    /// `addrs`. The text carries a secret: show it, never log it.
    pub fn open_pairing_code(&self, addrs: &[SocketAddr]) -> Result<String> {
        let psk = self.open_pairing()?;
        let dev = uuid::Uuid::parse_str(&self.identity.device_gid)
            .map_err(|_| SyncError::Wire("the device gid is not a UUID".into()))?;
        crate::qr::encode(&QrPayload {
            v: crate::qr::QR_VERSION,
            dev: *dev.as_bytes(),
            pk: self.identity.public,
            psk,
            addrs: addrs
                .iter()
                .copied()
                .take(crate::qr::MAX_QR_ADDRS)
                .collect(),
        })
    }

    /// Whether a pairing window is open and usable.
    pub fn pairing_open(&self) -> bool {
        self.window()
            .as_ref()
            .is_some_and(|w| w.usable(self.clock.as_ref()))
    }

    /// Closes the window (the QR is dropped).
    pub fn close_pairing(&self) {
        *self.window() = None;
    }

    /// Handshakes with an accepted stream and serves it: a pairing when the
    /// QR PSK was used, otherwise a session of the pinned device. A failed
    /// handshake is reported to `limits` for `peer_ip` (the abuse limits).
    pub fn serve<S: ByteStream>(
        &self,
        stream: S,
        peer_ip: Option<IpAddr>,
        limits: Option<&Limits>,
    ) -> Result<Served> {
        let qr = self
            .window()
            .as_ref()
            .filter(|w| w.acceptable(self.clock.as_ref()))
            .map(|w| w.psk().clone());
        let resolver = Resolver {
            store: self.store.as_ref(),
            qr,
            used_qr: Cell::new(false),
        };
        let mut t = NoiseTransport::respond(stream, &self.identity.secret, &resolver, &MAJORS)
            .inspect_err(|_| {
                if let (Some(l), Some(ip)) = (limits, peer_ip) {
                    l.report_handshake_failure(ip);
                }
            })?;
        if resolver.used_qr.get() {
            return self.pair_peer(&mut t, peer_ip, limits);
        }
        HubSession::new(Arc::clone(&self.store), Arc::clone(&self.clock), t)
            .serve()
            .map(Served::Session)
    }
}

impl HubNode {
    /// The pairing that follows a QR-PSK handshake. The window lock is held
    /// only to take a copy and to fold the outcome back: the network I/O
    /// (up to 20 s) runs without it, so sessions, `close_pairing` and
    /// `stop` never wait on a stalled pairer.
    fn pair_peer<S: ByteStream>(
        &self,
        t: &mut NoiseTransport<S>,
        peer_ip: Option<IpAddr>,
        limits: Option<&Limits>,
    ) -> Result<Served> {
        let mut run = {
            let mut guard = self.window();
            let Some(window) = guard.as_mut().filter(|w| w.usable(self.clock.as_ref())) else {
                return Err(SyncError::Wire("pairing window closed".into()));
            };
            let snapshot = window.clone();
            if !window.begin() {
                return Err(SyncError::Wire("a pairing is already running".into()));
            }
            snapshot
        };
        let paired = pair::accept_pairing(
            self.store.as_ref(),
            self.clock.as_ref(),
            &self.identity,
            &mut run,
            t,
            &self.name,
            self.port,
        );
        let mut guard = self.window();
        // The window may have been closed or replaced meanwhile: then the
        // outcome does not concern the new one.
        let same = guard.as_ref().is_some_and(|w| w.same_as(&run));
        match paired {
            Ok(p) => {
                if same {
                    *guard = None;
                }
                Ok(Served::Paired(p))
            }
            Err(e) => {
                if same && let Some(w) = guard.as_mut() {
                    w.finish(&run);
                }
                drop(guard);
                if let (Some(l), Some(ip)) = (limits, peer_ip) {
                    l.report_handshake_failure(ip);
                }
                Err(e)
            }
        }
    }

    /// The window was used up by failed pairings: a new QR is needed (the
    /// UI shows one). Not true for an expiry or a successful pairing.
    pub fn pairing_failed_out(&self) -> bool {
        self.window()
            .as_ref()
            .is_some_and(PairingWindow::failed_out)
    }
}

fn hub_gid_of(qr: &QrPayload) -> String {
    uuid::Uuid::from_bytes(qr.dev).to_string()
}

/// Spoke: pairs over an open stream to the hub named in the QR. The hub's
/// identity in the QR must match the one that answers.
pub fn pair_over<S: ByteStream>(
    store: &dyn SyncStore,
    identity: &Identity,
    qr: &QrPayload,
    stream: S,
    own_name: &str,
    platform: &str,
) -> Result<Paired> {
    let mut t = NoiseTransport::initiate(stream, &identity.secret, &qr.pk, &qr.psk, &MAJORS)?;
    let paired = pair::pair_with(store, identity, &mut t, own_name, platform)?;
    if paired.device_gid != hub_gid_of(qr) {
        let _ = store.unpin_device(&paired.device_gid);
        return Err(SyncError::Wire("the hub is not the one in the code".into()));
    }
    Ok(paired)
}

/// Spoke: pairs with the hub in the QR, trying each address in it. Returns the
/// pairing and the address that worked (kept as the hub's last address).
pub fn pair_via(
    store: &dyn SyncStore,
    identity: &Identity,
    qr: &QrPayload,
    own_name: &str,
    platform: &str,
) -> Result<(Paired, SocketAddr)> {
    let mut last = SyncError::Wire("the code has no address".into());
    for addr in &qr.addrs {
        let stream = match lan::connect(*addr, lan::CONNECT_TIMEOUT) {
            Ok(s) => s,
            Err(e) => {
                last = SyncError::Wire(format!("connect: {e}"));
                continue;
            }
        };
        match pair_over(store, identity, qr, stream, own_name, platform) {
            Ok(p) => {
                store.touch_device(&p.device_gid, Some(&addr.to_string()))?;
                return Ok((p, *addr));
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// The hub this spoke is paired with.
pub fn paired_hub(store: &dyn SyncStore) -> Result<Device> {
    store
        .devices()?
        .into_iter()
        .find(|d| d.role == DeviceRole::Hub)
        .ok_or_else(|| SyncError::Wire("not paired with a hub".into()))
}

/// Spoke: the Noise handshake with the paired hub over an open stream.
pub fn initiate_hub<S: ByteStream>(
    store: &dyn SyncStore,
    identity: &Identity,
    stream: S,
) -> Result<(NoiseTransport<S>, Device)> {
    let hub = paired_hub(store)?;
    let psk = Psk::from_bytes(*store.pair_psk(&hub.gid)?);
    let t = NoiseTransport::initiate(stream, &identity.secret, &hub.static_pub, &psk, &MAJORS)?;
    Ok((t, hub))
}

/// Spoke: one session (push, pull, audio) on an authenticated transport,
/// ended with `Bye`.
pub fn run_spoke<T: Transport>(
    store: Arc<dyn SyncStore>,
    clock: Arc<dyn Clock>,
    transport: T,
    hub: &Device,
) -> Result<SessionReport> {
    let mut s = SpokeSession::new(store, clock, transport, hub.gid.clone());
    let report = s.run_once();
    let _ = s.bye();
    report
}

/// Spoke: one session over an open stream.
pub fn session_over<S: ByteStream>(
    store: Arc<dyn SyncStore>,
    clock: Arc<dyn Clock>,
    identity: &Identity,
    stream: S,
) -> Result<SessionReport> {
    let (t, hub) = initiate_hub(store.as_ref(), identity, stream)?;
    run_spoke(store, clock, t, &hub)
}

/// Spoke: one session with the hub at the first of `candidates` that answers
/// (connect and handshake). The address that worked becomes the hub's last
/// address.
pub fn run_session(
    store: Arc<dyn SyncStore>,
    clock: Arc<dyn Clock>,
    identity: &Identity,
    candidates: &[SocketAddr],
) -> Result<(SessionReport, SocketAddr)> {
    let mut last = SyncError::Wire("no address to try".into());
    for addr in candidates {
        let stream = match lan::connect(*addr, lan::CONNECT_TIMEOUT) {
            Ok(s) => s,
            Err(e) => {
                last = SyncError::Wire(format!("connect: {e}"));
                continue;
            }
        };
        let (t, hub) = match initiate_hub(store.as_ref(), identity, stream) {
            Ok(x) => x,
            Err(e) => {
                last = e;
                continue;
            }
        };
        store.touch_device(&hub.gid, Some(&addr.to_string()))?;
        let report = run_spoke(Arc::clone(&store), clock, t, &hub)?;
        return Ok((report, *addr));
    }
    Err(last)
}
