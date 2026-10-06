// SPDX-License-Identifier: Apache-2.0
//! MITM (doc 07 §11; 15-L): a frame-aware relay between a real spoke and a
//! real hub, over memory pipes. A relay that only forwards must not break the
//! session and must never see content; every active attack must end with no
//! session and no change in either store.

mod common;

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use common::*;
use ghi_store::store::Store;
use ghi_sync::clock::SystemClock;
use ghi_sync::identity::{Identity, Psk};
use ghi_sync::mem::{MemStream, mem_pipe};
use ghi_sync::service::{HubNode, Served, pair_over, session_over};
use ghi_sync::transport::{ByteStream, NoiseTransport, PskResolver};
use ghi_sync::wire::MAJORS;

// ------------------------------------------------------------------- relay

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dir {
    ToHub,
    ToSpoke,
}

enum Act {
    Pass,
    /// Forward this body instead (same length: the length prefix stays valid).
    Replace(Vec<u8>),
}

type Seen = Arc<Mutex<Vec<(Dir, Vec<u8>)>>>;

struct Relay {
    seen: Seen,
    thread: JoinHandle<()>,
}

impl Relay {
    fn finish(self) -> Vec<(Dir, Vec<u8>)> {
        self.thread.join().unwrap();
        self.seen.lock().unwrap().clone()
    }
}

fn take_frame(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    if buf.len() < 2 {
        return None;
    }
    let len = u16::from_be_bytes([buf[0], buf[1]]) as usize;
    if buf.len() < 2 + len {
        return None;
    }
    let body = buf[2..2 + len].to_vec();
    buf.drain(..2 + len);
    Some(body)
}

/// Reads what is there on `from`, forwards whole frames to `to` (through
/// `tamper`), and says whether to go on.
fn pump(
    dir: Dir,
    from: &mut MemStream,
    to: &mut MemStream,
    buf: &mut Vec<u8>,
    n: &mut usize,
    seen: &Seen,
    tamper: &mut dyn FnMut(Dir, usize, &[u8]) -> Act,
) -> bool {
    let mut tmp = [0u8; 4096];
    match from.read(&mut tmp) {
        Ok(0) => return false,
        Ok(k) => buf.extend_from_slice(&tmp[..k]),
        Err(e) if e.kind() == io::ErrorKind::TimedOut => {}
        Err(_) => return false,
    }
    while let Some(body) = take_frame(buf) {
        seen.lock().unwrap().push((dir, body.clone()));
        let out = match tamper(dir, *n, &body) {
            Act::Pass => body,
            Act::Replace(b) => b,
        };
        *n += 1;
        let mut frame = (out.len() as u16).to_be_bytes().to_vec();
        frame.extend_from_slice(&out);
        if to.write_all(&frame).is_err() {
            return false;
        }
    }
    true
}

/// A relay between a spoke end and a hub end. `tamper` sees every frame body
/// with its direction and its index in that direction (0 is the first
/// handshake message, 1 the first transport message).
fn relay(
    mut tamper: impl FnMut(Dir, usize, &[u8]) -> Act + Send + 'static,
) -> (MemStream, MemStream, Relay) {
    let (spoke_end, mut from_spoke) = mem_pipe();
    let (mut from_hub, hub_end) = mem_pipe();
    let seen: Seen = Arc::default();
    let s2 = seen.clone();
    let thread = thread::spawn(move || {
        let tick = Some(Duration::from_millis(2));
        from_spoke.set_io_timeout(tick).unwrap();
        from_hub.set_io_timeout(tick).unwrap();
        let (mut bufs, mut idx) = ([Vec::new(), Vec::new()], [0usize, 0usize]);
        loop {
            let up = pump(
                Dir::ToHub,
                &mut from_spoke,
                &mut from_hub,
                &mut bufs[0],
                &mut idx[0],
                &s2,
                &mut tamper,
            );
            let down = pump(
                Dir::ToSpoke,
                &mut from_hub,
                &mut from_spoke,
                &mut bufs[1],
                &mut idx[1],
                &s2,
                &mut tamper,
            );
            if !(up && down) {
                break;
            }
        }
    });
    (spoke_end, hub_end, Relay { seen, thread })
}

