// SPDX-License-Identifier: Apache-2.0
//! Organizing meetings (phase 14d): migration 0007, folders, tags, sealed
//! per-meeting facts, overlap marks and the search filters.

mod common;

use ghi_store::migrate::{self, MIGRATIONS};
use ghi_store::organize::{
    MAX_FOLDER_NAME, MAX_FOLDERS, MAX_TAG_NAME, MAX_TAGS, MAX_TAGS_PER_MEETING, TrackSpeaker,
};
use ghi_store::search::{SearchFilter, SearchQuery};
use ghi_store::store::NewSpeaker;
use ghi_store::{StoreError, db};

fn raw(dir: &std::path::Path, keys: &common::Keys) -> rusqlite::Connection {
    db::open(&dir.join("ghira.db"), &common::db_key(keys)).unwrap()
}

fn lamport_of(conn: &rusqlite::Connection, meeting: &str) -> i64 {
    conn.query_row(
        "SELECT lamport FROM meetings WHERE gid = ?1",
        [meeting],
        |r| r.get(0),
    )
    .unwrap()
}

fn strs(v: &[&String]) -> Vec<String> {
    v.iter().map(|s| (*s).clone()).collect()
}

#[test]
fn migration_v7_keeps_old_meetings_and_adds_empty_organization() {
    let tmp = tempfile::tempdir().unwrap();
    let keys = common::keys();
    drop(common::open_with(tmp.path(), &keys, &MIGRATIONS[..6]).unwrap());
    let path = tmp.path().join("ghira.db");
    let conn = db::open(&path, &common::db_key(&keys)).unwrap();
    assert_eq!(db::user_version(&conn).unwrap(), 6);
    let gid = ghi_store::new_gid();
    let wrapped = common::ring(&keys).wrap_dek(&ghi_store::rowcrypt::Dek::generate(), &gid);
    conn.execute(
        "INSERT INTO meetings (gid, started_at, dek_wrapped) VALUES (?1, 1700000000000, ?2)",
        rusqlite::params![gid, wrapped],
    )
    .unwrap();
    let mid = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO segments (gid, meeting_id, version, t0_ms, t1_ms, text_ct)
         VALUES ('seg', ?1, 1, 0, 10, x'00')",
        [mid],
    )
    .unwrap();
    drop(conn);

    let store = common::open_with(tmp.path(), &keys, MIGRATIONS).unwrap();
    let m = store.get_meeting(&gid).unwrap();
    assert_eq!((m.folder_gid, m.source_app), (None, None));
    assert!(store.folders().unwrap().is_empty() && store.tags().unwrap().is_empty());
    assert_eq!(store.calendar_info(&gid).unwrap(), None);
    assert!(store.track_speakers(&gid).unwrap().is_empty());
    drop(store);
    let conn = db::open(&path, &common::db_key(&keys)).unwrap();
    assert_eq!(db::user_version(&conn).unwrap(), migrate::latest_version());
    let overlap: i64 = conn
        .query_row("SELECT overlap FROM segments WHERE gid = 'seg'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(overlap, 0);
    for ix in ["meetings_folder", "meeting_tags_tag"] {
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = ?1",
                [ix],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "{ix}");
    }
}

