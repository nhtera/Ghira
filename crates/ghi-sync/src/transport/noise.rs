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

use std::io;
use std::time::{Duration, Instant};

use super::{
    ByteStream, HANDSHAKE_TIMEOUT, MAX_FRAME, MAX_MESSAGE, REKEY_AFTER_BYTES, Transport,
    noise_params, prologue,
};
use crate::identity::{Psk, StaticSecret};
use crate::{Result, SyncError};

/// Where the PSK sits in `IKpsk2`.
const PSK_LOCATION: usize = 2;
/// The AEAD tag on every frame.
const TAG: usize = 16;
/// Most plaintext bytes of one frame, `flags` included.
const MAX_PLAINTEXT: usize = MAX_FRAME - TAG;
/// Handshake frames are 96 and 48 bytes; anything much bigger is not ours.
const MAX_HANDSHAKE_FRAME: usize = 256;
const FLAG_MORE: u8 = 1;

/// Picks the PSK for an initiator, given its static key (a responder's choice
/// after reading message 1).
pub trait PskResolver {
    /// `None` closes the connection without a reply.
    fn psk_for(&self, remote_static: &[u8; 32]) -> Option<Psk>;
}

fn noise_err(e: snow::Error) -> SyncError {
    // snow's errors name the step, never key material.
    SyncError::Noise(e.to_string())
}

/// A finished Noise session.
pub struct NoiseTransport<S: ByteStream> {
    stream: S,
    state: snow::TransportState,
    peer_static: [u8; 32],
    /// Bytes read from the stream, not yet a whole frame. Kept across a
    /// timeout so a slow peer loses nothing.
    inbound: Vec<u8>,
    /// Chunks of the message being reassembled (also kept across a timeout).
    partial: Vec<u8>,
    scratch: Vec<u8>,
    sent: u64,
    received: u64,
    rekey_after: u64,
    /// After an authentication or framing failure the nonces are lost: closed.
    broken: bool,
}

struct Deadline(Instant);

impl Deadline {
    fn new() -> Self {
        Deadline(Instant::now() + HANDSHAKE_TIMEOUT)
    }

    /// Arms the stream with what is left of the handshake budget.
    fn arm<S: ByteStream>(&self, stream: &mut S) -> Result<()> {
        let left = self.0.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(SyncError::Timeout);
        }
        stream.set_io_timeout(Some(left))?;
        Ok(())
    }
}

fn write_frame<S: ByteStream>(stream: &mut S, ct: &[u8]) -> Result<()> {
    debug_assert!(ct.len() <= MAX_FRAME);
    let mut frame = Vec::with_capacity(2 + ct.len());
    frame.extend_from_slice(&(ct.len() as u16).to_be_bytes());
    frame.extend_from_slice(ct);
    stream.write_all(&frame)?;
    stream.flush()?;
    Ok(())
}

fn read_handshake_frame<S: ByteStream>(stream: &mut S, deadline: &Deadline) -> Result<Vec<u8>> {
    deadline.arm(stream)?;
    let mut len = [0u8; 2];
    stream.read_exact(&mut len)?;
    let len = u16::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_HANDSHAKE_FRAME {
        return Err(SyncError::Noise("not a Ghira handshake".into()));
    }
    deadline.arm(stream)?;
    let mut ct = vec![0u8; len];
    stream.read_exact(&mut ct)?;
    Ok(ct)
}

fn builder_keys<'a>(local: &'a StaticSecret, prologue: &'a [u8]) -> Result<snow::Builder<'a>> {
    snow::Builder::new(noise_params()?)
        .local_private_key(local.as_bytes())
        .map_err(noise_err)?
        .prologue(prologue)
        .map_err(noise_err)
}

impl<S: ByteStream> NoiseTransport<S> {
    /// Initiator (the phone): we know the responder's static key (from the QR
    /// or the pin) and the PSK. `offered_majors` goes into the prologue.
    pub fn initiate(
        mut stream: S,
        local: &StaticSecret,
        remote_static: &[u8; 32],
        psk: &Psk,
        offered_majors: &[u8],
    ) -> Result<Self> {
        let deadline = Deadline::new();
        let prologue = prologue(offered_majors);
        let mut hs = builder_keys(local, &prologue)?
            .remote_public_key(remote_static)
            .map_err(noise_err)?
            .psk(PSK_LOCATION as u8, psk.as_bytes())
            .map_err(noise_err)?
            .build_initiator()
            .map_err(noise_err)?;
        let mut buf = [0u8; MAX_HANDSHAKE_FRAME];
        let n = hs.write_message(&[], &mut buf).map_err(noise_err)?;
        deadline.arm(&mut stream)?;
        write_frame(&mut stream, &buf[..n])?;
        // A responder that does not know us closes without a word: `Closed`.
        let msg2 = read_handshake_frame(&mut stream, &deadline)?;
        hs.read_message(&msg2, &mut buf).map_err(noise_err)?;
        Self::finish(stream, hs, *remote_static)
    }

