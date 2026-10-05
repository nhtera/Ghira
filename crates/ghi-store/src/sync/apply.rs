// SPDX-License-Identifier: Apache-2.0
//! Applying a peer's batch (slice 15-C2, doc 07 §7.3-§7.5).
//!
//! One transaction per batch: tombstones first and absorbing, then rows;
//! every `_ct` is opened with its AAD before anything commits, and any
//! failure rejects the whole batch with no partial write. Rows whose parent
//! is missing are parked ([`super::pending`]).

use serde::{Deserialize, Serialize};

use super::not_yet;
use super::records::{Record, SyncTombstone};
use crate::Result;
use crate::store::Store;

/// What happened to one gid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplyOutcome {
    Accepted,
    /// Concurrent: merged by the field rules (a conflict copy may exist).
    Merged,
    /// The gid (or its meeting) is tombstoned here; dropped.
    Tombstoned,
    /// The parent is missing; held in `sync_pending`.
    Parked,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ApplyResult {
    /// One entry per applied record, in order.
    pub results: Vec<(String, ApplyOutcome)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TombResult {
    /// Gids that were deleted by this batch (the rest were already gone).
    pub applied: Vec<String>,
    /// Gids refused (malformed or unknown kind), reported in the ack.
    pub rejected: Vec<String>,
    /// Meeting tombstones that need the user's confirmation first (D13).
    pub needs_confirm: usize,
}

impl Store {
    /// Applies tombstones from `from_device` (a device gid).
    pub fn apply_tombs(&self, _from_device: &str, _tombs: &[SyncTombstone]) -> Result<TombResult> {
        not_yet("sync::apply::apply_tombs")
    }

    /// Applies rows from `from_device`. Meeting records may carry a DEK.
    pub fn apply_rows(&self, _from_device: &str, _rows: &[Record]) -> Result<ApplyResult> {
        not_yet("sync::apply::apply_rows")
    }

    /// Spoke side, after an ack: the row is clean again if it is still the
    /// version that was pushed.
    pub fn mark_clean(&self, _gid: &str, _lamport: i64) -> Result<()> {
        not_yet("sync::apply::mark_clean")
    }
}
