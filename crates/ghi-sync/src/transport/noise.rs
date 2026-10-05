// SPDX-License-Identifier: Apache-2.0
//! `Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s` over a [`ByteStream`] (doc 07 §3.3,
//! §5.1; slice 15-E).
//!
//! Framing: `len u16 BE (<= 65 535) || noise ciphertext`; the plaintext is
//! `flags u8 (bit0 = more) || chunk`; a message is the chunks up to the first
//! without `more`, at most [`super::MAX_MESSAGE`]. Rekey every 2^30 bytes per
//! direction.
//!
//! Responder: read message 1, learn the initiator's static key with
//! `get_remote_static`, ask the [`PskResolver`] (pinned device -> its pair
//! PSK; unknown key while the pairing window is open -> the QR PSK; otherwise
//! `None`: close without a reply), then `set_psk(2, psk)` and answer.

use std::time::Duration;

use super::{ByteStream, Transport};
use crate::identity::{Psk, StaticSecret};
use crate::{Result, not_yet};

/// Picks the PSK for an initiator, given its static key (a responder's choice
/// after reading message 1).
pub trait PskResolver {
    /// `None` closes the connection without a reply.
    fn psk_for(&self, remote_static: &[u8; 32]) -> Option<Psk>;
}

/// A finished Noise session.
pub struct NoiseTransport<S: ByteStream> {
    _stream: S,
    peer_static: [u8; 32],
}

impl<S: ByteStream> NoiseTransport<S> {
    /// Initiator (the phone): we know the responder's static key (from the QR
    /// or the pin) and the PSK. `offered_majors` goes into the prologue.
    pub fn initiate(
        _stream: S,
        _local: &StaticSecret,
        _remote_static: &[u8; 32],
        _psk: &Psk,
        _offered_majors: &[u8],
    ) -> Result<Self> {
        not_yet("transport::noise::initiate")
    }

    /// Responder (the hub).
    pub fn respond(
        _stream: S,
        _local: &StaticSecret,
        _resolver: &dyn PskResolver,
        _offered_majors: &[u8],
    ) -> Result<Self> {
        not_yet("transport::noise::respond")
    }
}

impl<S: ByteStream> Transport for NoiseTransport<S> {
    fn send(&mut self, _msg: &[u8]) -> Result<()> {
        not_yet("transport::noise::send")
    }

    fn recv(&mut self) -> Result<Vec<u8>> {
        not_yet("transport::noise::recv")
    }

    fn peer_static(&self) -> [u8; 32] {
        self.peer_static
    }

    fn set_recv_timeout(&mut self, _timeout: Option<Duration>) -> Result<()> {
        not_yet("transport::noise::set_recv_timeout")
    }
}
