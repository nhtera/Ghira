// SPDX-License-Identifier: Apache-2.0
//! The rest of the facade: anchors, retention, jobs, export/import, recovery
//! phrase, crash recovery of open audio tracks, CRUD round trips.

mod common;

use ghi_store::jobs::JobState;
use ghi_store::keys::{KeyStore, Protection};
use ghi_store::recovery::RecoveryPhrase;
use ghi_store::search::SearchQuery;
use ghi_store::store::{
    MarkTag, NewActionItem, NewMeeting, NewNoteBlock, NewSegment, NewSpeaker, Provenance, Store,
    TrackKind, Word,
};
use ghi_store::{StoreError, new_gid};
use serde_json::json;

fn record(store: &Store, gid: &str, pages: &[&[u8]]) {
    let mut w = store.open_track(gid, TrackKind::Mic).unwrap();
    for p in pages {
        w.append(p).unwrap();
    }
    store.finish_track(gid, TrackKind::Mic, w).unwrap();
}

#[test]
fn regenerate_keeps_what_the_user_wrote_pinned_edited_or_ticked_off() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = store.create_meeting(NewMeeting::default()).unwrap();
    let block = |body: &str, provenance, pinned| NewNoteBlock {
        kind: "tldr".into(),
        provenance,
        body: body.into(),
        anchors: vec![],
        pinned,
    };
    let action = |text: &str| NewActionItem {
        text: text.into(),
        due_text: Some("thứ Sáu".into()),
        provenance: Provenance::Ai,
        ..Default::default()
    };
    store
        .replace_ai_notes(
            &m.gid,
            vec![
                block("ai old", Provenance::Ai, false),
                block("ai to edit", Provenance::Ai, false),
            ],
            vec![action("act old"), action("act done"), action("act edited")],
        )
        .unwrap();
    store
        .add_note_block(&m.gid, block("user wrote", Provenance::User, false))
        .unwrap();
    store
        .add_note_block(&m.gid, block("ai pinned", Provenance::Ai, true))
        .unwrap();
    let notes = store.note_blocks(&m.gid).unwrap();
    // Editing an AI block makes it ai_edited, which a regenerate keeps.
    store.update_note_block(&notes[1].gid, "ai edited").unwrap();
    let acts = store.action_items(&m.gid).unwrap();
    assert_eq!(acts[0].due_text.as_deref(), Some("thứ Sáu"));
    assert_eq!(acts[0].provenance, Provenance::Ai);
    store.set_action_done(&acts[1].gid, true).unwrap();
    store
        .update_action_item_text(&acts[2].gid, "act edited!")
        .unwrap();

    let r = store
        .replace_ai_notes(
            &m.gid,
            vec![block("ai new", Provenance::User, true)],
            vec![action("act new")],
        )
        .unwrap();
    assert_eq!((r.removed, r.kept, r.added), (2, 5, 2));
    let bodies: Vec<_> = store
        .note_blocks(&m.gid)
        .unwrap()
        .into_iter()
        .map(|b| (b.body, b.provenance, b.pinned))
        .collect();
    assert_eq!(
        bodies,
        vec![
            ("ai edited".to_string(), Provenance::AiEdited, false),
            ("user wrote".to_string(), Provenance::User, false),
            ("ai pinned".to_string(), Provenance::Ai, true),
            ("ai new".to_string(), Provenance::Ai, false),
        ]
    );
    let texts: Vec<_> = store
        .action_items(&m.gid)
        .unwrap()
        .into_iter()
        .map(|a| a.text)
        .collect();
    assert_eq!(texts, vec!["act done", "act edited!", "act new"]);
    assert!(store.is_tombstoned(&notes[0].gid).unwrap());
    assert!(store.is_tombstoned(&acts[0].gid).unwrap());

    assert!(!store.get_meeting(&m.gid).unwrap().cloud_used);
    store
        .record_cloud_request(&m.gid, "anthropic", "claude-x", 1200, 300)
        .unwrap();
    assert!(store.get_meeting(&m.gid).unwrap().cloud_used);
}

