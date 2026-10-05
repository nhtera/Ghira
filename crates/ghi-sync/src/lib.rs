// SPDX-License-Identifier: Apache-2.0
//! LAN sync between a desktop (the hub) and phones (spokes), phase 15 (doc 07).
//!
//! Layers, bottom up:
//! - [`transport`]: [`transport::Transport`], an authenticated message channel
//!   (`send` / `recv` / `peer_static`), and its Noise implementation over any
//!   [`transport::ByteStream`]. Sockets are `ghi-net`'s ([`ghi_net::lan`]);
//!   [`mem`] has in-memory stand-ins so everything above runs in tests with no
//!   network.
//! - [`wire`]: the CBOR messages of doc 07 §6, record wrappers and
//!   [`wire::ErrorCode`]; [`qr`], [`identity`] and [`pair`]: pairing.
//! - [`session`]: the spoke state machine and the hub that answers it, over a
//!   [`SyncStore`] (the store operations a session needs; implemented for
//!   `ghi_store::store::Store` in [`store`]). [`lease`], [`audio`] and
//!   [`control`] are the parts of a session.
//! - [`clock`]: the sleep-inclusive clock leases use, with a fake for tests.
//! - [`export`]: the sealed-archive fallback.
//!
//! The merge engine is not here: it needs the store's internals and lives in
//! `ghi_store::sync`. Secrets ([`identity::Psk`], [`identity::StaticSecret`])
//! print `<redacted>` and zeroize on drop; nothing in this crate logs a QR
//! payload, a PSK or a key.
//!
//! Slice 15-A: every module and signature is here; bodies other than the
//! small ones noted in their docs return [`SyncError::NotYet`].

pub mod audio;
pub mod clock;
pub mod control;
pub mod export;
pub mod identity;
pub mod lease;
pub mod mem;
pub mod pair;
pub mod qr;
pub mod session;
pub mod store;
pub mod transport;
pub mod wire;

use std::fmt;
use std::io;

pub use store::SyncStore;

/// Crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Errors of sync. Messages never contain meeting content, keys or PSKs.
#[derive(Debug)]
pub enum SyncError {
    /// Not implemented yet (a W0 stub); the text names the function.
    NotYet(&'static str),
    Io(io::Error),
    /// The store refused or failed.
    Store(ghi_store::StoreError),
    /// Malformed bytes or CBOR from the peer.
    Wire(String),
    /// The Noise handshake or a transport message failed to authenticate.
    Noise(String),
    /// The peer sent an `Error` message, or we are about to.
    Peer(wire::ErrorCode),
    /// The peer, or the connection, is gone.
    Closed,
    Timeout,
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::NotYet(what) => write!(f, "not implemented yet: {what}"),
            SyncError::Io(e) => write!(f, "io: {e}"),
            SyncError::Store(e) => write!(f, "store: {e}"),
            SyncError::Wire(what) => write!(f, "bad message: {what}"),
            SyncError::Noise(what) => write!(f, "secure channel: {what}"),
            SyncError::Peer(code) => write!(f, "peer error: {code:?}"),
            SyncError::Closed => f.write_str("connection closed"),
            SyncError::Timeout => f.write_str("timed out"),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<io::Error> for SyncError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => SyncError::Timeout,
            io::ErrorKind::UnexpectedEof
            | io::ErrorKind::BrokenPipe
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted => SyncError::Closed,
            _ => SyncError::Io(e),
        }
    }
}

impl From<ghi_store::StoreError> for SyncError {
    fn from(e: ghi_store::StoreError) -> Self {
        SyncError::Store(e)
    }
}

pub type Result<T, E = SyncError> = std::result::Result<T, E>;

/// The error every W0 stub returns.
pub(crate) fn not_yet<T>(what: &'static str) -> Result<T> {
    Err(SyncError::NotYet(what))
}
