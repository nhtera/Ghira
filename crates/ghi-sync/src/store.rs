// SPDX-License-Identifier: Apache-2.0
//! What a session needs from the store: [`SyncStore`].
//!
//! Sessions, leases, audio and pairing are written against this trait, so slice
//! 15-F tests them against an in-memory fake and then against the real store.
//! The implementation for [`ghi_store::store::Store`] below only forwards to
//! the `ghi_store::sync` methods (slices 15-C1, 15-C2, 15-D); it lives here,
//! not in `ghi-store`, because `ghi-store` can't depend on this crate.
//! Peers are named by device gid.

use ghi_store::Result;
use ghi_store::StoreError;
use ghi_store::store::Store;
use ghi_store::sync::apply::{ApplyResult, TombResult};
use ghi_store::sync::devices::{Device, NewDevice};
use ghi_store::sync::feed::{ChangeBatch, TombBatch};
use ghi_store::sync::leases::Lease;
use ghi_store::sync::records::{Record, SettingRec, SyncTombstone};
use ghi_store::sync::wipe::WipeReport;
use zeroize::Zeroizing;

use crate::audio::{OfferResult, TrackInfo};
use crate::wire::TrackOffer;

/// The store operations of a sync session.
pub trait SyncStore: Send + Sync {
    // --- feed
    fn feed_id(&self) -> Result<String>;
    fn regen_feed_id(&self) -> Result<String>;
    /// This device's own gid.
    fn device_gid(&self) -> Result<String>;
    fn changes_since(&self, seq: i64, max: usize) -> Result<ChangeBatch>;
    fn tombs_since(&self, seq: i64, max: usize) -> Result<TombBatch>;
    fn relog_meeting(&self, meeting_gid: &str) -> Result<()>;

    // --- apply (the merge engine)
    fn apply_tombs(&self, from_device: &str, tombs: &[SyncTombstone]) -> Result<TombResult>;
    fn apply_rows(&self, from_device: &str, rows: &[Record]) -> Result<ApplyResult>;
    /// Spoke: the row is clean again after an ack.
    fn mark_clean(&self, gid: &str, lamport: i64) -> Result<()>;
    fn retry_pending(&self) -> Result<usize>;
    fn observe_lamport(&self, remote: i64) -> Result<()>;

    // --- devices and pins
    fn devices(&self) -> Result<Vec<Device>>;
    fn device(&self, gid: &str) -> Result<Option<Device>>;
    fn device_by_key(&self, static_pub: &[u8; 32]) -> Result<Option<Device>>;
    fn pin_device(&self, device: &NewDevice, pair_psk: &[u8; 32]) -> Result<Device>;
    fn unpin_device(&self, gid: &str) -> Result<()>;
    fn set_wipe_pending(&self, gid: &str) -> Result<()>;
    fn touch_device(&self, gid: &str, addr: Option<&str>) -> Result<()>;
    fn pair_psk(&self, gid: &str) -> Result<Zeroizing<[u8; 32]>>;
    fn set_cursors(
        &self,
        gid: &str,
        push_seq: i64,
        pull_feed_id: Option<&str>,
        pull_seq: i64,
    ) -> Result<()>;

    // --- keys
    fn meeting_dek_for_peer(
        &self,
        device_gid: &str,
        meeting_gid: &str,
    ) -> Result<Option<Zeroizing<[u8; 32]>>>;
    fn mark_key_sent(&self, device_gid: &str, meeting_gid: &str) -> Result<()>;
    fn accept_dek(&self, meeting_gid: &str, dek: &[u8; 32]) -> Result<()>;
    fn peer_meetings(&self, device_gid: &str) -> Result<Vec<String>>;

