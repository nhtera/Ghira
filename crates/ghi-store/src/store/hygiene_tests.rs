// SPDX-License-Identifier: Apache-2.0
//! Phase 15-D: what sync relies on. Tombstoned gids stay dead on every insert
//! path, content order does not depend on rowids, monotone fields leave the
//! meeting's Lamport alone, and a result commit is fenced by its lease.

use std::sync::{Arc, Mutex};

use rusqlite::params;

use super::*;
use crate::forced_gids;
use crate::keys::{KeyRing, KeyStore, Protection};
use crate::voice::{ThirdPartyApproved, VoiceConsent, VoiceExemplar};

#[derive(Default)]
struct MemKeys(Mutex<Option<KeyRing>>);

impl KeyStore for MemKeys {
    fn load(&self) -> Result<Option<KeyRing>> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save(&self, ring: &KeyRing, _: Protection) -> Result<()> {
        *self.0.lock().unwrap() = Some(ring.clone());
        Ok(())
    }
    fn delete(&self) -> Result<()> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}

fn open() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(
        dir.path(),
        Arc::new(MemKeys::default()),
        Protection::default(),
    )
    .expect("open");
    (dir, store)
}

fn meeting(store: &Store) -> String {
    store
        .create_meeting(NewMeeting {
            title: "m".into(),
            started_at: 1_700_000_000_000,
            ..Default::default()
        })
        .unwrap()
        .gid
}

fn seg(gid: Option<&str>, t0: i64, text: &str) -> NewSegment {
    NewSegment {
        gid: gid.map(String::from),
        t0_ms: t0,
        t1_ms: t0 + 500,
        text: text.into(),
        ..Default::default()
    }
}

fn note(body: &str) -> NewNoteBlock {
    NewNoteBlock {
        kind: "paragraph".into(),
        provenance: Provenance::User,
        body: body.into(),
        anchors: vec![],
        pinned: false,
    }
}

fn action(text: &str) -> NewActionItem {
    NewActionItem {
        text: text.into(),
        ..Default::default()
    }
}

fn one<T: rusqlite::types::FromSql>(store: &Store, sql: &str, p: impl rusqlite::Params) -> T {
    store.conn().query_row(sql, p, |r| r.get(0)).unwrap()
}

fn kill(store: &Store, gid: &str) {
    store
        .conn()
        .execute(
            "INSERT INTO tombstones (gid, kind, lamport, deleted_at, cause) VALUES (?1, 'x', 1, 1, 'user')",
            [gid],
        )
        .unwrap();
}

/// Tombstones a fresh gid and queues it as the next one `new_gid` returns.
fn dead_gid(store: &Store) -> String {
    let gid = uuid::Uuid::now_v7().to_string();
    kill(store, &gid);
    forced_gids::clear();
    forced_gids::push(&gid);
    gid
}

fn refused<T: std::fmt::Debug>(r: Result<T>, gid: &str) {
    forced_gids::clear();
    match r {
        Err(StoreError::Tombstoned { gid: g }) => assert_eq!(g, gid),
        other => panic!("expected Tombstoned({gid}), got {other:?}"),
    }
}

// ------------------------------------------------------------ insert guards

