// SPDX-License-Identifier: Apache-2.0
//! "Unpair and wipe" (slice 15-C1, doc 07 §3.5).

use super::not_yet;
use crate::Result;
use crate::store::Store;

/// What a wipe removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WipeReport {
    pub meetings: usize,
}

impl Store {
    /// Shreds, locally and **without tombstones**, every meeting exchanged
    /// with `device_gid` (and its synced voice profiles), then removes the pin.
    pub fn wipe_peer(&self, _device_gid: &str) -> Result<WipeReport> {
        not_yet("sync::wipe::wipe_peer")
    }
}
