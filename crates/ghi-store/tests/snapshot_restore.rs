// SPDX-License-Identifier: Apache-2.0
//! Pre-migration snapshots: taken before each migration of an existing
//! database, encrypted, pruned to the last N, and restorable.

mod common;

use ghi_store::keys::KeyRing;
use ghi_store::migrate::{self, KEEP_SNAPSHOTS, MIGRATIONS, Migration, Step};
use ghi_store::store::Store;
use ghi_store::{StoreError, db};

const fn add_table(version: u32, sql: &'static str) -> Migration {
    Migration {
        version,
        step: Step::Sql(sql),
    }
}

/// Today's schema: the "current" database the tests upgrade from (the store
/// API writes today's columns).
const BASE: &[Migration] = MIGRATIONS;

/// The schema version after one more (test) migration.
fn next() -> u32 {
    migrate::latest_version() + 1
}

/// Today's migrations plus one more.
fn plus(m: Migration) -> Vec<Migration> {
    let mut v = MIGRATIONS.to_vec();
    v.push(m);
    v
}

fn v2() -> Vec<Migration> {
    plus(add_table(next(), "CREATE TABLE extra2 (x INTEGER);"))
}
fn version(store: &Store, dir: &std::path::Path, master: &common::Keys) -> u32 {
    let _ = store;
    let conn = db::open(&dir.join("ghira.db"), &common::db_key(master)).unwrap();
    db::user_version(&conn).unwrap()
}

fn failing(_: &rusqlite::Transaction) -> rusqlite::Result<()> {
    Err(rusqlite::Error::InvalidQuery)
}

fn fail_v2() -> Vec<Migration> {
    plus(Migration {
        version: next(),
        step: Step::Func(failing),
    })
}

/// Makes a failed upgrade, which leaves its snapshot behind.
fn fail_upgrade(dir: &std::path::Path, master: &common::Keys) {
    assert!(common::open_with(dir, master, &fail_v2()).is_err());
}

#[test]
fn snapshots_exist_during_a_migration_and_are_deleted_after_success() {
    let tmp = tempfile::tempdir().unwrap();
    let master = common::keys();
    let snaps = tmp.path().join("snapshots");

    let store = common::open_with(tmp.path(), &master, BASE).unwrap();
    let m = common::meeting(&store, "Trước khi nâng cấp");
    drop(store);
    assert!(
        migrate::list_snapshots(&snaps).unwrap().is_empty(),
        "nothing to preserve on first run"
    );

    // A failed upgrade keeps its snapshot ...
    fail_upgrade(tmp.path(), &master);
    let list = migrate::list_snapshots(&snaps).unwrap();
    assert_eq!(list.len(), 1);
    assert!(
        list[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(&format!("-pre-v{:04}.db", migrate::latest_version()))
    );

    // ... and the next successful run deletes it (snapshots hold wrapped DEKs).
    let store = common::open_with(tmp.path(), &master, &v2()).unwrap();
    assert!(migrate::list_snapshots(&snaps).unwrap().is_empty());
    assert_eq!(store.get_meeting(&m).unwrap().title, "Trước khi nâng cấp");
    assert_eq!(version(&store, tmp.path(), &master), next());
}

#[test]
fn snapshots_are_encrypted_with_the_same_key() {
    let tmp = tempfile::tempdir().unwrap();
    let master = common::keys();
    let store = common::open_with(tmp.path(), &master, BASE).unwrap();
    common::meeting(&store, "Bí mật");
    drop(store);
    fail_upgrade(tmp.path(), &master);

    let snap = migrate::list_snapshots(&tmp.path().join("snapshots"))
        .unwrap()
        .remove(0);
    let bytes = std::fs::read(&snap).unwrap();
    assert!(
        !bytes.starts_with(b"SQLite format 3"),
        "a snapshot is not a plain SQLite file"
    );
    // Opens with the right key, not with another.
    let conn = db::open(&snap, &common::db_key(&master)).unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM meetings", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
    drop(conn);
    assert!(matches!(
        db::open(&snap, &KeyRing::generate().db_key()),
        Err(StoreError::Db(_))
    ));
}

#[test]
fn only_the_last_snapshots_are_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let master = common::keys();
    drop(common::open_with(tmp.path(), &master, BASE).unwrap());
    for _ in 0..6 {
        std::thread::sleep(std::time::Duration::from_millis(3)); // distinct names
        fail_upgrade(tmp.path(), &master);
    }
    assert_eq!(
        migrate::list_snapshots(&tmp.path().join("snapshots"))
            .unwrap()
            .len(),
        KEEP_SNAPSHOTS
    );
}

