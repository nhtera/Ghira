// SPDX-License-Identifier: Apache-2.0
//! Embedding rows: sealed under the meeting key, replaced on re-index, gone
//! with the meeting and unreadable after a crypto-shred.

mod common;

use ghi_store::StoreError;
use ghi_store::embeddings::EmbeddingChunk;
use ghi_store::store::Store;

const MODEL: &str = "test-embed";

fn chunk(i: u32, seed: f32) -> EmbeddingChunk {
    EmbeddingChunk {
        chunk: i,
        t0_ms: i64::from(i) * 60_000,
        t1_ms: i64::from(i + 1) * 60_000,
        vec: vec![seed, seed + 0.5, -seed, 1.0 / 3.0],
    }
}

/// A finished meeting with one transcript line.
fn ready_meeting(store: &Store, title: &str) -> String {
    let gid = common::meeting(store, title);
    store
        .add_segments(&gid, vec![common::seg(0, 1000, "Chốt ngân sách")])
        .unwrap();
    store.set_meeting_status(&gid, "ready").unwrap();
    gid
}

fn version(store: &Store, gid: &str) -> i64 {
    store.get_meeting(gid).unwrap().transcript_version
}

/// Rows of the embeddings table as stored: (gid-less) sealed blobs.
fn raw_blobs(dir: &std::path::Path, keys: &common::Keys) -> Vec<Vec<u8>> {
    let conn = ghi_store::db::open(&dir.join("ghira.db"), &common::db_key(keys)).unwrap();
    let mut stmt = conn.prepare("SELECT vec_ct FROM embeddings").unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

#[test]
fn round_trip_is_exact_and_ordered() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let gid = ready_meeting(&store, "A");
    let v = version(&store, &gid);
    let rows = vec![chunk(1, 0.25), chunk(0, 0.75)];
    store.put_embeddings(&gid, MODEL, v, rows).unwrap();
    let got = store.embeddings(&gid, MODEL).unwrap();
    assert_eq!(got, vec![chunk(0, 0.75), chunk(1, 0.25)]);
    assert!(store.embeddings(&gid, "other-model").unwrap().is_empty());

    let all = store.all_embeddings(MODEL).unwrap();
    assert_eq!(all.len(), 2);
    assert!(all.iter().all(|e| e.meeting_gid == gid));
    assert_eq!(all[1].vec, chunk(1, 0.25).vec);

    // Stored sealed: the raw column holds no plaintext floats.
    drop(store);
    let blobs = raw_blobs(tmp.path(), &keys);
    assert_eq!(blobs.len(), 2);
    let needle = 0.75f32.to_le_bytes();
    assert!(blobs.iter().all(|b| !b.windows(4).any(|w| w == needle)));
}

#[test]
fn reindexing_replaces_that_models_rows_only() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _) = common::open(tmp.path());
    let gid = ready_meeting(&store, "A");
    let v = version(&store, &gid);
    store
        .put_embeddings(
            &gid,
            MODEL,
            v,
            vec![chunk(0, 1.0), chunk(1, 2.0), chunk(2, 3.0)],
        )
        .unwrap();
    store
        .put_embeddings(&gid, "other", v, vec![chunk(0, 9.0)])
        .unwrap();
    store
        .put_embeddings(&gid, MODEL, v, vec![chunk(0, 4.0)])
        .unwrap();
    assert_eq!(store.embeddings(&gid, MODEL).unwrap(), vec![chunk(0, 4.0)]);
    assert_eq!(
        store.embeddings(&gid, "other").unwrap(),
        vec![chunk(0, 9.0)]
    );
    assert!(
        store
            .put_embeddings(
                &gid,
                MODEL,
                v,
                vec![
                    chunk(0, 1.0),
                    EmbeddingChunk {
                        vec: vec![1.0],
                        ..chunk(1, 1.0)
                    }
                ]
            )
            .is_err(),
        "mixed sizes are refused, and nothing changed"
    );
    assert_eq!(store.embeddings(&gid, MODEL).unwrap(), vec![chunk(0, 4.0)]);
}

