// SPDX-License-Identifier: Apache-2.0
//! The pairing window under load (doc 07 §3.4; review M1, M2): a pairing in
//! progress holds no lock, only a peer that holds the QR PSK can spend the
//! QR, and three failed pairings ask for a new one.

mod common;

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use common::{node, qr_of};
use ghi_sync::identity::{Identity, Psk};
use ghi_sync::mem::mem_pipe;
use ghi_sync::transport::{NoiseTransport, Transport};
use ghi_sync::wire::MAJORS;

/// A peer that holds the QR PSK and then says nothing.
fn staller(
    hub: &Arc<ghi_sync::service::HubNode>,
    qr: &ghi_sync::qr::QrPayload,
) -> (
    thread::JoinHandle<ghi_sync::Result<ghi_sync::service::Served>>,
    NoiseTransport<ghi_sync::mem::MemStream>,
) {
    let (a, b) = mem_pipe();
    let h = hub.clone();
    let server = thread::spawn(move || h.serve(a, None, None));
    let me = Identity::generate().unwrap();
    let t = NoiseTransport::initiate(b, &me.secret, &qr.pk, &qr.psk, &MAJORS).unwrap();
    (server, t)
}

#[test]
fn a_stalled_pairer_blocks_neither_a_paired_phone_nor_closing_the_window() {
    let (hub_n, phone) = (node(), node());
    let hub = hub_n.hub();
    common::pair(&hub, &hub_n, &phone);

    let qr = qr_of(&hub, &hub_n);
    let (server, stalled) = staller(&hub, &qr);
    // Give the hub time to be inside the pairing, waiting for PairHello.
    thread::sleep(Duration::from_millis(300));

    let started = Instant::now();
    let (a, b) = mem_pipe();
    let h = hub.clone();
    let session = thread::spawn(move || h.serve(a, None, None));
    ghi_sync::service::session_over(
        phone.dyn_store(),
        Arc::new(ghi_sync::clock::SystemClock),
        &phone.identity,
        b,
    )
    .unwrap();
    session.join().unwrap().unwrap();
    assert!(
        hub.pairing_open(),
        "the window is still shown while it runs"
    );
    hub.close_pairing();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "nothing waited for the stalled pairer: {:?}",
        started.elapsed()
    );
    assert!(!hub.pairing_open());

    drop(stalled);
    assert!(server.join().unwrap().is_err());
}

#[test]
fn a_second_peer_with_the_code_is_turned_away_while_one_pairs() {
    let (hub_n, _) = (node(), 0);
    let hub = hub_n.hub();
    let qr = qr_of(&hub, &hub_n);
    let (server, stalled) = staller(&hub, &qr);
    thread::sleep(Duration::from_millis(300));
    let (a, b) = mem_pipe();
    let h = hub.clone();
    let second = thread::spawn(move || h.serve(a, None, None));
    let me = Identity::generate().unwrap();
    // No reply for the second one: the QR PSK is not offered while busy.
    assert!(NoiseTransport::initiate(b, &me.secret, &qr.pk, &qr.psk, &MAJORS).is_err());
    assert!(second.join().unwrap().is_err());
    assert!(!hub.pairing_failed_out(), "turned away is not a failure");
    drop(stalled);
    assert!(server.join().unwrap().is_err());
}

#[test]
fn only_failed_pairings_by_a_holder_of_the_code_use_up_the_qr() {
    let (hub_n, _) = (node(), 0);
    let hub = hub_n.hub();
    let qr = qr_of(&hub, &hub_n);

    // Unknown keys and wrong PSKs, from anything on the LAN: not counted.
    for _ in 0..6 {
        let (a, b) = mem_pipe();
        let h = hub.clone();
        let server = thread::spawn(move || h.serve(a, None, None));
        let me = Identity::generate().unwrap();
        let wrong = Psk::random().unwrap();
        assert!(NoiseTransport::initiate(b, &me.secret, &qr.pk, &wrong, &MAJORS).is_err());
        assert!(server.join().unwrap().is_err());
    }
    assert!(hub.pairing_open());
    assert!(!hub.pairing_failed_out());

    // A phone with an old pin: handshake completes at the hub, then it hangs
    // up (it cannot verify the hub). Not counted either.
    for _ in 0..3 {
        let (server, stalled) = staller(&hub, &qr);
        drop(stalled);
        assert!(server.join().unwrap().is_err());
    }
    assert!(hub.pairing_open() && !hub.pairing_failed_out());

    // Three that prove they hold the PSK and then send nonsense: spent.
    for n in 1..=3 {
        let (server, mut junk) = staller(&hub, &qr);
        junk.send(b"not a PairHello").unwrap();
        drop(junk);
        assert!(server.join().unwrap().is_err());
        assert_eq!(hub.pairing_failed_out(), n == 3, "after {n}");
    }
    assert!(!hub.pairing_open());
}
