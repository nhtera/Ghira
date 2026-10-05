// SPDX-License-Identifier: Apache-2.0
//! Pairing (doc 07 §3.4; slice 15-F): `PairHello` / `PairAccept` / `PairDone`.
//!
//! The hub opens a [`PairingWindow`] (QR PSK, 120 s, single use, burned on
//! `PairHello`; a new QR after 3 failed handshakes). The pin is staged until
//! `PairDone` arrives (10 s) and only then committed, so a crash leaves no
//! half pin. The listener exists only while the window is open.

use std::time::Duration;

use crate::clock::Clock;
use crate::identity::{Identity, Psk};
use crate::store::SyncStore;
use crate::transport::Transport;
use crate::{Result, not_yet};

/// How long a QR PSK lives.
pub const QR_TTL: Duration = Duration::from_secs(120);
/// How long a staged pin waits for `PairDone`.
pub const PAIR_DONE_TIMEOUT: Duration = Duration::from_secs(10);
/// Failed handshakes with the QR PSK before a new QR is needed.
pub const MAX_QR_FAILURES: u32 = 3;

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
    pub fn open(_clock: &dyn Clock) -> Result<Self> {
        not_yet("pair::PairingWindow::open")
    }

    /// The PSK to put in the QR (and to resolve an unknown initiator to).
    pub fn psk(&self) -> &Psk {
        &self.psk
    }

    /// Whether the PSK may still be used: not burned, not expired, not too
    /// many failures.
    pub fn usable(&self, _clock: &dyn Clock) -> bool {
        !self.burned && self.failures < MAX_QR_FAILURES && self.expires_cont_ns > 0
    }

    /// Burns the PSK (on `PairHello`).
    pub fn burn(&mut self) {
        self.burned = true;
    }

    pub fn record_failure(&mut self) {
        self.failures += 1;
    }
}

/// The result of a pairing, for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paired {
    pub device_gid: String,
    pub name: String,
}

/// Hub side, after the Noise handshake with the QR PSK: reads `PairHello`,
/// stages the pin, answers `PairAccept`, commits on `PairDone`.
pub fn accept_pairing(
    _store: &dyn SyncStore,
    _identity: &Identity,
    _window: &mut PairingWindow,
    _transport: &mut dyn Transport,
    _own_name: &str,
    _port: u16,
) -> Result<Paired> {
    not_yet("pair::accept_pairing")
}

/// Spoke side: sends `PairHello`, stores the pin and the pair PSK from
/// `PairAccept`, answers `PairDone`.
pub fn pair_with(
    _store: &dyn SyncStore,
    _identity: &Identity,
    _transport: &mut dyn Transport,
    _own_name: &str,
    _platform: &str,
) -> Result<Paired> {
    not_yet("pair::pair_with")
}
