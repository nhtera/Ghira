// SPDX-License-Identifier: Apache-2.0
//! A failing migration leaves the pre-migration database intact and usable.

mod common;

use std::io::Write;

use ghi_store::migrate::{self, MIGRATIONS, Migration, Step};
use ghi_store::{StoreError, db};
use rusqlite::Transaction;

/// Does real work, then fails.
fn fails_after_partial_work(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "CREATE TABLE half_done (x INTEGER);
         INSERT INTO half_done VALUES (1);
         DELETE FROM segments;
         UPDATE meetings SET status = 'destroyed';",
    )?;
    Err(rusqlite::Error::InvalidQuery)
}

/// Damages the database file behind SQLite's back, then fails. A transaction
/// rollback can't undo this; only restoring the snapshot can.
fn corrupts_the_file_then_fails(tx: &Transaction) -> rusqlite::Result<()> {
    let path = tx.path().expect("file database").to_string();
    let mut f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.write_all(&[0xAB; 8192]).unwrap();
    f.sync_all().unwrap();
    Err(rusqlite::Error::InvalidQuery)
}

static BAD_FUNC: [Migration; 7] = [
    MIGRATIONS[0],
    MIGRATIONS[1],
    MIGRATIONS[2],
    MIGRATIONS[3],
    MIGRATIONS[4],
    MIGRATIONS[5],
    Migration {
        version: 7,
        step: Step::Func(fails_after_partial_work),
    },
];
static BAD_SQL: [Migration; 7] = [
    MIGRATIONS[0],
    MIGRATIONS[1],
    MIGRATIONS[2],
    MIGRATIONS[3],
    MIGRATIONS[4],
    MIGRATIONS[5],
    Migration {
        version: 7,
        step: Step::Sql("CREATE TABLE t (x); THIS IS NOT SQL;"),
    },
];
static CORRUPTING: [Migration; 7] = [
    MIGRATIONS[0],
    MIGRATIONS[1],
    MIGRATIONS[2],
    MIGRATIONS[3],
    MIGRATIONS[4],
    MIGRATIONS[5],
    Migration {
        version: 7,
        step: Step::Func(corrupts_the_file_then_fails),
    },
];
static GOOD_V2: [Migration; 7] = [
    MIGRATIONS[0],
    MIGRATIONS[1],
    MIGRATIONS[2],
    MIGRATIONS[3],
    MIGRATIONS[4],
    MIGRATIONS[5],
    Migration {
        version: 7,
        step: Step::Sql("CREATE TABLE ok2 (x INTEGER);"),
    },
];

struct Fixture {
    tmp: tempfile::TempDir,
    master: common::Keys,
    meeting: String,
}

fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let master = common::keys();
    let store = common::open_with(tmp.path(), &master, MIGRATIONS).unwrap();
    let meeting = common::meeting(&store, "Dữ liệu quan trọng");
    store
        .add_segments(
            &meeting,
            vec![
                common::seg(0, 1000, "Đoạn một"),
                common::seg(1000, 2000, "Đoạn hai"),
            ],
        )
        .unwrap();
    Fixture {
        tmp,
        master,
        meeting,
    }
}

/// The database is exactly as before, and fully usable.
fn assert_intact_and_usable(f: &Fixture) {
    let store = common::open_with(f.tmp.path(), &f.master, MIGRATIONS)
        .expect("reopens with the real migrations");
    let m = store.get_meeting(&f.meeting).unwrap();
    assert_eq!(m.title, "Dữ liệu quan trọng");
    assert_eq!(
        m.status, "recording",
        "the failed migration's UPDATE is gone"
    );
    let segs = store.segments(&f.meeting).unwrap();
    assert_eq!(segs.len(), 2, "the failed migration's DELETE is gone");
    assert_eq!(segs[1].text, "Đoạn hai");
    assert_eq!(
        store
            .search(&ghi_store::search::SearchQuery::new("doan hai"))
            .unwrap()
            .len(),
        1
    );

    // Writable, and no trace of the failed migration.
    store
        .add_segment(&f.meeting, common::seg(2000, 3000, "Đoạn ba"))
        .unwrap();
    assert_eq!(store.segments(&f.meeting).unwrap().len(), 3);
    drop(store);
    let conn = db::open(&f.tmp.path().join("ghira.db"), &common::db_key(&f.master)).unwrap();
    assert_eq!(db::user_version(&conn).unwrap(), 6);
    for t in ["half_done", "t"] {
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = ?1",
                [t],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "table {t} from the failed migration must not exist");
    }
    let check: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");
}