#[test]
fn crud_round_trip_with_encrypted_columns() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = store
        .create_meeting(NewMeeting {
            title: "Họp giao ban".into(),
            lang: Some("vi".into()),
            template: Some("standup".into()),
            sensitive: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(m.title, "Họp giao ban");
    assert!(m.sensitive && m.started_at > 0 && m.transcript_version == 1);

    let person = store.add_person("Ánh", 3).unwrap();
    let sp = store
        .add_speaker(
            &m.gid,
            NewSpeaker {
                label_idx: 0,
                display_name: Some("Ánh".into()),
                person_gid: Some(person.clone()),
                is_me: true,
                color_slot: 3,
            },
        )
        .unwrap();
    let seg = store
        .add_segment(
            &m.gid,
            NewSegment {
                gid: None,
                edited: false,
                speaker_gid: Some(sp.clone()),
                t0_ms: 100,
                t1_ms: 900,
                text: "Xin chào".into(),
                lang: Some("vi".into()),
                confidence: Some(0.9),
                words: vec![
                    Word {
                        t0_ms: 100,
                        t1_ms: 400,
                        conf: Some(0.8),
                    },
                    Word {
                        t0_ms: 400,
                        t1_ms: 900,
                        conf: None,
                    },
                ],
            },
        )
        .unwrap();
    assert_eq!(store.segment_words(&seg.gid).unwrap().len(), 2);
    let segs = store.segments(&m.gid).unwrap();
    assert_eq!(segs, vec![seg.clone()]);
    let speakers = store.speakers(&m.gid).unwrap();
    assert_eq!(speakers[0].person_gid.as_deref(), Some(person.as_str()));
    assert_eq!(speakers[0].display_name.as_deref(), Some("Ánh"));
    // The display name is ciphertext in the database.
    store.checkpoint().unwrap();
    let master = common::ring(&_k);
    let raw = ghi_store::db::open(&tmp.path().join("ghira.db"), &master.db_key()).unwrap();
    let ct: Vec<u8> = raw
        .query_row("SELECT display_name_ct FROM speakers", [], |r| r.get(0))
        .unwrap();
    assert!(!ct.windows("Ánh".len()).any(|w| w == "Ánh".as_bytes()));
    drop(raw);

    let anchor = store.anchor_for_segment(&m.gid, &seg).unwrap();
    let note = store
        .add_note_block(
            &m.gid,
            NewNoteBlock {
                kind: "bullet".into(),
                provenance: Provenance::Ai,
                body: "Chào hỏi".into(),
                anchors: vec![anchor.clone()],
                pinned: true,
            },
        )
        .unwrap();
    store
        .update_note_block(&note.gid, "Chào hỏi (đã sửa)")
        .unwrap();
    let notes = store.note_blocks(&m.gid).unwrap();
    assert_eq!(notes[0].body, "Chào hỏi (đã sửa)");
    assert_eq!(notes[0].provenance, Provenance::AiEdited);
    assert_eq!(notes[0].anchors, vec![anchor.clone()]);
    assert_eq!(store.search(&SearchQuery::new("da sua")).unwrap().len(), 1);

    let ai = store
        .add_action_item(
            &m.gid,
            NewActionItem {
                text: "Gửi báo cáo".into(),
                owner_speaker_gid: Some(sp),
                due: Some(42),
                anchor: Some(anchor),
                ..Default::default()
            },
        )
        .unwrap();
    store.set_action_done(&ai.gid, true).unwrap();
    let items = store.action_items(&m.gid).unwrap();
    assert!(items[0].done && items[0].text == "Gửi báo cáo" && items[0].due == Some(42));

    store.add_mark(&m.gid, 500, MarkTag::Star).unwrap();
    store.add_mark(&m.gid, 200, MarkTag::Question).unwrap();
    let marks = store.marks(&m.gid).unwrap();
    assert_eq!((marks[0].t_ms, marks[0].tag), (200, MarkTag::Question));

    store.set_meeting_title(&m.gid, "Họp đã đổi tên").unwrap();
    store.finish_meeting(&m.gid, 60_000).unwrap();
    let m2 = store.get_meeting(&m.gid).unwrap();
    assert_eq!(
        (m2.title.as_str(), m2.status.as_str(), m2.duration_ms),
        ("Họp đã đổi tên", "done", 60_000)
    );

    assert!(!m2.consent_confirmed);
    store.set_consent_confirmed(&m.gid, true).unwrap();
    assert!(store.get_meeting(&m.gid).unwrap().consent_confirmed);
    store.set_consent_confirmed(&m.gid, false).unwrap();
    assert!(!store.get_meeting(&m.gid).unwrap().consent_confirmed);
    assert!(store.set_consent_confirmed("nope", true).is_err());

    store.delete_note_block(&note.gid).unwrap();
    assert!(
        store
            .search(&SearchQuery::new("da sua"))
            .unwrap()
            .is_empty()
    );
    assert!(store.is_tombstoned(&note.gid).unwrap());

    store
        .set_setting("ui.theme", &json!({"mode": "dark"}))
        .unwrap();
    assert_eq!(
        store.get_setting("ui.theme").unwrap(),
        Some(json!({"mode": "dark"}))
    );
    assert!(store.set_setting("lamport", &json!(0)).is_err());
    assert!(matches!(
        store.get_meeting(&new_gid()),
        Err(StoreError::NotFound { .. })
    ));
}

#[test]
fn ciphertext_columns_never_hold_plaintext_and_are_bound_to_their_row() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "Tiêu đề bí mật");
    let a = store
        .add_segment(&m, common::seg(0, 1, "nội dung một"))
        .unwrap();
    let b = store
        .add_segment(&m, common::seg(1, 2, "nội dung hai"))
        .unwrap();
    store.checkpoint().unwrap();
    let conn = rusqlite::Connection::open(tmp.path().join("ghira.db")).unwrap();
    // Not readable without the key.
    assert!(
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r
            .get::<_, i64>(0))
            .is_err()
    );
    drop(conn);
    // Swap two ciphertexts inside the DB: the AAD binding rejects them.
    let master = common::ring(&_k);
    let raw = ghi_store::db::open(&tmp.path().join("ghira.db"), &master.db_key()).unwrap();
    raw.execute(
        "UPDATE segments SET text_ct = (SELECT text_ct FROM segments WHERE gid = ?1) WHERE gid = ?2",
        [&a.gid, &b.gid],
    )
    .unwrap();
    drop(raw);
    assert!(matches!(store.segments(&m), Err(StoreError::Decrypt)));
}

