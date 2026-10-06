// SPDX-License-Identifier: Apache-2.0
//! Job leases (slice 15-C1, doc 07 §8). Deadlines are durations measured on a
//! sleep-inclusive clock; `boot_id` names the boot they were measured in, so
//! a reboot suspends a lease until it is renewed.
//!
//! Lease state changes that must be atomic with a result commit (`granted ->
//! done` / `granted -> revoked`) are compare-and-set updates run inside that
//! commit's transaction.

use rusqlite::{OptionalExtension, Row, params};

use crate::store::{Store, check_gid, now_ms};
use crate::{Result, StoreError};

/// Lease states that still hold a meeting (retention and recover skip it).
pub const OPEN_STATES: [&str; 4] = ["offered", "granted", "running", "revoking"];
const ALL_STATES: [&str; 9] = [
    "offered",
    "granted",
    "running",
    "done",
    "revoking",
    "revoked",
    "self_taken",
    "expired",
    "fenced",
];

const LEASE_COLS: &str = "l.job_uuid, l.meeting_gid, l.role, l.epoch, l.kinds, l.state, l.ttl_ms,
                          l.deadline_cont_ns, l.boot_id, l.wall_deadline_ms, l.progress,
                          (SELECT d.gid FROM devices d WHERE d.id = l.peer_id)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseRole {
    /// The phone, which owns the audio.
    Grantor,
    /// The desktop running the pass.
    Holder,
}

impl LeaseRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grantor => "grantor",
            Self::Holder => "holder",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lease {
    pub job_uuid: String,
    pub meeting_gid: String,
    pub role: LeaseRole,
    pub epoch: i64,
    pub kinds: Vec<String>,
    /// `offered | granted | running | done | revoking | revoked | self_taken |
    /// expired | fenced`.
    pub state: String,
    pub ttl_ms: i64,
    pub deadline_cont_ns: Option<i64>,
    pub boot_id: Option<String>,
    pub wall_deadline_ms: Option<i64>,
    pub progress: f64,
    pub peer_gid: Option<String>,
}

fn lease_from_row(r: &Row) -> rusqlite::Result<Lease> {
    let bad = |what: &str| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            what.to_string().into(),
        )
    };
    let role: String = r.get(2)?;
    let kinds: String = r.get(4)?;
    Ok(Lease {
        job_uuid: r.get(0)?,
        meeting_gid: r.get(1)?,
        role: match role.as_str() {
            "grantor" => LeaseRole::Grantor,
            "holder" => LeaseRole::Holder,
            _ => return Err(bad("lease role")),
        },
        epoch: r.get(3)?,
        kinds: serde_json::from_str(&kinds).map_err(|_| bad("lease kinds"))?,
        state: r.get(5)?,
        ttl_ms: r.get(6)?,
        deadline_cont_ns: r.get(7)?,
        boot_id: r.get(8)?,
        wall_deadline_ms: r.get(9)?,
        progress: r.get(10)?,
        peer_gid: r.get(11)?,
    })
}

fn get(conn: &rusqlite::Connection, job_uuid: &str) -> Result<Option<Lease>> {
    Ok(conn
        .query_row(
            &format!("SELECT {LEASE_COLS} FROM leases l WHERE l.job_uuid = ?1"),
            [job_uuid],
            lease_from_row,
        )
        .optional()?)
}

impl Store {
    /// Opens a lease (idempotent by `job_uuid`: a second open returns the
    /// stored row unchanged, so a duplicate `ProcessRequest` is a no-op).
    pub fn lease_open(&self, lease: &Lease) -> Result<Lease> {
        check_gid(&lease.meeting_gid)?;
        if lease.job_uuid.is_empty() || lease.job_uuid.len() > 64 {
            return Err(StoreError::Invalid("lease job uuid".into()));
        }
        if !ALL_STATES.contains(&lease.state.as_str()) {
            return Err(StoreError::Invalid(format!("lease state {}", lease.state)));
        }
        if lease.ttl_ms <= 0 || lease.epoch < 0 || lease.kinds.len() > 8 {
            return Err(StoreError::Invalid("lease ttl, epoch or kinds".into()));
        }
        let kinds =
            serde_json::to_string(&lease.kinds).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let peer: Option<i64> = match &lease.peer_gid {
            Some(g) => Some(
                tx.query_row("SELECT id FROM devices WHERE gid = ?1", [g], |r| r.get(0))
                    .optional()?
                    .ok_or_else(|| StoreError::NotFound {
                        kind: "device",
                        gid: g.clone(),
                    })?,
            ),
            None => None,
        };
        tx.execute(
            "INSERT OR IGNORE INTO leases
                 (job_uuid, meeting_gid, role, epoch, kinds, state, ttl_ms, deadline_cont_ns,
                  boot_id, wall_deadline_ms, progress, peer_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                lease.job_uuid,
                lease.meeting_gid,
                lease.role.as_str(),
                lease.epoch,
                kinds,
                lease.state,
                lease.ttl_ms,
                lease.deadline_cont_ns,
                lease.boot_id,
                lease.wall_deadline_ms,
                lease.progress,
                peer
            ],
        )?;
        let stored = get(&tx, &lease.job_uuid)?.ok_or_else(|| StoreError::NotFound {
            kind: "lease",
            gid: lease.job_uuid.clone(),
        })?;
        tx.commit()?;
        Ok(stored)
    }

