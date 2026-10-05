// SPDX-License-Identifier: Apache-2.0
//! Unpair and wipe (doc 07 §3.5; slice 15-F).
//!
//! `Unpair` deletes the pin and the pair PSK; data stays. `Wipe` shreds every
//! meeting exchanged with the sender, locally and without tombstones, then
//! unpairs, and is answered with `WipeDone`. A device in `wipe_pending` only
//! takes part in a session that delivers `Wipe`.

use crate::store::SyncStore;
use crate::wire::Control;
use crate::{Result, not_yet};

/// What applying a command did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlOutcome {
    /// The pin is gone; close the session.
    Unpaired,
    /// Everything exchanged with the peer is gone and the pin too; answer
    /// `WipeDone`, then close.
    Wiped,
}

/// Applies a command received from `from_device`.
pub fn apply_control(
    _store: &dyn SyncStore,
    _from_device: &str,
    _control: &Control,
) -> Result<ControlOutcome> {
    not_yet("control::apply_control")
}

/// The command a session must deliver first to `device_gid` (`wipe_pending`),
/// if any.
pub fn pending_for(_store: &dyn SyncStore, _device_gid: &str) -> Result<Vec<Control>> {
    not_yet("control::pending_for")
}