    // --- leases
    fn lease_open(&self, lease: &Lease) -> Result<Lease>;
    fn lease_renew(
        &self,
        job_uuid: &str,
        ttl_ms: i64,
        deadline_cont_ns: i64,
        boot_id: &str,
    ) -> Result<()>;
    fn lease_state(&self, job_uuid: &str) -> Result<Option<Lease>>;
    fn lease_fence_ok(
        &self,
        job_uuid: &str,
        now_cont_ns: i64,
        boot_id: &str,
        margin_ms: i64,
    ) -> Result<bool>;
    fn lease_any_open_for(&self, meeting_gid: &str) -> Result<bool>;

    // --- wipe and settings
    fn wipe_peer(&self, device_gid: &str) -> Result<WipeReport>;
    fn put_synced(&self, key: &str, value_json: &str) -> Result<()>;
    fn apply_synced(&self, rec: &SettingRec) -> Result<()>;

    // --- raw audio (bundle::RawImport, slice 15-D)
    /// Finished tracks to offer `device_gid`: audio recorded here, rows and
    /// key acked.
    fn tracks_to_send(&self, device_gid: &str) -> Result<Vec<TrackInfo>>;
    /// Up to `max` verbatim page records of a local track from page `first`.
    fn track_read_pages(&self, track_gid: &str, first: u64, max: usize) -> Result<Vec<Vec<u8>>>;
    /// Receiver: decides an offer (refusals, resume point, complete).
    fn track_offer(&self, from_device: &str, offer: &TrackOffer) -> Result<OfferResult>;
    /// Receiver: verifies and appends page records; returns the new `have`.
    /// A bad page truncates the `.part` to the last good page and fails.
    fn track_push(
        &self,
        from_device: &str,
        track_gid: &str,
        prefix: &[u8],
        first: u64,
        records: &[Vec<u8>],
    ) -> Result<u64>;
}

fn audio_not_yet<T>(what: &'static str) -> Result<T> {
    Err(StoreError::NotYet(what))
}

impl SyncStore for Store {
    fn feed_id(&self) -> Result<String> {
        Store::feed_id(self)
    }
    fn regen_feed_id(&self) -> Result<String> {
        Store::regen_feed_id(self)
    }
    fn device_gid(&self) -> Result<String> {
        Store::sync_device_gid(self)
    }
    fn changes_since(&self, seq: i64, max: usize) -> Result<ChangeBatch> {
        Store::changes_since(self, seq, max)
    }
    fn tombs_since(&self, seq: i64, max: usize) -> Result<TombBatch> {
        Store::tombs_since(self, seq, max)
    }
    fn relog_meeting(&self, meeting_gid: &str) -> Result<()> {
        Store::relog_meeting(self, meeting_gid)
    }

    fn apply_tombs(&self, from_device: &str, tombs: &[SyncTombstone]) -> Result<TombResult> {
        Store::apply_tombs(self, from_device, tombs)
    }
    fn apply_rows(&self, from_device: &str, rows: &[Record]) -> Result<ApplyResult> {
        Store::apply_rows(self, from_device, rows)
    }
    fn mark_clean(&self, gid: &str, lamport: i64) -> Result<()> {
        Store::mark_clean(self, gid, lamport)
    }
    fn retry_pending(&self) -> Result<usize> {
        Store::retry_pending(self)
    }
    fn observe_lamport(&self, remote: i64) -> Result<()> {
        Store::observe_lamport(self, remote)
    }

    fn devices(&self) -> Result<Vec<Device>> {
        Store::devices(self)
    }
    fn device(&self, gid: &str) -> Result<Option<Device>> {
        Store::device(self, gid)
    }
    fn device_by_key(&self, static_pub: &[u8; 32]) -> Result<Option<Device>> {
        Store::device_by_key(self, static_pub)
    }
    fn pin_device(&self, device: &NewDevice, pair_psk: &[u8; 32]) -> Result<Device> {
        Store::pin_device(self, device, pair_psk)
    }
    fn unpin_device(&self, gid: &str) -> Result<()> {
        Store::unpin_device(self, gid)
    }
    fn set_wipe_pending(&self, gid: &str) -> Result<()> {
        Store::set_wipe_pending(self, gid)
    }
    fn touch_device(&self, gid: &str, addr: Option<&str>) -> Result<()> {
        Store::touch_device(self, gid, addr)
    }
    fn pair_psk(&self, gid: &str) -> Result<Zeroizing<[u8; 32]>> {
        Store::pair_psk(self, gid)
    }
    fn set_cursors(
        &self,
        gid: &str,
        push_seq: i64,
        pull_feed_id: Option<&str>,
        pull_seq: i64,
    ) -> Result<()> {
        Store::set_cursors(self, gid, push_seq, pull_feed_id, pull_seq)
    }