#[test]
fn anchors_resolve_by_time_across_transcript_versions() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "t");
    store
        .add_segments(
            &m,
            vec![
                common::seg(0, 3000, "live một"),
                common::seg(3000, 6000, "live hai"),
                common::seg(6000, 9000, "live ba"),
            ],
        )
        .unwrap();
    record(&store, &m, &[b"a", b"b"]);

    let anchor = store.anchor_for_range(&m, 2500, 3500).unwrap();
    let r = store.resolve_anchor(&anchor).unwrap();
    assert_eq!(
        r.segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>(),
        ["live một", "live hai"]
    );
    assert!(!r.stale && r.audio_available);

    // A range touching only a boundary overlaps nothing on the far side.
    let edge = store.anchor_for_range(&m, 3000, 6000).unwrap();
    assert_eq!(store.resolve_anchor(&edge).unwrap().segments.len(), 1);
    // A point resolves to the segment containing it.
    let point = store.anchor_for_range(&m, 6500, 6500).unwrap();
    assert_eq!(
        store.resolve_anchor(&point).unwrap().segments[0].text,
        "live ba"
    );

    // The final pass re-segments differently: the old anchor still lands on the same words.
    store
        .replace_transcript(
            &m,
            vec![
                common::seg(0, 3200, "bản cuối một hai"),
                common::seg(3200, 9000, "bản cuối ba"),
            ],
        )
        .unwrap();
    let r = store.resolve_anchor(&anchor).unwrap();
    assert!(r.stale);
    assert_eq!(
        r.segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>(),
        ["bản cuối một hai", "bản cuối ba"]
    );
}

