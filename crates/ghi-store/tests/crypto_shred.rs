// SPDX-License-Identifier: Apache-2.0
//! The key success criterion of phase 5: after a meeting is deleted, its
//! audio, transcript and notes can't be decrypted from what is left on disk,
//! even by someone who holds the key ring as it is after the delete.
//!
//! The attacker model: a copy of the data directory (DB + bundles) plus the
//! current key ring from the keystore.
//! - A copy taken BEFORE the delete holds the wrapped DEK, but the delete
//!   rotated the wrap secret, so the current ring can't unwrap it. (With the
//!   ring as it was *before* the delete the copy is readable: that is the
//!   control that shows the copy is genuine.)
//! - The state AFTER the delete has no wrapped DEK for the meeting anywhere.

mod common;

use std::path::Path;

use ghi_store::bundle::BundleReader;
use ghi_store::keys::KeyRing;
use ghi_store::rowcrypt::{Dek, open_text, row_aad};
use ghi_store::store::{MarkTag, NewActionItem, NewNoteBlock, Provenance, Store, TrackKind};
use ghi_store::{StoreError, db};
use rusqlite::types::Value;

const SECRET_TEXT: &str = "Ngân sách tuyệt mật của quý bốn là ba tỷ đồng";
const SECRET_NOTE: &str = "Quyết định bí mật: chốt thương vụ";
const SECRET_ACTION: &str = "Gọi cho luật sư về hợp đồng";
const AUDIO: [&[u8]; 5] = [
    b"audio-page-0",
    b"audio-page-1",
    b"audio-page-2",
    b"audio-page-3",
    b"audio-page-4",
];

/// What an attacker with `dir` and `master` manages to read for `gid`.
#[derive(Debug, Default)]
struct Loot {
    dek_found: bool,
    transcript: Vec<String>,
    notes: Vec<String>,
    actions: Vec<String>,
    audio_pages: Vec<Vec<u8>>,
}