#[test]
fn folders_are_unique_by_lowercase_name_and_bounded() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let hop = store.create_folder("  Họp   tuần ").unwrap();
    assert_eq!((hop.name.as_str(), hop.meetings), ("Họp tuần", 0));
    for dup in ["họp tuần", "HOP TUAN", "Hop  Tuan"] {
        assert!(matches!(
            store.create_folder(dup),
            Err(StoreError::Duplicate { kind: "folder" })
        ));
    }
    let other = store.create_folder("Khách hàng").unwrap();
    // Respelling a folder is fine; taking another's name is not.
    store.rename_folder(&hop.gid, "họp tuần").unwrap();
    store.rename_folder(&hop.gid, "Họp tuần 2").unwrap();
    assert!(matches!(
        store.rename_folder(&other.gid, "HỌP TUẦN 2"),
        Err(StoreError::Duplicate { .. })
    ));
    let names: Vec<_> = store
        .folders()
        .unwrap()
        .into_iter()
        .map(|f| f.name)
        .collect();
    assert_eq!(names, ["Họp tuần 2", "Khách hàng"]);
    // Names are limited.
    assert!(store.create_folder("   ").is_err());
    assert!(
        store
            .create_folder(&"x".repeat(MAX_FOLDER_NAME + 1))
            .is_err()
    );
    assert!(store.create_folder(&"x".repeat(MAX_FOLDER_NAME)).is_ok());
    assert!(store.rename_folder("nope", "A").is_err());
}

#[test]
fn moving_meetings_and_deleting_a_folder_bumps_lamport_and_tombstones() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let (a, b, c) = (
        common::meeting(&store, "A"),
        common::meeting(&store, "B"),
        common::meeting(&store, "C"),
    );
    let f = store.create_folder("Dự án").unwrap();
    let g = store.create_folder("Khác").unwrap();
    assert_eq!(
        store
            .set_meeting_folder(&[a.clone(), b.clone()], Some(&f.gid))
            .unwrap(),
        2
    );
    assert_eq!(
        store
            .set_meeting_folder(std::slice::from_ref(&a), Some(&f.gid))
            .unwrap(),
        0,
        "no change"
    );
    store
        .set_meeting_folder(std::slice::from_ref(&c), Some(&g.gid))
        .unwrap();
    assert_eq!(
        store.get_meeting(&a).unwrap().folder_gid.as_deref(),
        Some(f.gid.as_str())
    );
    let counts: Vec<_> = store
        .folders()
        .unwrap()
        .into_iter()
        .map(|f| (f.name, f.meetings))
        .collect();
    assert_eq!(counts, [("Dự án".to_string(), 2), ("Khác".to_string(), 1)]);
    assert!(
        store
            .set_meeting_folder(std::slice::from_ref(&a), Some("nope"))
            .is_err()
    );
    assert!(
        store
            .set_meeting_folder(&["nope".into()], Some(&f.gid))
            .is_err()
    );

    let conn = raw(tmp.path(), &keys);
    let (la, lc) = (lamport_of(&conn, &a), lamport_of(&conn, &c));
    drop(conn);
    assert_eq!(store.delete_folder(&f.gid).unwrap(), 2);
    assert!(store.is_tombstoned(&f.gid).unwrap());
    assert_eq!(store.get_meeting(&a).unwrap().folder_gid, None);
    assert_eq!(
        store.get_meeting(&c).unwrap().folder_gid.as_deref(),
        Some(g.gid.as_str())
    );
    let conn = raw(tmp.path(), &keys);
    assert!(
        lamport_of(&conn, &a) > la,
        "the cleared meetings' lamport moved"
    );
    assert_eq!(lamport_of(&conn, &c), lc, "others did not");
    drop(conn);
    // Moving out of a folder.
    store
        .set_meeting_folder(std::slice::from_ref(&c), None)
        .unwrap();
    assert_eq!(store.get_meeting(&c).unwrap().folder_gid, None);
    // The tombstone is in the sync stream.
    assert!(
        store
            .tombstones_since(0)
            .unwrap()
            .iter()
            .any(|t| t.gid == f.gid && t.kind == "folder")
    );
}