#[test]
fn retention_deletes_audio_and_keeps_text() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let expired = store
        .create_meeting(NewMeeting {
            title: "hết hạn".into(),
            audio_retained_until: Some(1_000),
            ..Default::default()
        })
        .unwrap()
        .gid;
    let kept = store
        .create_meeting(NewMeeting {
            title: "còn hạn".into(),
            audio_retained_until: Some(9_000_000_000_000),
            ..Default::default()
        })
        .unwrap()
        .gid;
    let forever = common::meeting(&store, "vĩnh viễn");
    // Expired, but its final pass waits (models not installed yet).
    let waiting = store
        .create_meeting(NewMeeting {
            title: "chờ mô hình".into(),
            audio_retained_until: Some(1_000),
            ..Default::default()
        })
        .unwrap()
        .gid;
    store
        .enqueue_job(Some(&waiting), "final_pass", 1, &serde_json::json!({}))
        .unwrap();
    for g in [&expired, &kept, &forever, &waiting] {
        store
            .add_segment(g, common::seg(0, 1000, "văn bản vẫn còn"))
            .unwrap();
        record(&store, g, &[b"pcm"]);
    }
    let anchor = store.anchor_for_range(&expired, 0, 500).unwrap();

    let report = store.retention_sweep(2_000).unwrap();
    assert_eq!((report.meetings, report.tracks), (1, 1));
    assert!(!store.audio_available(&expired).unwrap());
    assert!(store.audio_available(&kept).unwrap() && store.audio_available(&forever).unwrap());
    assert!(store.audio_available(&waiting).unwrap());
    assert!(
        !store
            .bundle_path(&expired, TrackKind::Mic)
            .unwrap()
            .exists()
    );
    assert!(store.bundle_path(&kept, TrackKind::Mic).unwrap().exists());
    // Text stays, and citations resolve to text only.
    assert_eq!(store.segments(&expired).unwrap()[0].text, "văn bản vẫn còn");
    let r = store.resolve_anchor(&anchor).unwrap();
    assert_eq!(r.segments.len(), 1);
    assert!(!r.audio_available);
    assert_eq!(store.search(&SearchQuery::new("van ban")).unwrap().len(), 4);
    // Idempotent.
    assert_eq!(store.retention_sweep(2_000).unwrap().meetings, 0);
    assert!(store.open_bundle(&expired, TrackKind::Mic).is_err());
    assert_eq!(
        store
            .tombstones_since(0)
            .unwrap()
            .iter()
            .filter(|t| t.kind == "track")
            .count(),
        1
    );
}

#[test]
fn jobs_follow_the_state_machine() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "t");
    let a = store
        .enqueue_job(Some(&m), "final_pass", 1, &json!({"model": "x"}))
        .unwrap();
    let b = store
        .enqueue_job(
            Some(&m),
            "final_pass",
            2,
            &json!({"model": "y", "new": true}),
        )
        .unwrap();
    let c = store.enqueue_job(None, "embed", 1, &json!({})).unwrap();

    // A worker that understands payload v1 skips the v2 job and other kinds.
    let job = store.claim_next_job("final_pass", 1).unwrap().unwrap();
    assert_eq!((job.id, job.state, job.attempts), (a, JobState::Running, 1));
    assert_eq!(job.payload, json!({"model": "x"}));
    assert_eq!(job.meeting_gid.as_deref(), Some(m.as_str()));
    assert!(store.claim_next_job("final_pass", 1).unwrap().is_none());
    assert_eq!(store.job(b).unwrap().state, JobState::Queued);

    store.set_job_progress(a, 0.5).unwrap();
    assert_eq!(store.job(a).unwrap().progress, 0.5);
    assert!(
        store.set_job_progress(c, 0.1).is_err(),
        "only running jobs have progress"
    );

    // Illegal moves are refused.
    assert!(store.complete_job(c).is_err(), "queued -> done");
    assert!(store.retry_job(a).is_err(), "running -> queued via retry");

    store.fail_job(a).unwrap();
    store.retry_job(a).unwrap();
    let again = store.claim_next_job("final_pass", 2).unwrap().unwrap();
    assert_eq!((again.id, again.attempts), (a, 2));
    store.complete_job(a).unwrap();
    let done = store.job(a).unwrap();
    assert_eq!((done.state, done.progress), (JobState::Done, 1.0));
    assert!(store.cancel_job(a).is_err(), "done is final");
    store.cancel_job(c).unwrap();
    assert!(store.retry_job(c).is_err(), "cancelled is final");
    assert_eq!(store.jobs_for_meeting(&m).unwrap().len(), 2);
}

#[test]
fn interrupted_jobs_are_requeued_on_open() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let id = store.enqueue_job(None, "embed", 1, &json!({})).unwrap();
    store.claim_next_job("embed", 1).unwrap().unwrap();
    drop(store); // "crash" while running
    let store = common::reopen(tmp.path(), &keys);
    assert_eq!(store.job(id).unwrap().state, JobState::Queued);
    assert_eq!(store.job(id).unwrap().attempts, 1);
}

