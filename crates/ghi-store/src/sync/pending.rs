// SPDX-License-Identifier: Apache-2.0
//! Orphans (slice 15-C2, doc 07 §9): records that arrived before their parent
//! wait in `sync_pending` (at most 10 000 rows or 7 days), are retried after
//! each batch, and are dropped if the parent is tombstoned.

use super::not_yet;
use crate::Result;
use crate::store::Store;

/// Most parked records.
pub const MAX_PENDING: usize = 10_000;
/// How long a record may wait.
pub const MAX_PENDING_AGE_MS: i64 = 7 * 24 * 3600 * 1000;

impl Store {
    /// Retries parked records whose parent has arrived; returns how many
    /// were applied.
    pub fn retry_pending(&self) -> Result<usize> {
        not_yet("sync::pending::retry_pending")
    }

    /// Parked records.
    pub fn pending_count(&self) -> Result<usize> {
        not_yet("sync::pending::pending_count")
    }
}