#[test]
fn a_failing_rust_migration_is_rolled_back() {
    let f = fixture();
    let Err(err) = common::open_with(f.tmp.path(), &f.master, &BAD_FUNC) else {
        panic!("the migration should fail");
    };
    assert!(
        matches!(err, StoreError::Migration { version: 7, .. }),
        "{err}"
    );
    assert_intact_and_usable(&f);
}

#[test]
fn a_sql_error_is_rolled_back() {
    let f = fixture();
    let Err(err) = common::open_with(f.tmp.path(), &f.master, &BAD_SQL) else {
        panic!("the migration should fail");
    };
    assert!(
        matches!(err, StoreError::Migration { version: 7, .. }),
        "{err}"
    );
    assert_intact_and_usable(&f);
}

#[test]
fn damage_a_rollback_cannot_undo_is_repaired_from_the_snapshot() {
    let f = fixture();
    let Err(err) = common::open_with(f.tmp.path(), &f.master, &CORRUPTING) else {
        panic!("the migration should fail");
    };
    assert!(
        matches!(err, StoreError::Migration { version: 7, .. }),
        "{err}"
    );
    // The snapshot survives the failure, and the database is back to normal.
    assert_eq!(
        migrate::list_snapshots(&f.tmp.path().join("snapshots"))
            .unwrap()
            .len(),
        1
    );
    assert_intact_and_usable(&f);
}

#[test]
fn after_a_failure_the_migration_can_be_retried() {
    let f = fixture();
    assert!(common::open_with(f.tmp.path(), &f.master, &BAD_FUNC).is_err());
    let store = common::open_with(f.tmp.path(), &f.master, &GOOD_V2).unwrap();
    assert_eq!(store.segments(&f.meeting).unwrap().len(), 2);
    drop(store);
    let conn = db::open(&f.tmp.path().join("ghira.db"), &common::db_key(&f.master)).unwrap();
    assert_eq!(db::user_version(&conn).unwrap(), 7);
}

/// Migration 0002 keeps a v1 action item's one citation in the new list.
#[test]
fn v2_copies_v1_action_anchors() {
    let tmp = tempfile::tempdir().unwrap();
    let master = common::keys();
    drop(common::open_with(tmp.path(), &master, &MIGRATIONS[..1]).unwrap());
    let path = tmp.path().join("ghira.db");
    let anchor = r#"{"meeting_gid":"m","t0_ms":1,"t1_ms":2,"transcript_version":1}"#;
    let conn = db::open(&path, &common::db_key(&master)).unwrap();
    // A v1 meeting row, written as v1 had it (today's code writes newer columns).
    let meeting = "m1";
    conn.execute(
        "INSERT INTO meetings (gid, started_at, dek_wrapped) VALUES (?1, 0, x'01')",
        [meeting],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO action_items (gid, meeting_id, text_ct, anchor_json)
         VALUES ('a1', (SELECT id FROM meetings WHERE gid = ?1), x'00', ?2)",
        rusqlite::params![meeting, anchor],
    )
    .unwrap();
    drop(conn);
    drop(common::open_with(tmp.path(), &master, MIGRATIONS).unwrap());
    let conn = db::open(&path, &common::db_key(&master)).unwrap();
    let (anchors, provenance): (String, String) = conn
        .query_row(
            "SELECT anchors_json, provenance FROM action_items WHERE gid = 'a1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(anchors, format!("[{anchor}]"));
    assert_eq!(provenance, "user");
}