#[test]
fn a_track_left_open_by_a_crash_is_recovered_on_open() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = common::meeting(&store, "t");
    let mut w = store.open_track(&m, TrackKind::Mic).unwrap();
    for i in 0..4u8 {
        w.append(&[i; 100]).unwrap();
    }
    w.sync(true).unwrap();
    std::mem::forget(w); // no finish(): the process "died"
    drop(store);

    let store = common::reopen(tmp.path(), &keys);
    assert_eq!(store.tracks(&m).unwrap(), [(TrackKind::Mic, 4)]);
    let r = store.open_bundle(&m, TrackKind::Mic).unwrap();
    assert_eq!(r.page(2).unwrap(), vec![2u8; 100]);
}

#[test]
fn one_track_per_kind() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "t");
    let _w = store.open_track(&m, TrackKind::Mic).unwrap();
    assert!(matches!(
        store.open_track(&m, TrackKind::Mic),
        Err(StoreError::Invalid(_))
    ));
    assert!(store.open_track(&m, TrackKind::System).is_ok());
}

#[test]
fn export_all_and_import_on_a_new_device() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(&tmp.path().join("old"));
    let m = common::meeting(&store, "Họp chuyển máy");
    store
        .add_segment(&m, common::seg(0, 1000, "Dữ liệu cần mang theo"))
        .unwrap();
    record(&store, &m, &[b"one", b"two"]);
    let gone = common::meeting(&store, "đã xóa");
    store.delete_meeting(&gone).unwrap();

    let archive = tmp.path().join("everything.ghx");
    store
        .export_all(&archive, "correct horse battery staple")
        .unwrap();
    let bytes = std::fs::read(&archive).unwrap();
    assert!(
        !bytes.windows(8).any(|w| w == b"SQLite f"),
        "archive is encrypted"
    );
    // No staging left behind.
    assert!(std::fs::read_dir(store.dir()).unwrap().all(|e| {
        !e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".export")
    }));

    let new_keys = common::keys();
    let dest = tmp.path().join("new");
    assert!(matches!(
        Store::import_archive(
            &archive,
            "wrong password",
            &dest,
            new_keys.clone(),
            Protection::default()
        ),
        Err(StoreError::Decrypt)
    ));
    assert!(new_keys.load().unwrap().is_none());

    // Never replaces an existing key or writes into existing data.
    let occupied = common::keys_with(ghi_store::keys::KeyRing::generate());
    let before = occupied.load().unwrap();
    let err = Store::import_archive(
        &archive,
        "correct horse battery staple",
        &tmp.path().join("other"),
        occupied.clone(),
        Protection::default(),
    );
    assert!(
        matches!(err, Err(StoreError::Invalid(ref m)) if m.contains("already has a Ghira key"))
    );
    assert!(occupied.load().unwrap() == before, "existing key untouched");
    assert!(!tmp.path().join("other").exists());
    let err = Store::import_archive(
        &archive,
        "correct horse battery staple",
        store.dir(),
        new_keys.clone(),
        Protection::default(),
    );
    assert!(matches!(err, Err(StoreError::Invalid(ref m)) if m.contains("not empty")));
    assert!(new_keys.load().unwrap().is_none());

    let moved = Store::import_archive(
        &archive,
        "correct horse battery staple",
        &dest,
        new_keys.clone(),
        Protection::default(),
    )
    .unwrap();
    assert!(
        new_keys.load().unwrap().is_some(),
        "the new device's keystore now holds the key"
    );
    assert!(!dest.join("master.key").exists() && !dest.join("keyring.bin").exists());
    assert_eq!(moved.get_meeting(&m).unwrap().title, "Họp chuyển máy");
    assert_eq!(
        moved.search(&SearchQuery::new("mang theo")).unwrap().len(),
        1
    );
    assert_eq!(
        moved
            .open_bundle(&m, TrackKind::Mic)
            .unwrap()
            .read_all()
            .unwrap(),
        b"onetwo"
    );
    assert!(matches!(
        moved.get_meeting(&gone),
        Err(StoreError::NotFound { .. })
    ));
    assert!(moved.is_tombstoned(&gone).unwrap());
}