#[test]
fn tags_get_or_create_dedupe_limit_and_tombstone_their_links() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let (a, b) = (common::meeting(&store, "A"), common::meeting(&store, "B"));
    let t1 = store.create_tag("Quan trọng").unwrap();
    assert_eq!(
        store.create_tag(" quan trong ").unwrap().gid,
        t1.gid,
        "get-or-create, folded"
    );
    let t2 = store.create_tag("Họp khách").unwrap();
    assert!(store.create_tag(&"x".repeat(MAX_TAG_NAME + 1)).is_err());
    assert!(matches!(
        store.rename_tag(&t2.gid, "QUAN TRỌNG"),
        Err(StoreError::Duplicate { kind: "tag" })
    ));
    store.rename_tag(&t2.gid, "Họp Khách").unwrap();

    assert_eq!(
        store
            .tag_meetings(&[a.clone(), b.clone()], &t1.gid)
            .unwrap(),
        2
    );
    assert_eq!(
        store
            .tag_meetings(std::slice::from_ref(&a), &t1.gid)
            .unwrap(),
        0,
        "already tagged"
    );
    store
        .tag_meetings(std::slice::from_ref(&a), &t2.gid)
        .unwrap();
    let tags = store
        .meeting_tags(&[a.clone(), b.clone(), "nope".into()])
        .unwrap();
    let names = |m: &str| tags[m].iter().map(|t| t.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&a), ["Họp Khách", "Quan trọng"], "by name");
    assert_eq!(names(&b), ["Quan trọng"]);
    assert!(!tags.contains_key("nope"));
    let counts: Vec<_> = store
        .tags()
        .unwrap()
        .into_iter()
        .map(|t| (t.name, t.meetings))
        .collect();
    assert_eq!(
        counts,
        [("Họp Khách".to_string(), 1), ("Quan trọng".to_string(), 2)]
    );

    // Untag: tombstone of that link; tagging again makes a fresh link gid.
    let link = |conn: &rusqlite::Connection, m: &str, t: &str| -> String {
        conn.query_row(
            "SELECT mt.gid FROM meeting_tags mt JOIN meetings m ON m.id = mt.meeting_id
             JOIN tags t ON t.id = mt.tag_id WHERE m.gid = ?1 AND t.gid = ?2",
            [m, t],
            |r| r.get(0),
        )
        .unwrap()
    };
    let old = link(&raw(tmp.path(), &keys), &a, &t1.gid);
    assert_eq!(
        store
            .untag_meetings(&[a.clone(), b.clone()], &t2.gid)
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .untag_meetings(std::slice::from_ref(&a), &t1.gid)
            .unwrap(),
        1
    );
    assert!(store.is_tombstoned(&old).unwrap());
    store
        .tag_meetings(std::slice::from_ref(&a), &t1.gid)
        .unwrap();
    let fresh = link(&raw(tmp.path(), &keys), &a, &t1.gid);
    assert_ne!(old, fresh);
    assert!(!store.is_tombstoned(&fresh).unwrap());

    // Deleting a tag tombstones the tag and every link.
    let links = [
        link(&raw(tmp.path(), &keys), &a, &t1.gid),
        link(&raw(tmp.path(), &keys), &b, &t1.gid),
    ];
    assert_eq!(store.delete_tag(&t1.gid).unwrap(), 2);
    assert!(store.is_tombstoned(&t1.gid).unwrap());
    assert!(links.iter().all(|l| store.is_tombstoned(l).unwrap()));
    assert!(
        store
            .meeting_tags(&[a.clone(), b.clone()])
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.tags().unwrap().len(), 1);
}

#[test]
fn a_meeting_holds_at_most_twenty_tags_and_deleting_it_tombstones_its_links() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let tags: Vec<_> = (0..=MAX_TAGS_PER_MEETING)
        .map(|i| store.create_tag(&format!("tag {i}")).unwrap())
        .collect();
    for t in &tags[..MAX_TAGS_PER_MEETING] {
        store
            .tag_meetings(std::slice::from_ref(&m), &t.gid)
            .unwrap();
    }
    assert!(matches!(
        store.tag_meetings(std::slice::from_ref(&m), &tags[MAX_TAGS_PER_MEETING].gid),
        Err(StoreError::Limit {
            kind: "tags per meeting",
            max: MAX_TAGS_PER_MEETING
        })
    ));
    assert_eq!(
        store.meeting_tags(std::slice::from_ref(&m)).unwrap()[&m].len(),
        MAX_TAGS_PER_MEETING
    );

    let links: Vec<String> = raw(tmp.path(), &keys)
        .prepare("SELECT gid FROM meeting_tags")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(links.len(), MAX_TAGS_PER_MEETING);
    store.delete_meeting(&m).unwrap();
    assert!(links.iter().all(|l| store.is_tombstoned(l).unwrap()));
    // The tags themselves stay for other meetings.
    assert_eq!(store.tags().unwrap().len(), MAX_TAGS_PER_MEETING + 1);
    assert!(store.tags().unwrap().iter().all(|t| t.meetings == 0));
}