    /// Responder (the hub).
    pub fn respond(
        mut stream: S,
        local: &StaticSecret,
        resolver: &dyn PskResolver,
        offered_majors: &[u8],
    ) -> Result<Self> {
        let deadline = Deadline::new();
        let prologue = prologue(offered_majors);
        let mut hs = builder_keys(local, &prologue)?
            .build_responder()
            .map_err(noise_err)?;
        let msg1 = read_handshake_frame(&mut stream, &deadline)?;
        let mut buf = [0u8; MAX_HANDSHAKE_FRAME];
        hs.read_message(&msg1, &mut buf).map_err(noise_err)?;
        let remote: [u8; 32] = hs
            .get_remote_static()
            .and_then(|k| <[u8; 32]>::try_from(k).ok())
            .ok_or_else(|| SyncError::Noise("no static key in the handshake".into()))?;
        // Unknown device and no open pairing window: close without a reply.
        let psk = resolver
            .psk_for(&remote)
            .ok_or_else(|| SyncError::Noise("unknown device".into()))?;
        hs.set_psk(PSK_LOCATION, psk.as_bytes())
            .map_err(noise_err)?;
        let n = hs.write_message(&[], &mut buf).map_err(noise_err)?;
        deadline.arm(&mut stream)?;
        write_frame(&mut stream, &buf[..n])?;
        Self::finish(stream, hs, remote)
    }

    fn finish(mut stream: S, hs: snow::HandshakeState, peer_static: [u8; 32]) -> Result<Self> {
        let state = hs.into_transport_mode().map_err(noise_err)?;
        stream.set_io_timeout(None)?;
        Ok(Self {
            stream,
            state,
            peer_static,
            inbound: Vec::new(),
            partial: Vec::new(),
            scratch: vec![0u8; MAX_FRAME],
            sent: 0,
            received: 0,
            rekey_after: REKEY_AFTER_BYTES,
            broken: false,
        })
    }

    #[cfg(test)]
    fn set_rekey_after(&mut self, bytes: u64) {
        self.rekey_after = bytes;
    }

    fn send_frame(&mut self, more: bool, chunk: &[u8]) -> Result<()> {
        let mut plain = Vec::with_capacity(1 + chunk.len());
        plain.push(if more { FLAG_MORE } else { 0 });
        plain.extend_from_slice(chunk);
        let n = self
            .state
            .write_message(&plain, &mut self.scratch)
            .map_err(noise_err)?;
        let result = write_frame(&mut self.stream, &self.scratch[..n]);
        if result.is_err() {
            self.broken = true;
        }
        result?;
        self.sent += n as u64;
        if self.sent >= self.rekey_after {
            self.state.rekey_outgoing();
            self.sent = 0;
        }
        Ok(())
    }

    /// The next whole frame's ciphertext.
    fn next_frame(&mut self) -> Result<Vec<u8>> {
        let mut tmp = [0u8; 16 * 1024];
        loop {
            if self.inbound.len() >= 2 {
                let len = u16::from_be_bytes([self.inbound[0], self.inbound[1]]) as usize;
                if len < TAG + 1 {
                    self.broken = true;
                    return Err(SyncError::Wire("frame too short".into()));
                }
                if self.inbound.len() >= 2 + len {
                    let ct = self.inbound[2..2 + len].to_vec();
                    self.inbound.drain(..2 + len);
                    return Ok(ct);
                }
            }
            match self.stream.read(&mut tmp) {
                Ok(0) => {
                    self.broken = true;
                    return Err(SyncError::Closed);
                }
                Ok(n) => self.inbound.extend_from_slice(&tmp[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    let err = SyncError::from(e);
                    // A timeout loses nothing; anything else ends the session.
                    self.broken = !matches!(err, SyncError::Timeout);
                    return Err(err);
                }
            }
        }
    }
}

impl<S: ByteStream> Transport for NoiseTransport<S> {
    fn send(&mut self, msg: &[u8]) -> Result<()> {
        if self.broken {
            return Err(SyncError::Closed);
        }
        if msg.len() > MAX_MESSAGE {
            return Err(SyncError::Wire("message too large".into()));
        }
        let chunks: Vec<&[u8]> = if msg.is_empty() {
            vec![msg]
        } else {
            msg.chunks(MAX_PLAINTEXT - 1).collect()
        };
        let last = chunks.len() - 1;
        for (i, chunk) in chunks.into_iter().enumerate() {
            self.send_frame(i != last, chunk)?;
        }
        Ok(())
    }