#[test]
fn a_snapshot_restores_the_pre_migration_database() {
    let tmp = tempfile::tempdir().unwrap();
    let master = common::keys();
    let store = common::open_with(tmp.path(), &master, BASE).unwrap();
    let keep = common::meeting(&store, "Giữ lại");
    drop(store);
    fail_upgrade(tmp.path(), &master);
    let snap = migrate::list_snapshots(&tmp.path().join("snapshots"))
        .unwrap()
        .remove(0);
    // The next successful open deletes snapshots, so keep a copy to restore from.
    let kept_copy = tmp.path().join("kept-copy.db");
    std::fs::copy(&snap, &kept_copy).unwrap();
    let snap = kept_copy;

    let store = common::open_with(tmp.path(), &master, BASE).unwrap();
    let later = common::meeting(&store, "Sau đó");
    drop(store);

    migrate::restore_snapshot(&snap, &tmp.path().join("ghira.db")).unwrap();

    let store = common::open_with(tmp.path(), &master, BASE).unwrap();
    assert_eq!(
        version(&store, tmp.path(), &master),
        migrate::latest_version()
    );
    assert_eq!(store.get_meeting(&keep).unwrap().title, "Giữ lại");
    assert!(matches!(
        store.get_meeting(&later),
        Err(StoreError::NotFound { .. })
    ));
}

#[test]
fn a_newer_database_is_refused_not_downgraded() {
    let tmp = tempfile::tempdir().unwrap();
    let master = common::keys();
    drop(common::open_with(tmp.path(), &master, &v2()).unwrap());
    let Err(err) = common::open_with(tmp.path(), &master, BASE) else {
        panic!("an old build must not open a newer database");
    };
    assert!(matches!(err, StoreError::Invalid(_)), "{err}");
    // And the data is untouched.
    assert_eq!(
        version(
            &common::open_with(tmp.path(), &master, &v2()).unwrap(),
            tmp.path(),
            &master
        ),
        next()
    );
}

#[test]
fn a_wrong_key_is_an_error_not_a_new_database() {
    let tmp = tempfile::tempdir().unwrap();
    drop(common::open_with(tmp.path(), &common::keys(), migrate::MIGRATIONS).unwrap());
    // Another ring in the keystore: the database won't open.
    let other = common::keys_with(KeyRing::generate());
    let Err(err) = common::open_with(tmp.path(), &other, migrate::MIGRATIONS) else {
        panic!("opened with the wrong key");
    };
    assert!(matches!(err, StoreError::Db(_)), "{err}");
    // An empty keystore next to an existing database is not a fresh install.
    let Err(err) = common::open_with(tmp.path(), &common::keys(), migrate::MIGRATIONS) else {
        panic!("created a useless key");
    };
    assert!(
        matches!(&err, StoreError::Invalid(m) if m.contains("recovery phrase")),
        "{err}"
    );
}

#[test]
fn connection_pragmas_are_set() {
    let tmp = tempfile::tempdir().unwrap();
    let master = common::keys();
    drop(common::open_with(tmp.path(), &master, migrate::MIGRATIONS).unwrap());
    let conn = db::open(&tmp.path().join("ghira.db"), &common::db_key(&master)).unwrap();
    let pragma = |name: &str| -> i64 {
        conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(
        pragma("auto_vacuum"),
        2,
        "INCREMENTAL, set before the first table"
    );
    assert_eq!(pragma("secure_delete"), 1);
    assert_eq!(pragma("foreign_keys"), 1);
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    let cipher: String = conn
        .query_row("PRAGMA cipher_version", [], |r| r.get(0))
        .unwrap();
    assert!(!cipher.is_empty(), "SQLCipher, not plain SQLite");
}

#[test]
fn a_wrong_key_leaves_the_file_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let master = common::keys();
    let store = common::open_with(tmp.path(), &master, migrate::MIGRATIONS).unwrap();
    common::meeting(&store, "x");
    drop(store);
    let path = tmp.path().join("ghira.db");
    let before = std::fs::read(&path).unwrap();
    for _ in 0..2 {
        assert!(matches!(
            db::open(&path, &KeyRing::generate().db_key()),
            Err(StoreError::Db(_))
        ));
    }
    assert_eq!(std::fs::read(&path).unwrap(), before);
    // And the right key still works.
    assert!(common::open_with(tmp.path(), &master, migrate::MIGRATIONS).is_ok());
}
