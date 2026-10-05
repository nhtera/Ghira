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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::mem::MemDuplex;
    use crate::session::fake::{FakeSyncStore, LocalTrack, make_page};
    use crate::session::tests::{
        Flaky, HUB_KEY, hub_store, meeting, paired_spoke, run_pair, spoke_key, sync_once,
    };

    const PREFIX: [u8; PREFIX_LEN] = [3; PREFIX_LEN];

    fn pages(n: u64, size: usize) -> Vec<Vec<u8>> {
        (0..n)
            .map(|i| make_page(i, &vec![(i % 251) as u8; size]))
            .collect()
    }

    fn track(gid: &str, prefix: [u8; PREFIX_LEN], pages: Vec<Vec<u8>>) -> LocalTrack {
        LocalTrack {
            info: TrackInfo {
                track_gid: gid.into(),
                meeting_gid: "m1".into(),
                magic: b"GHB1".to_vec(),
                version: 1,
                prefix: prefix.to_vec(),
                pages: pages.len() as u64,
                bytes: pages.iter().map(|p| p.len() as u64).sum(),
            },
            pages,
        }
    }

    /// A hub and a phone that already exchanged meeting `m1` and its key.
    fn rig() -> (Arc<FakeSyncStore>, Arc<FakeSyncStore>) {
        let hub = hub_store();
        let phone = paired_spoke(&hub, "phone", 1);
        phone.put_local(meeting("m1"));
        phone.set_dek("m1", [7; 32]);
        sync_once(&hub, &phone, 1).0.unwrap();
        assert!(phone.key_sent("hub", "m1"));
        (hub, phone)
    }

    /// Records the `TrackPages` messages a spoke sends.
    struct Tap {
        inner: MemDuplex,
        sizes: Arc<std::sync::Mutex<Vec<(usize, usize)>>>,
    }

    impl Transport for Tap {
        fn send(&mut self, msg: &[u8]) -> Result<()> {
            if let Ok((_, Message::TrackPages(p))) = wire::decode(msg) {
                let bytes = p.records.iter().map(|r| r.0.len()).sum();
                self.sizes.lock().unwrap().push((p.records.len(), bytes));
            }
            self.inner.send(msg)
        }
        fn recv(&mut self) -> Result<Vec<u8>> {
            self.inner.recv()
        }
        fn peer_static(&self) -> [u8; 32] {
            self.inner.peer_static()
        }
        fn set_recv_timeout(&mut self, t: Option<std::time::Duration>) -> Result<()> {
            self.inner.set_recv_timeout(t)
        }
    }

    #[test]
    fn a_finished_track_arrives_byte_identical_in_bounded_messages() {
        let (hub, phone) = rig();
        let all = pages(200, 20_000);
        phone.add_local_track(track("t1", PREFIX, all.clone()));
        let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (x, y) = MemDuplex::pair(spoke_key(1), HUB_KEY);
        let (a, b) = run_pair(
            &hub,
            &phone,
            Tap {
                inner: x,
                sizes: sizes.clone(),
            },
            y,
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a.tracks_sent, 1);
        assert_eq!(b.tracks_received, ["t1"]);
        let (prefix, got) = hub.complete_track("t1").unwrap();
        assert_eq!((prefix, got), (PREFIX.to_vec(), all));
        assert!(phone.track_acked("hub", "t1"));
        let sizes = sizes.lock().unwrap();
        assert!(sizes.len() >= 4);
        assert!(
            sizes
                .iter()
                .all(|&(n, bytes)| n <= wire::MAX_TRACK_PAGES
                    && bytes <= wire::MAX_TRACK_PAGES_BYTES),
            "{sizes:?}"
        );
        // A later session has nothing to send.
        assert_eq!(sync_once(&hub, &phone, 1).0.unwrap().tracks_sent, 0);
    }

    /// A tiny deterministic generator (no `rand` dependency).
    fn next(state: &mut u64) -> u64 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *state >> 33
    }

    #[test]
    fn random_drops_resume_to_a_byte_identical_track() {
        for seed in 1..=8u64 {
            let (hub, phone) = rig();
            let all = pages(1500, 3_000);
            phone.add_local_track(track("t1", PREFIX, all.clone()));
            let mut rng = seed;
            let mut attempts = 0;
            let mut have_before = 0;
            loop {
                attempts += 1;
                assert!(attempts < 500, "seed {seed}: never completes");
                // Drop somewhere in the middle of the exchange, or let it run.
                let budget = if attempts > 40 {
                    usize::MAX
                } else {
                    8 + (next(&mut rng) % 40) as usize
                };
                let (x, y) = MemDuplex::pair(spoke_key(1), HUB_KEY);
                let r = run_pair(
                    &hub,
                    &phone,
                    Flaky {
                        inner: Some(x),
                        budget,
                    },
                    y,
                );
                if r.0.is_ok() {
                    break;
                }
                // The part only grows between attempts: no restart from 0.
                let have = match hub.part_pages("t1") {
                    Some(n) => n,
                    // Finished, its last ack lost: the next offer says so.
                    None if hub.complete_track("t1").is_some() => usize::MAX,
                    None => 0,
                };
                assert!(have >= have_before, "seed {seed}: {have} < {have_before}");
                have_before = have;
            }
            assert_eq!(hub.complete_track("t1").unwrap().1, all, "seed {seed}");
            assert!(phone.track_acked("hub", "t1"));
            assert!(attempts > 1, "seed {seed} never dropped");
        }
    }

    #[test]
    fn a_changed_prefix_restarts_a_part_from_zero() {
        let (hub, phone) = rig();
        // A stale part from before the sender cut the track.
        hub.seed_part("t1", &[9; PREFIX_LEN], pages(5, 100), 50);
        let all = pages(12, 100);
        phone.add_local_track(track("t1", PREFIX, all.clone()));
        assert_eq!(sync_once(&hub, &phone, 1).0.unwrap().tracks_sent, 1);
        assert_eq!(hub.complete_track("t1").unwrap(), (PREFIX.to_vec(), all));
    }

    #[test]
    fn a_complete_track_is_final_whatever_the_prefix() {
        let (hub, phone) = rig();
        let all = pages(6, 100);
        phone.add_local_track(track("t1", PREFIX, all.clone()));
        sync_once(&hub, &phone, 1).0.unwrap();
        // The sender cuts the track and re-seals it under a new prefix; the
        // receiver already holds the whole track: TrackHave{complete}.
        let cut = pages(4, 100);
        let phone2 = paired_spoke(&hub, "phone2", 2);
        phone2.put_local(meeting("m1"));
        phone2.set_dek("m1", [7; 32]);
        sync_once(&hub, &phone2, 2).0.unwrap();
        phone2.add_local_track(track("t1", [8; PREFIX_LEN], cut));
        let r = sync_once(&hub, &phone2, 2).0.unwrap();
        assert_eq!(r.tracks_sent, 1, "counted as delivered");
        assert_eq!(hub.complete_track("t1").unwrap(), (PREFIX.to_vec(), all));
    }

    #[test]
    fn refusals_leave_the_track_for_later() {
        // Deleted: the meeting is tombstoned on the hub.
        let (hub, phone) = rig();
        phone.add_local_track(track("t1", PREFIX, pages(3, 100)));
        hub.delete_local(
            "m1",
            "meeting",
            Some(ghi_store::sync::records::TombCause::Meeting),
        );
        let r = sync_once(&hub, &phone, 1);
        // (The hub's own tombstone reaches the phone in the same pass.)
        assert_eq!(r.0.unwrap().tracks_sent, 0);
        assert!(hub.complete_track("t1").is_none());
        assert!(!phone.track_acked("hub", "t1"));

        // NoKey: the hub has no key for the meeting.
        let (hub, phone) = rig();
        phone.add_local_track(track("t1", PREFIX, pages(3, 100)));
        hub.set_dek("m1", [0; 32]);
        hub.forget_dek("m1");
        assert_eq!(sync_once(&hub, &phone, 1).0.unwrap().tracks_sent, 0);
        assert!(hub.complete_track("t1").is_none());

        // StorageFull: bytes + 256 MiB do not fit.
        let (hub, phone) = rig();
        let all = pages(3, 100);
        let need: u64 = all.iter().map(|p| p.len() as u64).sum();
        phone.add_local_track(track("t1", PREFIX, all));
        hub.set_free_bytes(need + STORAGE_HEADROOM_BYTES - 1);
        assert_eq!(sync_once(&hub, &phone, 1).0.unwrap().tracks_sent, 0);
        hub.set_free_bytes(need + STORAGE_HEADROOM_BYTES);
        assert_eq!(sync_once(&hub, &phone, 1).0.unwrap().tracks_sent, 1);
    }

    #[test]
    fn a_page_too_large_for_a_message_is_not_sent() {
        let (hub, phone) = rig();
        let mut all = pages(3, 100);
        all[1] = make_page(1, &vec![1u8; wire::MAX_TRACK_PAGES_BYTES + 1]);
        phone.add_local_track(track("t1", PREFIX, all));
        let r = sync_once(&hub, &phone, 1);
        assert_eq!(r.0.unwrap().tracks_sent, 0);
        assert!(hub.complete_track("t1").is_none());
        assert!(!phone.track_acked("hub", "t1"));
    }

    #[test]
    fn a_corrupt_page_ends_the_session_and_the_part_keeps_the_good_ones() {
        let (hub, phone) = rig();
        let mut all = pages(10, 100);
        all[6][12] ^= 0xFF; // flips payload, not the index: the checksum fails
        phone.add_local_track(track("t1", PREFIX, all));
        let (mine, _) = sync_once(&hub, &phone, 1);
        assert!(
            matches!(mine, Err(SyncError::Peer(wire::ErrorCode::BadRecord))),
            "{mine:?}"
        );
        assert_eq!(hub.part_pages("t1"), Some(6));
        assert!(hub.complete_track("t1").is_none());
    }

    #[test]
    fn pages_without_an_offer_are_refused() {
        let (hub, _phone) = rig();
        let (mut x, y) = MemDuplex::pair(spoke_key(1), HUB_KEY);
        let (hs, clk) = (hub.clone(), crate::session::tests::clock());
        let t =
            std::thread::spawn(move || crate::session::hub::HubSession::new(hs, clk, y).serve());
        let mut rpc = Rpc::default();
        let hello = Message::Hello(wire::Hello {
            proto: vec![wire::PROTO],
            app_version: "1".into(),
            device_gid: "phone".into(),
            feed_id: "f".into(),
            pull_cursor: 0,
            caps: vec![],
        });
        assert!(matches!(
            rpc.call(&mut x, &hello).unwrap(),
            Message::HelloOk(_)
        ));
        let err = rpc
            .call(
                &mut x,
                &Message::TrackPages(TrackPages {
                    track_gid: "t1".into(),
                    prefix: Bytes(PREFIX.to_vec()),
                    first: 0,
                    records: vec![Bytes(make_page(0, b"x"))],
                }),
            )
            .unwrap_err();
        assert!(matches!(err, SyncError::Peer(wire::ErrorCode::BadRecord)));
        assert!(t.join().unwrap().is_err());
    }
}
