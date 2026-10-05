// SPDX-License-Identifier: Apache-2.0
//! Meeting key transfer (slice 15-C1, doc 07 §7.6).
//!
//! A DEK goes to a peer once, inside the meeting record in the Noise channel
//! (`peer_meetings.key_sent`). The receiver refuses it for a tombstoned gid,
//! requires it to equal any DEK it already holds (T10), and stores it only
//! wrapped under its own KeyRing. Voice profile keys do not sync (v1).

use zeroize::Zeroizing;

use super::not_yet;
use crate::Result;
use crate::store::Store;

impl Store {
    /// The DEK to put in `meeting_gid`'s record for `device_gid`, or `None` if
    /// it was already sent. Records the meeting in `peer_meetings`.
    pub fn meeting_dek_for_peer(
        &self,
        _device_gid: &str,
        _meeting_gid: &str,
    ) -> Result<Option<Zeroizing<[u8; 32]>>> {
        not_yet("sync::keys::meeting_dek_for_peer")
    }

    /// Marks the DEK of a meeting as delivered (after the peer's ack).
    pub fn mark_key_sent(&self, _device_gid: &str, _meeting_gid: &str) -> Result<()> {
        not_yet("sync::keys::mark_key_sent")
    }

    /// Takes the DEK a peer sent for `meeting_gid`.
    pub fn accept_dek(&self, _meeting_gid: &str, _dek: &[u8; 32]) -> Result<()> {
        not_yet("sync::keys::accept_dek")
    }

    /// Meetings exchanged with a peer, in either direction (a Wipe's scope).
    pub fn peer_meetings(&self, _device_gid: &str) -> Result<Vec<String>> {
        not_yet("sync::keys::peer_meetings")
    }
}
