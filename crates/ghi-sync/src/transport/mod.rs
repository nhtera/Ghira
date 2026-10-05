// SPDX-License-Identifier: Apache-2.0
//! The seam between the protocol and the network (decision D3).
//!
//! [`Transport`] is an authenticated message channel. Everything above it
//! (messages, sessions, merge driving, leases, audio) is transport-agnostic and
//! tested over [`crate::mem::MemDuplex`]. [`noise::NoiseTransport`] implements
//! it over any [`ByteStream`]: a [`crate::mem::MemStream`] in tests,
//! [`ghi_net::lan::LanStream`] in production.

pub mod noise;

use std::io::{self, Read, Write};
use std::time::Duration;

use crate::{Result, SyncError};

pub use noise::{NoiseTransport, PskResolver};

/// The Noise pattern of every session.
pub const NOISE_PATTERN: &str = "Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s";
/// Largest app message (after reassembling chunks).
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;
/// Largest frame on the wire: `len u16 BE || noise ciphertext`.
pub const MAX_FRAME: usize = 65_535;
/// `snow` rekeys after this many bytes per direction.
pub const REKEY_AFTER_BYTES: u64 = 1 << 30;
/// Handshake time limit.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// An authenticated, ordered, message-oriented channel to one peer.
pub trait Transport: Send {
    /// Sends one message (at most [`MAX_MESSAGE`] bytes).
    fn send(&mut self, msg: &[u8]) -> Result<()>;
    /// Waits for the next message ([`SyncError::Closed`] when the peer is
    /// gone, [`SyncError::Timeout`] after the receive timeout).
    fn recv(&mut self) -> Result<Vec<u8>>;
    /// The peer's static public key, proven by the handshake.
    fn peer_static(&self) -> [u8; 32];
    /// Limits how long [`Transport::recv`] waits (`None` = forever); sessions
    /// use it for the 15 s ping and the 45 s silence limit.
    fn set_recv_timeout(&mut self, timeout: Option<Duration>) -> Result<()>;
}

/// A reliable ordered byte stream. Only [`ghi_net::lan::LanStream`] and the
/// in-memory pipe implement it: this crate opens no sockets.
pub trait ByteStream: Read + Write + Send {
    /// Limits how long a read or write may block (`None` = forever).
    fn set_io_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()>;
}

impl ByteStream for ghi_net::lan::LanStream {
    fn set_io_timeout(&mut self, timeout: Option<Duration>) -> io::Result<()> {
        self.set_read_timeout(timeout)?;
        self.set_write_timeout(timeout)
    }
}

/// The Noise prologue: `"ghira-sync" || 0x00 || offered_majors`. Both sides
/// must build the same one, so tampering with the offered majors breaks the
/// handshake (T6).
pub fn prologue(offered_majors: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(11 + offered_majors.len());
    p.extend_from_slice(b"ghira-sync");
    p.push(0);
    p.extend_from_slice(offered_majors);
    p
}

pub(crate) fn noise_params() -> Result<snow::params::NoiseParams> {
    NOISE_PATTERN
        .parse()
        .map_err(|e: snow::Error| SyncError::Noise(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prologue_binds_the_offered_majors() {
        assert_eq!(prologue(&[1]), b"ghira-sync\x00\x01");
        assert_ne!(prologue(&[1]), prologue(&[1, 2]));
    }

    #[test]
    fn the_pattern_parses() {
        assert!(noise_params().is_ok());
    }
}
