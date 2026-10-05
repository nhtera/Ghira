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
//! Batches are at most 256 records and 1 MiB; cursors advance only on `Ack`.
//! More than 10 meeting deletes in one batch end the session with
//! `Error{NeedsConfirm}` (D13).

pub mod hub;
pub mod spoke;

use std::time::Duration;

/// Ping interval, silence limit and idle limit (doc 07 §5.3).
pub const PING_EVERY: Duration = Duration::from_secs(15);
pub const SILENCE_LIMIT: Duration = Duration::from_secs(45);
pub const IDLE_LIMIT: Duration = Duration::from_secs(5 * 60);
/// Meeting tombstones in one session above which the user must confirm.
pub const MASS_DELETE_LIMIT: usize = 10;

/// What a session did, for the UI and the event log (counts only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SessionReport {
    pub rows_pushed: usize,
    pub rows_pulled: usize,
    pub tombs_pushed: usize,
    pub tombs_pulled: usize,
    pub tracks_sent: usize,
    /// Meeting deletes held for the user's confirmation.
    pub needs_confirm: usize,
}
