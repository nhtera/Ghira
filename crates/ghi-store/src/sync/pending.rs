// SPDX-License-Identifier: Apache-2.0
//! Orphans (slice 15-C2, doc 07 §9): records that arrived before their parent
//! wait in `sync_pending` (at most 10 000 rows or 7 days), are retried after
//! each batch, and are dropped if the parent is tombstoned.
//!
//! A parked record is stored as JSON with the gid of the device it came from,
//! so a retry applies it as that device sent it. A meeting record is never
//! parked whole (it carries its key): only a folder-only stub is.

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use super::apply::{ApplyOutcome, Ctx};
use super::records::Record;
use crate::Result;
use crate::store::{Store, now_ms};

/// Most parked records.
pub const MAX_PENDING: usize = 10_000;
/// How long a record may wait.
pub const MAX_PENDING_AGE_MS: i64 = 7 * 24 * 3600 * 1000;
/// Retry passes per batch (a retried record can unblock another).
const MAX_ROUNDS: usize = 16;

#[derive(Serialize, Deserialize)]
struct Parked {
    from: String,
    rec: Record,
}

impl Ctx<'_> {
    /// Holds `rec` until `parent` exists (or is tombstoned). A newer version
    /// of an already parked gid replaces it; the oldest parked row makes room
    /// when the table is full.
    pub(crate) fn park(&mut self, rec: &Record, parent: &str) -> Result<()> {
        let existing: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT record, received_at FROM sync_pending WHERE gid = ?1",
                [rec.gid()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let mut received_at = now_ms();
        if let Some((json, at)) = existing {
            received_at = at;
            let newer = serde_json::from_str::<Parked>(&json)
                .map(|p| p.rec.version() < rec.version())
                .unwrap_or(true);
            if !newer {
                return Ok(());
            }
        } else {
            let n: i64 = self
                .conn
                .query_row("SELECT count(*) FROM sync_pending", [], |r| r.get(0))?;
            if n as usize >= MAX_PENDING {
                self.conn.execute(
                    "DELETE FROM sync_pending WHERE gid IN
                     (SELECT gid FROM sync_pending ORDER BY received_at, gid LIMIT ?1)",
                    [n - MAX_PENDING as i64 + 1],
                )?;
            }
        }
        let json = serde_json::to_string(&Parked {
            from: self.sender_gid.clone(),
            rec: rec.clone(),
        })
        .map_err(|e| crate::StoreError::Invalid(e.to_string()))?;
        self.conn.execute(
            "INSERT OR REPLACE INTO sync_pending (gid, kind, parent_gid, record, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![rec.gid(), rec.kind().log_kind(), parent, json, received_at],
        )?;
        Ok(())
    }

    /// Forgets a parked record (it was applied, or its gid died).
    pub(crate) fn unpark(&mut self, gid: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM sync_pending WHERE gid = ?1", [gid])?;
        Ok(())
    }

    /// Drops what waited too long.
    pub(crate) fn expire_pending(&mut self) -> Result<()> {
        self.conn.execute(
            "DELETE FROM sync_pending WHERE received_at < ?1",
            [now_ms() - MAX_PENDING_AGE_MS],
        )?;
        Ok(())
    }

    /// Applies the parked records whose parent now exists or is tombstoned,
    /// until a pass changes nothing. A record that no longer validates is
    /// dropped (the batch that delivered its parent must not fail for it).
    /// Returns how many were applied.
    pub(crate) fn retry_parked(&mut self) -> Result<usize> {
        let (sender_id, sender_gid, spoke) = (self.sender_id, self.sender_gid.clone(), self.spoke);
        let mut applied = 0;
        for _ in 0..MAX_ROUNDS {
            let rows: Vec<(String, String, i64)> = self
                .conn
                .prepare(
                    "SELECT gid, record, received_at FROM sync_pending
                     WHERE parent_gid IN (SELECT gid FROM meetings
                                          UNION ALL SELECT gid FROM speakers
                                          UNION ALL SELECT gid FROM persons
                                          UNION ALL SELECT gid FROM folders
                                          UNION ALL SELECT gid FROM tags
                                          UNION ALL SELECT gid FROM tombstones)
                     ORDER BY received_at, gid",
                )?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let mut progress = false;
            for (gid, json, at) in rows {
                self.conn.execute_batch("SAVEPOINT retry_parked")?;
                let result = match serde_json::from_str::<Parked>(&json) {
                    Ok(p) => match self.set_sender(&p.from) {
                        Ok(()) => self.apply_one(&p.rec),
                        Err(e) => Err(e),
                    },
                    Err(e) => Err(crate::StoreError::Invalid(e.to_string())),
                };
                match result {
                    Ok(ApplyOutcome::Parked) => {
                        // Still waiting: it keeps its place in the queue.
                        self.conn.execute(
                            "UPDATE sync_pending SET received_at = ?2 WHERE gid = ?1",
                            rusqlite::params![gid, at],
                        )?;
                        self.conn.execute_batch("RELEASE retry_parked")?;
                    }
                    Ok(o) => {
                        self.conn.execute_batch("RELEASE retry_parked")?;
                        progress = true;
                        if matches!(o, ApplyOutcome::Accepted | ApplyOutcome::Merged) {
                            applied += 1;
                        }
                    }
                    Err(_) => {
                        self.conn
                            .execute_batch("ROLLBACK TO retry_parked; RELEASE retry_parked")?;
                        self.unpark(&gid)?;
                        progress = true;
                    }
                }
            }
            if !progress {
                break;
            }
        }
        self.sender_id = sender_id;
        self.sender_gid = sender_gid;
        self.spoke = spoke;
        Ok(applied)
    }
}

impl Store {
    /// Retries parked records whose parent has arrived; returns how many
    /// were applied.
    pub fn retry_pending(&self) -> Result<usize> {
        let (n, post) = {
            let mut conn = self.conn();
            let tx = conn.transaction()?;
            let mut ctx = Ctx::new(self, &tx, None)?;
            let n = ctx.retry_parked()?;
            ctx.expire_pending()?;
            ctx.finish()?;
            let post = ctx.into_post();
            tx.commit()?;
            (n, post)
        };
        self.finish_post(post)?;
        Ok(n)
    }

    /// Parked records.
    pub fn pending_count(&self) -> Result<usize> {
        let n: i64 = self
            .conn()
            .query_row("SELECT count(*) FROM sync_pending", [], |r| r.get(0))?;
        Ok(n as usize)
    }
}