/// Tries every blob in every table of the database as a wrapped DEK for
/// `gid` (so it doesn't matter where a copy might hide), then decrypts
/// whatever that key opens.
fn attack(dir: &Path, master: &KeyRing, gid: &str) -> Loot {
    let conn =
        db::open(&dir.join("ghira.db"), &master.db_key()).expect("the attacker has the master key");
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();

    let mut dek: Option<Dek> = None;
    for t in &tables {
        let Ok(mut stmt) = conn.prepare(&format!("SELECT * FROM \"{t}\"")) else {
            continue;
        };
        let cols = stmt.column_count();
        let Ok(mut rows) = stmt.query([]) else {
            continue;
        };
        while let Ok(Some(row)) = rows.next() {
            for c in 0..cols {
                if let Ok(Value::Blob(b)) = row.get::<_, Value>(c)
                    && let Ok(d) = master.unwrap_dek(&b, gid)
                {
                    dek = Some(d);
                }
            }
        }
    }
    let mut loot = Loot {
        dek_found: dek.is_some(),
        ..Default::default()
    };
    let Some(dek) = dek else {
        // Without a key, leftover bundle ciphertext is opaque: no DEK opens it.
        return loot;
    };

    let text_of = |table: &str, col: &str, select: &str| -> Vec<String> {
        let Ok(mut stmt) = conn.prepare(select) else {
            return vec![];
        };
        let rows: Vec<(String, Vec<u8>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        rows.into_iter()
            .filter_map(|(g, ct)| open_text(&dek, &ct, &row_aad(table, col, &g)).ok())
            .collect()
    };
    loot.transcript = text_of("segments", "text_ct", "SELECT gid, text_ct FROM segments");
    loot.notes = text_of(
        "notes_blocks",
        "body_ct",
        "SELECT gid, body_ct FROM notes_blocks",
    );
    loot.actions = text_of(
        "action_items",
        "text_ct",
        "SELECT gid, text_ct FROM action_items",
    );

    let bundle = dir.join("bundles").join(gid).join("mic.ghb");
    let track_gid: String = conn
        .query_row(
            "SELECT t.gid FROM tracks t JOIN meetings m ON m.id = t.meeting_id WHERE m.gid = ?1",
            [gid],
            |r| r.get(0),
        )
        .unwrap_or_default();
    if let Ok(r) = BundleReader::open(&bundle, &dek, &Store::bundle_aad(&track_gid)) {
        loot.audio_pages = (0..r.page_count()).map(|i| r.page(i).unwrap()).collect();
    }
    loot
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

fn all_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_dir() {
            all_files(&e.path(), out);
        } else {
            out.push(e.path());
        }
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    data: std::path::PathBuf,
    store: Store,
    keys: common::Keys,
    secret: String,
    bystander: String,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let (store, keys) = common::open(&data);

    let secret = common::meeting(&store, "Họp mật");
    store
        .add_segment(&secret, common::seg(0, 4000, SECRET_TEXT))
        .unwrap();
    store
        .add_note_block(
            &secret,
            NewNoteBlock {
                kind: "paragraph".into(),
                provenance: Provenance::User,
                body: SECRET_NOTE.into(),
                anchors: vec![],
                pinned: false,
            },
        )
        .unwrap();
    store
        .add_action_item(
            &secret,
            NewActionItem {
                text: SECRET_ACTION.into(),
                ..Default::default()
            },
        )
        .unwrap();
    store.add_mark(&secret, 1500, MarkTag::Decision).unwrap();
    let mut w = store.open_track(&secret, TrackKind::Mic).unwrap();
    for page in AUDIO {
        w.append(page).unwrap();
    }
    store.finish_track(&secret, TrackKind::Mic, w).unwrap();

    // Another meeting that must survive.
    let bystander = common::meeting(&store, "Họp thường");
    store
        .add_segment(&bystander, common::seg(0, 4000, "Lịch làm việc tuần sau"))
        .unwrap();
    let mut w = store.open_track(&bystander, TrackKind::Mic).unwrap();
    w.append(b"bystander-audio").unwrap();
    store.finish_track(&bystander, TrackKind::Mic, w).unwrap();

    Fixture {
        _tmp: tmp,
        data,
        store,
        keys,
        secret,
        bystander,
    }
}

#[test]
fn delete_makes_audio_transcript_and_notes_undecryptable() {
    let f = fixture();
    let ring_before = f.ring();

    // The attacker snapshot: the whole data dir, before the delete.
    f.store.checkpoint().unwrap();
    let before = f.data.parent().unwrap().join("attacker-before");
    common::copy_dir(&f.data, &before);

    f.store.delete_meeting(&f.secret).unwrap();

    // The delete truncated the WAL, so old page images don't linger in it.
    let wal = f.data.join("ghira.db-wal");
    assert!(
        !wal.exists() || std::fs::metadata(&wal).unwrap().len() == 0,
        "WAL not truncated"
    );

    // --- The pre-delete copy. ----------------------------------------------
    let master = f.ring();
    assert!(
        !master.is_rotating() && master != ring_before,
        "the wrap secret was rotated"
    );
    // Control: with the ring as it was before the delete the copy IS readable.
    let loot = attack(&before, &ring_before, &f.secret);
    assert!(loot.dek_found, "the pre-delete copy holds the wrapped DEK");
    assert_eq!(loot.transcript, [SECRET_TEXT]);
    assert_eq!(loot.notes, [SECRET_NOTE]);
    assert_eq!(loot.actions, [SECRET_ACTION]);
    assert_eq!(loot.audio_pages.len(), AUDIO.len());
    assert_eq!(loot.audio_pages[3], AUDIO[3]);
    // But with the ring the attacker can get now, nothing opens: not the
    // deleted meeting, and not the other meetings in the copy either.
    let loot = attack(&before, &master, &f.secret);
    assert!(!loot.dek_found, "the old wrap secret is gone");
    assert!(loot.transcript.is_empty() && loot.notes.is_empty() && loot.actions.is_empty());
    assert!(loot.audio_pages.is_empty());
    assert!(!attack(&before, &master, &f.bystander).dek_found);

    // --- After the delete: the live state and a fresh copy of it. ---------
    f.store.checkpoint().unwrap();
    let after = f.data.parent().unwrap().join("attacker-after");
    common::copy_dir(&f.data, &after);
    for dir in [&f.data, &after] {
        let loot = attack(dir, &master, &f.secret);
        assert!(
            !loot.dek_found,
            "no wrapped DEK for the meeting is left anywhere in the database"
        );
        assert!(loot.transcript.is_empty() && loot.notes.is_empty() && loot.actions.is_empty());
        assert!(loot.audio_pages.is_empty());
    }

    // Old audio ciphertext copied from before the delete is useless too: the
    // only key that opened it existed in the deleted row.
    let stale = before.join("bundles").join(&f.secret).join("mic.ghb");
    assert!(stale.is_file());
    let post_state_keys = attack(&after, &master, &f.secret);
    assert!(!post_state_keys.dek_found);
    let guess = BundleReader::open(&stale, &Dek::generate(), &Store::bundle_aad("any-track"));
    assert!(
        guess.map_or(true, |r| r.page(0).is_err()),
        "a guessed key opens no page"
    );

    // Files: the bundle directory is gone; nothing on disk holds plaintext.
    assert!(!f.data.join("bundles").join(&f.secret).exists());
    let mut files = Vec::new();
    all_files(&f.data, &mut files);
    for p in files {
        let bytes = std::fs::read(&p).unwrap();
        for secret in [SECRET_TEXT, SECRET_NOTE, SECRET_ACTION] {
            assert!(
                !contains_bytes(&bytes, secret.as_bytes()),
                "{} holds plaintext",
                p.display()
            );
        }
    }
    // Freed pages were returned to the OS.
    let conn = db::open(&f.data.join("ghira.db"), &master.db_key()).unwrap();
    let free: i64 = conn
        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .unwrap();
    assert_eq!(free, 0, "incremental_vacuum returned the freed pages");

    // --- The rest of the store is untouched. -------------------------------
    assert!(matches!(
        f.store.get_meeting(&f.secret),
        Err(StoreError::NotFound { .. })
    ));
    assert!(f.store.segments(&f.secret).is_err());
    assert_eq!(f.store.list_meetings(10, 0).unwrap().len(), 1);
    assert_eq!(
        f.store.segments(&f.bystander).unwrap()[0].text,
        "Lịch làm việc tuần sau"
    );
    let reader = f.store.open_bundle(&f.bystander, TrackKind::Mic).unwrap();
    assert_eq!(reader.page(0).unwrap(), b"bystander-audio");
    assert!(
        f.store
            .search(&ghi_store::search::SearchQuery::new("tuyet mat"))
            .unwrap()
            .is_empty()
    );
    assert!(
        f.store
            .search(&ghi_store::search::SearchQuery::new("chot thuong vu"))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        f.store
            .search(&ghi_store::search::SearchQuery::new("lich lam viec"))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn delete_writes_tombstones_for_the_meeting_and_its_children() {
    let f = fixture();
    let segs: Vec<String> = f
        .store
        .segments(&f.secret)
        .unwrap()
        .into_iter()
        .map(|s| s.gid)
        .collect();
    let notes: Vec<String> = f
        .store
        .note_blocks(&f.secret)
        .unwrap()
        .into_iter()
        .map(|n| n.gid)
        .collect();
    let actions: Vec<String> = f
        .store
        .action_items(&f.secret)
        .unwrap()
        .into_iter()
        .map(|a| a.gid)
        .collect();
    let marks: Vec<String> = f
        .store
        .marks(&f.secret)
        .unwrap()
        .into_iter()
        .map(|m| m.gid)
        .collect();
    f.store.delete_meeting(&f.secret).unwrap();

    let tombs = f.store.tombstones_since(0).unwrap();
    let kinds = |gid: &String| tombs.iter().find(|t| &t.gid == gid).map(|t| t.kind.clone());
    assert_eq!(kinds(&f.secret).as_deref(), Some("meeting"));
    assert_eq!(kinds(&segs[0]).as_deref(), Some("segment"));
    assert_eq!(kinds(&notes[0]).as_deref(), Some("note"));
    assert_eq!(kinds(&actions[0]).as_deref(), Some("action_item"));
    assert_eq!(kinds(&marks[0]).as_deref(), Some("mark"));
    assert_eq!(tombs.iter().filter(|t| t.kind == "track").count(), 1);
    assert!(!f.store.is_tombstoned(&f.bystander).unwrap());
    // Tombstones carry no content.
    assert!(
        tombs
            .iter()
            .all(|t| t.gid.len() == 36 && t.deleted_at > 0 && t.lamport > 0)
    );
}

#[test]
fn a_crash_after_the_key_is_shredded_finishes_on_next_open() {
    let f = fixture();
    let Fixture {
        _tmp,
        data,
        store,
        keys,
        secret,
        bystander,
    } = f;
    // Simulate power loss right after step 2 of the delete.
    store.shred_key(&secret).unwrap();
    // The meeting is already unreadable and invisible.
    assert!(store.segments(&secret).is_err());
    assert_eq!(store.list_meetings(10, 0).unwrap().len(), 1);
    assert!(
        store
            .search(&ghi_store::search::SearchQuery::new("tuyet mat"))
            .unwrap()
            .is_empty()
    );
    assert!(
        data.join("bundles").join(&secret).exists(),
        "files not deleted yet"
    );
    drop(store);

    let store = common::reopen(&data, &keys);
    let master = common::ring(&keys);
    assert!(
        !data.join("bundles").join(&secret).exists(),
        "the reopen finished the delete"
    );
    assert!(matches!(
        store.get_meeting(&secret),
        Err(StoreError::NotFound { .. })
    ));
    assert_eq!(store.segments(&bystander).unwrap().len(), 1);
    assert!(!attack(&data, &master, &secret).dek_found);
}

#[test]
fn pre_migration_snapshots_are_purged_by_delete() {
    // Snapshots hold wrapped DEKs, so a delete must not leave them behind.
    let f = fixture();
    let snap = f
        .data
        .join("snapshots")
        .join("000000000000001-pre-v0001.db");
    std::fs::copy(f.data.join("ghira.db"), &snap).unwrap();
    f.store.delete_meeting(&f.secret).unwrap();
    assert!(!snap.exists());
}

#[test]
fn delete_all_shreds_everything_and_rotates_the_key() {
    let f = fixture();
    let phrase = ghi_store::recovery::RecoveryPhrase::generate();
    f.store.set_recovery_phrase(&phrase).unwrap();
    let recovery = std::fs::read(f.data.join("recovery.bin")).unwrap();
    let old = common::ring(&f.keys);
    let Fixture {
        data, store, keys, ..
    } = f;
    store.delete_all(&*keys, Default::default()).unwrap();
    let new = common::ring(&keys);
    assert_ne!(new, old, "the master key was rotated");
    assert!(!data.join("ghira.db").exists());
    assert!(!data.join("recovery.bin").exists());
    assert!(!data.join("bundles").exists());
    // The old phrase is dead: it only unwraps the old key, which opens nothing.
    assert_eq!(
        ghi_store::recovery::unwrap_ring(&recovery, &phrase).unwrap(),
        old
    );
    let fresh = common::reopen(&data, &keys);
    assert!(fresh.list_meetings(10, 0).unwrap().is_empty());
}

#[test]
fn delete_leaves_no_tokens_snapshots_or_wal_and_a_wrong_key_fails_cleanly() {
    use ghi_store::search::SearchQuery;
    let f = fixture();
    let master = f.ring();
    // A token that appears nowhere else. Stored folded: "zqxjkvtoken".
    f.store
        .add_segment(
            &f.secret,
            common::seg(5000, 6000, "Từ khóa zqxjkvToken độc nhất"),
        )
        .unwrap();
    f.store
        .add_note_block(
            &f.secret,
            NewNoteBlock {
                kind: "p".into(),
                provenance: Provenance::User,
                body: "ghi chú zqxjkvToken".into(),
                anchors: vec![],
                pinned: false,
            },
        )
        .unwrap();
    std::fs::create_dir_all(f.data.join("snapshots")).unwrap();
    std::fs::write(
        f.data
            .join("snapshots")
            .join("000000000000001-pre-v0001.db"),
        b"x",
    )
    .unwrap();

    let token_in_index = |dir: &Path| -> bool {
        let conn = db::open(&dir.join("ghira.db"), &master.db_key()).unwrap();
        ["segments_fts_data", "notes_fts_data"].iter().any(|t| {
            let mut stmt = conn.prepare(&format!("SELECT block FROM {t}")).unwrap();
            let blocks: Vec<Vec<u8>> = stmt
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            blocks.iter().any(|b| contains_bytes(b, b"zqxjkvtoken"))
        })
    };
    f.store.checkpoint().unwrap();
    assert!(
        token_in_index(&f.data),
        "control: the folded token is in the index before the delete"
    );
    assert_eq!(
        f.store
            .search(&SearchQuery::new("zqxjkvtoken"))
            .unwrap()
            .len(),
        2
    );

    f.store.delete_meeting(&f.secret).unwrap();

    assert!(
        !token_in_index(&f.data),
        "optimize rewrote the deleted tokens away"
    );
    assert!(
        f.store
            .search(&SearchQuery::new("zqxjkvtoken"))
            .unwrap()
            .is_empty()
    );
    let wal = f.data.join("ghira.db-wal");
    assert!(
        !wal.exists() || std::fs::metadata(&wal).unwrap().len() == 0,
        "WAL not empty"
    );
    let conn = db::open(&f.data.join("ghira.db"), &master.db_key()).unwrap();
    let free: i64 = conn
        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .unwrap();
    assert_eq!(free, 0);
    drop(conn);
    assert_eq!(
        std::fs::read_dir(f.data.join("snapshots")).unwrap().count(),
        0,
        "snapshots deleted"
    );
    assert!(f.store.is_tombstoned(&f.secret).unwrap());

    // A wrong master key can't open the database, and the file is not touched.
    let copy = f.data.parent().unwrap().join("wrong-key-copy");
    common::copy_dir(&f.data, &copy);
    let before = std::fs::read(copy.join("ghira.db")).unwrap();
    let wrong = common::keys_with(KeyRing::generate());
    let Err(err) = Store::open(&copy, wrong, Default::default()) else {
        panic!("opened with a wrong key")
    };
    assert!(matches!(err, StoreError::Db(_)), "{err}");
    assert_eq!(std::fs::read(copy.join("ghira.db")).unwrap(), before);
}

impl Fixture {
    /// The key ring currently in the keystore.
    fn ring(&self) -> KeyRing {
        common::ring(&self.keys)
    }
}

#[test]
fn a_crash_in_the_middle_of_a_rotation_is_finished_on_open() {
    let f = fixture();
    let Fixture {
        _tmp,
        data,
        store,
        keys,
        secret,
        bystander,
    } = f;
    // The ring was saved with the new secret, but no DEK was re-wrapped yet.
    store.begin_rotation_only().unwrap();
    assert!(common::ring(&keys).is_rotating());
    // Both secrets work while rotating.
    assert_eq!(store.segments(&bystander).unwrap().len(), 1);
    drop(store);
    // A copy taken now still has every DEK under the old secret.
    let before = data.parent().unwrap().join("before-rotation");
    common::copy_dir(&data, &before);

    let store = common::reopen(&data, &keys);
    assert!(
        !common::ring(&keys).is_rotating(),
        "open finished the rotation"
    );
    assert_eq!(
        store.segments(&bystander).unwrap()[0].text,
        "Lịch làm việc tuần sau"
    );
    assert_eq!(store.segments(&secret).unwrap()[0].text, SECRET_TEXT);
    assert_eq!(
        store
            .open_bundle(&secret, TrackKind::Mic)
            .unwrap()
            .page(1)
            .unwrap(),
        AUDIO[1]
    );
    drop(store);
    let ring = common::ring(&keys);
    assert!(
        attack(&data, &ring, &secret).dek_found,
        "the live database is re-wrapped under the new secret"
    );
    // And a copy from before the rotation is now dead.
    assert!(!attack(&before, &ring, &secret).dek_found);
    assert!(!attack(&before, &ring, &bystander).dek_found);
}

#[test]
fn recovery_file_is_rewritten_after_the_rotation() {
    let f = fixture();
    let phrase = ghi_store::recovery::RecoveryPhrase::generate();
    f.store.set_recovery_phrase(&phrase).unwrap();
    f.store.delete_meeting(&f.secret).unwrap();
    let Fixture {
        data,
        store,
        keys,
        bystander,
        ..
    } = f;
    drop(store);
    // The keystore is lost; the phrase gets the rotated ring back from
    // recovery.bin, and the remaining meeting reads.
    let fresh = common::keys();
    let restored =
        Store::restore_with_phrase(&data, &phrase, fresh.clone(), Default::default()).unwrap();
    assert_eq!(common::ring(&fresh), common::ring(&keys));
    assert_eq!(restored.segments(&bystander).unwrap().len(), 1);
}

#[test]
fn a_second_open_of_the_same_directory_is_refused() {
    let f = fixture();
    let Err(err) = Store::open(&f.data, f.keys.clone(), Default::default()) else {
        panic!("two stores on one directory");
    };
    assert!(
        matches!(&err, StoreError::Invalid(m) if m.contains("in use")),
        "{err}"
    );
    // The first one keeps working, and the lock is released when it drops.
    assert_eq!(f.store.segments(&f.bystander).unwrap().len(), 1);
    let Fixture {
        data, store, keys, ..
    } = f;
    drop(store);
    assert!(Store::open(&data, keys, Default::default()).is_ok());
}

#[test]
fn a_crash_between_the_delete_and_the_rotation_is_finished_on_open() {
    let Fixture {
        _tmp,
        data,
        store,
        keys,
        secret,
        bystander,
    } = fixture();
    let before = data.parent().unwrap().join("before-delete");
    store.checkpoint().unwrap();
    common::copy_dir(&data, &before);
    // Power loss after the rows are gone, before the rotation finished.
    store.delete_meeting_before_rotation(&secret).unwrap();
    assert!(
        common::ring(&keys).is_rotating(),
        "the rotation starts before anything is deleted"
    );
    drop(store);

    let store = common::reopen(&data, &keys);
    let ring = common::ring(&keys);
    assert!(!ring.is_rotating(), "open finished the rotation");
    assert_eq!(store.segments(&bystander).unwrap().len(), 1);
    assert!(!attack(&before, &ring, &secret).dek_found);
}

#[test]
fn a_stale_recovery_file_is_rewritten_on_open() {
    let Fixture {
        _tmp,
        data,
        store,
        keys,
        secret,
        bystander,
    } = fixture();
    let phrase = ghi_store::recovery::RecoveryPhrase::generate();
    store.set_recovery_phrase(&phrase).unwrap();
    let stale = std::fs::read(data.join("recovery.bin")).unwrap();
    store.delete_meeting(&secret).unwrap();
    drop(store);
    // The last rewrite of recovery.bin was lost (crash, full disk).
    std::fs::write(data.join("recovery.bin"), &stale).unwrap();

    drop(common::reopen(&data, &keys));
    // The keychain is lost: the phrase still gets a ring that opens the
    // remaining meeting.
    let fresh = common::keys();
    let restored =
        Store::restore_with_phrase(&data, &phrase, fresh.clone(), Default::default()).unwrap();
    assert_eq!(restored.segments(&bystander).unwrap().len(), 1);
}

#[test]
fn a_key_no_secret_opens_does_not_block_rotations() {
    let Fixture {
        _tmp,
        data,
        store,
        keys,
        secret,
        bystander,
    } = fixture();
    let broken = common::meeting(&store, "Hỏng");
    drop(store);
    // A wrapped DEK from a secret destroyed long ago (here: garbage).
    {
        let conn = db::open(&data.join("ghira.db"), &common::db_key(&keys)).unwrap();
        conn.execute(
            "UPDATE meetings SET dek_wrapped = randomblob(length(dek_wrapped)) WHERE gid = ?1",
            [&broken],
        )
        .unwrap();
    }
    let store = common::reopen(&data, &keys);
    store.delete_meeting(&secret).unwrap();
    assert!(!common::ring(&keys).is_rotating());
    assert_eq!(store.segments(&bystander).unwrap().len(), 1);
    assert!(matches!(
        store.get_meeting(&broken),
        Err(StoreError::Decrypt)
    ));
}

#[test]
fn meetings_created_during_deletes_stay_readable() {
    let Fixture {
        _tmp,
        data,
        store,
        keys,
        ..
    } = fixture();
    let victims: Vec<String> = (0..8)
        .map(|i| common::meeting(&store, &format!("xóa {i}")))
        .collect();
    let created = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        s.spawn(|| {
            for v in &victims {
                store.delete_meeting(v).unwrap();
            }
        });
        for t in 0..2 {
            let (store, created) = (&store, &created);
            s.spawn(move || {
                for i in 0..40 {
                    let gid = common::meeting(store, &format!("mới {t}-{i}"));
                    created.lock().unwrap().push(gid);
                }
            });
        }
    });
    drop(store);
    // A fresh open has no cached DEKs: every key must unwrap from the ring.
    let store = common::reopen(&data, &keys);
    for gid in created.into_inner().unwrap() {
        store
            .get_meeting(&gid)
            .expect("readable after concurrent rotations");
    }
}

/// Stores every ring, but reports failure once `fail` is set (a save that
/// wrote the item and then failed).
#[derive(Default)]
struct FlakyKeyStore {
    inner: common::MemKeyStore,
    fail: std::sync::atomic::AtomicBool,
}

impl ghi_store::keys::KeyStore for FlakyKeyStore {
    fn load(&self) -> Result<Option<KeyRing>, StoreError> {
        self.inner.load()
    }
    fn save(&self, ring: &KeyRing, p: ghi_store::keys::Protection) -> Result<(), StoreError> {
        self.inner.save(ring, p)?;
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(StoreError::KeyLocked);
        }
        Ok(())
    }
    fn delete(&self) -> Result<(), StoreError> {
        self.inner.delete()
    }
}

