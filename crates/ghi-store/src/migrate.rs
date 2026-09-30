// SPDX-License-Identifier: Apache-2.0
//! Schema migrations.
//!
//! `PRAGMA user_version` is the number of the last applied migration. Before
//! each migration of an existing database:
//! 1. the WAL is checkpointed (`TRUNCATE`) and the (encrypted) database file is
//!    copied to `<dir>/snapshots/` (a failed run leaves its snapshot, at most
//!    [`KEEP_SNAPSHOTS`]; a successful run deletes them all);
//! 2. the migration and the `user_version` bump run in one transaction.
//!
//! If a migration fails the transaction is rolled back and the snapshot is
//! restored over the database file (belt and braces: it also covers failures
//! a rollback can't, such as a crash in `COMMIT` or a partly-run Rust step),
//! and the caller gets [`StoreError::Migration`]. Snapshots hold wrapped
//! meeting keys, so they are deleted after a successful migration and by
//! `Store::delete_meeting`.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, Transaction};

use crate::db;
use crate::{Result, StoreError};

/// How many pre-migration snapshots are kept.
pub const KEEP_SNAPSHOTS: usize = 3;

/// The body of a migration.
#[derive(Clone, Copy)]
pub enum Step {
    Sql(&'static str),
    /// For data migrations that need Rust. Also the test hook for injecting
    /// failures.
    Func(fn(&Transaction) -> rusqlite::Result<()>),
}

#[derive(Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub step: Step,
}

/// All migrations, in order. Versions start at 1 and have no gaps.
pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    step: Step::Sql(include_str!("migrations/0001_init.sql")),
}];

/// The schema version this build expects.
pub fn latest_version() -> u32 {
    MIGRATIONS.last().map_or(0, |m| m.version)
}

/// Brings `conn` (the database at `db_path`) up to date.
///
/// Takes the connection by value: on failure it is closed so the snapshot can
/// be restored over the file; reopen with [`db::open`] afterwards.
pub fn run(
    mut conn: Connection,
    db_path: &Path,
    migrations: &[Migration],
    snapshots_dir: &Path,
    keep: usize,
) -> Result<Connection> {
    let start = db::user_version(&conn)?;
    let mut current = start;
    if let Some(newest) = migrations.last()
        && start > newest.version
    {
        return Err(StoreError::Invalid(format!(
            "database is version {start}, this build only knows up to {}",
            newest.version
        )));
    }
    for m in migrations.iter().filter(|m| m.version > start) {
        let snapshot = if current > 0 {
            Some(take_snapshot(&conn, db_path, snapshots_dir, current, keep)?)
        } else {
            None
        };
        match apply(&mut conn, m) {
            Ok(()) => current = m.version,
            Err(e) => {
                let detail = e.to_string();
                let _ = conn.close();
                if let Some(snap) = snapshot {
                    restore_snapshot(&snap, db_path)?;
                }
                return Err(StoreError::Migration {
                    version: m.version,
                    detail,
                });
            }
        }
    }
    // Snapshots hold every wrapped meeting key: don't keep them once the
    // database is known good.
    purge_snapshots(snapshots_dir)?;
    Ok(conn)
}

fn apply(conn: &mut Connection, m: &Migration) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    match m.step {
        Step::Sql(sql) => tx.execute_batch(sql)?,
        Step::Func(f) => f(&tx)?,
    }
    tx.pragma_update(None, "user_version", m.version)?;
    tx.commit()
}

fn sidecar(db_path: &Path, suffix: &str) -> PathBuf {
    let mut s: OsString = db_path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Copies the database file (after folding the WAL into it) to the snapshots dir.
fn take_snapshot(
    conn: &Connection,
    db_path: &Path,
    dir: &Path,
    version: u32,
    keep: usize,
) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;
    db::checkpoint(conn)?;
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let snap = dir.join(format!("{ms:015}-pre-v{version:04}.db"));
    fs::copy(db_path, &snap)?;
    fs::File::open(&snap)?.sync_all()?;
    let mut all = list_snapshots(dir)?;
    while all.len() > keep.max(1) {
        fs::remove_file(all.remove(0))?;
    }
    Ok(snap)
}

/// Snapshots, oldest first.
pub fn list_snapshots(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    match fs::read_dir(dir) {
        Ok(rd) => {
            for e in rd {
                let p = e?.path();
                if p.extension().is_some_and(|x| x == "db") {
                    out.push(p);
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    out.sort();
    Ok(out)
}

/// Deletes every snapshot (they contain wrapped meeting keys).
pub fn purge_snapshots(dir: &Path) -> Result<()> {
    for p in list_snapshots(dir)? {
        fs::remove_file(p)?;
    }
    Ok(())
}

/// Replaces the database file with `snapshot` (and drops stale WAL/SHM files).
pub fn restore_snapshot(snapshot: &Path, db_path: &Path) -> Result<()> {
    let tmp = sidecar(db_path, ".restore");
    fs::copy(snapshot, &tmp)?;
    fs::File::open(&tmp)?.sync_all()?;
    for suffix in ["-wal", "-shm"] {
        match fs::remove_file(sidecar(db_path, suffix)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    fs::rename(tmp, db_path)?;
    Ok(())
}