#[test]
fn needing_embeddings_follows_version_and_status() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _) = common::open(tmp.path());
    let ready = ready_meeting(&store, "ready");
    // Still recording, and a meeting with no transcript: not indexable yet.
    let recording = common::meeting(&store, "recording");
    store
        .add_segments(&recording, vec![common::seg(0, 1000, "x")])
        .unwrap();
    let empty = common::meeting(&store, "empty");
    store.set_meeting_status(&empty, "ready").unwrap();

    let need = |s: &Store| s.meetings_needing_embeddings(MODEL, 10).unwrap();
    assert_eq!(need(&store), vec![ready.clone()]);
    assert!(
        store
            .meetings_needing_embeddings(MODEL, 0)
            .unwrap()
            .is_empty()
    );

    let v = version(&store, &ready);
    store
        .put_embeddings(&ready, MODEL, v, vec![chunk(0, 1.0)])
        .unwrap();
    assert!(need(&store).is_empty());
    assert_eq!(
        store
            .meetings_needing_embeddings("newer-model", 10)
            .unwrap(),
        vec![ready.clone()]
    );

    // A new transcript version makes the old rows stale: not served, re-indexed.
    store
        .replace_transcript(
            &ready,
            vec![ghi_store::store::NewSegment {
                ..common::seg(0, 2000, "Bản chính thức")
            }],
        )
        .unwrap();
    assert_eq!(need(&store), vec![ready.clone()]);
    assert!(store.embeddings(&ready, MODEL).unwrap().is_empty());
    assert!(store.all_embeddings(MODEL).unwrap().is_empty());
    let v2 = version(&store, &ready);
    assert!(v2 > v);
    store
        .put_embeddings(&ready, MODEL, v2, vec![chunk(0, 2.0)])
        .unwrap();
    assert!(need(&store).is_empty());
}

#[test]
fn gone_after_delete_meeting() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let keep = ready_meeting(&store, "keep");
    let gone = ready_meeting(&store, "gone");
    for g in [&keep, &gone] {
        let v = version(&store, g);
        store
            .put_embeddings(g, MODEL, v, vec![chunk(0, 1.0)])
            .unwrap();
    }
    store.delete_meeting(&gone).unwrap();
    let all = store.all_embeddings(MODEL).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].meeting_gid, keep);
    // The row itself is gone, not just unreadable.
    drop(store);
    assert_eq!(raw_blobs(tmp.path(), &keys).len(), 1);
}

#[test]
fn unreadable_after_the_key_is_shredded() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _) = common::open(tmp.path());
    let keep = ready_meeting(&store, "keep");
    let shred = ready_meeting(&store, "shred");
    for g in [&keep, &shred] {
        let v = version(&store, g);
        store
            .put_embeddings(g, MODEL, v, vec![chunk(0, 1.0)])
            .unwrap();
    }
    // The crash window of a delete: key destroyed, rows not yet removed.
    store.shred_key(&shred).unwrap();
    assert!(matches!(
        store.embeddings(&shred, MODEL),
        Err(StoreError::Decrypt)
    ));
    let all = store.all_embeddings(MODEL).unwrap();
    assert_eq!(all.len(), 1, "the shredded meeting is skipped");
    assert_eq!(all[0].meeting_gid, keep);
}

#[test]
fn discarding_the_end_of_a_meeting_drops_chunks_that_reach_into_it() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _) = common::open(tmp.path());
    let gid = ready_meeting(&store, "A");
    let v = version(&store, &gid);
    store
        .put_embeddings(
            &gid,
            MODEL,
            v,
            vec![chunk(0, 1.0), chunk(1, 2.0), chunk(2, 3.0)],
        )
        .unwrap();
    // Cut at 70 s: chunk 1 (60-120 s) and 2 reach into it.
    store
        .discard_after(&gid, 70_000, 130_000, &[], false)
        .unwrap();
    assert_eq!(store.embeddings(&gid, MODEL).unwrap(), vec![chunk(0, 1.0)]);
}