    fn meeting_dek_for_peer(
        &self,
        device_gid: &str,
        meeting_gid: &str,
    ) -> Result<Option<Zeroizing<[u8; 32]>>> {
        Store::meeting_dek_for_peer(self, device_gid, meeting_gid)
    }
    fn mark_key_sent(&self, device_gid: &str, meeting_gid: &str) -> Result<()> {
        Store::mark_key_sent(self, device_gid, meeting_gid)
    }
    fn accept_dek(&self, meeting_gid: &str, dek: &[u8; 32]) -> Result<()> {
        Store::accept_dek(self, meeting_gid, dek)
    }
    fn peer_meetings(&self, device_gid: &str) -> Result<Vec<String>> {
        Store::peer_meetings(self, device_gid)
    }

    fn lease_open(&self, lease: &Lease) -> Result<Lease> {
        Store::lease_open(self, lease)
    }
    fn lease_renew(
        &self,
        job_uuid: &str,
        ttl_ms: i64,
        deadline_cont_ns: i64,
        boot_id: &str,
    ) -> Result<()> {
        Store::lease_renew(self, job_uuid, ttl_ms, deadline_cont_ns, boot_id)
    }
    fn lease_state(&self, job_uuid: &str) -> Result<Option<Lease>> {
        Store::lease_state(self, job_uuid)
    }
    fn lease_fence_ok(
        &self,
        job_uuid: &str,
        now_cont_ns: i64,
        boot_id: &str,
        margin_ms: i64,
    ) -> Result<bool> {
        Store::lease_fence_ok(self, job_uuid, now_cont_ns, boot_id, margin_ms)
    }
    fn lease_any_open_for(&self, meeting_gid: &str) -> Result<bool> {
        Store::lease_any_open_for(self, meeting_gid)
    }

    fn wipe_peer(&self, device_gid: &str) -> Result<WipeReport> {
        Store::wipe_peer(self, device_gid)
    }
    fn put_synced(&self, key: &str, value_json: &str) -> Result<()> {
        Store::put_synced(self, key, value_json)
    }
    fn apply_synced(&self, rec: &SettingRec) -> Result<()> {
        Store::apply_synced(self, rec)
    }

    // The raw audio hooks wait for `bundle::RawImport` (slice 15-D).
    fn tracks_to_send(&self, _device_gid: &str) -> Result<Vec<TrackInfo>> {
        audio_not_yet("sync store: tracks_to_send")
    }
    fn track_read_pages(&self, _track_gid: &str, _first: u64, _max: usize) -> Result<Vec<Vec<u8>>> {
        audio_not_yet("sync store: track_read_pages")
    }
    fn track_offer(&self, _from_device: &str, _offer: &TrackOffer) -> Result<OfferResult> {
        audio_not_yet("sync store: track_offer")
    }
    fn track_push(
        &self,
        _from_device: &str,
        _track_gid: &str,
        _prefix: &[u8],
        _first: u64,
        _records: &[Vec<u8>],
    ) -> Result<u64> {
        audio_not_yet("sync store: track_push")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ghi_store::keys::{MemoryKeyStore, Protection};

    use super::*;

    #[test]
    fn the_real_store_is_a_sync_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(
            dir.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        let dynamic: &dyn SyncStore = &store;
        // The Lamport clock is real from W0: it follows a remote value.
        dynamic.observe_lamport(1_000).unwrap();
        dynamic.observe_lamport(3).unwrap();
    }
}