    fn recv(&mut self) -> Result<Vec<u8>> {
        if self.broken {
            return Err(SyncError::Closed);
        }
        loop {
            let ct = self.next_frame()?;
            let mut plain = vec![0u8; ct.len()];
            let n = match self.state.read_message(&ct, &mut plain) {
                Ok(n) => n,
                Err(_) => {
                    self.broken = true;
                    return Err(SyncError::Noise("message failed authentication".into()));
                }
            };
            self.received += ct.len() as u64;
            if self.received >= self.rekey_after {
                self.state.rekey_incoming();
                self.received = 0;
            }
            plain.truncate(n);
            let Some((&flags, chunk)) = plain.split_first() else {
                self.broken = true;
                return Err(SyncError::Wire("empty frame".into()));
            };
            if flags & !FLAG_MORE != 0 || self.partial.len() + chunk.len() > MAX_MESSAGE {
                self.broken = true;
                return Err(SyncError::Wire("bad frame".into()));
            }
            self.partial.extend_from_slice(chunk);
            if flags & FLAG_MORE == 0 {
                return Ok(std::mem::take(&mut self.partial));
            }
        }
    }

    fn peer_static(&self) -> [u8; 32] {
        self.peer_static
    }

    fn set_recv_timeout(&mut self, timeout: Option<Duration>) -> Result<()> {
        self.stream.set_io_timeout(timeout)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::thread;

    use super::*;
    use crate::identity::Identity;
    use crate::mem::{MemStream, mem_pipe};

    /// A stream that counts what it writes and can corrupt the next write.
    struct Probe {
        inner: MemStream,
        written: Arc<AtomicUsize>,
        flip_next: Arc<AtomicBool>,
    }

    struct Handles {
        written: Arc<AtomicUsize>,
        flip_next: Arc<AtomicBool>,
    }

    fn probe(inner: MemStream) -> (Probe, Handles) {
        let written = Arc::new(AtomicUsize::new(0));
        let flip_next = Arc::new(AtomicBool::new(false));
        (
            Probe {
                inner,
                written: written.clone(),
                flip_next: flip_next.clone(),
            },
            Handles { written, flip_next },
        )
    }

    impl Read for Probe {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.inner.read(buf)
        }
    }

    impl Write for Probe {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.written.fetch_add(data.len(), Ordering::SeqCst);
            if self.flip_next.swap(false, Ordering::SeqCst) && !data.is_empty() {
                let mut bad = data.to_vec();
                *bad.last_mut().unwrap() ^= 1;
                return self.inner.write(&bad);
            }
            self.inner.write(data)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()
        }
    }

    impl ByteStream for Probe {
        fn set_io_timeout(&mut self, t: Option<Duration>) -> io::Result<()> {
            self.inner.set_io_timeout(t)
        }
    }

    struct Pinned {
        key: [u8; 32],
        psk: [u8; 32],
    }

    impl PskResolver for Pinned {
        fn psk_for(&self, remote: &[u8; 32]) -> Option<Psk> {
            (remote == &self.key).then(|| Psk::from_bytes(self.psk))
        }
    }

    struct Setup {
        phone: Identity,
        hub: Identity,
        psk: [u8; 32],
    }

    fn setup() -> Setup {
        Setup {
            phone: Identity::generate().unwrap(),
            hub: Identity::generate().unwrap(),
            psk: [42; 32],
        }
    }

    type T = NoiseTransport<MemStream>;

    /// A connected hub and phone, handshaken on two threads.
    fn connect(s: &Setup) -> (T, T) {
        let (phone_end, hub_end) = mem_pipe();
        let (hub_secret, phone_pub) = (
            StaticSecret::from_bytes(*s.hub.secret.as_bytes()),
            s.phone.public,
        );
        let psk = s.psk;
        let hub = thread::spawn(move || {
            let r = Pinned {
                key: phone_pub,
                psk,
            };
            NoiseTransport::respond(hub_end, &hub_secret, &r, &[1]).unwrap()
        });
        let phone = NoiseTransport::initiate(
            phone_end,
            &s.phone.secret,
            &s.hub.public,
            &Psk::from_bytes(s.psk),
            &[1],
        )
        .unwrap();
        (phone, hub.join().unwrap())
    }

