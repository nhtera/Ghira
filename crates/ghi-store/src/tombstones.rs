// SPDX-License-Identifier: Apache-2.0
//! Tombstones: a record that a syncable row was deleted.
//!
//! They carry only `(gid, kind, lamport, deleted_at)`, never content, so they
//! are kept forever (never garbage-collected) and drive sync deletes
//! (phase 15). Deleting a meeting writes one for the meeting and one for every
//! child row before anything is removed.

use rusqlite::{Connection, Params, params};

use crate::Result;
use crate::store::{Store, now_ms};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tombstone {
    pub gid: String,
    pub kind: String,
    pub lamport: i64,
    pub deleted_at: i64,
}

/// Records one deletion (idempotent).
pub(crate) fn write(conn: &Connection, gid: &str, kind: &str, lamport: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO tombstones (gid, kind, lamport, deleted_at) VALUES (?1, ?2, ?3, ?4)",
        params![gid, kind, lamport, now_ms()],
    )?;
    Ok(())
}

/// Records a deletion for every gid `select_gids` returns. `select_gids` is a
/// literal from this crate, never user input.
pub(crate) fn write_where(
    conn: &Connection,
    kind: &'static str,
    select_gids: &'static str,
    params: impl Params,
    lamport: i64,
) -> Result<()> {
    conn.execute(
        &format!(
            "INSERT OR IGNORE INTO tombstones (gid, kind, lamport, deleted_at)
             SELECT gid, '{kind}', {lamport}, {} FROM ({select_gids})",
            now_ms()
        ),
        params,
    )?;
    Ok(())
}

/// Tombstones for all child rows of a meeting.
pub(crate) fn write_children(conn: &Connection, meeting_id: i64, lamport: i64) -> Result<()> {
    write_where(
        conn,
        "track",
        "SELECT gid FROM tracks WHERE meeting_id = ?1",
        [meeting_id],
        lamport,
    )?;
    write_where(
        conn,
        "speaker",
        "SELECT gid FROM speakers WHERE meeting_id = ?1",
        [meeting_id],
        lamport,
    )?;
    write_where(
        conn,
        "segment",
        "SELECT gid FROM segments WHERE meeting_id = ?1",
        [meeting_id],
        lamport,
    )?;
    write_where(
        conn,
        "note",
        "SELECT gid FROM notes_blocks WHERE meeting_id = ?1",
        [meeting_id],
        lamport,
    )?;
    write_where(
        conn,
        "action_item",
        "SELECT gid FROM action_items WHERE meeting_id = ?1",
        [meeting_id],
        lamport,
    )?;
    write_where(
        conn,
        "mark",
        "SELECT gid FROM marks WHERE meeting_id = ?1",
        [meeting_id],
        lamport,
    )?;
    Ok(())
}

impl Store {
    /// Tombstones with `lamport > since`, oldest first (for sync).
    pub fn tombstones_since(&self, since: i64) -> Result<Vec<Tombstone>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT gid, kind, lamport, deleted_at FROM tombstones WHERE lamport > ?1 ORDER BY lamport, gid",
        )?;
        let rows = stmt.query_map([since], |r| {
            Ok(Tombstone {
                gid: r.get(0)?,
                kind: r.get(1)?,
                lamport: r.get(2)?,
                deleted_at: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn is_tombstoned(&self, gid: &str) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM tombstones WHERE gid = ?1)",
            [gid],
            |r| r.get(0),
        )?)
    }
}
