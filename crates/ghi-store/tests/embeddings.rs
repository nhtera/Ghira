// SPDX-License-Identifier: Apache-2.0
//! Embedding rows: sealed under the meeting key, replaced on re-index, gone
//! with the meeting and unreadable after a crypto-shred.

mod common;

use std::sync::Arc;

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
    store
        .put_embeddings(&gid, MODEL, v, store.index_gen(&gid).unwrap(), rows)
        .unwrap();
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
            store.index_gen(&gid).unwrap(),
            vec![chunk(0, 1.0), chunk(1, 2.0), chunk(2, 3.0)],
        )
        .unwrap();
    store
        .put_embeddings(
            &gid,
            "other",
            v,
            store.index_gen(&gid).unwrap(),
            vec![chunk(0, 9.0)],
        )
        .unwrap();
    store
        .put_embeddings(
            &gid,
            MODEL,
            v,
            store.index_gen(&gid).unwrap(),
            vec![chunk(0, 4.0)],
        )
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
                store.index_gen(&gid).unwrap(),
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
        .put_embeddings(
            &ready,
            MODEL,
            v,
            store.index_gen(&ready).unwrap(),
            vec![chunk(0, 1.0)],
        )
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
        .put_embeddings(
            &ready,
            MODEL,
            v2,
            store.index_gen(&ready).unwrap(),
            vec![chunk(0, 2.0)],
        )
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
            .put_embeddings(
                g,
                MODEL,
                v,
                store.index_gen(g).unwrap(),
                vec![chunk(0, 1.0)],
            )
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
            .put_embeddings(
                g,
                MODEL,
                v,
                store.index_gen(g).unwrap(),
                vec![chunk(0, 1.0)],
            )
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
            store.index_gen(&gid).unwrap(),
            vec![chunk(0, 1.0), chunk(1, 2.0), chunk(2, 3.0)],
        )
        .unwrap();
    // Cut at 70 s: chunk 1 (60-120 s) and 2 reach into it.
    store
        .discard_after(&gid, 70_000, 130_000, &[], false)
        .unwrap();
    assert_eq!(store.embeddings(&gid, MODEL).unwrap(), vec![chunk(0, 1.0)]);
}

// ---------------------------------------------------------- index generation

#[test]
fn vectors_built_before_a_change_are_refused_and_stale_rows_are_reindexed() {
    use ghi_store::store::NewSpeaker;
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let gid = ready_meeting(&store, "A");
    let v = version(&store, &gid);
    let sp = store.add_speaker(&gid, NewSpeaker::default()).unwrap();
    let g0 = store.index_gen(&gid).unwrap();
    store
        .put_embeddings(&gid, MODEL, v, g0, vec![chunk(0, 1.0)])
        .unwrap();
    assert!(
        store
            .meetings_needing_embeddings(MODEL, 10)
            .unwrap()
            .is_empty()
    );

    // Every change to what a chunk's text is built from bumps the generation,
    // makes the stored rows stale, and refuses an indexer that read before it.
    let seg = store.segments(&gid).unwrap()[0].gid.clone();
    type Change<'a> = (&'a str, Box<dyn Fn() + 'a>);
    let changes: Vec<Change> = vec![
        (
            "rename",
            Box::new(|| store.rename_speaker(&sp, Some("An")).unwrap()),
        ),
        (
            "edit text",
            Box::new(|| store.update_segment_text(&seg, "đã sửa").unwrap()),
        ),
        (
            "move line",
            Box::new(|| store.set_segment_speaker(&seg, Some(&sp)).unwrap()),
        ),
        (
            "not a person",
            Box::new(|| store.set_speaker_not_person(&sp, true).unwrap()),
        ),
        ("me", Box::new(|| store.set_speaker_me(&sp).unwrap())),
        (
            "split",
            Box::new(|| {
                store
                    .split_speaker(&sp, std::slice::from_ref(&seg), 1)
                    .unwrap();
            }),
        ),
    ];
    for (what, change) in changes {
        let before = store.index_gen(&gid).unwrap();
        store
            .put_embeddings(&gid, MODEL, v, before, vec![chunk(0, 1.0)])
            .unwrap();
        assert!(
            store
                .meetings_needing_embeddings(MODEL, 10)
                .unwrap()
                .is_empty(),
            "{what}"
        );
        change();
        assert!(store.index_gen(&gid).unwrap() > before, "{what}");
        assert_eq!(
            store.meetings_needing_embeddings(MODEL, 10).unwrap(),
            vec![gid.clone()],
            "{what}: stale rows are rebuilt"
        );
        assert!(
            matches!(
                store.put_embeddings(&gid, MODEL, v, before, vec![chunk(0, 2.0)]),
                Err(StoreError::IndexStale)
            ),
            "{what}"
        );
        // Nothing was stored by the refused call.
        assert_eq!(store.embeddings(&gid, MODEL).unwrap(), vec![chunk(0, 1.0)]);
    }
}

fn index_of(store: &Store) -> Vec<(String, u32, Vec<f32>)> {
    store
        .embedding_index(MODEL)
        .unwrap()
        .iter()
        .flat_map(|rows| {
            rows.iter()
                .map(|e| (e.meeting_gid.clone(), e.chunk, e.vec.clone()))
        })
        .collect()
}