    fn pattern(n: usize, seed: u8) -> Vec<u8> {
        (0..n)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    #[test]
    fn pinned_pair_handshakes_and_both_learn_the_other_key() {
        let s = setup();
        let (phone, hub) = connect(&s);
        assert_eq!(phone.peer_static(), s.hub.public);
        assert_eq!(hub.peer_static(), s.phone.public);
    }

    #[test]
    fn messages_round_trip_including_empty_boundaries_and_4_mib() {
        let s = setup();
        let (mut phone, mut hub) = connect(&s);
        let sizes = [
            0,
            1,
            MAX_PLAINTEXT - 2,
            MAX_PLAINTEXT - 1,
            MAX_PLAINTEXT,
            2 * (MAX_PLAINTEXT - 1),
            MAX_MESSAGE,
        ];
        let t = thread::spawn(move || {
            for (i, n) in sizes.iter().enumerate() {
                assert_eq!(hub.recv().unwrap(), pattern(*n, i as u8), "size {n}");
                hub.send(&pattern(*n, 100 + i as u8)).unwrap();
            }
            hub
        });
        for (i, n) in sizes.iter().enumerate() {
            phone.send(&pattern(*n, i as u8)).unwrap();
            assert_eq!(
                phone.recv().unwrap(),
                pattern(*n, 100 + i as u8),
                "size {n}"
            );
        }
        t.join().unwrap();
    }

    #[test]
    fn oversize_messages_are_refused_both_ways() {
        let s = setup();
        let (mut phone, mut hub) = connect(&s);
        assert!(matches!(
            phone.send(&vec![0; MAX_MESSAGE + 1]),
            Err(SyncError::Wire(_))
        ));
        // A peer that never ends a message is cut off at 4 MiB.
        let chunk = vec![7u8; MAX_PLAINTEXT - 1];
        let t = thread::spawn(move || while phone.send_frame(true, &chunk).is_ok() {});
        assert!(matches!(hub.recv(), Err(SyncError::Wire(_))));
        assert!(matches!(hub.recv(), Err(SyncError::Closed)));
        drop(hub);
        t.join().unwrap();
    }

    #[test]
    fn unknown_key_gets_no_reply_at_all() {
        let s = setup();
        let (phone_end, hub_end) = mem_pipe();
        let (hub_end, hub_probe) = probe(hub_end);
        let stranger = Identity::generate().unwrap();
        let hub_secret = StaticSecret::from_bytes(*s.hub.secret.as_bytes());
        let hub = thread::spawn(move || {
            let r = Pinned {
                key: s.phone.public,
                psk: [42; 32],
            };
            NoiseTransport::respond(hub_end, &hub_secret, &r, &[1]).err()
        });
        let r = NoiseTransport::initiate(
            phone_end,
            &stranger.secret,
            &s.hub.public,
            &Psk::from_bytes(s.psk),
            &[1],
        );
        assert!(matches!(r, Err(SyncError::Closed)), "{:?}", r.err());
        assert!(matches!(hub.join().unwrap(), Some(SyncError::Noise(_))));
        assert_eq!(hub_probe.written.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn wrong_psk_fails_the_initiator_at_message_2() {
        let s = setup();
        let (phone_end, hub_end) = mem_pipe();
        let hub_secret = StaticSecret::from_bytes(*s.hub.secret.as_bytes());
        let phone_pub = s.phone.public;
        let hub = thread::spawn(move || {
            let r = Pinned {
                key: phone_pub,
                psk: [1; 32],
            };
            NoiseTransport::respond(hub_end, &hub_secret, &r, &[1])
        });
        let r = NoiseTransport::initiate(
            phone_end,
            &s.phone.secret,
            &s.hub.public,
            &Psk::from_bytes([2; 32]),
            &[1],
        );
        assert!(matches!(r, Err(SyncError::Noise(_))), "{:?}", r.err());
        // The responder only finds out at the first transport message, which
        // never comes: its next read ends.
        if let Ok(mut hub) = hub.join().unwrap() {
            assert!(matches!(hub.recv(), Err(SyncError::Closed)));
        }
    }

    #[test]
    fn prologue_mismatch_fails_at_message_1() {
        let s = setup();
        let (phone_end, hub_end) = mem_pipe();
        let (hub_end, hub_probe) = probe(hub_end);
        let hub_secret = StaticSecret::from_bytes(*s.hub.secret.as_bytes());
        let phone_pub = s.phone.public;
        let hub = thread::spawn(move || {
            let r = Pinned {
                key: phone_pub,
                psk: [42; 32],
            };
            NoiseTransport::respond(hub_end, &hub_secret, &r, &[1, 2]).err()
        });
        let r = NoiseTransport::initiate(
            phone_end,
            &s.phone.secret,
            &s.hub.public,
            &Psk::from_bytes(s.psk),
            &[1],
        );
        assert!(r.is_err());
        assert!(matches!(hub.join().unwrap(), Some(SyncError::Noise(_))));
        assert_eq!(hub_probe.written.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_flipped_ciphertext_byte_is_an_error_and_ends_the_session() {
        let s = setup();
        let (phone_end, hub_end) = mem_pipe();
        let (phone_end, phone_probe) = probe(phone_end);
        let hub_secret = StaticSecret::from_bytes(*s.hub.secret.as_bytes());
        let phone_pub = s.phone.public;
        let hub = thread::spawn(move || {
            let r = Pinned {
                key: phone_pub,
                psk: [42; 32],
            };
            NoiseTransport::respond(hub_end, &hub_secret, &r, &[1]).unwrap()
        });
        let mut phone = NoiseTransport::initiate(
            phone_end,
            &s.phone.secret,
            &s.hub.public,
            &Psk::from_bytes(s.psk),
            &[1],
        )
        .unwrap();
        let mut hub = hub.join().unwrap();
        phone.send(b"fine").unwrap();
        assert_eq!(hub.recv().unwrap(), b"fine");
        phone_probe.flip_next.store(true, Ordering::SeqCst);
        phone.send(b"tampered").unwrap();
        assert!(matches!(hub.recv(), Err(SyncError::Noise(_))));
        assert!(matches!(hub.recv(), Err(SyncError::Closed)));
        assert!(matches!(hub.send(b"x"), Err(SyncError::Closed)));
    }

    #[test]
    fn rekeying_keeps_both_sides_in_step_and_a_mismatch_breaks_them() {
        let s = setup();
        let (mut phone, mut hub) = connect(&s);
        phone.set_rekey_after(700);
        hub.set_rekey_after(700);
        for i in 0..40u8 {
            phone.send(&pattern(100, i)).unwrap();
            assert_eq!(hub.recv().unwrap(), pattern(100, i));
            hub.send(&pattern(50, i)).unwrap();
            assert_eq!(phone.recv().unwrap(), pattern(50, i));
        }
        // One side rekeying early is a different key: authentication fails.
        phone.set_rekey_after(1);
        phone.send(b"a").unwrap();
        phone.send(b"b").unwrap();
        assert_eq!(hub.recv().unwrap(), b"a");
        assert!(matches!(hub.recv(), Err(SyncError::Noise(_))));
    }

    #[test]
    fn recv_times_out_without_losing_anything() {
        let s = setup();
        let (mut phone, mut hub) = connect(&s);
        hub.set_recv_timeout(Some(Duration::from_millis(30)))
            .unwrap();
        assert!(matches!(hub.recv(), Err(SyncError::Timeout)));
        phone.send(&pattern(200_000, 3)).unwrap();
        assert_eq!(hub.recv().unwrap(), pattern(200_000, 3));
        drop(phone);
        assert!(matches!(hub.recv(), Err(SyncError::Closed)));
    }

    #[test]
    fn junk_and_silence_are_not_a_handshake() {
        let s = setup();
        let hub_secret = StaticSecret::from_bytes(*s.hub.secret.as_bytes());
        let resolver = Pinned {
            key: s.phone.public,
            psk: [42; 32],
        };
        let (mut peer, hub_end) = mem_pipe();
        peer.write_all(b"GET / HTTP/1.1\r\n\r\n").unwrap();
        let r = NoiseTransport::respond(hub_end, &hub_secret, &resolver, &[1]);
        assert!(matches!(r, Err(SyncError::Noise(_))));
        // A frame of the right length that is not message 1.
        let (mut peer, hub_end) = mem_pipe();
        let mut frame = vec![0, 96];
        frame.extend(pattern(96, 9));
        peer.write_all(&frame).unwrap();
        let r = NoiseTransport::respond(hub_end, &hub_secret, &resolver, &[1]);
        assert!(matches!(r, Err(SyncError::Noise(_))));
        // A peer that connects and says nothing is dropped when the peer goes.
        let (peer, hub_end) = mem_pipe();
        drop(peer);
        let r = NoiseTransport::respond(hub_end, &hub_secret, &resolver, &[1]);
        assert!(matches!(r, Err(SyncError::Closed)));
    }
}
