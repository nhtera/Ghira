// SPDX-License-Identifier: Apache-2.0
//! Background jobs (final pass, summaries, embeddings, retention, ...).
//!
//! State machine:
//!
//! ```text
//! queued ──claim──▶ running ──▶ done
//!    │                 │  └────▶ failed ──retry──▶ queued
//!    └──────cancel─────┴────────▶ cancelled         (failed ──cancel──▶ cancelled)
//! ```
//!
//! `done` and `cancelled` are final. After [`MAX_ATTEMPTS`] claims a job is no
//! longer claimed or retried; a crash-interrupted job at the limit becomes `failed`. A `running` job found at startup was
//! interrupted by a crash and goes back to `queued` (attempts are kept).
//!
//! The payload must never carry user content, only gids, numbers, booleans and
//! short identifiers: [`enqueue_job`](Store::enqueue_job) rejects anything
//! else (a string that is not a UUID or a `[A-Za-z0-9_.:-]` token of at most 64
//! chars, so free text with spaces or non-ASCII letters can't get in).
//! Workers fetch content by gid from the encrypted tables.
//!
//! `payload_version` versions the JSON payload's shape. A worker passes the
//! newest version it understands to [`Store::claim_next_job`]; newer jobs
//! (written by a later build) stay queued instead of being misread.

use rusqlite::{OptionalExtension, params};

use crate::store::Store;
use crate::{Result, StoreError};