#[test]
fn a_tombstoned_gid_is_refused_on_every_insert_path() {
    let (_d, s) = open();
    let m = meeting(&s);
    let sp = s
        .add_speaker(
            &m,
            NewSpeaker {
                label_idx: 1,
                ..Default::default()
            },
        )
        .unwrap();
    let seg_gid = s.add_segment(&m, seg(None, 0, "hello")).unwrap().gid;
    let person = s.add_person("Linh", 1).unwrap();
    let tag = s.create_tag("t").unwrap().gid;

    let g = dead_gid(&s);
    refused(
        s.create_meeting(NewMeeting {
            title: "x".into(),
            ..Default::default()
        }),
        &g,
    );
    assert_eq!(one::<i64>(&s, "SELECT count(*) FROM meetings", []), 1);

    let g = dead_gid(&s);
    refused(s.add_segment(&m, seg(None, 1000, "x")), &g);
    // A caller-chosen gid goes through the same check.
    let g = uuid::Uuid::now_v7().to_string();
    kill(&s, &g);
    refused(s.add_segment(&m, seg(Some(&g), 1000, "x")), &g);
    let g = uuid::Uuid::now_v7().to_string();
    kill(&s, &g);
    refused(s.replace_transcript(&m, vec![seg(Some(&g), 0, "new")]), &g);
    // ... and the failed replace changed nothing.
    assert_eq!(s.segments(&m).unwrap().len(), 1);
    assert_eq!(s.segments(&m).unwrap()[0].gid, seg_gid);

    let g = dead_gid(&s);
    refused(s.add_note_block(&m, note("n")), &g);
    let g = dead_gid(&s);
    refused(s.add_action_item(&m, action("a")), &g);
    let g = dead_gid(&s);
    refused(s.add_mark(&m, 5, MarkTag::Star), &g);
    let g = dead_gid(&s);
    refused(s.add_speaker(&m, NewSpeaker::default()), &g);
    let g = dead_gid(&s);
    refused(s.split_speaker(&sp, std::slice::from_ref(&seg_gid), 2), &g);
    let g = dead_gid(&s);
    refused(s.add_person("Other", 2), &g);
    // find_or_create_person, through renaming a speaker to a new name.
    let g = dead_gid(&s);
    refused(s.rename_speaker(&sp, Some("Brand New")), &g);
    let g = dead_gid(&s);
    refused(s.create_folder("f"), &g);
    let g = dead_gid(&s);
    refused(s.create_tag("t2"), &g);
    let g = dead_gid(&s);
    refused(s.tag_meetings(std::slice::from_ref(&m), &tag), &g);
    let g = dead_gid(&s);
    refused(s.open_track(&m, TrackKind::Mic).map(|_| ()), &g);

    let g = dead_gid(&s);
    refused(
        s.put_voice_profile(
            &person,
            &VoiceConsent {
                method: "self_checkbox".into(),
                at_ms: 1,
                text_key: "consent.self.body".into(),
                clip: None,
            },
            None,
            "campplus",
            vec![(
                "vi".into(),
                vec![VoiceExemplar {
                    vec: vec![0.1, 0.2],
                    source: None,
                }],
            )],
            Some(ThirdPartyApproved::assert_flag_checked()),
        ),
        &g,
    );
}

#[test]
fn a_deleted_row_stays_dead_whatever_its_lamport() {
    let (_d, s) = open();
    let m = meeting(&s);
    let gid = s.add_segment(&m, seg(None, 0, "one")).unwrap().gid;
    s.replace_transcript(&m, vec![seg(None, 0, "two")]).unwrap();
    assert!(s.is_tombstoned(&gid).unwrap());
    // A relayed copy with a Lamport far ahead of every tombstone.
    s.observe_lamport(1_000_000).unwrap();
    refused(s.add_segment(&m, seg(Some(&gid), 0, "one")), &gid);
    assert_eq!(
        one::<i64>(&s, "SELECT count(*) FROM segments WHERE gid = ?1", [&gid]),
        0
    );
}

// --------------------------------------------------------------- tombstones

fn cause(s: &Store, gid: &str) -> String {
    one(s, "SELECT cause FROM tombstones WHERE gid = ?1", [gid])
}

#[test]
fn every_deletion_records_why() {
    let (_d, s) = open();
    let m = meeting(&s);
    let n1 = s.add_note_block(&m, note("a")).unwrap().gid;
    s.delete_note_block(&n1).unwrap();
    assert_eq!(cause(&s, &n1), "user");

    let ai = s
        .add_note_block(
            &m,
            NewNoteBlock {
                provenance: Provenance::Ai,
                ..note("ai")
            },
        )
        .unwrap()
        .gid;
    s.replace_ai_notes(&m, vec![note("new")], vec![]).unwrap();
    assert_eq!(cause(&s, &ai), "regenerate");

    let old = s.add_segment(&m, seg(None, 0, "x")).unwrap().gid;
    s.replace_transcript(&m, vec![seg(None, 0, "y")]).unwrap();
    assert_eq!(cause(&s, &old), "transcript");

    let late = s.add_segment(&m, seg(None, 50_000, "late")).unwrap().gid;
    s.discard_after(&m, 40_000, 60_000, &[], false).unwrap();
    assert_eq!(cause(&s, &late), "discard");

    let f = s.create_folder("f").unwrap().gid;
    s.delete_folder(&f).unwrap();
    assert_eq!(cause(&s, &f), "user");

    let tr = s.open_track(&m, TrackKind::Mic).unwrap();
    let track_gid: String = one(&s, "SELECT gid FROM tracks", []);
    s.finish_track(&m, TrackKind::Mic, tr).unwrap();
    s.delete_audio(&m).unwrap();
    assert_eq!(cause(&s, &track_gid), "retention");

    let kept = s.add_segment(&m, seg(None, 0, "z")).unwrap().gid;
    s.delete_meeting(&m).unwrap();
    assert_eq!(cause(&s, &m), "meeting");
    assert_eq!(cause(&s, &kept), "meeting");
}

