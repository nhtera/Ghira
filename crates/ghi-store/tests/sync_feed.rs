// SPDX-License-Identifier: Apache-2.0
//! Slice 15-C1: the change feed, paired devices, key transfer, leases, wipe and
//! synced settings (doc 07 §3, §7, §8).

mod common;

use std::sync::{Arc, Barrier};

use ghi_store::StoreError;
use ghi_store::store::{NewNoteBlock, NewSpeaker, Provenance, Store, TrackKind};
use ghi_store::sync::devices::{DeviceRole, DeviceState, NewDevice};
use ghi_store::sync::feed::FeedChange;
use ghi_store::sync::leases::{Lease, LeaseRole};
use ghi_store::sync::records::{Record, SettingRec, Version};

const PSK: [u8; 32] = [7; 32];

fn peer(n: u8) -> NewDevice {
    NewDevice {
        gid: common::gid(9, n as u16),
        name: format!("Peer {n}"),
        platform: "ios".into(),
        role: DeviceRole::Spoke,
        static_pub: [n; 32],
    }
}

fn open() -> (tempfile::TempDir, Store) {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _) = common::open(tmp.path());
    (tmp, store)
}

fn finished(store: &Store, title: &str) -> String {
    let gid = common::meeting(store, title);
    store
        .add_segment(&gid, common::seg(0, 1000, "xin chào"))
        .unwrap();
    store.finish_meeting(&gid, 1000).unwrap();
    gid
}

fn gids(changes: &[FeedChange]) -> Vec<String> {
    changes.iter().map(|c| c.record.gid().to_string()).collect()
}

fn has_meeting(changes: &[FeedChange], gid: &str) -> bool {
    changes.iter().any(|c| c.record.gid() == gid)
}

// ------------------------------------------------------------------- feed

#[test]
fn recording_meetings_are_held_back_until_they_finish() {
    let (_tmp, store) = open();
    let live = common::meeting(&store, "Đang ghi");
    store
        .add_segment(&live, common::seg(0, 500, "một"))
        .unwrap();
    let done = finished(&store, "Xong rồi");

    let batch = store.changes_since(0, 256).unwrap();
    assert!(has_meeting(&batch.changes, &done));
    for c in &batch.changes {
        assert_ne!(c.record.gid(), live);
        assert_ne!(c.record.meeting_gid(), Some(live.as_str()));
    }
    let cursor = batch.upto_seq;

    store.finish_meeting(&live, 500).unwrap();
    store.relog_meeting(&live).unwrap();
    let after = store.changes_since(cursor, 256).unwrap();
    assert!(has_meeting(&after.changes, &live));
    assert!(
        after
            .changes
            .iter()
            .any(|c| matches!(&c.record, Record::Segment(s) if s.meeting_gid == live))
    );
    assert!(!after.more);
}

#[test]
fn a_meeting_comes_before_its_children_in_a_batch() {
    let (_tmp, store) = open();
    let gid = common::meeting(&store, "Cha và con");
    let sp = store
        .add_speaker(
            &gid,
            NewSpeaker {
                label_idx: 0,
                ..Default::default()
            },
        )
        .unwrap();
    store
        .add_segment(&gid, common::seg(0, 1000, "lời thoại"))
        .unwrap();
    store
        .add_note_block(
            &gid,
            NewNoteBlock {
                kind: "paragraph".into(),
                provenance: Provenance::User,
                body: "ghi chú".into(),
                anchors: vec![],
                pinned: false,
            },
        )
        .unwrap();
    store
        .add_mark(&gid, 10, ghi_store::store::MarkTag::Star)
        .unwrap();
    // The meeting row changes last, so its log entry is the newest.
    store.finish_meeting(&gid, 1000).unwrap();
    store.relog_meeting(&gid).unwrap();
    let _ = sp;

    let batch = store.changes_since(0, 256).unwrap();
    let ids = gids(&batch.changes);
    let m = ids.iter().position(|g| *g == gid).expect("meeting emitted");
    for (i, c) in batch.changes.iter().enumerate() {
        if c.record.meeting_gid() == Some(gid.as_str()) {
            assert!(m < i, "child {:?} before its meeting", c.record.kind());
        }
    }
    assert!(matches!(batch.changes[m].record, Record::Meeting(_)));
}

