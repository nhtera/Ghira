// SPDX-License-Identifier: Apache-2.0
//! "Unpair and wipe" (slice 15-C1, doc 07 §3.5).

use crate::Result;
use crate::store::Store;

/// What a wipe removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WipeReport {
    pub meetings: usize,
}

impl Store {
    /// Shreds, locally and **without tombstones**, every meeting exchanged
    /// with `device_gid`, then removes the pin. Voice profiles do not sync in
    /// v1, so there are none to remove. A meeting already gone counts for
    /// nothing; any other failure stops the wipe and keeps the pin (and the
    /// rest of the scope), so a retry finishes it.
    pub fn wipe_peer(&self, device_gid: &str) -> Result<WipeReport> {
        // One rotation of the wrap secret for the whole scope, not one per
        // meeting.
        let meetings = self.delete_meetings_local(&self.peer_meetings(device_gid)?)?;
        let report = WipeReport { meetings };
        self.unpin_device(device_gid)?;
        Ok(report)
    }
}
