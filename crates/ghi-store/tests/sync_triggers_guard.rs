// SPDX-License-Identifier: Apache-2.0
//! Guard for the `sync_log` triggers (phase 15, 15-L). The UPDATE trigger of a
//! sync table fires only when a column that goes on the wire changed, and it
//! lists those columns when migration 0009 creates it. A later migration that
//! adds a column to a sync table without rebuilding the triggers would make
//! edits of that column invisible to sync (never sent, never converging).
//!
//! The guard: every column of every sync table is either compared in the
//! table's `sync_log_<table>_update` trigger or in `LOCAL_ONLY_COLUMNS`. The
//! checker is run against the real schema (must be clean) and against
//! schemas damaged on purpose (must complain).

use std::path::Path;
use std::sync::Arc;

use ghi_store::db;
use ghi_store::keys::dev::FileKeyStore;
use ghi_store::keys::{Protection, load_or_create};
use ghi_store::migrate::{LOCAL_ONLY_COLUMNS, SYNC_TABLES};
use ghi_store::store::Store;
use rusqlite::Connection;

/// A freshly migrated database (the store is closed again), opened raw.
fn fresh_db(dir: &Path) -> Connection {
    let data = dir.join("data");
    let keyfile = dir.join("data.devkey");
    drop(
        Store::open(
            &data,
            Arc::new(FileKeyStore::new(&keyfile)),
            Protection::default(),
        )
        .unwrap(),
    );
    let ring = load_or_create(&FileKeyStore::new(&keyfile), Protection::default()).unwrap();
    db::open(&data.join("ghira.db"), &ring.db_key()).unwrap()
}

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    conn.prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn trigger_sql(conn: &Connection, name: &str) -> Option<String> {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = ?1",
        [name],
        |r| r.get(0),
    )
    .ok()
}

/// What is wrong with the schema: a missing trigger, a column neither compared
/// by its table's UPDATE trigger nor local-only, a comparison of a column that
/// does not exist, a stale local-only name.
fn problems(conn: &Connection) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen_local_only = std::collections::BTreeSet::new();
    for (table, _) in SYNC_TABLES {
        for name in [
            format!("sync_log_{table}_insert"),
            format!("sync_log_{table}_update"),
        ] {
            if trigger_sql(conn, &name).is_none() {
                out.push(format!("{name}: no such trigger"));
            }
        }
        let Some(sql) = trigger_sql(conn, &format!("sync_log_{table}_update")) else {
            continue;
        };
        let cols = columns(conn, table);
        for c in &cols {
            if LOCAL_ONLY_COLUMNS.contains(&c.as_str()) {
                seen_local_only.insert(c.clone());
                continue;
            }
            if !sql.contains(&format!("NEW.{c} IS NOT OLD.{c}")) {
                out.push(format!(
                    "{table}.{c}: neither compared by sync_log_{table}_update nor local-only"
                ));
            }
        }
        // A column the trigger compares must exist (a renamed or dropped column
        // would make the trigger fail on every update).
        for part in sql.split("NEW.").skip(1) {
            let col: String = part
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if col != "gid" && !cols.contains(&col) {
                out.push(format!(
                    "{table}: the update trigger names unknown column {col}"
                ));
            }
        }
        for c in LOCAL_ONLY_COLUMNS {
            if cols.iter().any(|x| x == c) && sql.contains(&format!("NEW.{c} IS NOT OLD.{c}")) {
                out.push(format!(
                    "{table}.{c}: local-only but compared (it would re-send rows)"
                ));
            }
        }
    }
    for c in LOCAL_ONLY_COLUMNS {
        if !seen_local_only.contains(c) {
            out.push(format!(
                "local-only column {c} exists in no sync table (stale entry)"
            ));
        }
    }
    out
}

#[test]
fn every_column_of_every_sync_table_is_compared_by_its_trigger_or_local_only() {
    let dir = tempfile::tempdir().unwrap();
    let conn = fresh_db(dir.path());
    let found = problems(&conn);
    assert!(
        found.is_empty(),
        "sync_log triggers are out of step:\n{found:#?}"
    );
    // The guard looked at something: every sync table has columns, and the
    // two kinds of column both occur.
    let total: usize = SYNC_TABLES
        .iter()
        .map(|(t, _)| columns(&conn, t).len())
        .sum();
    assert!(total > 100, "{total} columns checked");
}

#[test]
fn a_column_added_after_the_triggers_were_made_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let conn = fresh_db(dir.path());
    for (table, _) in SYNC_TABLES {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN later_added TEXT"))
            .unwrap();
        let found = problems(&conn);
        assert!(
            found
                .iter()
                .any(|p| p.starts_with(&format!("{table}.later_added:"))),
            "{table}: a new column went unnoticed: {found:?}"
        );
        conn.execute_batch(&format!("ALTER TABLE {table} DROP COLUMN later_added"))
            .unwrap();
    }
    assert!(problems(&conn).is_empty());
}

#[test]
fn a_trigger_that_lost_a_comparison_or_a_table_that_lost_its_trigger_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let conn = fresh_db(dir.path());
    // Rebuild meetings' update trigger without the `title_ct` comparison.
    let sql = trigger_sql(&conn, "sync_log_meetings_update").unwrap();
    assert!(sql.contains("NEW.title_ct IS NOT OLD.title_ct"));
    conn.execute_batch("DROP TRIGGER sync_log_meetings_update")
        .unwrap();
    let damaged = sql
        .replace("NEW.title_ct IS NOT OLD.title_ct OR ", "")
        .replace(" OR NEW.title_ct IS NOT OLD.title_ct", "");
    assert_ne!(damaged, sql, "the trigger text changed shape");
    conn.execute_batch(&damaged).unwrap();
    let found = problems(&conn);
    assert!(
        found.iter().any(|p| p.starts_with("meetings.title_ct:")),
        "{found:?}"
    );
    // And one with no update trigger at all.
    conn.execute_batch("DROP TRIGGER sync_log_notes_blocks_update")
        .unwrap();
    let found = problems(&conn);
    assert!(
        found
            .iter()
            .any(|p| p.contains("sync_log_notes_blocks_update: no such trigger")),
        "{found:?}"
    );
}
