// SPDX-License-Identifier: Apache-2.0
//! Sessions (doc 07 §6; slice 15-F): the phone (spoke) always initiates and
//! drives a request/response exchange, push then pull; the desktop (hub)
//! answers, carries pending `Control` in its replies and never connects out.
//!
//! ```text
//! Connect -> Handshake -> Hello -> Control -> PushTombs -> PushRows -> Leases
//!   -> Audio -> PullTombs -> PullRows -> Idle
//! ```
//!
//! Batches are at most 256 records and 1 MiB; cursors advance only on `Ack`
//! (pushes) or after the batch is applied (pulls). Tombstones and rows share
//! one sequence space (`sync_log.seq`), so the persisted cursor of a phase
//! pair is the lower of the two phases' positions. More than 10 meeting
//! deletes in one session end it with `Error{NeedsConfirm}` (D13) until the
//! user confirms that device.

pub mod hub;
pub mod spoke;

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
pub(crate) mod tests;

use std::time::Duration;

use zeroize::Zeroize;

use crate::control::ControlOutcome;
use crate::store::SyncStore;
use crate::transport::Transport;
use crate::wire::{self, Decoded, ErrorBody, ErrorCode, Message, Proto, SyncTombstoneExt};
use crate::{Result, SyncError};

/// Ping interval, silence limit and idle limit (doc 07 §5.3).
pub const PING_EVERY: Duration = Duration::from_secs(15);
pub const SILENCE_LIMIT: Duration = Duration::from_secs(45);
pub const IDLE_LIMIT: Duration = Duration::from_secs(5 * 60);
/// Meeting tombstones in one session above which the user must confirm.
pub const MASS_DELETE_LIMIT: usize = 10;

/// The limits a session runs under; tests shorten them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    pub ping_every: Duration,
    pub silence: Duration,
    pub idle: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            ping_every: PING_EVERY,
            silence: SILENCE_LIMIT,
            idle: IDLE_LIMIT,
        }
    }
}

/// What a session did, for the UI and the event log (counts only). Both
/// sides count in the spoke's terms: `pushed` is spoke to hub, `pulled` hub
/// to spoke.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SessionReport {
    pub rows_pushed: usize,
    pub rows_pulled: usize,
    pub tombs_pushed: usize,
    pub tombs_pulled: usize,
    pub tracks_sent: usize,
    /// Meeting deletes held for the user's confirmation.
    pub needs_confirm: usize,
    /// Set when a `Wipe` or `Unpair` ended the session.
    pub closed_by: Option<ControlOutcome>,
    /// Hub: leases opened by this session (`job_uuid`s); the caller enqueues
    /// the local jobs for them.
    pub new_leases: Vec<String>,
    /// Spoke: the desktop's state of each leased job (the final-pass chip).
    pub lease_infos: Vec<crate::wire::LeaseInfo>,
    /// Spoke: jobs taken back after a revoke; the phone runs them itself.
    pub take_back: Vec<crate::lease::TakeBack>,
    /// Hub: tracks whose last page arrived (`track_gid`s).
    pub tracks_received: Vec<String>,
}

/// The versions this build offers.
pub fn default_protos() -> Vec<Proto> {
    vec![wire::PROTO]
}

pub(crate) fn send_msg(t: &mut dyn Transport, id: u32, msg: &Message) -> Result<()> {
    let mut bytes = wire::encode(id, msg)?;
    let sent = t.send(&bytes);
    // A meeting record may carry a key: don't leave the buffer behind.
    bytes.zeroize();
    sent
}

pub(crate) fn recv_msg(t: &mut dyn Transport) -> Result<(u32, Decoded)> {
    let bytes = t.recv()?;
    wire::decode_any(&bytes)
}

pub(crate) fn error_msg(code: ErrorCode, detail: Option<String>) -> Message {
    Message::Error(ErrorBody { code, detail })
}

/// The code a local failure is reported to the peer as.
pub(crate) fn code_for(e: &SyncError) -> ErrorCode {
    match e {
        SyncError::Store(ghi_store::StoreError::Invalid(_))
        | SyncError::Store(ghi_store::StoreError::Decrypt)
        | SyncError::Store(ghi_store::StoreError::Tombstoned { .. })
        | SyncError::Wire(_) => ErrorCode::BadRecord,
        // SQLITE_FULL surfaces as the engine's own text; there is no typed
        // variant to match (the store wraps rusqlite).
        SyncError::Store(ghi_store::StoreError::Db(d))
            if d.to_string().contains("disk is full") =>
        {
            ErrorCode::StorageFull
        }
        _ => ErrorCode::Internal,
    }
}

/// The spoke's side of request/reply: numbers requests, matches the reply
/// and turns an `Error` into [`SyncError::Peer`].
#[derive(Debug, Default)]
pub struct Rpc {
    next: u32,
    /// The last `Error` the peer sent (code and detail, never content).
    pub last_error: Option<ErrorBody>,
}

impl Rpc {
    pub fn call(&mut self, t: &mut dyn Transport, msg: &Message) -> Result<Message> {
        self.next = self.next.wrapping_add(1);
        let id = self.next;
        send_msg(t, id, msg)?;
        let (rid, decoded) = recv_msg(t)?;
        match decoded {
            Decoded::Known(Message::Error(e)) => {
                let code = e.code;
                self.last_error = Some(e);
                Err(SyncError::Peer(code))
            }
            Decoded::Known(reply) if rid == id => Ok(reply),
            Decoded::Known(_) => Err(SyncError::Wire("reply to another request".into())),
            Decoded::Unknown(_) => Err(SyncError::Wire("unsupported reply".into())),
        }
    }
}

/// Per-session count of meeting deletes from one peer, and the user's
/// confirmation (D13, amendment 5: stored per device so a resent batch
/// applies without asking again).
#[derive(Debug, Default)]
pub(crate) struct MassDeleteGuard {
    count: usize,
    confirmed: bool,
    used_confirmation: bool,
}

impl MassDeleteGuard {
    /// Whether this batch of tombstones from `peer` may be applied. A `false`
    /// leaves the cumulative count in [`MassDeleteGuard::held`].
    pub(crate) fn allows(
        &mut self,
        store: &dyn SyncStore,
        peer: &str,
        tombs: &[wire::SyncTombstone],
    ) -> Result<bool> {
        let n = tombs
            .iter()
            .filter(|t| t.is_counted_meeting_delete())
            .count();
        if self.count + n <= MASS_DELETE_LIMIT {
            self.count += n;
            return Ok(true);
        }
        if !self.confirmed && store.mass_delete_confirmed(peer)? {
            self.confirmed = true;
            self.used_confirmation = true;
        }
        if self.confirmed {
            self.count += n;
            return Ok(true);
        }
        self.count += n;
        Ok(false)
    }

    pub(crate) fn held(&self) -> usize {
        self.count
    }

    /// After the confirmed batch applied: the confirmation is spent.
    pub(crate) fn settle(&mut self, store: &dyn SyncStore, peer: &str) -> Result<()> {
        if self.used_confirmation {
            self.used_confirmation = false;
            store.set_mass_delete_confirmed(peer, false)?;
        }
        Ok(())
    }
}
