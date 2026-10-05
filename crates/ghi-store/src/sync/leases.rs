// SPDX-License-Identifier: Apache-2.0
//! Job leases (slice 15-C1, doc 07 §8). Deadlines are durations measured on a
//! sleep-inclusive clock; `boot_id` names the boot they were measured in, so
//! a reboot suspends a lease until it is renewed.
//!
//! Lease state changes that must be atomic with a result commit (`granted ->
//! done` / `granted -> revoked`) are compare-and-set updates run inside that
//! commit's transaction.

use super::not_yet;
use crate::Result;
use crate::store::Store;

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

impl Store {
    /// Opens a lease (idempotent by `job_uuid`).
    pub fn lease_open(&self, _lease: &Lease) -> Result<Lease> {
        not_yet("sync::leases::lease_open")
    }

    /// Moves the deadline forward (same epoch).
    pub fn lease_renew(
        &self,
        _job_uuid: &str,
        _ttl_ms: i64,
        _deadline_cont_ns: i64,
        _boot_id: &str,
    ) -> Result<()> {
        not_yet("sync::leases::lease_renew")
    }

    pub fn lease_state(&self, _job_uuid: &str) -> Result<Option<Lease>> {
        not_yet("sync::leases::lease_state")
    }

    /// Whether the holder may still run or commit: the lease is open, the boot
    /// is the one the deadline was set in, and `now_cont_ns + margin_ms` is
    /// before the deadline.
    pub fn lease_fence_ok(
        &self,
        _job_uuid: &str,
        _now_cont_ns: i64,
        _boot_id: &str,
        _margin_ms: i64,
    ) -> Result<bool> {
        not_yet("sync::leases::lease_fence_ok")
    }

    /// Whether `meeting_gid` has an open lease (retention and recover skip it).
    pub fn lease_any_open_for(&self, _meeting_gid: &str) -> Result<bool> {
        not_yet("sync::leases::lease_any_open_for")
    }
}
