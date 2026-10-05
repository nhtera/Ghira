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

use ghi_store::sync::records::Bytes;

use crate::session::Rpc;
use crate::store::SyncStore;
use crate::transport::Transport;
use crate::wire::{
    self, BundleHeader, Message, RefuseReason, TrackAck, TrackHave, TrackOffer, TrackPages,
};
use crate::{Result, SyncError};

/// Free space that must remain after a track is stored.
pub const STORAGE_HEADROOM_BYTES: u64 = 256 * 1024 * 1024;
/// The receiver fsyncs this often.
pub const FSYNC_EVERY_PAGES: u64 = 64;
/// Length of a bundle's nonce prefix.
pub const PREFIX_LEN: usize = 19;
/// Pages a track may claim (a sanity bound on a peer's offer).
const MAX_TRACK_PAGES_TOTAL: u64 = 100_000_000;

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

fn unexpected(what: &str) -> SyncError {
    SyncError::Wire(format!("unexpected reply to {what}"))
}

/// Sender side: offers each finished track and streams the missing pages.
/// Returns how many tracks the peer holds completely afterwards.
pub fn send_tracks(
    store: &dyn SyncStore,
    transport: &mut dyn Transport,
    rpc: &mut Rpc,
    peer_device: &str,
) -> Result<usize> {
    let mut done = 0;
    for info in store.tracks_to_send(peer_device)? {
        let offer = TrackOffer {
            track_gid: info.track_gid.clone(),
            meeting_gid: info.meeting_gid.clone(),
            header: BundleHeader {
                magic: Bytes(info.magic.clone()),
                version: info.version,
                prefix: Bytes(info.prefix.clone()),
            },
            pages: info.pages,
            bytes: info.bytes,
            complete: true,
        };
        match rpc.call(transport, &Message::TrackOffer(offer))? {
            Message::TrackHave(h) if h.complete || h.have >= info.pages => {
                store.mark_track_sent(peer_device, &info.track_gid)?;
                done += 1;
            }
            Message::TrackHave(h) => {
                if send_pages(store, transport, rpc, &info, h.have)? {
                    store.mark_track_sent(peer_device, &info.track_gid)?;
                    done += 1;
                }
            }
            // Not now (or never): the next session offers it again.
            Message::Refuse(_) => {}
            _ => return Err(unexpected("TrackOffer")),
        }
    }
    Ok(done)
}

/// Streams pages `have..` of a track. `false` when a page is too large for a
/// message (the track stays unsent).
fn send_pages(
    store: &dyn SyncStore,
    transport: &mut dyn Transport,
    rpc: &mut Rpc,
    info: &TrackInfo,
    mut have: u64,
) -> Result<bool> {
    while have < info.pages {
        let read = store.track_read_pages(&info.track_gid, have, wire::MAX_TRACK_PAGES)?;
        if read.is_empty() {
            return Err(SyncError::Wire("track shorter than its offer".into()));
        }
        let mut bytes = 0usize;
        let mut records = Vec::new();
        for page in read {
            if page.len() > wire::MAX_TRACK_PAGES_BYTES {
                if records.is_empty() {
                    return Ok(false);
                }
                break;
            }
            if bytes + page.len() > wire::MAX_TRACK_PAGES_BYTES {
                break;
            }
            bytes += page.len();
            records.push(Bytes(page));
        }
        let sent = records.len() as u64;
        let reply = rpc.call(
            transport,
            &Message::TrackPages(TrackPages {
                track_gid: info.track_gid.clone(),
                prefix: Bytes(info.prefix.clone()),
                first: have,
                records,
            }),
        )?;
        let Message::TrackAck(TrackAck { have: acked }) = reply else {
            return Err(unexpected("TrackPages"));
        };
        // The receiver may hold fewer than we sent (it re-verifies); it may
        // never claim more, or nothing at all.
        if acked <= have || acked > have + sent {
            return Err(SyncError::Wire("track ack out of range".into()));
        }
        have = acked;
    }
    Ok(true)
}

fn valid_offer(offer: &TrackOffer) -> bool {
    !offer.track_gid.is_empty()
        && offer.track_gid.len() <= 64
        && !offer.meeting_gid.is_empty()
        && offer.meeting_gid.len() <= 64
        && offer.header.prefix.0.len() == PREFIX_LEN
        && offer.header.magic.0.len() <= 16
        && offer.pages > 0
        && offer.pages <= MAX_TRACK_PAGES_TOTAL
}

/// Receiver side: answers a `TrackOffer`.
pub fn on_offer(
    store: &dyn SyncStore,
    from_device: &str,
    offer: &TrackOffer,
) -> Result<OfferResult> {
    if !valid_offer(offer) {
        return Err(SyncError::Wire("invalid track offer".into()));
    }
    store.track_offer(from_device, offer).map_err(Into::into)
}

/// What a `TrackOffer` is answered with.
pub fn offer_reply(result: OfferResult, offer: &TrackOffer) -> Message {
    match result {
        OfferResult::Have(have) => Message::TrackHave(TrackHave {
            have,
            complete: false,
        }),
        OfferResult::Complete => Message::TrackHave(TrackHave {
            have: offer.pages,
            complete: true,
        }),
        OfferResult::Refuse(reason) => Message::Refuse(reason),
    }
}

/// Receiver side: verifies and stores a batch of pages; returns the new
/// `have`. A bad page fails with the store's error (the `.part` was cut back
/// to the last good page).
pub fn on_pages(store: &dyn SyncStore, from_device: &str, pages: &TrackPages) -> Result<u64> {
    if pages.prefix.0.len() != PREFIX_LEN || pages.records.is_empty() {
        return Err(SyncError::Wire("invalid track pages".into()));
    }
    let records: Vec<Vec<u8>> = pages.records.iter().map(|r| r.0.clone()).collect();
    store
        .track_push(
            from_device,
            &pages.track_gid,
            &pages.prefix.0,
            pages.first,
            &records,
        )
        .map_err(Into::into)
}