#[test]
fn a_local_delete_for_a_wipe_leaves_no_tombstones() {
    let (_d, s) = open();
    let m = meeting(&s);
    let seg_gid = s.add_segment(&m, seg(None, 0, "x")).unwrap().gid;
    s.delete_meeting_local(&m).unwrap();
    assert_eq!(one::<i64>(&s, "SELECT count(*) FROM tombstones", []), 0);
    assert!(!s.is_tombstoned(&seg_gid).unwrap());
    assert_eq!(one::<i64>(&s, "SELECT count(*) FROM meetings", []), 0);
}

// ----------------------------------------------------------------- ordering

#[test]
fn notes_and_actions_follow_ord_then_gid_not_rowid() {
    let (_d, s) = open();
    let m = meeting(&s);
    let n: Vec<String> = ["a", "b", "c"]
        .iter()
        .map(|b| s.add_note_block(&m, note(b)).unwrap().gid)
        .collect();
    let a: Vec<String> = ["a", "b", "c"]
        .iter()
        .map(|b| s.add_action_item(&m, action(b)).unwrap().gid)
        .collect();
    // New rows go after the last.
    let ords: Vec<String> = {
        let c = s.conn();
        let mut st = c
            .prepare("SELECT ord FROM notes_blocks ORDER BY id")
            .unwrap();
        st.query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert!(ords.windows(2).all(|w| w[0] < w[1]), "{ords:?}");

    // Rowid order (a, b, c) differs from ord order (c, a, b); b and a tie.
    {
        let c = s.conn();
        for (table, gids) in [("notes_blocks", &n), ("action_items", &a)] {
            for (gid, ord) in [(&gids[2], "0001"), (&gids[0], "0002"), (&gids[1], "0002")] {
                c.execute(
                    &format!("UPDATE {table} SET ord = ?1 WHERE gid = ?2"),
                    params![ord, gid],
                )
                .unwrap();
            }
        }
    }
    let mut tie = [n[0].clone(), n[1].clone()];
    tie.sort();
    let got: Vec<String> = s
        .note_blocks(&m)
        .unwrap()
        .into_iter()
        .map(|b| b.gid)
        .collect();
    assert_eq!(got, [n[2].clone(), tie[0].clone(), tie[1].clone()]);
    let mut tie = [a[0].clone(), a[1].clone()];
    tie.sort();
    let got: Vec<String> = s
        .action_items(&m)
        .unwrap()
        .into_iter()
        .map(|b| b.gid)
        .collect();
    assert_eq!(got, [a[2].clone(), tie[0].clone(), tie[1].clone()]);
    // A row added now still goes last.
    let last = s.add_note_block(&m, note("d")).unwrap().gid;
    assert_eq!(s.note_blocks(&m).unwrap().last().unwrap().gid, last);
}

#[test]
fn segments_with_the_same_start_order_by_gid() {
    let (_d, s) = open();
    let m = meeting(&s);
    let hi = "ffffffff-ffff-7fff-8fff-ffffffffffff";
    let lo = "00000000-0000-7000-8000-000000000000";
    s.add_segment(&m, seg(Some(hi), 0, "x")).unwrap();
    s.add_segment(&m, seg(Some(lo), 0, "y")).unwrap();
    let got: Vec<String> = s.segments(&m).unwrap().into_iter().map(|g| g.gid).collect();
    assert_eq!(got, [lo, hi]);
}

#[test]
fn ord_between_puts_a_row_between_neighbours() {
    let mut ords = vec![crate::migrate::rank_ord(1), crate::migrate::rank_ord(2)];
    // Repeatedly insert between the first two, and after the last.
    for _ in 0..40 {
        let mid = ord_between(Some(&ords[0]), Some(&ords[1]));
        assert!(ords[0] < mid && mid < ords[1], "{ords:?} {mid}");
        ords.insert(1, mid);
        let after = ord_between(ords.last().map(String::as_str), None);
        assert!(after.as_str() > ords.last().unwrap().as_str());
        ords.push(after);
    }
    // And before the first (no lower bound).
    let first = ord_between(None, Some(&ords[0]));
    assert!(first < ords[0]);
    assert!(ord_between(Some("000z"), None).as_str() > "000z");
    let mut sorted = ords.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ords.len());
}

