// SPDX-License-Identifier: Apache-2.0
//! Job leases (doc 07 §8; slice 15-F): the phone (grantor) hands a meeting's
//! final pass and notes to the desktop (holder), with fencing so exactly one
//! result is kept.
//!
//! Grantor states: `Offered -> Granted -> Done | Revoking -> SelfTaken |
//! Expired -> SelfTaken(epoch + 1)` (if capable and N hours have passed).
//! Holder: `H = recv + ttl`, renewed on every `LeaseStatus`; it checks `now <
//! H` before claiming, at each checkpoint and before the commit (`H - 1 min`).
//! Deadlines are durations on the injected [`crate::clock::Clock`]; after a
//! holder reboot the lease is suspended until renewed; the grantor uses wall
//! time plus an hour of grace after its own reboot. `Revoked` and
//! `AlreadyDone` are decided by a compare-and-set on the lease row inside the
//! commit's transaction, so exactly one is ever sent.

use crate::clock::Clock;
use crate::store::SyncStore;
use crate::wire::{LeaseInfo, ProcessRequest};
use crate::{Result, not_yet};

/// Default `ttl_ms`: 12 hours.
pub const DEFAULT_TTL_MS: i64 = 12 * 3600 * 1000;
/// The grantor's extra wait past `ttl` before it may self-take.
pub const GRACE_MS: i64 = 15 * 60 * 1000;
/// The holder stops this long before `H` (it must not commit inside it).
pub const COMMIT_MARGIN_MS: i64 = 60 * 1000;
/// Extra grace the grantor uses after its own reboot (it has only wall time).
pub const REBOOT_GRACE_MS: i64 = 3600 * 1000;

/// The grantor's view of a lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantorState {
    Offered,
    Granted,
    Done,
    Revoking,
    SelfTaken,
    Expired,
}

/// What the grantor decides when polled.
#[derive(Debug, Clone, PartialEq)]
pub enum GrantorAction {
    Nothing,
    /// Send this request (an offer, or a re-issue after `AlreadyDone` isn't
    /// needed: only `Offered` leases produce one).
    Send(ProcessRequest),
    /// Take the job back with this epoch (`epoch + 1`).
    SelfTake {
        epoch: i64,
    },
}

/// The phone's side.
#[derive(Debug, Default)]
pub struct Grantor;

impl Grantor {
    /// Records an offer for a meeting whose rows, key and audio are acked.
    pub fn offer(
        &self,
        _store: &dyn SyncStore,
        _clock: &dyn Clock,
        _meeting_gid: &str,
        _kinds: &[String],
        _ttl_ms: i64,
    ) -> Result<ProcessRequest> {
        not_yet("lease::Grantor::offer")
    }

    /// Applies a `LeaseStatus` reply: renews, completes or expires leases.
    pub fn on_status(
        &self,
        _store: &dyn SyncStore,
        _clock: &dyn Clock,
        _replies: &[LeaseInfo],
    ) -> Result<()> {
        not_yet("lease::Grantor::on_status")
    }

    /// "Process on this phone now": starts a revoke.
    pub fn revoke(&self, _store: &dyn SyncStore, _meeting_gid: &str) -> Result<()> {
        not_yet("lease::Grantor::revoke")
    }

    /// Whether anything is due: an expired lease the phone may take back after
    /// `offline_after_ms` of the desktop being away.
    pub fn poll(
        &self,
        _store: &dyn SyncStore,
        _clock: &dyn Clock,
        _offline_after_ms: i64,
    ) -> Result<Vec<GrantorAction>> {
        not_yet("lease::Grantor::poll")
    }
}

/// The desktop's side.
#[derive(Debug, Default)]
pub struct Holder;

impl Holder {
    /// Handles a `ProcessRequest` (idempotent by `job_uuid`): opens the lease
    /// and enqueues the local job.
    pub fn on_request(
        &self,
        _store: &dyn SyncStore,
        _clock: &dyn Clock,
        _from_device: &str,
        _req: &ProcessRequest,
    ) -> Result<()> {
        not_yet("lease::Holder::on_request")
    }

    /// The fence the job runner calls: may the job still run, with `margin`
    /// before the deadline?
    pub fn fence_ok(
        &self,
        _store: &dyn SyncStore,
        _clock: &dyn Clock,
        _job_uuid: &str,
        _margin_ms: i64,
    ) -> Result<bool> {
        not_yet("lease::Holder::fence_ok")
    }
}