/// What a store holds that a rejected attack must not move. `strict` also
/// covers the bookkeeping an authenticated `Hello` legitimately updates
/// (`last_seen`, the cursors); public keys are left out of the text.
fn snapshot_with(n: &Node, strict: bool) -> String {
    let s: &Store = n.store();
    let mut out = String::new();
    for d in s.devices().unwrap() {
        out.push_str(&format!(
            "{} {} {:?} {:?} {}\n",
            d.gid, d.name, d.role, d.state, d.paired_at
        ));
        if strict {
            out.push_str(&format!(
                "{:?} {:?} {} {}\n",
                d.last_seen, d.pull_feed_id, d.push_seq, d.pull_seq
            ));
        }
    }
    for m in s.list_meetings(1000, 0).unwrap() {
        out.push_str(&format!("{m:?} {:?}\n", s.segments(&m.gid).unwrap()));
    }
    out.push_str(&format!("{:?}\n", s.tombstones_since(0).unwrap()));
    out.push_str(&format!(
        "{}\n",
        s.changes_since(0, 10_000).unwrap().changes.len()
    ));
    out
}

fn snapshot(n: &Node) -> String {
    snapshot_with(n, true)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

struct Always(Psk);

impl PskResolver for Always {
    fn psk_for(&self, _: &[u8; 32]) -> Option<Psk> {
        Some(self.0.clone())
    }
}

/// Counts the bytes written through it.
struct Tap {
    inner: MemStream,
    wrote: Arc<AtomicUsize>,
}

impl Read for Tap {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.inner.read(b)
    }
}

