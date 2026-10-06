// SPDX-License-Identifier: Apache-2.0
//! Writes seed corpora for the sync fuzz targets from real data: records and
//! messages a store really produces, a pairing code, and the like. Run once
//! after changing the wire format:
//!
//! ```text
//! cargo +nightly run --manifest-path crates/ghi-sync/fuzz/Cargo.toml --bin seed -- crates/ghi-sync/fuzz/seeds
//! cd crates/ghi-sync && cargo +nightly fuzz run record fuzz/corpus/record fuzz/seeds/record -- -max_total_time=60
//! ```
//!
//! Give the seeds as the second corpus directory: libFuzzer writes new inputs
//! into the first one (`fuzz/corpus/<target>`, ignored by git), never into the
//! seeds. The first byte of a `message` / `qr` seed is the target's mode byte.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::{
    NewActionItem, NewMeeting, NewNoteBlock, NewSegment, NewSpeaker, Provenance, Store, TrackKind,
};
use ghi_sync::identity::Psk;
use ghi_sync::qr::{self, QrPayload};
use ghi_sync::wire::{self, Message, Proto, Record};

fn put(dir: &Path, name: &str, bytes: &[u8]) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), bytes).unwrap();
}

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).expect("the seeds directory"));
    let tmp = tempfile::tempdir().unwrap();
    let store = Store::open(
        tmp.path(),
        Arc::new(MemoryKeyStore::default()),
        Protection::default(),
    )
    .unwrap();

    // A meeting with everything a record kind can be.
    let m = store
        .create_meeting(NewMeeting {
            title: "Họp tuần".into(),
            ..Default::default()
        })
        .unwrap();
    let sp = store
        .add_speaker(
            &m.gid,
            NewSpeaker {
                label_idx: 0,
                display_name: Some("Linh".into()),
                ..Default::default()
            },
        )
        .unwrap();
    store
        .add_segment(
            &m.gid,
            NewSegment {
                speaker_gid: Some(sp),
                t0_ms: 0,
                t1_ms: 900,
                text: "Xin chào mọi người".into(),
                ..Default::default()
            },
        )
        .unwrap();
    store
        .add_note_block(
            &m.gid,
            NewNoteBlock {
                kind: "paragraph".into(),
                provenance: Provenance::User,
                body: "Ghi chú".into(),
                anchors: vec![],
                pinned: false,
            },
        )
        .unwrap();
    store
        .add_action_item(
            &m.gid,
            NewActionItem {
                text: "Gửi báo cáo".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let folder = store.create_folder("Work").unwrap();
    store
        .set_meeting_folder(std::slice::from_ref(&m.gid), Some(&folder.gid))
        .unwrap();
    let tag = store.create_tag("alpha").unwrap();
    store
        .tag_meetings(std::slice::from_ref(&m.gid), &tag.gid)
        .unwrap();
    let mut w = store.open_track(&m.gid, TrackKind::Mic).unwrap();
    w.append(&[7u8; 300]).unwrap();
    store.finish_track(&m.gid, TrackKind::Mic, w).unwrap();
    store.finish_meeting(&m.gid, 3_000).unwrap();
    let gone = store
        .create_meeting(NewMeeting {
            title: "Deleted".into(),
            ..Default::default()
        })
        .unwrap();
    store.delete_meeting(&gone.gid).unwrap();

    let batch = store.changes_since(0, 256).unwrap();
    let mut rows = Vec::new();
    for (i, c) in batch.changes.iter().enumerate() {
        let mut bytes = Vec::new();
        ciborium::into_writer(&c.record, &mut bytes).unwrap();
        put(
            &out.join("record"),
            &format!("{i:02}-{}", c.record.kind().log_kind()),
            &bytes,
        );
        rows.push(Record(c.record.clone()));
    }
    assert!(batch.changes.len() >= 8, "{} records", batch.changes.len());
    let tombs = store.tombs_since(0, 256).unwrap();
    assert!(!tombs.tombs.is_empty());

    // Messages, as `message` mode 0 (raw envelope) seeds.
    let proto = Proto { major: 1, minor: 0 };
    let messages: Vec<(&str, Message)> = vec![
        (
            "hello",
            Message::Hello(wire::Hello {
                proto: vec![proto],
                app_version: "0.1.0".into(),
                device_gid: store.sync_device_gid().unwrap(),
                feed_id: store.feed_id().unwrap(),
                pull_cursor: 0,
                caps: vec!["audio".into()],
            }),
        ),
        (
            "hello_ok",
            Message::HelloOk(wire::HelloOk {
                proto,
                device_gid: store.sync_device_gid().unwrap(),
                feed_id: store.feed_id().unwrap(),
                pending: vec![wire::Control::Unpair],
            }),
        ),
        (
            "push_rows",
            Message::PushRows(wire::PushRows {
                rows: rows.clone(),
                upto_seq: batch.upto_seq,
            }),
        ),
        (
            "push_tombs",
            Message::PushTombs(wire::PushTombs {
                tombs: tombs.tombs.clone(),
                upto_seq: tombs.upto_seq,
            }),
        ),
        (
            "rows",
            Message::Rows(wire::RowsBatch {
                rows,
                upto_seq: batch.upto_seq,
                more: false,
            }),
        ),
        (
            "process_request",
            Message::ProcessRequest(wire::ProcessRequest {
                job_uuid: "job-1".into(),
                meeting_gid: m.gid.clone(),
                epoch: 1,
                kinds: vec!["final_pass".into()],
                ttl_ms: 3_600_000,
            }),
        ),
        (
            "track_offer",
            Message::TrackOffer(wire::TrackOffer {
                track_gid: "t1".into(),
                meeting_gid: m.gid.clone(),
                header: wire::BundleHeader {
                    magic: ghi_store::sync::records::Bytes(b"GHB1".to_vec()),
                    version: 1,
                    prefix: ghi_store::sync::records::Bytes(vec![1; 19]),
                },
                pages: 3,
                bytes: 900,
                complete: true,
            }),
        ),
        (
            "track_pages",
            Message::TrackPages(wire::TrackPages {
                track_gid: "t1".into(),
                prefix: ghi_store::sync::records::Bytes(vec![1; 19]),
                first: 0,
                records: vec![ghi_store::sync::records::Bytes(vec![9; 64])],
            }),
        ),
        (
            "pull",
            Message::PullRows(wire::Pull {
                feed_id: store.feed_id().unwrap(),
                since_seq: 0,
                max: 256,
            }),
        ),
        (
            "error",
            Message::Error(wire::ErrorBody {
                code: wire::ErrorCode::StorageFull,
                detail: None,
            }),
        ),
        ("ping", Message::Ping),
        ("bye", Message::Bye),
    ];
    for (name, msg) in &messages {
        let mut seed = vec![0u8];
        seed.extend(wire::encode(1, msg).unwrap());
        put(&out.join("message"), name, &seed);
    }

    // A pairing code: raw text (mode 0) and its CBOR body (mode 1).
    let payload = QrPayload {
        v: qr::QR_VERSION,
        dev: [1; 16],
        pk: [2; 32],
        psk: Psk::from_bytes([3; 32]),
        addrs: vec!["192.168.1.5:7000".parse::<SocketAddr>().unwrap()],
    };
    let text = qr::encode(&payload).unwrap();
    let mut raw = vec![0u8];
    raw.extend(text.as_bytes());
    put(&out.join("qr"), "code", &raw);
    let body = base45::decode(text.strip_prefix(qr::QR_PREFIX).unwrap()).unwrap();
    let mut cbor = vec![1u8];
    cbor.extend(body);
    put(&out.join("qr"), "cbor", &cbor);
}