#[test]
fn batches_are_bounded_and_resume_from_upto_seq() {
    let (_tmp, store) = open();
    for i in 0..5 {
        finished(&store, &format!("Cuộc họp {i}"));
    }
    let mut seen = Vec::new();
    let mut cursor = 0;
    let mut rounds = 0;
    loop {
        let b = store.changes_since(cursor, 3).unwrap();
        assert!(b.changes.len() <= 3);
        assert!(b.upto_seq >= cursor);
        seen.extend(gids(&b.changes));
        cursor = b.upto_seq;
        rounds += 1;
        if !b.more {
            break;
        }
        assert!(rounds < 100, "no progress");
    }
    let all = store.changes_since(0, 256).unwrap();
    let mut a = seen.clone();
    let mut b = gids(&all.changes);
    a.sort();
    b.sort();
    assert_eq!(a, b);
    assert!(store.changes_since(cursor, 3).unwrap().changes.is_empty());
}

#[test]
fn cursors_and_feed_id_survive_a_reopen() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    finished(&store, "Một");
    let p = peer(1);
    store.pin_device(&p, &PSK).unwrap();
    let batch = store.changes_since(0, 256).unwrap();
    let feed = store.feed_id().unwrap();
    store
        .set_cursors(&p.gid, batch.upto_seq, Some("peer-feed"), 42)
        .unwrap();
    drop(store);

    let store = common::reopen(tmp.path(), &keys);
    assert_eq!(store.feed_id().unwrap(), feed);
    let d = store.device(&p.gid).unwrap().unwrap();
    assert_eq!((d.push_seq, d.pull_seq), (batch.upto_seq, 42));
    assert_eq!(d.pull_feed_id.as_deref(), Some("peer-feed"));
    assert!(
        store
            .changes_since(d.push_seq, 256)
            .unwrap()
            .changes
            .is_empty()
    );
    // A peer that announces a different feed starts over.
    assert_eq!(store.pull_cursor_for(&p.gid, "peer-feed").unwrap(), 42);
    assert_eq!(store.pull_cursor_for(&p.gid, "other-feed").unwrap(), 0);
}

#[test]
fn a_new_feed_id_resets_every_cursor() {
    let (_tmp, store) = open();
    let p = peer(1);
    store.pin_device(&p, &PSK).unwrap();
    store.set_cursors(&p.gid, 9, Some("f"), 4).unwrap();
    let old = store.feed_id().unwrap();
    let new = store.regen_feed_id().unwrap();
    assert_ne!(old, new);
    let d = store.device(&p.gid).unwrap().unwrap();
    assert_eq!((d.push_seq, d.pull_seq, d.pull_feed_id), (0, 0, None));
}

#[test]
fn tombstones_come_by_sequence_and_are_not_rows() {
    let (_tmp, store) = open();
    let a = finished(&store, "A");
    let b = finished(&store, "B");
    store.delete_meeting(&a).unwrap();
    let first = store.tombs_since(0, 256).unwrap();
    assert!(
        first
            .tombs
            .iter()
            .any(|t| t.gid == a && t.kind == "meeting")
    );
    assert!(!first.more);
    let cursor = first.upto_seq;
    store.delete_meeting(&b).unwrap();
    let second = store.tombs_since(cursor, 256).unwrap();
    assert!(second.tombs.iter().any(|t| t.gid == b));
    assert!(second.tombs.iter().all(|t| t.gid != a));
    // Rows never include tombstoned gids.
    let rows = store.changes_since(0, 256).unwrap();
    assert!(!has_meeting(&rows.changes, &a) && !has_meeting(&rows.changes, &b));
    // A page limit paginates.
    let one = store.tombs_since(0, 1).unwrap();
    assert_eq!(one.tombs.len(), 1);
    assert!(one.more);
}