impl Write for Tap {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.wrote.fetch_add(b.len(), Ordering::SeqCst);
        self.inner.write(b)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl ByteStream for Tap {
    fn set_io_timeout(&mut self, t: Option<Duration>) -> io::Result<()> {
        self.inner.set_io_timeout(t)
    }
}

const MARKER: &str = "MARKER-Zq93-quarterly-budget";
const LINE_MARKER: &str = "MARKER-Lm41-secret-line";

fn flip(body: &[u8], at: usize) -> Act {
    let mut b = body.to_vec();
    let i = at % b.len();
    b[i] ^= 0x01;
    Act::Replace(b)
}

/// A hub and a spoke already paired (directly), the spoke holding a meeting
/// the hub has not seen.
fn paired_pair() -> (Node, Node, Arc<HubNode>, String) {
    let (hub_n, spoke) = (node(), node());
    let hub = hub_n.hub();
    pair(&hub, &hub_n, &spoke);
    let gid = meeting(spoke.store(), MARKER);
    spoke
        .store()
        .add_segment(&gid, seg(LINE_MARKER, 9_000))
        .unwrap();
    (hub_n, spoke, hub, gid)
}

/// Runs a spoke session through a relay; both ends' results.
fn session_through(
    hub: &Arc<HubNode>,
    spoke: &Node,
    tamper: impl FnMut(Dir, usize, &[u8]) -> Act + Send + 'static,
) -> (Ran, Vec<(Dir, Vec<u8>)>) {
    let (spoke_end, hub_end, relay) = relay(tamper);
    let h = hub.clone();
    let server = thread::spawn(move || h.serve(hub_end, None, None));
    let mine = session_over(
        spoke.dyn_store(),
        Arc::new(SystemClock),
        &spoke.identity,
        spoke_end,
    );
    let theirs = server.join().unwrap();
    (
        Ran {
            spoke: mine,
            hub: theirs,
            spoke_frames: 0,
            hub_frames: 0,
        },
        relay.finish(),
    )
}

// ------------------------------------------------------------------- tests

#[test]
fn a_relay_that_only_forwards_pairs_and_syncs_and_never_sees_content() {
    let (hub_n, spoke) = (node(), node());
    let hub = hub_n.hub();

    // Pairing through the relay.
    let qr = qr_of(&hub, &hub_n);
    let (spoke_end, hub_end, pairing_relay) = relay(|_, _, _| Act::Pass);
    let h = hub.clone();
    let server = thread::spawn(move || h.serve(hub_end, None, None));
    let paired = pair_over(
        spoke.store().as_ref(),
        &spoke.identity,
        &qr,
        spoke_end,
        "iPhone",
        "ios",
    )
    .unwrap();
    assert_eq!(paired.device_gid, hub_n.gid());
    assert!(matches!(server.join().unwrap().unwrap(), Served::Paired(_)));
    let pairing_frames = pairing_relay.finish();

    // A session through the relay with content the relay would love to read.
    let gid = meeting(spoke.store(), MARKER);
    spoke
        .store()
        .add_segment(&gid, seg(LINE_MARKER, 9_000))
        .unwrap();
    record_track(spoke.store(), &gid, ghi_store::store::TrackKind::Mic, 4);
    let (ran, frames) = session_through(&hub, &spoke, |_, _, _| Act::Pass);
    let (mine, theirs) = ran.ok();
    assert!(mine.rows_pushed >= 5, "{mine:?}");
    assert_eq!(theirs.rows_pushed, mine.rows_pushed);
    assert_eq!(mine.tracks_sent, 1);

    // The hub really has it all...
    assert_eq!(hub_n.store().get_meeting(&gid).unwrap().title, MARKER);
    assert!(texts(hub_n.store(), &gid).contains(&LINE_MARKER.to_string()));
    assert_eq!(
        read(
            &spoke
                .store()
                .bundle_path(&gid, ghi_store::store::TrackKind::Mic)
                .unwrap()
        ),
        read(
            &hub_n
                .store()
                .bundle_path(&gid, ghi_store::store::TrackKind::Mic)
                .unwrap()
        )
    );
    // ...and the relay saw only ciphertext: neither marker, nor the Vietnamese
    // transcript, nor the note, in any frame of either exchange.
    let all: Vec<u8> = pairing_frames
        .iter()
        .chain(frames.iter())
        .flat_map(|(_, f)| f.clone())
        .collect();
    assert!(
        frames.iter().filter(|(d, _)| *d == Dir::ToHub).count() > 4
            && frames.iter().filter(|(d, _)| *d == Dir::ToSpoke).count() > 4,
        "the relay carried a real exchange"
    );
    assert!(all.len() > 3_000, "{} bytes seen", all.len());
    for secret in [
        MARKER,
        LINE_MARKER,
        "Xin chào mọi người",
        "Cảm ơn",
        "Ghi chú: ship it",
    ] {
        assert!(
            !contains(&all, secret.as_bytes()),
            "plaintext {secret:?} on the wire"
        );
    }
    // The finder finds what is there (the check above can fail).
    assert!(contains(
        b"..MARKER-Zq93-quarterly-budget..",
        MARKER.as_bytes()
    ));
}

#[test]
fn a_substituted_hub_key_never_gets_a_session_or_a_pin() {
    // Pairing: the attacker answers in the hub's place with its own static key
    // and even knows the QR secret.
    let (hub_n, spoke) = (node(), node());
    let hub = hub_n.hub();
    let qr = qr_of(&hub, &hub_n);
    let (spoke_end, attacker_end) = mem_pipe();
    let attacker = Identity::generate().unwrap();
    let psk = qr.psk.clone();
    let t = thread::spawn(move || {
        NoiseTransport::respond(attacker_end, &attacker.secret, &Always(psk), &MAJORS).map(|_| ())
    });
    let (before_hub, before_spoke) = (snapshot(&hub_n), snapshot(&spoke));
    let r = pair_over(
        spoke.store().as_ref(),
        &spoke.identity,
        &qr,
        spoke_end,
        "iPhone",
        "ios",
    );
    assert!(r.is_err(), "paired with an impostor");
    assert!(
        t.join().unwrap().is_err(),
        "the impostor got through the handshake"
    );
    assert!(spoke.store().devices().unwrap().is_empty());
    assert_eq!(
        (snapshot(&hub_n), snapshot(&spoke)),
        (before_hub, before_spoke)
    );

    // A paired spoke: the impostor holds neither the pinned key nor its PSK.
    let (hub_n, spoke) = (node(), node());
    let hub = hub_n.hub();
    pair(&hub, &hub_n, &spoke);
    let gid = meeting(spoke.store(), MARKER);
    let (spoke_end, attacker_end) = mem_pipe();
    let attacker = Identity::generate().unwrap();
    let t = thread::spawn(move || {
        NoiseTransport::respond(
            attacker_end,
            &attacker.secret,
            &Always(Psk::random().unwrap()),
            &MAJORS,
        )
        .map(|_| ())
    });
    let (before_hub, before_spoke) = (snapshot(&hub_n), snapshot(&spoke));
    let r = session_over(
        spoke.dyn_store(),
        Arc::new(SystemClock),
        &spoke.identity,
        spoke_end,
    );
    assert!(r.is_err(), "a session with an impostor");
    assert!(t.join().unwrap().is_err());
    assert_eq!(
        (snapshot(&hub_n), snapshot(&spoke)),
        (before_hub, before_spoke)
    );
    assert!(hub_n.store().get_meeting(&gid).is_err());
}

#[test]
fn a_wrong_psk_gives_no_pairing_and_burns_nothing_it_should_not() {
    let (hub_n, spoke) = (node(), node());
    let hub = hub_n.hub();
    let mut qr = qr_of(&hub, &hub_n);
    let mut bad = *qr.psk.as_bytes();
    bad[0] ^= 0xff;
    qr.psk = Psk::from_bytes(bad);
    let (before_hub, before_spoke) = (snapshot(&hub_n), snapshot(&spoke));
    let (spoke_end, hub_end) = mem_pipe();
    let h = hub.clone();
    let server = thread::spawn(move || h.serve(hub_end, None, None));
    let r = pair_over(
        spoke.store().as_ref(),
        &spoke.identity,
        &qr,
        spoke_end,
        "iPhone",
        "ios",
    );
    assert!(r.is_err(), "paired with the wrong PSK");
    assert!(server.join().unwrap().is_err());
    assert!(hub_n.store().devices().unwrap().is_empty());
    assert!(spoke.store().devices().unwrap().is_empty());
    assert_eq!(
        (snapshot(&hub_n), snapshot(&spoke)),
        (before_hub, before_spoke)
    );
}

#[test]
fn a_flipped_byte_in_message_1_message_2_or_the_first_transport_frames_ends_the_session() {
    // (direction, frame index): msg1 is ToHub 0, msg2 is ToSpoke 0, then the
    // first transport frames (Hello and its answer).
    for (dir, idx) in [
        (Dir::ToHub, 0),
        (Dir::ToSpoke, 0),
        (Dir::ToHub, 1),
        (Dir::ToSpoke, 1),
    ] {
        for at in [0, 7, 40] {
            let (hub_n, spoke, hub, gid) = paired_pair();
            // The hub's reply to a good Hello is the one frame after which
            // the hub has legitimately noted the peer (last seen, cursors).
            let strict = !(dir == Dir::ToSpoke && idx == 1);
            let snap = |n: &Node| snapshot_with(n, strict);
            let (before_hub, before_spoke) = (snap(&hub_n), snap(&spoke));
            let (ran, _) = session_through(&hub, &spoke, move |d, i, body| {
                if d == dir && i == idx {
                    flip(body, at)
                } else {
                    Act::Pass
                }
            });
            assert!(
                !matches!((&ran.spoke, &ran.hub), (Ok(_), Ok(Served::Session(_)))),
                "a session survived a flipped byte at {dir:?} frame {idx} offset {at}"
            );
            assert!(ran.spoke.is_err(), "{dir:?} {idx} {at}: the spoke went on");
            // A hub whose peer vanished before saying Hello may end quietly:
            // then it exchanged nothing.
            match &ran.hub {
                Err(_) => {}
                Ok(Served::Session(r)) => {
                    assert!(idle(r), "{dir:?} {idx} {at}: the hub went on: {r:?}")
                }
                Ok(other) => panic!("{dir:?} {idx} {at}: {other:?}"),
            }
            assert_eq!(
                (snap(&hub_n), snap(&spoke)),
                (before_hub, before_spoke),
                "{dir:?} frame {idx} offset {at} changed a store"
            );
            assert!(hub_n.store().get_meeting(&gid).is_err());
        }
    }
}

#[test]
fn a_flipped_byte_in_a_pairing_handshake_pins_nothing() {
    for dir in [Dir::ToHub, Dir::ToSpoke] {
        let (hub_n, spoke) = (node(), node());
        let hub = hub_n.hub();
        let qr = qr_of(&hub, &hub_n);
        let (before_hub, before_spoke) = (snapshot(&hub_n), snapshot(&spoke));
        let (spoke_end, hub_end, relay) = relay(move |d, i, body| {
            if d == dir && i == 0 {
                flip(body, 3)
            } else {
                Act::Pass
            }
        });
        let h = hub.clone();
        let server = thread::spawn(move || h.serve(hub_end, None, None));
        let r = pair_over(
            spoke.store().as_ref(),
            &spoke.identity,
            &qr,
            spoke_end,
            "iPhone",
            "ios",
        );
        assert!(r.is_err(), "paired through a flipped {dir:?} handshake");
        assert!(server.join().unwrap().is_err());
        relay.finish();
        assert_eq!(
            (snapshot(&hub_n), snapshot(&spoke)),
            (before_hub, before_spoke)
        );
    }
}

#[test]
fn a_flipped_byte_in_any_later_frame_ends_the_session_and_the_next_one_converges() {
    // How many frames a clean session of this shape has, each way.
    let (_, spoke, hub, _) = paired_pair();
    let (ran, frames) = session_through(&hub, &spoke, |_, _, _| Act::Pass);
    ran.ok();
    let count = |d: Dir| frames.iter().filter(|(x, _)| *x == d).count();
    let (up, down) = (count(Dir::ToHub), count(Dir::ToSpoke));
    assert!(up >= 6 && down >= 6, "a session has {up} + {down} frames");

    for (dir, n) in [(Dir::ToHub, up), (Dir::ToSpoke, down)] {
        for idx in 2..n {
            let (hub_n, spoke, hub, gid) = paired_pair();
            let (ran, _) = session_through(&hub, &spoke, move |d, i, body| {
                if d == dir && i == idx {
                    flip(body, 11)
                } else {
                    Act::Pass
                }
            });
            assert!(
                ran.spoke.is_err() || ran.hub.is_err(),
                "{dir:?} {idx}: tampering went unnoticed"
            );
            // Whatever the cut session managed to apply, clean runs finish the
            // job, and the data is exactly the spoke's.
            sync(&hub, &spoke);
            let (m2, t2) = sync(&hub, &spoke);
            assert!(idle(&m2) && idle(&t2), "{dir:?} {idx}: {m2:?} {t2:?}");
            assert_eq!(hub_n.store().get_meeting(&gid).unwrap().title, MARKER);
            assert_eq!(
                texts(hub_n.store(), &gid),
                texts(spoke.store(), &gid),
                "{dir:?} {idx}"
            );
        }
    }
}

#[test]
fn a_replayed_message_1_gets_no_session_and_changes_nothing() {
    let (hub_n, spoke, hub, _gid) = paired_pair();
    // Record a real session's message 1.
    let (ran, frames) = session_through(&hub, &spoke, |_, _, _| Act::Pass);
    ran.ok();
    let msg1 = frames
        .iter()
        .find(|(d, _)| *d == Dir::ToHub)
        .map(|(_, f)| f.clone())
        .unwrap();
    let before = snapshot(&hub_n);

    for _ in 0..2 {
        let (mut attacker, hub_end) = mem_pipe();
        let h = hub.clone();
        let server = thread::spawn(move || h.serve(hub_end, None, None));
        let mut frame = (msg1.len() as u16).to_be_bytes().to_vec();
        frame.extend_from_slice(&msg1);
        attacker.write_all(&frame).unwrap();
        // The hub answers message 2 (it cannot tell a replay from a retry)...
        attacker
            .set_io_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut head = [0u8; 2];
        let _ = attacker.read_exact(&mut head);
        // ...and then waits for a transport message the attacker cannot make.
        attacker
            .write_all(&[
                0, 20, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
            ])
            .unwrap();
        let served = server.join().unwrap();
        assert!(
            served.is_err(),
            "a replayed message 1 got a session: {served:?}"
        );
        assert_eq!(snapshot(&hub_n), before, "a replay moved the hub");
    }

    // The replay did not hurt the real spoke either: it syncs as before.
    let (m, t) = sync(&hub, &spoke);
    assert!(idle(&m) && idle(&t), "{m:?} {t:?}");
}

#[test]
fn a_replayed_pairing_message_1_does_not_pair_again_after_the_window_closed() {
    let (hub_n, spoke) = (node(), node());
    let hub = hub_n.hub();
    let qr = qr_of(&hub, &hub_n);
    let (spoke_end, hub_end, relay) = relay(|_, _, _| Act::Pass);
    let h = hub.clone();
    let server = thread::spawn(move || h.serve(hub_end, None, None));
    pair_over(
        spoke.store().as_ref(),
        &spoke.identity,
        &qr,
        spoke_end,
        "iPhone",
        "ios",
    )
    .unwrap();
    assert!(matches!(server.join().unwrap().unwrap(), Served::Paired(_)));
    let msg1 = relay
        .finish()
        .into_iter()
        .find(|(d, _)| *d == Dir::ToHub)
        .map(|(_, f)| f)
        .unwrap();
    assert!(!hub.pairing_open(), "the window closes with the pairing");
    let before = snapshot(&hub_n);

    let (mut attacker, hub_end) = mem_pipe();
    let h = hub.clone();
    let server = thread::spawn(move || h.serve(hub_end, None, None));
    let mut frame = (msg1.len() as u16).to_be_bytes().to_vec();
    frame.extend_from_slice(&msg1);
    attacker.write_all(&frame).unwrap();
    drop(attacker);
    let served = server.join().unwrap();
    assert!(served.is_err(), "{served:?}");
    assert_eq!(snapshot(&hub_n), before);
    assert_eq!(hub_n.store().devices().unwrap().len(), 1);
}

#[test]
fn an_unpinned_key_is_dropped_before_message_2() {
    // A device the hub never paired, with the hub's key and a guess at a PSK.
    let (hub_n, stranger) = (node(), node());
    let hub = hub_n.hub();
    let (a, b) = mem_pipe();
    let wrote = Arc::new(AtomicUsize::new(0));
    let tap = Tap {
        inner: b,
        wrote: wrote.clone(),
    };
    let h = hub.clone();
    let server = thread::spawn(move || h.serve(tap, None, None));
    let before = snapshot(&hub_n);
    let r = NoiseTransport::initiate(
        a,
        &stranger.identity.secret,
        &hub_n.identity.public,
        &Psk::random().unwrap(),
        &MAJORS,
    );
    assert!(r.is_err(), "an unknown key got a channel");
    assert!(server.join().unwrap().is_err());
    assert_eq!(
        wrote.load(Ordering::SeqCst),
        0,
        "the hub answered an unknown key"
    );
    assert_eq!(snapshot(&hub_n), before);
}

#[test]
fn a_revoked_key_is_dropped_before_message_2() {
    let (hub_n, spoke, hub, gid) = paired_pair();
    sync(&hub, &spoke);
    assert_eq!(hub_n.store().get_meeting(&gid).unwrap().title, MARKER);
    // The user unpairs the phone on the hub.
    hub_n.store().unpin_device(spoke.gid()).unwrap();
    let before = snapshot(&hub_n);

    let (a, b) = mem_pipe();
    let wrote = Arc::new(AtomicUsize::new(0));
    let tap = Tap {
        inner: b,
        wrote: wrote.clone(),
    };
    let h = hub.clone();
    let server = thread::spawn(move || h.serve(tap, None, None));
    // The phone still has its pin and tries again.
    let r = session_over(spoke.dyn_store(), Arc::new(SystemClock), &spoke.identity, a);
    assert!(r.is_err(), "a revoked device got a session");
    assert!(server.join().unwrap().is_err());
    assert_eq!(
        wrote.load(Ordering::SeqCst),
        0,
        "the hub answered a revoked key"
    );
    assert_eq!(snapshot(&hub_n), before);

    // New content from the revoked phone does not reach the hub.
    spoke
        .store()
        .set_meeting_title(&gid, "after revoke")
        .unwrap();
    assert_eq!(hub_n.store().get_meeting(&gid).unwrap().title, MARKER);
}