// ------------------------------------------------------------------ lamport

fn meeting_lamport(s: &Store, m: &str) -> i64 {
    one(s, "SELECT lamport FROM meetings WHERE gid = ?1", [m])
}

#[test]
fn monotone_fields_do_not_move_the_meeting_lamport() {
    let (_d, s) = open();
    let m = meeting(&s);
    let l0 = meeting_lamport(&s, &m);
    s.set_meeting_status(&m, "processing").unwrap();
    s.set_consent_confirmed(&m, true).unwrap();
    s.extend_meeting_duration(&m, 5000).unwrap();
    s.record_cloud_request(&m, "p", "model", 1, 1).unwrap();
    s.finish_meeting(&m, 9000).unwrap();
    assert_eq!(meeting_lamport(&s, &m), l0);
    assert_eq!(one::<i64>(&s, "SELECT duration_ms FROM meetings", []), 9000);
    assert_eq!(one::<i64>(&s, "SELECT cloud_used FROM meetings", []), 1);
    assert_eq!(one::<String>(&s, "SELECT status FROM meetings", []), "done");
    // LWW fields still do.
    s.set_audio_retained_until(&m, Some(1)).unwrap();
    let l1 = meeting_lamport(&s, &m);
    assert!(l1 > l0);
    s.set_sensitive(&m, true).unwrap();
    assert!(meeting_lamport(&s, &m) > l1);
}

#[test]
fn retention_policy_changes_move_only_the_meetings_they_change() {
    let (_d, s) = open();
    let (a, b) = (meeting(&s), meeting(&s));
    s.apply_retention_days(Some(30)).unwrap();
    let (la, lb) = (meeting_lamport(&s, &a), meeting_lamport(&s, &b));
    assert_eq!(s.apply_retention_days(Some(30)).unwrap(), 0);
    assert_eq!((meeting_lamport(&s, &a), meeting_lamport(&s, &b)), (la, lb));
    assert_eq!(s.apply_retention_days(None).unwrap(), 2);
    assert!(meeting_lamport(&s, &a) > la);
}

#[test]
fn a_track_has_its_own_version() {
    let (_d, s) = open();
    let m = meeting(&s);
    let l0 = meeting_lamport(&s, &m);
    let mut w = s.open_track(&m, TrackKind::Mic).unwrap();
    for i in 0..4u8 {
        w.append(&[i; 8]).unwrap();
    }
    let t0: i64 = one(&s, "SELECT lamport FROM tracks", []);
    assert!(t0 > 0);
    s.finish_track(&m, TrackKind::Mic, w).unwrap();
    let t1: i64 = one(&s, "SELECT lamport FROM tracks", []);
    assert!(t1 > t0);
    assert_eq!(s.truncate_track(&m, TrackKind::Mic, 2).unwrap(), 2);
    let t2: i64 = one(&s, "SELECT lamport FROM tracks", []);
    assert!(t2 > t1);
    assert_eq!(one::<i64>(&s, "SELECT cut_pages FROM tracks", []), 2);
    assert_eq!(one::<i64>(&s, "SELECT page_count FROM tracks", []), 2);
    assert_eq!(meeting_lamport(&s, &m), l0);
}

#[test]
fn finishing_a_meeting_logs_its_rows_again() {
    let (_d, s) = open();
    let m = meeting(&s);
    let seg_gid = s.add_segment(&m, seg(None, 0, "x")).unwrap().gid;
    let other = meeting(&s);
    let n = s.add_note_block(&other, note("n")).unwrap().gid;
    let seq = |g: &str| -> i64 { one(&s, "SELECT seq FROM sync_log WHERE gid = ?1", [g]) };
    let (before, m_before, other_before) = (seq(&seg_gid), seq(&m), seq(&n));
    s.finish_meeting(&m, 1000).unwrap();
    assert!(seq(&seg_gid) > before);
    assert!(seq(&m) > other_before.max(before).max(m_before));
    assert!(seq(&m) < seq(&seg_gid), "the meeting comes before its rows");
    assert_eq!(seq(&n), other_before, "other meetings are not touched");
}