#[test]
fn recovery_phrase_restores_after_the_keychain_is_lost() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = common::meeting(&store, "Cần khôi phục");
    store
        .add_segment(&m, common::seg(0, 1, "vẫn đọc được"))
        .unwrap();
    assert!(!store.has_recovery_phrase());
    let phrase = RecoveryPhrase::generate();
    store.set_recovery_phrase(&phrase).unwrap();
    assert!(store.has_recovery_phrase());
    drop(store);

    drop(keys); // new machine / wiped keychain
    // Without the key the data is unreadable.
    let fresh = common::keys();
    let err = Store::open(tmp.path(), fresh.clone(), Protection::default())
        .err()
        .unwrap();
    assert!(
        matches!(&err, StoreError::Invalid(m) if m.contains("recovery phrase")),
        "{err}"
    );
    assert!(
        fresh.load().unwrap().is_none(),
        "no useless key was created"
    );

    let words = phrase.words().join(" ");
    let wrong = RecoveryPhrase::generate();
    assert!(matches!(
        Store::restore_with_phrase(tmp.path(), &wrong, fresh.clone(), Protection::default()),
        Err(StoreError::Decrypt)
    ));
    let store = Store::restore_with_phrase(
        tmp.path(),
        &RecoveryPhrase::parse(&words).unwrap(),
        fresh.clone(),
        Protection::default(),
    )
    .unwrap();
    assert_eq!(store.segments(&m).unwrap()[0].text, "vẫn đọc được");
    assert!(fresh.load().unwrap().is_some());
}

#[test]
fn data_dir_is_excluded_from_backups() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(&tmp.path().join("data"));
    assert!(ghi_store::backup::is_excluded_from_backup(store.dir()).unwrap());
}

#[test]
fn job_payloads_carry_no_user_content() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "t");
    let seg = new_gid();
    // Gids, numbers, flags and short identifiers are fine.
    store
        .enqueue_job(
            Some(&m),
            "summarize",
            1,
            &json!({"segment": seg, "model": "qwen3-4b", "n": 3, "ids": [seg, seg]}),
        )
        .unwrap();
    // Free text, even a single word with accents or spaces, is refused.
    for bad in [
        json!({"text": "Ngân sách tuyệt mật"}),
        json!({"title": "Họp"}),
        json!({"nested": {"note": "hello world"}}),
        json!({"list": ["ok", "not ok"]}),
        json!({"key with space": 1}),
        json!("x".repeat(65)),
    ] {
        assert!(
            matches!(
                store.enqueue_job(Some(&m), "k", 1, &bad),
                Err(StoreError::Invalid(_))
            ),
            "{bad}"
        );
    }
}

#[test]
fn a_truncated_finished_bundle_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "t");
    record(&store, &m, &[b"a", b"b", b"c"]);
    assert_eq!(store.tracks(&m).unwrap(), [(TrackKind::Mic, 3)]);
    assert_eq!(
        store.open_bundle(&m, TrackKind::Mic).unwrap().page_count(),
        3
    );

    // The bundle is swapped for an honestly finished but shorter one (a rollback attack).
    let path = store.bundle_path(&m, TrackKind::Mic).unwrap();
    let track_gid: String = {
        let master = common::ring(&_k);
        let raw = ghi_store::db::open(&tmp.path().join("ghira.db"), &master.db_key()).unwrap();
        raw.query_row("SELECT gid FROM tracks", [], |r| r.get(0))
            .unwrap()
    };
    let dek = {
        let master = common::ring(&_k);
        let raw = ghi_store::db::open(&tmp.path().join("ghira.db"), &master.db_key()).unwrap();
        let w: Vec<u8> = raw
            .query_row("SELECT dek_wrapped FROM meetings", [], |r| r.get(0))
            .unwrap();
        master.unwrap_dek(&w, &m).unwrap()
    };
    std::fs::remove_file(&path).unwrap();
    let _ = std::fs::remove_file(ghi_store::bundle::index_path(&path));
    let mut w =
        ghi_store::bundle::BundleWriter::create(&path, &dek, &Store::bundle_aad(&track_gid))
            .unwrap();
    w.append(b"a").unwrap();
    w.finish().unwrap();
    assert!(matches!(
        store.open_bundle(&m, TrackKind::Mic),
        Err(StoreError::Decrypt)
    ));
}

