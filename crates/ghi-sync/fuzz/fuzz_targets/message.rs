// SPDX-License-Identifier: Apache-2.0
//! The wire message decoder (`{ t, id, b }` CBOR): arbitrary bytes, and
//! arbitrary CBOR bodies under every message type. Never a panic; what
//! decodes keeps the caps of doc 07 §5.3 and re-encodes without a panic.
#![no_main]

use ciborium::value::Value;
use ghi_sync::wire::{self, Decoded, Message};
use libfuzzer_sys::fuzz_target;

fn check(bytes: &[u8]) {
    let Ok((id, Decoded::Known(msg))) = wire::decode_any(bytes) else {
        return;
    };
    match &msg {
        Message::PushRows(b) => assert!(b.rows.len() <= wire::MAX_BATCH_RECORDS),
        Message::Rows(b) => assert!(b.rows.len() <= wire::MAX_BATCH_RECORDS),
        Message::PushTombs(b) => assert!(b.tombs.len() <= wire::MAX_BATCH_RECORDS),
        Message::Tombs(b) => assert!(b.tombs.len() <= wire::MAX_BATCH_RECORDS),
        Message::TrackPages(b) => {
            assert!(b.records.len() <= wire::MAX_TRACK_PAGES);
            let bytes: usize = b.records.iter().map(|r| r.0.len()).sum();
            assert!(bytes <= wire::MAX_TRACK_PAGES_BYTES);
        }
        _ => {}
    }
    // Encoding what we decoded may refuse (a size cap), never panic.
    let _ = wire::encode(id, &msg);
}

fuzz_target!(|data: &[u8]| {
    let Some((mode, rest)) = data.split_first() else {
        return;
    };
    if mode % 2 == 0 {
        check(rest);
        return;
    }
    // A well-formed envelope around an arbitrary CBOR body, for the message
    // type in the next byte (so every body decoder is reached).
    let Some((t, body)) = rest.split_first() else {
        return;
    };
    let mut body = body;
    let Ok(body) = ciborium::from_reader::<Value, _>(&mut body) else {
        return;
    };
    let envelope = Value::Map(vec![
        (Value::Text("t".into()), Value::Integer((*t % 40).into())),
        (Value::Text("id".into()), Value::Integer(7u32.into())),
        (Value::Text("b".into()), body),
    ]);
    let mut out = Vec::new();
    if ciborium::into_writer(&envelope, &mut out).is_ok() {
        check(&out);
    }
});
