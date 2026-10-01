// SPDX-License-Identifier: Apache-2.0
//! Audio retention.
//!
//! A meeting may carry `audio_retained_until` (unix ms). [`Store::retention_sweep`]
//! deletes the audio of every meeting past that time and keeps the text
//! (transcript, notes, action items). Afterwards its anchors still resolve to
//! segments, with `audio_available = false` (doc 05 §2.3).
//!
//! A meeting whose final pass is still queued or running keeps its audio (a
//! recording made before the models arrived has no transcript without it).
//!
//! This removes the bundle files and the `tracks` rows (with tombstones). It is
//! not a crypto-shred: the meeting's key must stay for the text. Use
//! [`Store::delete_meeting`] to make everything unreadable.

use rusqlite::params;

use crate::Result;
use crate::store::{Store, now_ms};
use crate::tombstones;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RetentionReport {
    pub meetings: u32,
    pub tracks: u32,
}

impl Store {
    /// Deletes the audio of meetings whose `audio_retained_until <= now_ms`.
    pub fn retention_sweep(&self, now_ms_: i64) -> Result<RetentionReport> {
        let due: Vec<(i64, String)> = {
            let conn = self.conn();
            let mut stmt = conn.prepare_cached(
                "SELECT m.id, m.gid FROM meetings m
                 WHERE m.audio_retained_until IS NOT NULL AND m.audio_retained_until <= ?1
                   AND EXISTS (SELECT 1 FROM tracks t WHERE t.meeting_id = m.id)
                   AND NOT EXISTS (SELECT 1 FROM jobs j WHERE j.meeting_id = m.id
                       AND j.kind = 'final_pass' AND j.state IN ('queued', 'running'))
                 ORDER BY m.id",
            )?;
            let rows = stmt.query_map([now_ms_], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut report = RetentionReport::default();
        for (id, gid) in due {
            // Files first: if we crash before the rows go, the next sweep
            // repeats this (removing missing files is fine).
            // A malformed gid (never written by the store) must not stop
            // the sweep for every other meeting.
            let Ok(dir) = self.bundle_dir(&gid) else {
                continue;
            };
            match std::fs::remove_dir_all(dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            let mut conn = self.conn();
            let tx = conn.transaction()?;
            let lamport = Store::alloc_lamport(&tx, 1)?;
            tombstones::write_where(
                &tx,
                "track",
                "SELECT gid FROM tracks WHERE meeting_id = ?1",
                [id],
                lamport,
            )?;
            let n = tx.execute("DELETE FROM tracks WHERE meeting_id = ?1", params![id])?;
            tx.commit()?;
            report.meetings += 1;
            report.tracks += n as u32;
        }
        Ok(report)
    }

    /// Sweep with the current time.
    pub fn retention_sweep_now(&self) -> Result<RetentionReport> {
        self.retention_sweep(now_ms())
    }

    /// Whether the meeting still has audio.
    pub fn audio_available(&self, meeting_gid: &str) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM tracks t JOIN meetings m ON m.id = t.meeting_id WHERE m.gid = ?1)",
            [meeting_gid],
            |r| r.get(0),
        )?)
    }
}
