// SPDX-License-Identifier: Apache-2.0
//! The change feed (slice 15-C1): `sync_log` joined to its rows.
//!
//! `sync_log.seq` is the cursor for both rows and tombstones (Lamport values
//! can't be: relayed rows keep old ones). Rows of a meeting that is still
//! `recording` are held back; [`Store::relog_meeting`] re-logs a meeting's rows
//! when it finishes. A meeting's own record comes before its children in a
//! batch. `feed_id` identifies this database's log: a peer that sees a new one
//! starts over from sequence 0.

use super::not_yet;
use super::records::{Record, SyncTombstone};
use crate::Result;
use crate::store::Store;

/// One changed row and its position in the log.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedChange {
    pub seq: i64,
    pub record: Record,
}

/// Up to `max` changes after a sequence number.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChangeBatch {
    pub changes: Vec<FeedChange>,
    /// The highest sequence number this batch covers (acked as `upto_seq`);
    /// it may exceed the last change's when entries were skipped.
    pub upto_seq: i64,
    pub more: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TombBatch {
    pub tombs: Vec<SyncTombstone>,
    pub upto_seq: i64,
    pub more: bool,
}

impl Store {
    /// This database's feed id (`settings['sync.feed_id']`).
    pub fn feed_id(&self) -> Result<String> {
        not_yet("sync::feed::feed_id")
    }

    /// Draws a new feed id (after a restore, an import or a wipe).
    pub fn regen_feed_id(&self) -> Result<String> {
        not_yet("sync::feed::regen_feed_id")
    }

    /// This device's own gid (`settings['sync.device_gid']`).
    pub fn sync_device_gid(&self) -> Result<String> {
        not_yet("sync::feed::sync_device_gid")
    }

    /// Row changes after `seq`, at most `max` records and 1 MiB, parents first,
    /// without meetings that are recording.
    pub fn changes_since(&self, _seq: i64, _max: usize) -> Result<ChangeBatch> {
        not_yet("sync::feed::changes_since")
    }

    /// Tombstones after `seq`, at most `max`.
    pub fn tombs_since(&self, _seq: i64, _max: usize) -> Result<TombBatch> {
        not_yet("sync::feed::tombs_since")
    }

    /// Logs every row of a meeting again (when it stops recording).
    pub fn relog_meeting(&self, _meeting_gid: &str) -> Result<()> {
        not_yet("sync::feed::relog_meeting")
    }
}