// ---------------------------------------------------------------- devices

#[test]
fn pins_hold_the_psk_and_refuse_duplicates() {
    let (_tmp, store) = open();
    let p = peer(1);
    let d = store.pin_device(&p, &PSK).unwrap();
    assert_eq!(d.state, DeviceState::Paired);
    assert_eq!(*store.pair_psk(&p.gid).unwrap(), PSK);
    assert_eq!(
        store.device_by_key(&p.static_pub).unwrap().unwrap().gid,
        p.gid
    );
    assert!(matches!(
        store.pin_device(&p, &PSK),
        Err(StoreError::Duplicate { .. })
    ));
    let mut same_key = peer(2);
    same_key.static_pub = p.static_pub;
    assert!(matches!(
        store.pin_device(&same_key, &PSK),
        Err(StoreError::Duplicate { .. })
    ));
    let mut own = peer(3);
    own.gid = store.sync_device_gid().unwrap();
    assert!(store.pin_device(&own, &PSK).is_err());

    store
        .touch_device(&p.gid, Some("192.168.1.5:7000"))
        .unwrap();
    store.touch_device(&p.gid, None).unwrap();
    let d = store.device(&p.gid).unwrap().unwrap();
    assert!(d.last_seen.is_some());
    assert_eq!(d.last_addr.as_deref(), Some("192.168.1.5:7000"));

    store.set_wipe_pending(&p.gid).unwrap();
    assert_eq!(
        store.device(&p.gid).unwrap().unwrap().state,
        DeviceState::WipePending
    );
    store.unpin_device(&p.gid).unwrap();
    assert!(store.device(&p.gid).unwrap().is_none());
    assert!(matches!(
        store.pair_psk(&p.gid),
        Err(StoreError::NotFound { .. })
    ));
    assert!(store.devices().unwrap().is_empty());
}

#[test]
fn the_psk_survives_wrap_rotations_and_reopen() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let p = peer(1);
    store.pin_device(&p, &PSK).unwrap();
    // Every delete rotates the wrap secret.
    let m = finished(&store, "Bỏ");
    store.delete_meeting(&m).unwrap();
    assert_eq!(*store.pair_psk(&p.gid).unwrap(), PSK);
    drop(store);
    let store = common::reopen(tmp.path(), &keys);
    assert_eq!(*store.pair_psk(&p.gid).unwrap(), PSK);
}

#[test]
fn mass_delete_confirmation_is_per_device() {
    let (_tmp, store) = open();
    let (a, b) = (peer(1), peer(2));
    store.pin_device(&a, &PSK).unwrap();
    store.pin_device(&b, &PSK).unwrap();
    assert!(!store.mass_delete_confirmed(&a.gid).unwrap());
    store.set_mass_delete_confirmed(&a.gid, true).unwrap();
    assert!(store.mass_delete_confirmed(&a.gid).unwrap());
    assert!(!store.mass_delete_confirmed(&b.gid).unwrap());
    store.unpin_device(&a.gid).unwrap();
    assert!(!store.mass_delete_confirmed(&a.gid).unwrap());
}

// ------------------------------------------------------------------- keys

#[test]
fn a_key_goes_to_each_peer_once() {
    let (_tmp, store) = open();
    let (a, b) = (peer(1), peer(2));
    store.pin_device(&a, &PSK).unwrap();
    store.pin_device(&b, &PSK).unwrap();
    let m = finished(&store, "Khóa");

    let k1 = store.meeting_dek_for_peer(&a.gid, &m).unwrap().unwrap();
    // Until the peer acks, it is offered again.
    assert!(store.meeting_dek_for_peer(&a.gid, &m).unwrap().is_some());
    store.mark_key_sent(&a.gid, &m).unwrap();
    assert!(store.meeting_dek_for_peer(&a.gid, &m).unwrap().is_none());
    // Another peer still gets it, and it is the same key.
    let k2 = store.meeting_dek_for_peer(&b.gid, &m).unwrap().unwrap();
    assert_eq!(*k1, *k2);
    assert_eq!(store.peer_meetings(&a.gid).unwrap(), vec![m.clone()]);
    assert!(matches!(
        store.meeting_dek_for_peer(&common::gid(9, 99), &m),
        Err(StoreError::NotFound { .. })
    ));
}

