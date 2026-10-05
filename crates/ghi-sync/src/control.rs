// SPDX-License-Identifier: Apache-2.0
//! Unpair and wipe (doc 07 §3.5; slice 15-F).
//!
//! `Unpair` deletes the pin and the pair PSK; data stays. `Wipe` shreds every
//! meeting exchanged with the sender, locally and without tombstones, then
//! unpairs, and is answered with `WipeDone`. A device in `wipe_pending` only
//! takes part in a session that delivers `Wipe`.

use ghi_store::StoreError;
use ghi_store::sync::devices::DeviceState;

use crate::store::SyncStore;
use crate::wire::Control;
use crate::{Result, SyncError};

/// What applying a command did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlOutcome {
    /// The pin is gone; close the session.
    Unpaired,
    /// Everything exchanged with the peer is gone and the pin too; answer
    /// `WipeDone`, then close.
    Wiped,
}

/// A pin that is already gone is what an unpair wants.
fn unpin(store: &dyn SyncStore, gid: &str) -> Result<()> {
    match store.unpin_device(gid) {
        Ok(()) | Err(StoreError::NotFound { .. }) => Ok(()),
        Err(e) => Err(SyncError::Store(e)),
    }
}

/// Applies a command received from `from_device`.
pub fn apply_control(
    store: &dyn SyncStore,
    from_device: &str,
    control: &Control,
) -> Result<ControlOutcome> {
    match control {
        Control::Unpair => {
            unpin(store, from_device)?;
            Ok(ControlOutcome::Unpaired)
        }
        Control::Wipe { .. } => {
            // Shred first: if it fails the pin stays and the command can be
            // delivered again.
            store.wipe_peer(from_device)?;
            unpin(store, from_device)?;
            Ok(ControlOutcome::Wiped)
        }
    }
}

/// The command a session must deliver first to `device_gid` (`wipe_pending`),
/// if any.
pub fn pending_for(store: &dyn SyncStore, device_gid: &str) -> Result<Vec<Control>> {
    Ok(match store.device(device_gid)? {
        Some(d) if d.state == DeviceState::WipePending => vec![Control::Wipe {
            reason: "wipe".to_string(),
        }],
        _ => Vec::new(),
    })
}
