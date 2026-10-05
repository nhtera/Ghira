// SPDX-License-Identifier: Apache-2.0
//! Tombstones: a record that a syncable row was deleted.
//!
//! They carry only `(gid, kind, lamport, deleted_at)`, never content, so they
//! are kept forever (never garbage-collected) and drive sync deletes
//! (phase 15). Deleting a meeting writes one for the meeting and one for every
//! child row before anything is removed.

use rusqlite::{Connection, Params, params};

use crate::store::{Store, now_ms};
use crate::{Result, StoreError};

/// Why a row was deleted (doc 07 §7.2). Sync's rules differ per cause: only
/// `Regenerate` may keep a user's concurrent edit as a copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    /// The user deleted the row.
    User,
    /// The meeting was deleted (it and every child).
    Meeting,
    /// Notes were regenerated.
    Regenerate,
    /// The transcript was cut back (`discard_after`).
    Discard,
    /// The audio retention sweep.
    Retention,
    /// A new transcript version replaced the old one.
    Transcript,
    /// Dropped by a newer epoch while merging.
    Superseded,
}

impl Cause {
    pub fn as_str(self) -> &'static str {
        match self {
            Cause::User => "user",
            Cause::Meeting => "meeting",
            Cause::Regenerate => "regenerate",
            Cause::Discard => "discard",
            Cause::Retention => "retention",
            Cause::Transcript => "transcript",
            Cause::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tombstone {
    pub gid: String,
    pub kind: String,
    pub lamport: i64,
    pub deleted_at: i64,
}

/// Records one deletion (idempotent).
pub(crate) fn write(
    conn: &Connection,
    gid: &str,
    kind: &str,
    lamport: i64,
    cause: Cause,
) -> Result<()> {
    write_from(conn, gid, kind, lamport, cause, None)
}

/// [`write`] for a deletion that came from a peer (`origin` = `devices.id`).
pub(crate) fn write_from(
    conn: &Connection,
    gid: &str,
    kind: &str,
    lamport: i64,
    cause: Cause,
    origin: Option<i64>,
) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO tombstones (gid, kind, lamport, deleted_at, cause, origin)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![gid, kind, lamport, now_ms(), cause.as_str(), origin],
    )?;
    Ok(())
}

/// Fails with [`StoreError::Tombstoned`] when `gid` was deleted: a tombstoned
/// gid is never inserted again, whatever its version (doc 07 §7.5). Call it
/// on every insert path, inside the inserting transaction.
pub(crate) fn assert_live(conn: &Connection, gid: &str) -> Result<()> {
    let dead: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM tombstones WHERE gid = ?1)",
        [gid],
        |r| r.get(0),
    )?;
    if dead {
        return Err(StoreError::Tombstoned {
            gid: gid.to_string(),
        });
    }
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
    cause: Cause,
) -> Result<()> {
    conn.execute(
        &format!(
            "INSERT OR IGNORE INTO tombstones (gid, kind, lamport, deleted_at, cause)
             SELECT gid, '{kind}', {lamport}, {}, '{}' FROM ({select_gids})",
            now_ms(),
            cause.as_str()
        ),
        params,
    )?;
    Ok(())
}

/// Tombstones for all child rows of a meeting.
pub(crate) fn write_children(
    conn: &Connection,
    meeting_id: i64,
    lamport: i64,
    cause: Cause,
) -> Result<()> {
    write_where(
        conn,
        "track",
        "SELECT gid FROM tracks WHERE meeting_id = ?1",
        [meeting_id],
        lamport,
        cause,
    )?;
    write_where(
        conn,
        "speaker",
        "SELECT gid FROM speakers WHERE meeting_id = ?1",
        [meeting_id],
        lamport,
        cause,
    )?;
    write_where(
        conn,
        "segment",
        "SELECT gid FROM segments WHERE meeting_id = ?1",
        [meeting_id],
        lamport,
        cause,
    )?;
    write_where(
        conn,
        "note",
        "SELECT gid FROM notes_blocks WHERE meeting_id = ?1",
        [meeting_id],
        lamport,
        cause,
    )?;
    write_where(
        conn,
        "action_item",
        "SELECT gid FROM action_items WHERE meeting_id = ?1",
        [meeting_id],
        lamport,
        cause,
    )?;
    write_where(
        conn,
        "meeting_tag",
        "SELECT gid FROM meeting_tags WHERE meeting_id = ?1",
        [meeting_id],
        lamport,
        cause,
    )?;
    write_where(
        conn,
        "mark",
        "SELECT gid FROM marks WHERE meeting_id = ?1",
        [meeting_id],
        lamport,
        cause,
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