#[test]
fn a_different_key_is_rejected_and_the_same_one_accepted() {
    let (_tmp, store) = open();
    let p = peer(1);
    store.pin_device(&p, &PSK).unwrap();
    let m = finished(&store, "T10");
    let dek = store.meeting_dek_for_peer(&p.gid, &m).unwrap().unwrap();

    store.accept_dek(&m, &dek, &p.gid).unwrap();
    // The peer is known to hold the key now.
    assert!(store.meeting_dek_for_peer(&p.gid, &m).unwrap().is_none());
    let mut other = *dek;
    other[0] ^= 1;
    assert!(matches!(
        store.accept_dek(&m, &other, &p.gid),
        Err(StoreError::Invalid(_))
    ));
    // The title still decrypts: nothing was overwritten.
    assert_eq!(store.get_meeting(&m).unwrap().title, "T10");
}

#[test]
fn a_tombstoned_gid_refuses_a_key() {
    let (_tmp, store) = open();
    let p = peer(1);
    store.pin_device(&p, &PSK).unwrap();
    let m = finished(&store, "Đã xóa");
    let dek = store.meeting_dek_for_peer(&p.gid, &m).unwrap().unwrap();
    store.delete_meeting(&m).unwrap();
    assert!(matches!(
        store.accept_dek(&m, &dek, &p.gid),
        Err(StoreError::Tombstoned { .. })
    ));
}

// ------------------------------------------------------------------- wipe

#[test]
fn wipe_removes_the_peers_meetings_without_tombstones() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _) = common::open(tmp.path());
    let (a, b) = (peer(1), peer(2));
    store.pin_device(&a, &PSK).unwrap();
    store.pin_device(&b, &PSK).unwrap();

    let make = |title: &str| {
        let m = finished(&store, title);
        let mut w = store.open_track(&m, TrackKind::Mic).unwrap();
        w.append(b"audio-page").unwrap();
        store.finish_track(&m, TrackKind::Mic, w).unwrap();
        m
    };
    let (shared, also_shared, mine, theirs) = (
        make("Chung 1"),
        make("Chung 2"),
        make("Của tôi"),
        make("Của b"),
    );
    for m in [&shared, &also_shared] {
        store.meeting_dek_for_peer(&a.gid, m).unwrap();
    }
    store.meeting_dek_for_peer(&b.gid, &theirs).unwrap();
    let before = store.tombs_since(0, 256).unwrap();

    let report = store.wipe_peer(&a.gid).unwrap();
    assert_eq!(report.meetings, 2);
    for m in [&shared, &also_shared] {
        assert!(matches!(
            store.get_meeting(m),
            Err(StoreError::NotFound { .. })
        ));
        assert!(!tmp.path().join("bundles").join(m).exists());
        assert!(
            store
                .changes_since(0, 256)
                .unwrap()
                .changes
                .iter()
                .all(|c| c.record.gid() != m && c.record.meeting_gid() != Some(m.as_str()))
        );
    }
    // No tombstones for the wiped meetings, so no other device is told.
    let after = store.tombs_since(0, 256).unwrap();
    assert_eq!(after.tombs, before.tombs);
    assert!(!store.is_tombstoned(&shared).unwrap());
    assert!(store.device(&a.gid).unwrap().is_none());
    // Everything else is untouched.
    for m in [&mine, &theirs] {
        assert_eq!(store.segments(m).unwrap().len(), 1);
        assert!(tmp.path().join("bundles").join(m).exists());
    }
    assert_eq!(store.peer_meetings(&b.gid).unwrap(), vec![theirs]);
    assert!(store.device(&b.gid).unwrap().is_some());
}