#[test]
fn source_app_is_validated_and_stored() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    store.set_source_app(&m, Some("zoom")).unwrap();
    assert_eq!(
        store.get_meeting(&m).unwrap().source_app.as_deref(),
        Some("zoom")
    );
    assert!(store.set_source_app(&m, Some("skype")).is_err());
    store.set_source_app(&m, None).unwrap();
    assert_eq!(store.get_meeting(&m).unwrap().source_app, None);
    assert!(matches!(
        store.set_source_app("nope", Some("meet")),
        Err(StoreError::NotFound { .. })
    ));
}

#[test]
fn calendar_info_and_track_speakers_are_sealed_and_shredded_with_the_meeting() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let sp = store
        .add_speaker(
            &m,
            NewSpeaker {
                label_idx: 0,
                ..Default::default()
            },
        )
        .unwrap();
    let info = serde_json::json!({
        "event": "Họp ngân sách quý bốn BÍ-MẬT",
        "attendees": ["an@example.com", "Bình"],
        "calendar": "Work",
    });
    store.set_calendar_info(&m, Some(&info)).unwrap();
    assert_eq!(store.calendar_info(&m).unwrap(), Some(info.clone()));
    let spans = vec![TrackSpeaker {
        label: "Nguyễn Văn An".into(),
        speaker_gid: sp.clone(),
        spans: vec![[0, 4000], [9000, 12_000]],
    }];
    store.set_track_speakers(&m, &spans).unwrap();
    assert_eq!(store.track_speakers(&m).unwrap(), spans);

    // On disk: ciphertext only.
    store.checkpoint().unwrap();
    let conn = raw(tmp.path(), &keys);
    let (cal, tr): (Vec<u8>, Vec<u8>) = conn
        .query_row(
            "SELECT calendar_ct, track_speakers_ct FROM meetings",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let has =
        |blob: &[u8], needle: &str| blob.windows(needle.len()).any(|w| w == needle.as_bytes());
    assert!(!has(&cal, "BÍ-MẬT") && !has(&cal, "Work"));
    assert!(!has(&tr, "Nguyễn"));
    // Swapped into the other column, they don't open (the AAD binds the column).
    conn.execute("UPDATE meetings SET calendar_ct = track_speakers_ct", [])
        .unwrap();
    drop(conn);
    let store2 = {
        drop(store);
        common::reopen(tmp.path(), &keys)
    };
    assert!(matches!(store2.calendar_info(&m), Err(StoreError::Decrypt)));
    // Clearing.
    store2.set_calendar_info(&m, None).unwrap();
    store2.set_track_speakers(&m, &[]).unwrap();
    assert_eq!(store2.calendar_info(&m).unwrap(), None);
    assert!(store2.track_speakers(&m).unwrap().is_empty());

    // Shred: with the meeting's key gone, they can't be read.
    store2.set_calendar_info(&m, Some(&info)).unwrap();
    store2.set_track_speakers(&m, &spans).unwrap();
    store2.shred_key(&m).unwrap();
    assert!(matches!(store2.calendar_info(&m), Err(StoreError::Decrypt)));
    assert!(matches!(
        store2.track_speakers(&m),
        Err(StoreError::Decrypt)
    ));
    store2.delete_meeting(&m).unwrap();
    assert!(store2.calendar_info(&m).is_err());
}

