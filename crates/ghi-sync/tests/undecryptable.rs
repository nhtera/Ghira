// SPDX-License-Identifier: Apache-2.0
//! Delete → sync → undecryptable (doc 07 §11; 15-L, the phase's key success
//! criterion): a meeting deleted on the phone is gone on the desktop too, and
//! neither device's pre-delete copy of its data directory opens with the key
//! ring each device holds afterwards (the delete rotated the wrap secret).
//!
//! The attacker has a copy of a device's data directory (DB + bundles) taken
//! before the delete, and the device's key ring as it is now.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use ghi_store::bundle::BundleReader;
use ghi_store::db;
use ghi_store::keys::dev::FileKeyStore;
use ghi_store::keys::{KeyRing, Protection, load_or_create};
use ghi_store::rowcrypt::{Dek, open_text, row_aad};
use ghi_store::store::{NewActionItem, Provenance, Store, TrackKind};

const SECRET_LINE: &str = "Ngân sách tuyệt mật của quý bốn là ba tỷ đồng";
const SECRET_NOTE: &str = "Quyết định bí mật: chốt thương vụ";
const SECRET_ACTION: &str = "Gọi cho luật sư về hợp đồng";
const PAGES: usize = 6;

/// What an attacker manages to read for one meeting.
#[derive(Debug, Default)]
struct Loot {
    dek_found: bool,
    transcript: Vec<String>,
    notes: Vec<String>,
    actions: Vec<String>,
    audio_pages: usize,
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

fn ring_at(keyfile: &Path) -> KeyRing {
    load_or_create(&FileKeyStore::new(keyfile), Protection::default()).unwrap()
}

/// Tries every blob of every table of the copy's database as the wrapped key
/// of `gid` (so it does not matter where a copy hides), then reads what that
/// key opens.
fn attack(dir: &Path, ring: &KeyRing, gid: &str) -> Loot {
    let conn = db::open(&dir.join("ghira.db"), &ring.db_key())
        .expect("the attacker holds the master key and opens the database");
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
                if let Ok(b) = row.get::<_, Vec<u8>>(c)
                    && let Ok(d) = ring.unwrap_dek(&b, gid)
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
        return loot;
    };
    let texts = |table: &str, col: &str| -> Vec<String> {
        let mut stmt = conn
            .prepare(&format!("SELECT gid, {col} FROM {table}"))
            .unwrap();
        let rows: Vec<(String, Vec<u8>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        rows.into_iter()
            .filter_map(|(g, ct)| open_text(&dek, &ct, &row_aad(table, col, &g)).ok())
            .collect()
    };
    loot.transcript = texts("segments", "text_ct");
    loot.notes = texts("notes_blocks", "body_ct");
    loot.actions = texts("action_items", "text_ct");
    let track_gid: String = conn
        .query_row(
            "SELECT t.gid FROM tracks t JOIN meetings m ON m.id = t.meeting_id WHERE m.gid = ?1",
            [gid],
            |r| r.get(0),
        )
        .unwrap_or_default();
    let bundle = dir.join("bundles").join(gid).join("mic.ghb");
    if let Ok(r) = BundleReader::open(&bundle, &dek, &Store::bundle_aad(&track_gid)) {
        loot.audio_pages = (0..r.page_count()).filter(|i| r.page(*i).is_ok()).count();
    }
    loot
}

/// A device's data as an attacker's copy: the data directory, and the key
/// ring file as it was at that moment.
struct Copy {
    data: PathBuf,
    ring: KeyRing,
}

fn snapshot(n: &Node, name: &str, scratch: &Path) -> Copy {
    n.store().checkpoint().unwrap();
    let data = scratch.join(name);
    copy_dir(&n.data_dir(), &data);
    Copy {
        data,
        ring: ring_at(&n.dir.path().join("data.devkey")),
    }
}

fn plant_secret(s: &Store) -> String {
    let m = meeting_with_audio(s, "Họp mật", PAGES);
    s.add_segment(&m, seg(SECRET_LINE, 7_000)).unwrap();
    s.add_note_block(&m, note(SECRET_NOTE, Provenance::User))
        .unwrap();
    s.add_action_item(
        &m,
        NewActionItem {
            text: SECRET_ACTION.into(),
            ..Default::default()
        },
    )
    .unwrap();
    m
}

#[test]
fn a_meeting_deleted_on_the_phone_is_unrecoverable_on_both_devices_even_from_old_copies() {
    let (hub_n, phone) = (node(), node());
    let hub = hub_n.hub();
    pair(&hub, &hub_n, &phone);
    let secret = plant_secret(phone.store());
    let bystander = meeting(phone.store(), "Họp thường");
    sync(&hub, &phone);
    for n in [&hub_n, &phone] {
        assert_eq!(n.store().list_meetings(10, 0).unwrap().len(), 2);
        assert!(texts(n.store(), &secret).contains(&SECRET_LINE.to_string()));
    }
    assert_eq!(
        hub_n
            .store()
            .open_bundle(&secret, TrackKind::Mic)
            .unwrap()
            .page_count() as usize,
        PAGES,
        "the audio is on the desktop"
    );

    // Both devices' copies, before the delete.
    let scratch = tempfile::tempdir().unwrap();
    let before = [
        ("hub", snapshot(&hub_n, "hub-before", scratch.path())),
        ("phone", snapshot(&phone, "phone-before", scratch.path())),
    ];
    // Control: each copy opens with the ring it was taken with, so the copies
    // are genuine and the attack function works.
    for (who, c) in &before {
        let loot = attack(&c.data, &c.ring, &secret);
        assert!(loot.dek_found, "{who}: the copy holds the wrapped DEK");
        assert!(loot.transcript.contains(&SECRET_LINE.to_string()), "{who}");
        assert_eq!(loot.notes.iter().filter(|n| *n == SECRET_NOTE).count(), 1);
        assert_eq!(loot.actions, [SECRET_ACTION]);
        assert_eq!(loot.audio_pages, PAGES, "{who}");
    }

    // The delete on the phone, then a sync.
    phone.store().delete_meeting(&secret).unwrap();
    let (mine, theirs) = sync(&hub, &phone);
    assert!(mine.tombs_pushed >= 1, "{mine:?}");
    assert_eq!(theirs.tombs_pushed, mine.tombs_pushed);
    let (m, t) = sync(&hub, &phone);
    assert!(idle(&m) && idle(&t), "{m:?} {t:?}");

    for (who, n) in [("hub", &hub_n), ("phone", &phone)] {
        // Gone from the store...
        assert!(n.store().get_meeting(&secret).is_err(), "{who}");
        assert!(n.store().segments(&secret).is_err(), "{who}");
        assert!(n.store().note_blocks(&secret).is_err(), "{who}");
        assert!(n.store().is_tombstoned(&secret).unwrap(), "{who}");
        assert!(
            !n.data_dir().join("bundles").join(&secret).exists(),
            "{who}: the audio directory is gone"
        );
        let left = n.store().list_meetings(10, 0).unwrap();
        assert_eq!(left.len(), 1, "{who}");
        assert_eq!(left[0].gid, bystander);
        assert_eq!(texts(n.store(), &bystander).len(), 3, "{who}");
    }

    // ...and nothing opens it. The attacker's best case: the copy from before
    // the delete, with the ring the device holds now.
    for (who, copy) in &before {
        let node = if *who == "hub" { &hub_n } else { &phone };
        let now = ring_at(&node.dir.path().join("data.devkey"));
        assert!(
            now != copy.ring,
            "{who}: the wrap secret was rotated by the delete"
        );
        let loot = attack(&copy.data, &now, &secret);
        assert!(
            !loot.dek_found,
            "{who}: the pre-delete copy still opens with today's ring"
        );
        assert!(
            loot.transcript.is_empty()
                && loot.notes.is_empty()
                && loot.actions.is_empty()
                && loot.audio_pages == 0,
            "{who}: {loot:?}"
        );
        // Not even the bystander of the same copy: the whole wrap rotated.
        assert!(!attack(&copy.data, &now, &bystander).dek_found, "{who}");
    }
    // Opening the copied directory as a store with today's ring fails to read
    // the meeting (the DEK cannot be unwrapped); done on a third copy so the
    // devices' own stores stay open.
    for (who, copy) in &before {
        let node = if *who == "hub" { &hub_n } else { &phone };
        let reopened = scratch.path().join(format!("{who}-reopen"));
        copy_dir(&copy.data, &reopened);
        let keyfile = scratch.path().join(format!("{who}.devkey"));
        std::fs::copy(node.dir.path().join("data.devkey"), &keyfile).unwrap();
        let opened = Store::open(
            &reopened,
            std::sync::Arc::new(FileKeyStore::new(&keyfile)),
            Protection::default(),
        );
        match opened {
            Err(_) => {} // refusing the copy is just as good
            Ok(s) => {
                assert!(
                    s.segments(&secret).is_err() && s.get_meeting(&secret).is_err(),
                    "{who}: the copy reads the deleted meeting with today's ring"
                );
            }
        }
    }

    // After the sync the live state and a fresh copy of it hold no wrapped
    // DEK for the meeting anywhere.
    for (who, n) in [("hub", &hub_n), ("phone", &phone)] {
        let after = snapshot(n, &format!("{who}-after"), scratch.path());
        let loot = attack(&after.data, &after.ring, &secret);
        assert!(!loot.dek_found, "{who}: a wrapped DEK is left behind");
        // The bystander survives with its key.
        assert!(
            attack(&after.data, &after.ring, &bystander).dek_found,
            "{who}"
        );
    }
}

#[test]
fn a_delete_on_the_desktop_reaches_the_phone_the_same_way() {
    let (hub_n, phone) = (node(), node());
    let hub = hub_n.hub();
    pair(&hub, &hub_n, &phone);
    let secret = plant_secret(phone.store());
    sync(&hub, &phone);
    let scratch = tempfile::tempdir().unwrap();
    let before = snapshot(&phone, "phone-before", scratch.path());
    assert!(attack(&before.data, &before.ring, &secret).dek_found);

    hub_n.store().delete_meeting(&secret).unwrap();
    sync(&hub, &phone);
    sync(&hub, &phone);
    assert!(phone.store().get_meeting(&secret).is_err());
    assert!(phone.store().is_tombstoned(&secret).unwrap());
    let now = ring_at(&phone.dir.path().join("data.devkey"));
    assert!(!attack(&before.data, &now, &secret).dek_found);
}