#[test]
fn a_new_feed_id_replaces_the_old_one() {
    let (_d, s) = open();
    let before = s.get_setting("sync.feed_id").unwrap().unwrap();
    s.reset_feed_id().unwrap();
    let after = s.get_setting("sync.feed_id").unwrap().unwrap();
    assert_ne!(before, after);
}

// -------------------------------------------------------------------- epochs

fn lease(s: &Store, job: &str, meeting_gid: &str, state: &str) {
    s.conn()
        .execute(
            "INSERT INTO leases (job_uuid, meeting_gid, role, epoch, state, ttl_ms)
             VALUES (?1, ?2, 'holder', 3, ?3, 1000)",
            params![job, meeting_gid, state],
        )
        .unwrap();
}

fn lease_state(s: &Store, job: &str) -> String {
    one(s, "SELECT state FROM leases WHERE job_uuid = ?1", [job])
}

#[test]
fn a_transcript_commit_with_a_granted_lease_finishes_it_atomically() {
    let (_d, s) = open();
    let m = meeting(&s);
    s.add_segment(&m, seg(None, 0, "v1")).unwrap();
    let v0: i64 = one(&s, "SELECT transcript_version FROM meetings", []);
    lease(&s, "job-1", &m, "granted");
    let v = s
        .replace_transcript_marked_epoch(&m, vec![seg(None, 0, "v2")], &[], 3, Some("job-1"))
        .unwrap();
    assert_eq!(v, v0 + 1);
    assert_eq!(lease_state(&s, "job-1"), "done");
    assert_eq!(
        one::<i64>(&s, "SELECT transcript_epoch FROM meetings", []),
        3
    );
    assert_eq!(one::<i64>(&s, "SELECT epoch FROM segments", []), 3);
    // Live lines added afterwards carry the meeting's epoch.
    s.add_segment(&m, seg(None, 900, "later")).unwrap();
    assert_eq!(one::<i64>(&s, "SELECT min(epoch) FROM segments", []), 3);
}

#[test]
fn a_transcript_commit_without_its_lease_is_fenced_and_writes_nothing() {
    let (_d, s) = open();
    let m = meeting(&s);
    let old = s.add_segment(&m, seg(None, 0, "v1")).unwrap().gid;
    let v0: i64 = one(&s, "SELECT transcript_version FROM meetings", []);
    for (job, state) in [
        ("j-revoked", "revoked"),
        ("j-done", "done"),
        ("j-exp", "expired"),
    ] {
        lease(&s, job, &m, state);
        let r = s.replace_transcript_epoch(&m, vec![seg(None, 0, "v2")], 4, Some(job));
        assert!(matches!(r, Err(StoreError::Fenced)), "{state}: {r:?}");
        assert_eq!(lease_state(&s, job), state);
    }
    // An unknown job is fenced too.
    assert!(matches!(
        s.replace_transcript_epoch(&m, vec![seg(None, 0, "v2")], 4, Some("nope")),
        Err(StoreError::Fenced)
    ));
    let segs = s.segments(&m).unwrap();
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].gid, old);
    assert!(!s.is_tombstoned(&old).unwrap());
    assert_eq!(
        one::<i64>(&s, "SELECT transcript_epoch FROM meetings", []),
        0
    );
    assert_eq!(
        one::<i64>(&s, "SELECT transcript_version FROM meetings", []),
        v0
    );
    // The same lease, once granted, still commits (it was never consumed).
    lease(&s, "j-ok", &m, "granted");
    s.replace_transcript_epoch(&m, vec![seg(None, 0, "v2")], 4, Some("j-ok"))
        .unwrap();
}