/// A job is claimed at most this many times; after that it stays `failed`.
pub const MAX_ATTEMPTS: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Running => "running",
            JobState::Done => "done",
            JobState::Failed => "failed",
            JobState::Cancelled => "cancelled",
        }
    }

    fn parse(s: &str) -> JobState {
        match s {
            "running" => JobState::Running,
            "done" => JobState::Done,
            "failed" => JobState::Failed,
            "cancelled" => JobState::Cancelled,
            _ => JobState::Queued,
        }
    }

    /// Whether `self -> to` is a legal transition.
    pub fn can_move_to(self, to: JobState) -> bool {
        use JobState::*;
        matches!(
            (self, to),
            (Queued, Running)
                | (Queued, Cancelled)
                | (Running, Done)
                | (Running, Failed)
                | (Running, Cancelled)
                | (Running, Queued)
                | (Failed, Queued)
                | (Failed, Cancelled)
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: i64,
    pub meeting_gid: Option<String>,
    pub kind: String,
    pub state: JobState,
    pub progress: f64,
    pub attempts: u32,
    pub payload_version: u32,
    pub payload: serde_json::Value,
}

const JOB_SELECT: &str =
    "SELECT j.id, m.gid, j.kind, j.state, j.progress, j.attempts, j.payload_version, j.payload_json
                          FROM jobs j LEFT JOIN meetings m ON m.id = j.meeting_id";

fn job_from_row(r: &rusqlite::Row) -> rusqlite::Result<Job> {
    let payload: String = r.get(7)?;
    Ok(Job {
        id: r.get(0)?,
        meeting_gid: r.get(1)?,
        kind: r.get(2)?,
        state: JobState::parse(&r.get::<_, String>(3)?),
        progress: r.get(4)?,
        attempts: r.get(5)?,
        payload_version: r.get(6)?,
        payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
    })
}

/// Rejects payloads that could hold user content.
fn check_payload(v: &serde_json::Value) -> Result<()> {
    use serde_json::Value;
    let ok_token = |s: &str| {
        !s.is_empty()
            && s.len() <= 64
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
    };
    match v {
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(()),
        Value::String(s) if ok_token(s) => Ok(()),
        Value::String(_) => Err(StoreError::Invalid(
            "job payloads carry gids and identifiers, not content".into(),
        )),
        Value::Array(a) => a.iter().try_for_each(check_payload),
        Value::Object(o) => o.iter().try_for_each(|(k, v)| {
            if !ok_token(k) {
                return Err(StoreError::Invalid(
                    "job payloads carry gids and identifiers, not content".into(),
                ));
            }
            check_payload(v)
        }),
    }
}

impl Store {
    /// Queues a job, optionally tied to a meeting (deleted with it).
    pub fn enqueue_job(
        &self,
        meeting_gid: Option<&str>,
        kind: &str,
        payload_version: u32,
        payload: &serde_json::Value,
    ) -> Result<i64> {
        check_payload(payload)?;
        let conn = self.conn();
        let meeting_id = match meeting_gid {
            Some(g) => Some(Store::meeting_ref(&conn, g)?.id),
            None => None,
        };
        conn.execute(
            "INSERT INTO jobs (meeting_id, kind, payload_version, payload_json) VALUES (?1, ?2, ?3, ?4)",
            params![meeting_id, kind, payload_version, payload.to_string()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn job(&self, id: i64) -> Result<Job> {
        self.conn()
            .query_row(&format!("{JOB_SELECT} WHERE j.id = ?1"), [id], job_from_row)
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "job",
                gid: id.to_string(),
            })
    }

    pub fn jobs_for_meeting(&self, meeting_gid: &str) -> Result<Vec<Job>> {
        let conn = self.conn();
        let mut stmt =
            conn.prepare_cached(&format!("{JOB_SELECT} WHERE m.gid = ?1 ORDER BY j.id"))?;
        let rows = stmt.query_map([meeting_gid], job_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Takes the oldest queued job of `kind` whose payload version is at most
    /// `max_payload_version`, moving it to `running` and counting the attempt.
    pub fn claim_next_job(&self, kind: &str, max_payload_version: u32) -> Result<Option<Job>> {
        self.claim_next_job_except(kind, max_payload_version, &[])
    }

    /// [`Store::claim_next_job`] passing over the jobs in `except` (held back
    /// by the runner).
    pub fn claim_next_job_except(
        &self,
        kind: &str,
        max_payload_version: u32,
        except: &[i64],
    ) -> Result<Option<Job>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        // Integers only: nothing but digits and commas reaches the SQL.
        let skip = except
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let id: Option<i64> = tx
            .query_row(
                &format!(
                    "SELECT id FROM jobs WHERE state = 'queued' AND kind = ?1 AND payload_version <= ?2 AND attempts < ?3
                     AND id NOT IN ({skip})
                     ORDER BY id LIMIT 1"
                ),
                params![kind, max_payload_version, MAX_ATTEMPTS],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = id else { return Ok(None) };
        tx.execute("UPDATE jobs SET state = 'running', attempts = attempts + 1, progress = 0 WHERE id = ?1", [id])?;
        let job = tx.query_row(&format!("{JOB_SELECT} WHERE j.id = ?1"), [id], job_from_row)?;
        tx.commit()?;
        Ok(Some(job))
    }

    /// Gives back the attempt a running job's claim counted: the run made
    /// progress that is kept (a checkpoint), so being killed later must not
    /// push a long job toward [`MAX_ATTEMPTS`]. Harmless on a job that is not
    /// running.
    pub fn refund_job_attempt(&self, id: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE jobs SET attempts = max(attempts - 1, 0) WHERE id = ?1 AND state = 'running'",
            [id],
        )?;
        Ok(())
    }

    /// Progress (0..=1) of a running job.
    pub fn set_job_progress(&self, id: i64, progress: f64) -> Result<()> {
        let n = self.conn().execute(
            "UPDATE jobs SET progress = ?1 WHERE id = ?2 AND state = 'running'",
            params![progress.clamp(0.0, 1.0), id],
        )?;
        if n == 0 {
            return Err(StoreError::Invalid(format!("job {id} is not running")));
        }
        Ok(())
    }

    /// Saves a running job's progress and resume point (`payload`, numbers
    /// and identifiers only, like at enqueue).
    pub fn checkpoint_job(
        &self,
        id: i64,
        progress: f64,
        payload: &serde_json::Value,
    ) -> Result<()> {
        check_payload(payload)?;
        let n = self.conn().execute(
            "UPDATE jobs SET progress = ?1, payload_json = ?2 WHERE id = ?3 AND state = 'running'",
            params![progress.clamp(0.0, 1.0), payload.to_string(), id],
        )?;
        if n == 0 {
            return Err(StoreError::Invalid(format!("job {id} is not running")));
        }
        Ok(())
    }

    /// Hands a running job back to the queue without counting the claim (it
    /// was preempted, e.g. by a recording starting [RT-10]), saving its resume
    /// point. A crash-interrupted job still counts its attempt.
    pub fn release_job(&self, id: i64, payload: &serde_json::Value) -> Result<()> {
        check_payload(payload)?;
        let n = self.conn().execute(
            "UPDATE jobs SET state = 'queued', attempts = max(attempts - 1, 0), payload_json = ?1
             WHERE id = ?2 AND state = 'running'",
            params![payload.to_string(), id],
        )?;
        if n == 0 {
            return Err(StoreError::Invalid(format!("job {id} is not running")));
        }
        Ok(())
    }

    /// Queued or running jobs of `kind` for a meeting (to avoid duplicates).
    pub fn active_job(&self, meeting_gid: &str, kind: &str) -> Result<Option<Job>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!(
            "{JOB_SELECT} WHERE m.gid = ?1 AND j.kind = ?2 AND j.state IN ('queued', 'running')
             ORDER BY j.id LIMIT 1"
        ))?;
        Ok(stmt
            .query_row(params![meeting_gid, kind], job_from_row)
            .optional()?)
    }

    /// Every queued or running job, oldest first, in one query.
    pub fn active_jobs(&self) -> Result<Vec<Job>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!(
            "{JOB_SELECT} WHERE j.state IN ('queued', 'running') ORDER BY j.id"
        ))?;
        Ok(stmt
            .query_map([], job_from_row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn complete_job(&self, id: i64) -> Result<()> {
        self.move_job(id, JobState::Done, None)
    }

    pub fn fail_job(&self, id: i64) -> Result<()> {
        self.move_job(id, JobState::Failed, None)
    }

    /// `failed -> queued`.
    pub fn retry_job(&self, id: i64) -> Result<()> {
        self.move_job(id, JobState::Queued, Some(JobState::Failed))
    }

    pub fn cancel_job(&self, id: i64) -> Result<()> {
        self.move_job(id, JobState::Cancelled, None)
    }

    fn move_job(&self, id: i64, to: JobState, only_from: Option<JobState>) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let from: String = tx
            .query_row("SELECT state FROM jobs WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "job",
                gid: id.to_string(),
            })?;
        let from = JobState::parse(&from);
        if !from.can_move_to(to) || only_from.is_some_and(|f| f != from) {
            return Err(StoreError::Invalid(format!(
                "job {id}: {} -> {} is not allowed",
                from.as_str(),
                to.as_str()
            )));
        }
        if to == JobState::Queued && from == JobState::Failed {
            let attempts: u32 =
                tx.query_row("SELECT attempts FROM jobs WHERE id = ?1", [id], |r| {
                    r.get(0)
                })?;
            if attempts >= MAX_ATTEMPTS {
                return Err(StoreError::Invalid(format!(
                    "job {id} failed {attempts} times; not retrying"
                )));
            }
        }
        let progress = if to == JobState::Done {
            "progress = 1,"
        } else {
            ""
        };
        tx.execute(
            &format!("UPDATE jobs SET {progress} state = ?1 WHERE id = ?2"),
            params![to.as_str(), id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Startup recovery: jobs left `running` by a crash go back to `queued`.
    pub(crate) fn requeue_interrupted_jobs(&self) -> Result<()> {
        self.conn().execute(
            "UPDATE jobs SET state = CASE WHEN attempts >= ?1 THEN 'failed' ELSE 'queued' END
             WHERE state = 'running'",
            [MAX_ATTEMPTS],
        )?;
        Ok(())
    }
}