#[test]
fn a_recovery_phrase_that_failed_to_set_never_takes_effect() {
    use ghi_store::keys::KeyStore;
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let keys = std::sync::Arc::new(FlakyKeyStore::default());
    let store = Store::open(&data, keys.clone(), Default::default()).unwrap();
    keys.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    let phrase = ghi_store::recovery::RecoveryPhrase::generate();
    assert!(store.set_recovery_phrase(&phrase).is_err());
    keys.fail.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(keys.load().unwrap().unwrap().recovery_key().is_none());
    drop(store);
    let store = Store::open(&data, keys.clone(), Default::default()).unwrap();
    assert!(!store.has_recovery_phrase());
}

#[test]
fn a_wrong_ring_never_finishes_a_rotation() {
    use ghi_store::keys::KeyStore;
    let Fixture {
        _tmp,
        data,
        store,
        keys,
        secret,
        bystander,
    } = fixture();
    drop(store);
    let right = common::ring(&keys);
    // Same master (the database opens), a different wrap secret: a stale
    // keystore item.
    let mut wrong = right.clone();
    wrong.begin_rotation();
    wrong.finish_rotation();
    keys.save(&wrong, Default::default()).unwrap();
    let store = common::reopen(&data, &keys);
    assert!(store.delete_meeting(&secret).is_err());
    assert!(
        common::ring(&keys).is_rotating(),
        "the rotation must not finish with a ring that opens nothing"
    );
    drop(store);
    // The right ring still opens everything that wasn't deleted.
    keys.save(&right, Default::default()).unwrap();
    let store = common::reopen(&data, &keys);
    assert_eq!(store.segments(&bystander).unwrap().len(), 1);
}

#[test]
fn an_old_recovery_file_never_replaces_the_current_key() {
    let Fixture {
        _tmp,
        data,
        store,
        keys,
        secret,
        bystander,
    } = fixture();
    let phrase = ghi_store::recovery::RecoveryPhrase::generate();
    store.set_recovery_phrase(&phrase).unwrap();
    let old = std::fs::read(data.join("recovery.bin")).unwrap();
    store.delete_meeting(&secret).unwrap();
    drop(store);
    // recovery.bin put back from an old backup, while the key still exists.
    std::fs::write(data.join("recovery.bin"), &old).unwrap();
    let before = common::ring(&keys);
    assert!(matches!(
        Store::restore_with_phrase(&data, &phrase, keys.clone(), Default::default()),
        Err(StoreError::Invalid(_))
    ));
    assert!(common::ring(&keys) == before, "the current ring was kept");
    let store = common::reopen(&data, &keys);
    assert_eq!(store.segments(&bystander).unwrap().len(), 1);
}