#[test]
fn embedding_index_matches_all_embeddings_and_follows_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _) = common::open(tmp.path());
    let a = ready_meeting(&store, "A");
    let b = ready_meeting(&store, "B");
    for (gid, seed) in [(&a, 1.0), (&b, 2.0)] {
        let v = version(&store, gid);
        store
            .put_embeddings(
                gid,
                MODEL,
                v,
                store.index_gen(gid).unwrap(),
                vec![chunk(0, seed), chunk(1, seed + 0.1)],
            )
            .unwrap();
    }
    let expect = |store: &Store| -> Vec<(String, u32, Vec<f32>)> {
        store
            .all_embeddings(MODEL)
            .unwrap()
            .into_iter()
            .map(|e| (e.meeting_gid, e.chunk, e.vec))
            .collect()
    };
    assert_eq!(index_of(&store), expect(&store));
    // Second call is served from the cache and is the same.
    assert_eq!(index_of(&store), expect(&store));

    // A rebuild at the same version and generation is seen at once.
    let v = version(&store, &a);
    store
        .put_embeddings(
            &a,
            MODEL,
            v,
            store.index_gen(&a).unwrap(),
            vec![chunk(0, 7.0), chunk(1, 8.0)],
        )
        .unwrap();
    assert_eq!(index_of(&store), expect(&store));
    assert!(index_of(&store).iter().any(|(_, _, v)| v[0] == 7.0));

    // A transcript edit that stales the rows takes them out of the index.
    store
        .add_segments(&b, vec![common::seg(2000, 3000, "thêm một dòng")])
        .unwrap();
    assert_eq!(index_of(&store), expect(&store));

    // Deleting a meeting drops its vectors from the index.
    store.delete_meeting(&a).unwrap();
    assert_eq!(index_of(&store), expect(&store));
    assert!(index_of(&store).iter().all(|(g, _, _)| *g != a));
}

#[test]
fn embedding_index_forgets_a_shredded_meeting() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _) = common::open(tmp.path());
    let a = ready_meeting(&store, "A");
    let v = version(&store, &a);
    store
        .put_embeddings(
            &a,
            MODEL,
            v,
            store.index_gen(&a).unwrap(),
            vec![chunk(0, 1.0)],
        )
        .unwrap();
    assert_eq!(index_of(&store).len(), 1);
    store.shred_key(&a).unwrap();
    assert!(index_of(&store).is_empty());
}

fn put(store: &Store, gid: &str, model: &str, seed: f32) {
    let v = version(store, gid);
    store
        .put_embeddings(
            gid,
            model,
            v,
            store.index_gen(gid).unwrap(),
            vec![chunk(0, seed)],
        )
        .unwrap();
}

#[test]
fn embedding_index_is_served_from_the_cache_until_something_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _) = common::open(tmp.path());
    let a = ready_meeting(&store, "A");
    let b = ready_meeting(&store, "B");
    put(&store, &a, MODEL, 1.0);
    put(&store, &b, MODEL, 2.0);
    let first = store.embedding_index(MODEL).unwrap();
    let again = store.embedding_index(MODEL).unwrap();
    assert!(first.iter().zip(&again).all(|(x, y)| Arc::ptr_eq(x, y)));

    // Rebuilding one meeting reloads only that one.
    put(&store, &b, MODEL, 3.0);
    let after = store.embedding_index(MODEL).unwrap();
    assert!(Arc::ptr_eq(&first[0], &after[0]));
    assert!(!Arc::ptr_eq(&first[1], &after[1]));

    // A rename (the chunk text changed: index generation moves) reloads too.
    let sp = store
        .add_speaker(
            &a,
            ghi_store::store::NewSpeaker {
                label_idx: 0,
                ..Default::default()
            },
        )
        .unwrap();
    let before = store.embedding_index(MODEL).unwrap();
    store.rename_speaker(&sp, Some("Lan")).unwrap();
    let renamed = store.embedding_index(MODEL).unwrap();
    assert!(!Arc::ptr_eq(&before[0], &renamed[0]));
    assert!(Arc::ptr_eq(&before[1], &renamed[1]));
}

#[test]
fn embedding_index_keeps_only_the_model_asked_for() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _) = common::open(tmp.path());
    let a = ready_meeting(&store, "A");
    put(&store, &a, MODEL, 1.0);
    put(&store, &a, "other", 2.0);
    let first = store.embedding_index(MODEL).unwrap();
    store.embedding_index("other").unwrap();
    // The first model's entry went when the other model was asked for.
    let second = store.embedding_index(MODEL).unwrap();
    assert!(!Arc::ptr_eq(&first[0], &second[0]));
    assert_eq!(first[0][0].vec, second[0][0].vec);
}

#[test]
fn clearing_the_embedding_cache_drops_it_but_not_the_data() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _) = common::open(tmp.path());
    let a = ready_meeting(&store, "A");
    put(&store, &a, MODEL, 1.0);
    let first = store.embedding_index(MODEL).unwrap();
    store.clear_embedding_cache();
    let second = store.embedding_index(MODEL).unwrap();
    assert!(!Arc::ptr_eq(&first[0], &second[0]));
    // The query that held the old rows still reads them.
    assert_eq!(first[0][0].vec, second[0][0].vec);
}