// ----------------------------------------------------------------- leases

fn lease(job: &str, meeting: &str, state: &str) -> Lease {
    Lease {
        job_uuid: job.into(),
        meeting_gid: meeting.into(),
        role: LeaseRole::Holder,
        epoch: 1,
        kinds: vec!["final_pass".into()],
        state: state.into(),
        ttl_ms: 60_000,
        deadline_cont_ns: Some(10_000_000_000),
        boot_id: Some("boot-1".into()),
        wall_deadline_ms: None,
        progress: 0.0,
        peer_gid: None,
    }
}

#[test]
fn a_lease_opens_once_and_lists() {
    let (_tmp, store) = open();
    let p = peer(1);
    store.pin_device(&p, &PSK).unwrap();
    let m = finished(&store, "Lease");
    assert!(!store.lease_any_open_for(&m).unwrap());
    let mut l = lease("job-1", &m, "granted");
    l.peer_gid = Some(p.gid.clone());
    let stored = store.lease_open(&l).unwrap();
    assert_eq!(stored, l);
    // A duplicate request is a no-op, even with other values.
    let mut dup = l.clone();
    dup.epoch = 5;
    assert_eq!(store.lease_open(&dup).unwrap().epoch, 1);
    assert!(store.lease_any_open_for(&m).unwrap());
    assert_eq!(store.leases_for_meeting(&m).unwrap().len(), 1);
    assert_eq!(store.leases_open().unwrap().len(), 1);
    store.lease_set_progress("job-1", 0.5).unwrap();
    assert_eq!(store.lease_state("job-1").unwrap().unwrap().progress, 0.5);
    assert!(store.lease_state("nope").unwrap().is_none());

    let mut bad = lease("job-2", &m, "weird");
    assert!(store.lease_open(&bad).is_err());
    bad.state = "granted".into();
    bad.ttl_ms = 0;
    assert!(store.lease_open(&bad).is_err());

    assert!(
        store
            .lease_transition("job-1", &["granted"], "done")
            .unwrap()
    );
    assert!(!store.lease_any_open_for(&m).unwrap());
    assert!(
        !store
            .lease_transition("job-1", &["granted"], "done")
            .unwrap()
    );
}

#[test]
fn renew_moves_the_deadline_only_while_open() {
    let (_tmp, store) = open();
    let m = finished(&store, "Renew");
    store.lease_open(&lease("job-1", &m, "granted")).unwrap();
    store
        .lease_renew("job-1", 120_000, 50_000_000_000, "boot-2")
        .unwrap();
    let l = store.lease_state("job-1").unwrap().unwrap();
    assert_eq!(
        (l.ttl_ms, l.deadline_cont_ns, l.boot_id.as_deref()),
        (120_000, Some(50_000_000_000), Some("boot-2"))
    );
    assert!(l.wall_deadline_ms.is_some());
    store.lease_revoke("job-1").unwrap();
    assert!(matches!(
        store.lease_renew("job-1", 1, 1, "b"),
        Err(StoreError::Fenced)
    ));
    assert!(matches!(
        store.lease_renew("zzz", 1, 1, "b"),
        Err(StoreError::NotFound { .. })
    ));
}

