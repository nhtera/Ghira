// SPDX-License-Identifier: Apache-2.0
//! Final-pass checkpoints (`final_pass_ckpt`): sealed under the meeting key,
//! tied to a stamp, gone with the audio, the meeting and a crypto-shred.

mod common;

use ghi_store::keys::KeyRing;
use ghi_store::store::{Store, TrackKind};
use ghi_store::{db, jobs::JobState};
use serde_json::json;

const SECRET: &[u8] = b"Ngan sach tuyet mat cua quy bon la ba ty dong";

fn with_audio(store: &Store, title: &str) -> String {
    let m = common::meeting(store, title);
    let mut w = store.open_track(&m, TrackKind::Mic).unwrap();
    w.append(b"pcm").unwrap();
    store.finish_track(&m, TrackKind::Mic, w).unwrap();
    m
}

fn raw_rows(dir: &std::path::Path, ring: &KeyRing) -> Vec<(String, Vec<u8>)> {
    let conn = db::open(&dir.join("ghira.db"), &ring.db_key()).unwrap();
    conn.prepare("SELECT part, data_ct FROM final_pass_ckpt ORDER BY part")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn parts_round_trip_and_another_stamp_drops_the_old_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = with_audio(&store, "A");
    let other = with_audio(&store, "B");
    store
        .put_pass_checkpoint(&m, "s1", "asr.0.0", SECRET)
        .unwrap();
    store
        .put_pass_checkpoint(&m, "s1", "asr.0.1", b"two")
        .unwrap();
    store
        .put_pass_checkpoint(&m, "s1", "asr.0.1", b"three")
        .unwrap();
    store
        .put_pass_checkpoint(&other, "s1", "asr.0.0", b"x")
        .unwrap();
    let got = store.pass_checkpoints(&m, "s1").unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got["asr.0.0"], SECRET);
    assert_eq!(got["asr.0.1"], b"three", "a part is replaced, not added");
    // The rows are sealed: the plaintext is nowhere in them.
    for (_, ct) in raw_rows(tmp.path(), &common::ring(&keys)) {
        assert!(!ct.windows(SECRET.len()).any(|w| w == SECRET));
    }
    // Reading with another stamp serves nothing and drops what was there.
    assert!(store.pass_checkpoints(&m, "s2").unwrap().is_empty());
    assert!(store.pass_checkpoints(&m, "s1").unwrap().is_empty());
    assert_eq!(store.pass_checkpoints(&other, "s1").unwrap().len(), 1);
    // Writing under a new stamp drops the old stamp's rows too.
    store
        .put_pass_checkpoint(&other, "s2", "asr.0.0", b"y")
        .unwrap();
    let got = store.pass_checkpoints(&other, "s2").unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got["asr.0.0"], b"y");
    // Clear.
    store
        .put_pass_checkpoint(&other, "s2", "diar", b"z")
        .unwrap();
    assert_eq!(store.clear_pass_checkpoints(&other).unwrap(), 2);
    assert!(store.pass_checkpoints(&other, "s2").unwrap().is_empty());
    // Names are tokens, not content.
    assert!(
        store
            .put_pass_checkpoint(&m, "s1", "free text", b"x")
            .is_err()
    );
}

#[test]
fn nothing_is_kept_without_audio_or_for_a_sensitive_meeting() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let bare = common::meeting(&store, "no audio");
    assert!(!store.put_pass_checkpoint(&bare, "s", "p", b"x").unwrap());
    assert!(store.pass_checkpoints(&bare, "s").unwrap().is_empty());
    let m = with_audio(&store, "S");
    assert!(store.put_pass_checkpoint(&m, "s", "p", b"x").unwrap());
    store.set_sensitive(&m, true).unwrap();
    assert!(!store.put_pass_checkpoint(&m, "s", "q", b"x").unwrap());
    assert_eq!(store.pass_checkpoints(&m, "s").unwrap().len(), 1, "only p");
    // Deleting the audio takes them.
    store.delete_audio(&m).unwrap();
    assert!(store.pass_checkpoints(&m, "s").unwrap().is_empty());
}

#[test]
fn a_crypto_shred_makes_them_unreadable_and_the_delete_removes_them() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = with_audio(&store, "Mật");
    let keep = with_audio(&store, "Giữ");
    store
        .put_pass_checkpoint(&m, "s", "asr.0.0", SECRET)
        .unwrap();
    store
        .put_pass_checkpoint(&keep, "s", "asr.0.0", b"keep")
        .unwrap();
    // The key goes first (a delete a crash interrupted): the rows are still
    // there but nothing opens them.
    store.shred_key(&m).unwrap();
    assert!(
        store
            .pass_checkpoints(&m, "s")
            .map_or(true, |p| p.is_empty())
    );
    assert_eq!(raw_rows(tmp.path(), &common::ring(&keys)).len(), 2);
    store.delete_meeting(&m).unwrap();
    let rows = raw_rows(tmp.path(), &common::ring(&keys));
    assert_eq!(rows.len(), 1, "only the other meeting's row is left");
    assert_eq!(
        store.pass_checkpoints(&keep, "s").unwrap()["asr.0.0"],
        b"keep"
    );
}

#[test]
fn claims_skip_held_jobs_and_a_refund_gives_the_attempt_back() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "J");
    let a = store.enqueue_job(Some(&m), "k", 1, &json!({})).unwrap();
    let b = store.enqueue_job(Some(&m), "k", 1, &json!({})).unwrap();
    assert_eq!(
        store
            .claim_next_job_except("k", 1, &[a])
            .unwrap()
            .unwrap()
            .id,
        b
    );
    assert!(store.claim_next_job_except("k", 1, &[a]).unwrap().is_none());
    let j = store.claim_next_job_except("k", 1, &[]).unwrap().unwrap();
    assert_eq!((j.id, j.attempts), (a, 1));
    store.refund_job_attempt(a).unwrap();
    assert_eq!(store.job(a).unwrap().attempts, 0);
    store.refund_job_attempt(a).unwrap();
    assert_eq!(store.job(a).unwrap().attempts, 0, "never below zero");
    // Not running: nothing to refund.
    store.complete_job(a).unwrap();
    store.refund_job_attempt(a).unwrap();
    assert_eq!(store.job(a).unwrap().state, JobState::Done);
}