#[test]
fn search_narrows_by_folder_and_any_of_tags() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let mk = |t: &str| {
        let g = common::meeting(&store, t);
        store
            .add_segments(&g, vec![common::seg(0, 1000, "Chốt ngân sách quý bốn")])
            .unwrap();
        g
    };
    let (a, b, c) = (mk("A"), mk("B"), mk("C"));
    let f = store.create_folder("Dự án").unwrap();
    store
        .set_meeting_folder(&[a.clone(), b.clone()], Some(&f.gid))
        .unwrap();
    let (t1, t2) = (
        store.create_tag("một").unwrap(),
        store.create_tag("hai").unwrap(),
    );
    store
        .tag_meetings(std::slice::from_ref(&a), &t1.gid)
        .unwrap();
    store
        .tag_meetings(std::slice::from_ref(&c), &t2.gid)
        .unwrap();
    store
        .add_note_block(
            &c,
            ghi_store::store::NewNoteBlock {
                kind: "bullet".into(),
                provenance: ghi_store::store::Provenance::User,
                body: "ngân sách trong ghi chú".into(),
                anchors: vec![],
                pinned: false,
            },
        )
        .unwrap();
    let run = |filter: SearchFilter| -> Vec<String> {
        let mut q = SearchQuery::new("ngan sach");
        q.filter = filter;
        let mut v: Vec<String> = store
            .search(&q)
            .unwrap()
            .into_iter()
            .map(|h| h.meeting_gid)
            .collect();
        v.sort();
        v.dedup();
        v
    };
    let sorted = |v: &[&String]| {
        let mut s = strs(v);
        s.sort();
        s
    };
    assert_eq!(run(SearchFilter::default()), sorted(&[&a, &b, &c]));
    assert_eq!(
        run(SearchFilter {
            folder: Some(f.gid.clone()),
            ..Default::default()
        }),
        sorted(&[&a, &b])
    );
    assert_eq!(
        run(SearchFilter {
            tags: vec![t1.gid.clone()],
            ..Default::default()
        }),
        sorted(&[&a])
    );
    // Any-of.
    assert_eq!(
        run(SearchFilter {
            tags: vec![t1.gid.clone(), t2.gid.clone()],
            ..Default::default()
        }),
        sorted(&[&a, &c])
    );
    // Combined (and notes obey the filter too).
    assert_eq!(
        run(SearchFilter {
            folder: Some(f.gid.clone()),
            tags: vec![t1.gid.clone(), t2.gid.clone()],
            ..Default::default()
        }),
        sorted(&[&a])
    );
    assert!(
        run(SearchFilter {
            folder: Some("nope".into()),
            ..Default::default()
        })
        .is_empty()
    );
    assert!(
        run(SearchFilter {
            tags: vec!["nope".into()],
            ..Default::default()
        })
        .is_empty()
    );
}

#[test]
fn overlap_marks_round_trip_and_stay_inside_the_meeting() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let other = common::meeting(&store, "N");
    let segs = store
        .add_segments(
            &m,
            vec![
                common::seg(0, 1000, "một"),
                common::seg(1000, 2000, "hai"),
                common::seg(2000, 3000, "ba"),
            ],
        )
        .unwrap();
    let theirs = store
        .add_segment(&other, common::seg(0, 1000, "khác"))
        .unwrap();
    assert!(store.segments(&m).unwrap().iter().all(|s| !s.overlap));
    assert_eq!(store.mark_overlaps(&m, &[]).unwrap(), 0);
    let marked = store
        .mark_overlaps(
            &m,
            &[
                segs[0].gid.clone(),
                segs[2].gid.clone(),
                theirs.gid.clone(),
                "nope".into(),
            ],
        )
        .unwrap();
    assert_eq!(marked, 2, "the other meeting's line is not touched");
    let flags: Vec<bool> = store
        .segments(&m)
        .unwrap()
        .iter()
        .map(|s| s.overlap)
        .collect();
    assert_eq!(flags, [true, false, true]);
    assert!(!store.segments(&other).unwrap()[0].overlap);
    assert_eq!(
        store.mark_overlaps(&m, &[segs[0].gid.clone()]).unwrap(),
        0,
        "idempotent"
    );
    // Survives a reopen; a new transcript version starts clean.
    drop(store);
    let store = common::reopen(tmp.path(), &keys);
    assert!(store.segments(&m).unwrap()[0].overlap);
    store
        .replace_transcript(&m, vec![common::seg(0, 500, "bản mới")])
        .unwrap();
    assert!(!store.segments(&m).unwrap()[0].overlap);
}

