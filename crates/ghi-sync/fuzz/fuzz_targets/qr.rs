// SPDX-License-Identifier: Apache-2.0
//! The pairing code parser: scanned text is hostile. Never a panic; what
//! parses has at most four addresses, all private with a port, and survives
//! an encode and parse round trip unchanged.
#![no_main]

use ghi_sync::qr;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((mode, rest)) = data.split_first() else {
        return;
    };
    // Mode 0: the bytes are the scanned text. Mode 1: they are the CBOR body
    // under a good prefix and base45, so the CBOR checks are reached at once.
    let text = if mode % 2 == 0 {
        match std::str::from_utf8(rest) {
            Ok(t) => t.to_string(),
            Err(_) => return,
        }
    } else {
        format!("{}{}", qr::QR_PREFIX, base45::encode(rest))
    };
    let Ok(payload) = qr::parse(&text) else {
        return;
    };
    assert!(payload.addrs.len() <= qr::MAX_QR_ADDRS);
    for a in &payload.addrs {
        assert!(ghi_net::is_lan(a.ip()), "a non-LAN address got through");
        assert_ne!(a.port(), 0);
    }
    let again = qr::encode(&payload).expect("a parsed payload encodes");
    assert_eq!(qr::parse(&again).expect("and parses again"), payload);
});
