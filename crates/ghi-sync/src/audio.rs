// SPDX-License-Identifier: Apache-2.0
//! Audio transfer, phone to desktop (doc 07 §7.7; slice 15-F), over the
//! store's `bundle::RawImport` (slice 15-D).
//!
//! Only finished tracks of meetings whose `audio_origin` is this phone go out,
//! after the meeting's rows and key were acked. Pages travel verbatim (the
//! sender's STREAM ciphertext) and are each verified on receipt, so the
//! receiver never seals under the sender's nonce prefix. At most 64 pages and
//! 1 MiB per message; larger pages are refused. Refusals: `Deleted`,
//! `StorageFull` (needs bytes + 256 MiB free) and `NoKey`. A changed prefix
//! restarts a `.part` from 0; a complete track is `TrackHave{complete}`
//! whatever the prefix.

use crate::store::SyncStore;
use crate::transport::Transport;
use crate::wire::{RefuseReason, TrackOffer};
use crate::{Result, not_yet};

/// Free space that must remain after a track is stored.
pub const STORAGE_HEADROOM_BYTES: u64 = 256 * 1024 * 1024;
/// The receiver fsyncs this often.
pub const FSYNC_EVERY_PAGES: u64 = 64;

/// A finished track waiting to go to a peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackInfo {
    pub track_gid: String,
    pub meeting_gid: String,
    /// Bundle magic and version.
    pub magic: Vec<u8>,
    pub version: u8,
    /// The 19-byte nonce prefix of the bundle.
    pub prefix: Vec<u8>,
    pub pages: u64,
    pub bytes: u64,
}

/// The receiver's answer to an offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferResult {
    /// Resume from this many contiguous verified pages.
    Have(u64),
    /// The whole track is already here.
    Complete,
    Refuse(RefuseReason),
}

/// Sender side: offers each of `tracks` and streams the missing pages.
pub fn send_tracks(
    _store: &dyn SyncStore,
    _transport: &mut dyn Transport,
    _peer_device: &str,
) -> Result<usize> {
    not_yet("audio::send_tracks")
}

/// Receiver side: answers a `TrackOffer`.
pub fn on_offer(
    _store: &dyn SyncStore,
    _from_device: &str,
    _offer: &TrackOffer,
) -> Result<OfferResult> {
    not_yet("audio::on_offer")
}