    /// Moves the deadline forward (same epoch). `wall_deadline_ms` becomes now
    /// plus the ttl. A lease that is no longer open is [`StoreError::Fenced`].
    pub fn lease_renew(
        &self,
        job_uuid: &str,
        ttl_ms: i64,
        deadline_cont_ns: i64,
        boot_id: &str,
    ) -> Result<()> {
        if ttl_ms <= 0 {
            return Err(StoreError::Invalid("lease ttl".into()));
        }
        let conn = self.conn();
        let n = conn.execute(
            "UPDATE leases SET ttl_ms = ?2, deadline_cont_ns = ?3, boot_id = ?4,
                    wall_deadline_ms = ?5
             WHERE job_uuid = ?1 AND state IN ('offered', 'granted', 'running', 'revoking')",
            params![
                job_uuid,
                ttl_ms,
                deadline_cont_ns,
                boot_id,
                now_ms() + ttl_ms
            ],
        )?;
        if n == 1 {
            return Ok(());
        }
        match get(&conn, job_uuid)? {
            Some(_) => Err(StoreError::Fenced),
            None => Err(StoreError::NotFound {
                kind: "lease",
                gid: job_uuid.to_string(),
            }),
        }
    }

    pub fn lease_state(&self, job_uuid: &str) -> Result<Option<Lease>> {
        get(&self.conn(), job_uuid)
    }

    /// Compare-and-set of the lease state: `from` must hold now. True when
    /// this call made the change. A commit's `granted -> done` runs inside the
    /// result transaction (`finish_lease`); this is the same CAS for the
    /// other transitions, so exactly one of them wins.
    pub fn lease_transition(&self, job_uuid: &str, from: &[&str], to: &str) -> Result<bool> {
        if !ALL_STATES.contains(&to) || from.iter().any(|s| !ALL_STATES.contains(s)) {
            return Err(StoreError::Invalid("lease state".into()));
        }
        let conn = self.conn();
        let current: Option<String> = conn
            .query_row(
                "SELECT state FROM leases WHERE job_uuid = ?1",
                [job_uuid],
                |r| r.get(0),
            )
            .optional()?;
        let Some(current) = current else {
            return Err(StoreError::NotFound {
                kind: "lease",
                gid: job_uuid.to_string(),
            });
        };
        if !from.contains(&current.as_str()) {
            return Ok(false);
        }
        // The state may have changed since the read only through another
        // connection; the WHERE keeps the CAS exact either way.
        let n = conn.execute(
            "UPDATE leases SET state = ?2 WHERE job_uuid = ?1 AND state = ?3",
            params![job_uuid, to, current],
        )?;
        Ok(n == 1)
    }

    /// The revoke side of the revoke/commit race: `granted -> revoked`. True
    /// means the revoke won (answer `Revoked`); false means the commit got
    /// there first (answer `AlreadyDone`).
    pub fn lease_revoke(&self, job_uuid: &str) -> Result<bool> {
        self.lease_transition(job_uuid, &["granted"], "revoked")
    }

    /// Records the holder's progress (0..=1) for `LeaseStatus`.
    pub fn lease_set_progress(&self, job_uuid: &str, progress: f64) -> Result<()> {
        if !(0.0..=1.0).contains(&progress) {
            return Err(StoreError::Invalid("lease progress".into()));
        }
        self.conn().execute(
            "UPDATE leases SET progress = ?2 WHERE job_uuid = ?1",
            params![job_uuid, progress],
        )?;
        Ok(())
    }

    /// Whether the holder may still run or commit: the lease is `granted`, the
    /// boot is the one the deadline was set in, and `now_cont_ns + margin_ms`
    /// is before the deadline.
    pub fn lease_fence_ok(
        &self,
        job_uuid: &str,
        now_cont_ns: i64,
        boot_id: &str,
        margin_ms: i64,
    ) -> Result<bool> {
        let Some(l) = get(&self.conn(), job_uuid)? else {
            return Ok(false);
        };
        let (Some(deadline), Some(boot)) = (l.deadline_cont_ns, l.boot_id.as_deref()) else {
            return Ok(false);
        };
        let margin_ns = margin_ms.saturating_mul(1_000_000);
        Ok(l.state == "granted"
            && boot == boot_id
            && now_cont_ns.saturating_add(margin_ns) < deadline)
    }

    /// Whether `meeting_gid` has an open lease (retention and recover skip it).
    pub fn lease_any_open_for(&self, meeting_gid: &str) -> Result<bool> {
        Ok(self.conn().query_row(
            "SELECT EXISTS (SELECT 1 FROM leases WHERE meeting_gid = ?1
                            AND state IN ('offered', 'granted', 'running', 'revoking'))",
            [meeting_gid],
            |r| r.get(0),
        )?)
    }

    /// Every lease of a meeting, newest epoch first (the source of
    /// `LeaseStatus` replies).
    pub fn leases_for_meeting(&self, meeting_gid: &str) -> Result<Vec<Lease>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {LEASE_COLS} FROM leases l WHERE l.meeting_gid = ?1
             ORDER BY l.epoch DESC, l.id DESC"
        ))?;
        let rows = stmt.query_map([meeting_gid], lease_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every lease that is still open.
    pub fn leases_open(&self) -> Result<Vec<Lease>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT {LEASE_COLS} FROM leases l
             WHERE l.state IN ('offered', 'granted', 'running', 'revoking') ORDER BY l.id"
        ))?;
        let rows = stmt.query_map([], lease_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Meetings whose newest grantor lease expired and was never replaced: the
    /// phone could not run the pass itself and the desktop did not finish, so
    /// they must be offered again at the next epoch.
    pub fn meetings_with_expired_lease(&self) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT l.meeting_gid FROM leases l
             WHERE l.role = 'grantor' AND l.state = 'expired'
               AND NOT EXISTS (SELECT 1 FROM leases n
                               WHERE n.meeting_gid = l.meeting_gid AND n.role = 'grantor'
                                 AND n.id <> l.id AND n.epoch >= l.epoch)
             ORDER BY l.id",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
