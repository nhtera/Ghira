// SPDX-License-Identifier: Apache-2.0
//! Opening the SQLCipher database.
//!
//! The key is the raw 256-bit HKDF-derived database key
//! ([`MasterKey::db_key`](crate::keys::MasterKey::db_key)), passed as
//! `PRAGMA key = "x'<hex>'"` so SQLCipher skips its PBKDF. Pragmas: see [`open`].

use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::rowcrypt::Dek;
use crate::{Result, StoreError};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Opens (creating if missing) the encrypted database at `path` with `key`.
///
/// Order matters: key, `cipher_compatibility = 4`, verify the key (a wrong key
/// is a [`StoreError::Db`] and the file is not touched), then WAL, then
/// `auto_vacuum=INCREMENTAL` (only at creation, before the first table; on an
/// existing database it is asserted instead), then the per-connection pragmas.
pub fn open(path: &Path, key: &Dek) -> Result<Connection> {
    let conn = Connection::open(path)?;
    apply_key(&conn, key)?;
    conn.execute_batch("PRAGMA cipher_compatibility = 4;")?;
    // Freed pages are wiped, not just returned (measured on the Mac: hybrid
    // search p95 218 -> 246 ms at 1,000 meetings). Not on iOS yet: its page
    // cache is small (8 MB) and the cost (every free, recording included) is
    // unmeasured on a phone.
    if !cfg!(target_os = "ios") {
        conn.execute_batch("PRAGMA cipher_memory_security = ON;")?;
    }
    let fresh = conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })? == 0;
    if fresh {
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
    }
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL;", [], |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StoreError::Invalid(format!(
            "database refused WAL mode ({mode})"
        )));
    }
    let auto_vacuum: i64 = conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?;
    if !fresh && auto_vacuum != 2 {
        return Err(StoreError::Invalid(
            "database was not created with auto_vacuum=INCREMENTAL".into(),
        ));
    }
    // Pages are decrypted on every cache miss, and a keyword search reads
    // thousands of them (about half of a 1,000-meeting hybrid search), so the
    // page cache is raised from the 2 MB default. iOS keeps it small: the
    // memory limit (jetsam) is far lower there. The app lock
    // gives the pages back (`Store::release_page_cache`).
    conn.execute_batch(if cfg!(target_os = "ios") {
        "PRAGMA cache_size = -8192;"
    } else {
        "PRAGMA cache_size = -65536;"
    })?;
    conn.execute_batch(
        "PRAGMA secure_delete = ON;
         PRAGMA foreign_keys = ON;
         PRAGMA temp_store = MEMORY;
         PRAGMA journal_size_limit = 4194304;",
    )?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    Ok(conn)
}

/// Sets the SQLCipher key on a fresh connection.
pub fn apply_key(conn: &Connection, key: &Dek) -> Result<()> {
    let mut sql = Zeroizing::new(String::with_capacity(64 + 16));
    sql.push_str("PRAGMA key = \"x'");
    for b in key.as_bytes() {
        sql.push_str(&format!("{b:02x}"));
    }
    sql.push_str("'\";");
    conn.execute_batch(&sql)?;
    Ok(())
}

/// `PRAGMA user_version`: the number of the last applied migration.
pub fn user_version(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// Flushes the WAL into the main file and truncates it to zero bytes. The
/// checkpoint reports `busy` when a reader holds the WAL; retry briefly, then
/// fail rather than pretend the WAL is empty.
pub fn checkpoint(conn: &Connection) -> Result<()> {
    for attempt in 0..20 {
        let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
        if busy == 0 {
            return Ok(());
        }
        if attempt < 19 {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    Err(StoreError::Invalid(
        "could not checkpoint the database: it is busy".into(),
    ))
}

/// A consistent, encrypted copy of the database (SQLite backup API, same
/// key) at `dest`, which must not exist. No `-wal`/`-shm` file is left behind.
pub fn backup_to(conn: &Connection, dest: &Path, key: &Dek) -> Result<()> {
    if dest.exists() {
        return Err(StoreError::Invalid("backup destination exists".into()));
    }
    {
        let mut out = Connection::open(dest)?;
        apply_key(&out, key)?;
        out.execute_batch("PRAGMA cipher_compatibility = 4;")?;
        let backup = rusqlite::backup::Backup::new(conn, &mut out)?;
        backup.run_to_completion(256, Duration::from_millis(2), None)?;
        drop(backup);
        out.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        let _ = out.close();
    }
    for suffix in ["-wal", "-shm"] {
        let mut p = dest.as_os_str().to_owned();
        p.push(suffix);
        let _ = std::fs::remove_file(std::path::PathBuf::from(p));
    }
    Ok(())
}

/// Returns free pages to the OS (needs `auto_vacuum=INCREMENTAL`).
pub fn incremental_vacuum(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA incremental_vacuum;")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypted_pages_are_wiped_when_freed_and_the_cache_is_sized() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = open(&tmp.path().join("t.db"), &Dek::generate()).unwrap();
        let sec: String = conn
            .query_row("PRAGMA cipher_memory_security", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            matches!(sec.as_str(), "1" | "ON"),
            !cfg!(target_os = "ios"),
            "{sec}"
        );
        let cache: i64 = conn
            .query_row("PRAGMA cache_size", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            cache,
            if cfg!(target_os = "ios") {
                -8192
            } else {
                -65536
            }
        );
    }
}