#[test]
fn the_fence_closes_on_deadline_boot_change_and_revoke() {
    let (_tmp, store) = open();
    let m = finished(&store, "Fence");
    store.lease_open(&lease("job-1", &m, "granted")).unwrap();
    let s = 1_000_000_000i64;
    // deadline = 10 s; margin 1 s.
    assert!(
        store
            .lease_fence_ok("job-1", 5 * s, "boot-1", 1000)
            .unwrap()
    );
    assert!(
        store
            .lease_fence_ok("job-1", 8 * s, "boot-1", 1000)
            .unwrap()
    );
    assert!(
        !store
            .lease_fence_ok("job-1", 9 * s, "boot-1", 1000)
            .unwrap()
    );
    assert!(!store.lease_fence_ok("job-1", 10 * s, "boot-1", 0).unwrap());
    assert!(!store.lease_fence_ok("job-1", 11 * s, "boot-1", 0).unwrap());
    // Another boot: suspended until renewed.
    assert!(!store.lease_fence_ok("job-1", 5 * s, "boot-2", 0).unwrap());
    store
        .lease_renew("job-1", 60_000, 100 * s, "boot-2")
        .unwrap();
    assert!(store.lease_fence_ok("job-1", 5 * s, "boot-2", 0).unwrap());
    // Unknown, offered and revoked leases never pass.
    assert!(!store.lease_fence_ok("nope", 0, "boot-2", 0).unwrap());
    store.lease_open(&lease("job-2", &m, "offered")).unwrap();
    assert!(!store.lease_fence_ok("job-2", 0, "boot-1", 0).unwrap());
    assert!(store.lease_revoke("job-1").unwrap());
    assert!(!store.lease_fence_ok("job-1", 5 * s, "boot-2", 0).unwrap());
}

fn commit(store: &Store, m: &str, job: &str) -> Result<i64, StoreError> {
    store.replace_transcript_epoch(m, vec![common::seg(0, 500, "kết quả")], 2, Some(job))
}

#[test]
fn revoke_and_commit_in_order_have_one_winner() {
    let (_tmp, store) = open();
    let m = finished(&store, "Race");
    store.lease_open(&lease("a", &m, "granted")).unwrap();
    assert!(store.lease_revoke("a").unwrap());
    assert!(matches!(commit(&store, &m, "a"), Err(StoreError::Fenced)));

    store.lease_open(&lease("b", &m, "granted")).unwrap();
    commit(&store, &m, "b").unwrap();
    assert!(!store.lease_revoke("b").unwrap());
    assert_eq!(store.lease_state("b").unwrap().unwrap().state, "done");
}

#[test]
fn revoke_racing_a_commit_has_exactly_one_winner() {
    let (_tmp, store) = open();
    let store = Arc::new(store);
    for i in 0..20 {
        let m = finished(&store, &format!("Đua {i}"));
        let job = format!("job-{i}");
        store.lease_open(&lease(&job, &m, "granted")).unwrap();
        let gate = Arc::new(Barrier::new(2));
        let (s1, g1, m1, j1) = (store.clone(), gate.clone(), m.clone(), job.clone());
        let committer = std::thread::spawn(move || {
            g1.wait();
            commit(&s1, &m1, &j1)
        });
        gate.wait();
        let revoked = store.lease_revoke(&job).unwrap();
        let committed = committer.join().unwrap();
        match (revoked, committed) {
            (true, Err(StoreError::Fenced)) => {
                assert_eq!(store.lease_state(&job).unwrap().unwrap().state, "revoked")
            }
            (false, Ok(_)) => {
                assert_eq!(store.lease_state(&job).unwrap().unwrap().state, "done")
            }
            other => panic!("not exactly one winner: {other:?}"),
        }
    }
}

// --------------------------------------------------------------- settings

fn setting(key: &str, value: &str, lamport: i64, origin: &str) -> SettingRec {
    SettingRec {
        gid: ghi_store::sync::settings::setting_gid(key),
        version: Version {
            lamport,
            origin: origin.into(),
        },
        key: key.into(),
        value_json: Some(value.into()),
        ..Default::default()
    }
}

