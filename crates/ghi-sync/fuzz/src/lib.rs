// SPDX-License-Identifier: Apache-2.0
//! Shared setup of the sync fuzz targets (15-L). Targets: `frame` (Noise
//! framing and handshake), `message` (wire decoder), `record` (typed records
//! through the merge engine), `qr` (pairing code parser), `track_pages` (audio
//! page verification).

use std::io::{Read, Write};
use std::sync::OnceLock;
use std::thread;

use ghi_sync::identity::{Identity, Psk, StaticSecret};
use ghi_sync::mem::{mem_pipe, MemStream};
use ghi_sync::transport::{NoiseTransport, PskResolver};
use ghi_sync::wire::MAJORS;

/// Fixed keys, so the runs are reproducible: `(hub, spoke, psk)`.
fn keys() -> &'static (Identity, Identity, [u8; 32]) {
    static KEYS: OnceLock<(Identity, Identity, [u8; 32])> = OnceLock::new();
    KEYS.get_or_init(|| {
        (
            Identity::generate().unwrap(),
            Identity::generate().unwrap(),
            [9; 32],
        )
    })
}

/// Answers every key with the same PSK (the responder under test never
/// refuses before it has read message 1).
pub struct Always(pub [u8; 32]);

impl PskResolver for Always {
    fn psk_for(&self, _: &[u8; 32]) -> Option<Psk> {
        Some(Psk::from_bytes(self.0))
    }
}

/// Responder over `stream` with the fixed hub key.
pub fn respond(stream: MemStream) -> ghi_sync::Result<NoiseTransport<MemStream>> {
    let (hub, _, psk) = keys();
    NoiseTransport::respond(
        stream,
        &StaticSecret::from_bytes(*hub.secret.as_bytes()),
        &Always(*psk),
        &MAJORS,
    )
}

fn read_frame(s: &mut MemStream) -> Vec<u8> {
    let mut len = [0u8; 2];
    s.read_exact(&mut len).unwrap();
    let mut frame = len.to_vec();
    frame.resize(2 + u16::from_be_bytes(len) as usize, 0);
    s.read_exact(&mut frame[2..]).unwrap();
    frame
}

/// A responder whose handshake with an honest initiator is done, and the
/// writing end of its stream: whatever is written there is what the responder
/// reads next, as raw frames.
pub fn established_responder() -> (NoiseTransport<MemStream>, MemStream) {
    let (hub, spoke, psk) = keys();
    let (inject, responder_end) = mem_pipe();
    let (initiator_end, mut relay_end) = mem_pipe();
    let (hub_pub, spoke_secret) = (
        hub.public,
        StaticSecret::from_bytes(*spoke.secret.as_bytes()),
    );
    let responder = thread::spawn(move || respond(responder_end).unwrap());
    let initiator = thread::spawn(move || {
        NoiseTransport::initiate(
            initiator_end,
            &spoke_secret,
            &hub_pub,
            &Psk::from_bytes(*psk),
            &MAJORS,
        )
        .unwrap()
    });
    // Carry the two handshake messages across by hand.
    let mut inject = inject;
    let msg1 = read_frame(&mut relay_end);
    inject.write_all(&msg1).unwrap();
    let msg2 = read_frame(&mut inject);
    relay_end.write_all(&msg2).unwrap();
    let _initiator = initiator.join().unwrap();
    (responder.join().unwrap(), inject)
}