#[test]
fn jobs_stop_after_the_attempts_limit() {
    use ghi_store::jobs::MAX_ATTEMPTS;
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let id = store.enqueue_job(None, "embed", 1, &json!({})).unwrap();
    for n in 1..=MAX_ATTEMPTS {
        let job = store.claim_next_job("embed", 1).unwrap().unwrap();
        assert_eq!(job.attempts, n);
        store.fail_job(id).unwrap();
        if n < MAX_ATTEMPTS {
            store.retry_job(id).unwrap();
        }
    }
    assert!(
        matches!(store.retry_job(id), Err(StoreError::Invalid(_))),
        "exhausted"
    );
    assert!(store.claim_next_job("embed", 1).unwrap().is_none());
    // A crash on the last allowed attempt ends as failed, not queued.
    let id2 = store.enqueue_job(None, "other", 1, &json!({})).unwrap();
    for _ in 0..MAX_ATTEMPTS {
        store.claim_next_job("other", 1).unwrap().unwrap();
        store.fail_job(id2).unwrap();
        let _ = store.retry_job(id2);
    }
    drop(store);
    let store = common::reopen(tmp.path(), &keys);
    assert_eq!(store.job(id2).unwrap().state, JobState::Failed);
}

#[test]
fn backup_inclusion_is_a_setting_that_open_respects() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("data");
    let (store, keys) = common::open(&dir);
    assert!(ghi_store::backup::is_excluded_from_backup(&dir).unwrap());
    store.set_include_in_backups(true).unwrap();
    assert!(!ghi_store::backup::is_excluded_from_backup(&dir).unwrap());
    drop(store);
    let store = common::reopen(&dir, &keys);
    assert!(
        !ghi_store::backup::is_excluded_from_backup(&dir).unwrap(),
        "open keeps the choice"
    );
    store.set_include_in_backups(false).unwrap();
    assert!(ghi_store::backup::is_excluded_from_backup(&dir).unwrap());
}

#[test]
fn crafted_gids_never_reach_the_filesystem() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("data");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(outside.join("victim")).unwrap();
    std::fs::write(outside.join("victim").join("keep.txt"), b"keep").unwrap();
    let (store, keys) = common::open(&dir);
    let m = store
        .create_meeting(NewMeeting {
            title: "x".into(),
            audio_retained_until: Some(1),
            ..Default::default()
        })
        .unwrap()
        .gid;
    record(&store, &m, &[b"a"]);
    store.checkpoint().unwrap();
    // Malicious database content: the gid becomes a path traversal.
    let evil = "../../outside/victim".to_string();
    let raw = ghi_store::db::open(&dir.join("ghira.db"), &common::db_key(&keys)).unwrap();
    raw.execute("UPDATE meetings SET gid = ?1", [&evil])
        .unwrap();
    drop(raw);
    drop(store);

    let store = common::reopen(&dir, &keys);
    // The sweep skips the bad row (one bad row must not stop it for the rest).
    assert_eq!(store.retention_sweep(10).unwrap().meetings, 0);
    for r in [
        store.bundle_path(&evil, TrackKind::Mic).map(|_| ()),
        store.delete_meeting(&evil),
        store.open_track(&evil, TrackKind::System).map(|_| ()),
    ] {
        assert!(matches!(r, Err(StoreError::Invalid(_))), "{r:?}");
    }
    assert_eq!(
        std::fs::read(outside.join("victim").join("keep.txt")).unwrap(),
        b"keep"
    );
    // Same for an imported archive: a database with such a gid is refused, nothing is moved.
    let archive = tmp.path().join("evil.ghx");
    store.export_all(&archive, "pw").unwrap();
    let keys2 = common::keys();
    let dest = tmp.path().join("imported");
    let err = Store::import_archive(&archive, "pw", &dest, keys2.clone(), Protection::default())
        .err()
        .unwrap();
    assert!(matches!(err, StoreError::Invalid(_)), "{err}");
    assert!(!dest.exists());
    assert!(
        keys2.load().unwrap().is_none(),
        "no key saved for a refused import"
    );
    assert_eq!(
        std::fs::read(outside.join("victim").join("keep.txt")).unwrap(),
        b"keep"
    );
}

#[test]
fn ghi_bundle_path_signature_is_a_result() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "t");
    let p: Result<std::path::PathBuf, StoreError> = store.bundle_path(&m, TrackKind::File);
    assert!(p.unwrap().ends_with(format!("{m}/file.ghb")));
}