#[test]
fn notes_commit_is_fenced_by_its_lease_too() {
    let (_d, s) = open();
    let m = meeting(&s);
    let ai = s
        .add_note_block(
            &m,
            NewNoteBlock {
                provenance: Provenance::Ai,
                ..note("old")
            },
        )
        .unwrap()
        .gid;
    lease(&s, "j-rev", &m, "revoked");
    let r = s.replace_ai_notes_epoch(&m, vec![note("new")], vec![action("a")], 2, Some("j-rev"));
    assert!(matches!(r, Err(StoreError::Fenced)));
    let blocks = s.note_blocks(&m).unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].gid, ai);
    assert!(s.action_items(&m).unwrap().is_empty());
    assert_eq!(one::<i64>(&s, "SELECT ai_epoch FROM meetings", []), 0);

    lease(&s, "j-ok", &m, "granted");
    let out = s
        .replace_ai_notes_epoch(&m, vec![note("new")], vec![action("a")], 2, Some("j-ok"))
        .unwrap();
    assert_eq!((out.removed, out.added), (1, 2));
    assert_eq!(lease_state(&s, "j-ok"), "done");
    assert_eq!(one::<i64>(&s, "SELECT ai_epoch FROM meetings", []), 2);
    assert_eq!(one::<i64>(&s, "SELECT min(epoch) FROM notes_blocks", []), 2);
    assert_eq!(one::<i64>(&s, "SELECT min(epoch) FROM action_items", []), 2);
    // The old fns keep the meeting's epoch and need no lease.
    s.replace_ai_notes(&m, vec![note("again")], vec![]).unwrap();
    assert_eq!(one::<i64>(&s, "SELECT ai_epoch FROM meetings", []), 2);
    assert_eq!(one::<i64>(&s, "SELECT min(epoch) FROM notes_blocks", []), 2);
}

// ----------------------------------------------------------------- retention

#[test]
fn the_retention_sweep_skips_a_meeting_with_an_open_lease() {
    let (_d, s) = open();
    let (leased, free) = (meeting(&s), meeting(&s));
    for m in [&leased, &free] {
        let w = s.open_track(m, TrackKind::Mic).unwrap();
        s.finish_track(m, TrackKind::Mic, w).unwrap();
        s.set_audio_retained_until(m, Some(1)).unwrap();
    }
    lease(&s, "j", &leased, "granted");
    let rep = s.retention_sweep(10).unwrap();
    assert_eq!(rep.meetings, 1);
    assert!(s.audio_available(&leased).unwrap());
    assert!(!s.audio_available(&free).unwrap());
    // Once the lease is over, the audio goes.
    s.conn()
        .execute("UPDATE leases SET state = 'done'", [])
        .unwrap();
    assert_eq!(s.retention_sweep(10).unwrap().meetings, 1);
    assert!(!s.audio_available(&leased).unwrap());
}

// ---------------------------------------------------------------- raw import

#[test]
fn a_track_is_received_through_the_store_and_its_page_count_recorded() {
    use crate::bundle::{RawBegin, raw_header, raw_records};
    let (_d, s) = open();
    let m = meeting(&s);
    let mut w = s.open_track(&m, TrackKind::Mic).unwrap();
    for i in 0..5u8 {
        w.append(&[i; 32]).unwrap();
    }
    s.finish_track(&m, TrackKind::Mic, w).unwrap();
    let path = s.bundle_path(&m, TrackKind::Mic).unwrap();
    let header = raw_header(&path).unwrap();
    let recs = raw_records(&path, 0, u32::MAX).unwrap();
    let original = std::fs::read(&path).unwrap();

    // The receiver's copy is lost (the row stays, as after a metadata-only sync).
    std::fs::remove_file(&path).unwrap();
    s.conn()
        .execute("UPDATE tracks SET page_count = 0", [])
        .unwrap();
    assert!(s.audio_available(&m).unwrap());
    let RawBegin::Resume(mut imp) = s.raw_import_begin(&m, TrackKind::Mic, &header).unwrap() else {
        panic!("nothing local, so a part starts")
    };
    assert_eq!(imp.have(), 0);
    imp.push(&recs).unwrap();
    assert_eq!(s.raw_import_finish(&m, TrackKind::Mic, imp).unwrap(), 5);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert_eq!(one::<i64>(&s, "SELECT page_count FROM tracks", []), 5);
    assert_eq!(s.open_bundle(&m, TrackKind::Mic).unwrap().page_count(), 5);
    // Now complete, it says so whatever header is offered.
    assert!(matches!(
        s.raw_import_begin(&m, TrackKind::Mic, &header).unwrap(),
        RawBegin::Complete { pages: 5 }
    ));
    // An unknown track is not found.
    assert!(matches!(
        s.raw_import_begin(&m, TrackKind::System, &header),
        Err(StoreError::NotFound { .. })
    ));
}