// ------------------------------------------------------- review follow-ups

#[test]
fn accents_make_names_distinct_but_a_lone_unaccented_match_is_reused() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    // Họp and Hộp are two tags; Ban, Bàn and Bạn three.
    let hop = store.create_tag("Họp").unwrap();
    let hop2 = store.create_tag("Hộp").unwrap();
    assert_ne!(hop.gid, hop2.gid);
    let ban: Vec<_> = ["Ban", "Bàn", "Bạn"]
        .iter()
        .map(|n| store.create_tag(n).unwrap().gid)
        .collect();
    assert!(ban[0] != ban[1] && ban[1] != ban[2] && ban[0] != ban[2]);
    // Exact (case and NFD-insensitive) matches return the same tag.
    assert_eq!(store.create_tag("HỌP").unwrap().gid, hop.gid);
    assert_eq!(store.create_tag("Ho\u{323}p").unwrap().gid, hop.gid);
    assert!(matches!(
        store.rename_tag(&hop2.gid, "họp"),
        Err(StoreError::Duplicate { kind: "tag" })
    ));

    // "Tuần" is alone under its unaccented form: "tuan" reuses it.
    let tuan = store.create_tag("Tuần").unwrap();
    assert_eq!(store.create_tag("tuan").unwrap().gid, tuan.gid);
    assert_eq!(store.create_tag(" TUAN ").unwrap().gid, tuan.gid);
    // With two candidates ("Ban" exists exactly, so that wins; use "hop"):
    // "hop" fits both Họp and Hộp, so it is a new name, and then exact.
    let plain = store.create_tag("hop").unwrap();
    assert!(plain.gid != hop.gid && plain.gid != hop2.gid);
    assert_eq!(plain.name, "hop");
    assert_eq!(store.create_tag("HOP").unwrap().gid, plain.gid);
    // A typed name that has accents never falls back.
    assert_ne!(store.create_tag("Hợp").unwrap().gid, hop.gid);

    // Folders: the lone candidate makes "hop tuan" a duplicate; two do not.
    store.create_folder("Họp tuần").unwrap();
    assert!(matches!(
        store.create_folder("hop tuan"),
        Err(StoreError::Duplicate { kind: "folder" })
    ));
    store.create_folder("Hộp tuần").unwrap();
    assert!(store.create_folder("hop tuan").is_ok());
    assert!(matches!(
        store.create_folder("Hộp TUẦN"),
        Err(StoreError::Duplicate { .. })
    ));
}

#[test]
fn tagging_does_not_move_the_meeting_lamport_but_moving_does() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let m = common::meeting(&store, "M");
    let tag = store.create_tag("t").unwrap();
    let f = store.create_folder("f").unwrap();
    let lamport = || lamport_of(&raw(tmp.path(), &keys), &m);
    let start = lamport();
    store
        .tag_meetings(std::slice::from_ref(&m), &tag.gid)
        .unwrap();
    assert_eq!(lamport(), start, "tagging");
    store
        .untag_meetings(std::slice::from_ref(&m), &tag.gid)
        .unwrap();
    assert_eq!(lamport(), start, "untagging");
    store
        .tag_meetings(std::slice::from_ref(&m), &tag.gid)
        .unwrap();
    store.delete_tag(&tag.gid).unwrap();
    assert_eq!(lamport(), start, "deleting a tag");
    store
        .set_meeting_folder(std::slice::from_ref(&m), Some(&f.gid))
        .unwrap();
    let moved = lamport();
    assert!(moved > start, "a folder is a meeting column");
    store.delete_folder(&f.gid).unwrap();
    assert!(lamport() > moved);
}