#[test]
fn only_allowlisted_settings_sync() {
    let (_tmp, store) = open();
    assert!(matches!(
        store.put_synced("onboardingDone", "true"),
        Err(StoreError::Invalid(_))
    ));
    assert!(store.put_synced("meetingLanguage", "\"klingon\"").is_err());
    assert!(store.put_synced("cloudRedact", "\"yes\"").is_err());

    store.put_synced("cloudRedact", "true").unwrap();
    let feed = store.changes_since(0, 256).unwrap();
    let rec = feed
        .changes
        .iter()
        .find_map(|c| match &c.record {
            Record::Setting(s) if s.key == "cloudRedact" => Some(s.clone()),
            _ => None,
        })
        .expect("the setting is in the feed");
    assert_eq!(rec.value_json.as_deref(), Some("true"));
    assert_eq!(
        rec.gid,
        ghi_store::sync::settings::setting_gid("cloudRedact")
    );
    assert_eq!(rec.version.origin, store.sync_device_gid().unwrap());

    // Unknown keys from a peer are ignored: nothing is stored or logged.
    let before = store.changes_since(0, 256).unwrap().changes.len();
    store
        .apply_synced(&setting("onboardingDone", "true", 99, "peer"))
        .unwrap();
    store
        .apply_synced(&setting("appLock", "true", 99, "peer"))
        .unwrap();
    assert_eq!(store.changes_since(0, 256).unwrap().changes.len(), before);
    assert!(store.get_setting("onboardingDone").unwrap().is_none());
}

#[test]
fn a_synced_value_writes_the_underlying_setting_last_writer_wins() {
    let (_tmp, store) = open();
    store
        .set_setting(
            "app",
            &serde_json::json!({"onboardingDone": true, "cloudRedact": false}),
        )
        .unwrap();
    let (p1, p2) = (peer(1), peer(2));
    store.pin_device(&p1, &PSK).unwrap();
    store.pin_device(&p2, &PSK).unwrap();
    let peer_gid = p1.gid.clone();

    store
        .apply_synced(&setting("cloudRedact", "true", 50, &peer_gid))
        .unwrap();
    let app = store.get_setting("app").unwrap().unwrap();
    assert_eq!(app["cloudRedact"], true);
    // The other fields of the app JSON are kept.
    assert_eq!(app["onboardingDone"], true);

    // An older write loses, a newer one wins.
    store
        .apply_synced(&setting("cloudRedact", "false", 40, &peer_gid))
        .unwrap();
    assert_eq!(
        store.get_setting("app").unwrap().unwrap()["cloudRedact"],
        true
    );
    store
        .apply_synced(&setting("cloudRedact", "false", 60, &peer_gid))
        .unwrap();
    assert_eq!(
        store.get_setting("app").unwrap().unwrap()["cloudRedact"],
        false
    );
    // Same lamport: the higher origin gid wins, and a replay changes nothing.
    let higher = p2.gid.clone();
    store
        .apply_synced(&setting("cloudRedact", "true", 60, &higher))
        .unwrap();
    assert_eq!(
        store.get_setting("app").unwrap().unwrap()["cloudRedact"],
        true
    );
    store
        .apply_synced(&setting("cloudRedact", "true", 60, &higher))
        .unwrap();

    // The vocabulary is a plain setting; a local write after a remote one sorts
    // after it (the clock followed the remote value).
    store
        .apply_synced(&setting("vocabulary", "[\"Ghira\"]", 70, &peer_gid))
        .unwrap();
    assert_eq!(
        store.get_setting("vocabulary").unwrap().unwrap(),
        serde_json::json!(["Ghira"])
    );
    store
        .put_synced("vocabulary", "[\"Ghira\",\"VinAI\"]")
        .unwrap();
    let feed = store.changes_since(0, 256).unwrap();
    let local = feed
        .changes
        .iter()
        .find_map(|c| match &c.record {
            Record::Setting(s) if s.key == "vocabulary" => Some(s.version.clone()),
            _ => None,
        })
        .unwrap();
    assert!(local.lamport > 70);

    // A value of the wrong shape never reaches `app`.
    assert!(
        store
            .apply_synced(&setting("audioRetentionDays", "\"many\"", 99, &peer_gid))
            .is_err()
    );
    assert!(
        store
            .get_setting("app")
            .unwrap()
            .unwrap()
            .get("audioRetentionDays")
            .is_none()
    );
}
