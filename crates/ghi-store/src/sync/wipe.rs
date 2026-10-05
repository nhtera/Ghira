// SPDX-License-Identifier: Apache-2.0
//! "Unpair and wipe" (slice 15-C1, doc 07 §3.5).

use crate::store::Store;
use crate::{Result, StoreError};

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
        let mut report = WipeReport::default();
        for gid in self.peer_meetings(device_gid)? {
            match self.delete_meeting_local(&gid) {
                Ok(()) => report.meetings += 1,
                Err(StoreError::NotFound { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        self.unpin_device(device_gid)?;
        Ok(report)
    }
}