#[test]
fn limits_are_typed_and_bulk_calls_are_all_or_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let (a, b) = (common::meeting(&store, "A"), common::meeting(&store, "B"));
    let f = store.create_folder("f").unwrap();
    let t = store.create_tag("t").unwrap();
    store
        .tag_meetings(std::slice::from_ref(&b), &t.gid)
        .unwrap();
    let bad = "no-such-meeting".to_string();

    assert!(matches!(
        store.set_meeting_folder(&[a.clone(), bad.clone()], Some(&f.gid)),
        Err(StoreError::NotFound {
            kind: "meeting",
            ..
        })
    ));
    assert_eq!(
        store.get_meeting(&a).unwrap().folder_gid,
        None,
        "nothing moved"
    );
    assert!(matches!(
        store.tag_meetings(&[a.clone(), bad.clone()], &t.gid),
        Err(StoreError::NotFound {
            kind: "meeting",
            ..
        })
    ));
    assert!(
        !store
            .meeting_tags(std::slice::from_ref(&a))
            .unwrap()
            .contains_key(&a)
    );
    assert!(store.untag_meetings(&[b.clone(), bad], &t.gid).is_err());
    assert_eq!(
        store.meeting_tags(std::slice::from_ref(&b)).unwrap()[&b].len(),
        1,
        "still tagged"
    );
    assert!(matches!(
        store.set_meeting_folder(std::slice::from_ref(&a), Some("nope")),
        Err(StoreError::NotFound { kind: "folder", .. })
    ));
    assert!(matches!(
        store.tag_meetings(std::slice::from_ref(&a), "nope"),
        Err(StoreError::NotFound { kind: "tag", .. })
    ));
    assert!(matches!(
        store.create_folder(&"x".repeat(MAX_FOLDER_NAME + 1)),
        Err(StoreError::Limit {
            max: MAX_FOLDER_NAME,
            ..
        })
    ));
    // The totals.
    for i in 0..MAX_FOLDERS - 1 {
        store.create_folder(&format!("folder {i}")).unwrap();
    }
    assert!(matches!(
        store.create_folder("one too many"),
        Err(StoreError::Limit {
            kind: "folders",
            max: MAX_FOLDERS
        })
    ));
    for i in 0..MAX_TAGS - 1 {
        store.create_tag(&format!("tag {i}")).unwrap();
    }
    assert!(matches!(
        store.create_tag("one too many"),
        Err(StoreError::Limit {
            kind: "tags",
            max: MAX_TAGS
        })
    ));
    // An existing name still resolves at the limit.
    assert_eq!(store.create_tag("t").unwrap().gid, t.gid);
}

#[test]
fn names_lose_invisible_characters_before_the_checks() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let t = store.create_tag("a\u{200b}b\u{202e}c\u{0}\tD\n e").unwrap();
    assert_eq!(t.name, "abc D e");
    assert_eq!(store.create_tag("ABC D E").unwrap().gid, t.gid);
    assert!(
        store
            .create_folder("\u{200b}\u{feff}\u{202e} \u{0}")
            .is_err()
    );
    // The length is of what is kept.
    let padded = format!("{}\u{200b}", "x".repeat(MAX_TAG_NAME));
    assert!(store.create_tag(&padded).is_ok());
}

#[test]
fn sealed_data_is_bound_to_its_meeting_checked_and_capped() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, keys) = common::open(tmp.path());
    let (a, b) = (common::meeting(&store, "A"), common::meeting(&store, "B"));
    let info = serde_json::json!({ "event": "x" });
    store.set_calendar_info(&a, Some(&info)).unwrap();
    store.set_calendar_info(&b, Some(&info)).unwrap();
    // One meeting's sealed column copied onto the other does not open.
    let conn = raw(tmp.path(), &keys);
    conn.execute(
        "UPDATE meetings SET calendar_ct = (SELECT calendar_ct FROM meetings WHERE gid = ?1)
         WHERE gid = ?2",
        [&a, &b],
    )
    .unwrap();
    drop(conn);
    drop(store);
    let store = common::reopen(tmp.path(), &keys);
    assert_eq!(store.calendar_info(&a).unwrap(), Some(info.clone()));
    assert!(matches!(store.calendar_info(&b), Err(StoreError::Decrypt)));

    // The size cap.
    let big = serde_json::json!({ "x": "y".repeat(ghi_store::organize::MAX_SEALED_JSON) });
    assert!(matches!(
        store.set_calendar_info(&a, Some(&big)),
        Err(StoreError::Limit { .. })
    ));
    assert_eq!(store.calendar_info(&a).unwrap(), Some(info), "unchanged");

    // A track speaker must be a speaker of this meeting; fields default.
    let mine = store
        .add_speaker(
            &a,
            NewSpeaker {
                label_idx: 0,
                ..Default::default()
            },
        )
        .unwrap();
    let theirs = store
        .add_speaker(
            &b,
            NewSpeaker {
                label_idx: 0,
                ..Default::default()
            },
        )
        .unwrap();
    let ts = |gid: &str| TrackSpeaker {
        label: "An".into(),
        speaker_gid: gid.into(),
        spans: vec![[0, 10]],
    };
    assert!(matches!(
        store.set_track_speakers(&a, &[ts(&theirs)]),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        store.set_track_speakers(&a, &[ts("nope")]),
        Err(StoreError::Invalid(_))
    ));
    store.set_track_speakers(&a, &[ts(&mine)]).unwrap();
    let parsed: TrackSpeaker = serde_json::from_str(r#"{"label":"Bo"}"#).unwrap();
    assert_eq!((parsed.speaker_gid.as_str(), parsed.spans.len()), ("", 0));
    assert_eq!(store.track_speakers(&a).unwrap().len(), 1);
}

#[test]
fn search_can_ask_for_meetings_in_no_folder_and_clamps_the_tag_list() {
    let tmp = tempfile::tempdir().unwrap();
    let (store, _k) = common::open(tmp.path());
    let mk = |t: &str| {
        let g = common::meeting(&store, t);
        store
            .add_segments(&g, vec![common::seg(0, 1000, "ngân sách")])
            .unwrap();
        g
    };
    let (a, b) = (mk("A"), mk("B"));
    let f = store.create_folder("f").unwrap();
    store
        .set_meeting_folder(std::slice::from_ref(&a), Some(&f.gid))
        .unwrap();
    let t = store.create_tag("t").unwrap();
    store
        .tag_meetings(std::slice::from_ref(&b), &t.gid)
        .unwrap();
    let run = |filter: SearchFilter| -> Vec<String> {
        let mut q = SearchQuery::new("ngan sach");
        q.filter = filter;
        store
            .search(&q)
            .unwrap()
            .into_iter()
            .map(|h| h.meeting_gid)
            .collect()
    };
    assert_eq!(
        run(SearchFilter {
            folder: Some(String::new()),
            ..Default::default()
        }),
        vec![b.clone()]
    );
    // A huge tag list is clamped to MAX_TAGS gids (the real one is first).
    let mut tags = vec![t.gid.clone()];
    tags.extend((0..5_000).map(|i| format!("junk-{i}")));
    assert_eq!(
        run(SearchFilter {
            tags,
            ..Default::default()
        }),
        vec![b]
    );
}
