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
use ghi_store::store::Store;
use ghi_store::sync::apply::{ApplyResult, TombResult};
use ghi_store::sync::audio::RawOffer;
use ghi_store::sync::devices::{Device, NewDevice};
use ghi_store::sync::feed::{ChangeBatch, TombBatch};
use ghi_store::sync::leases::Lease;
use ghi_store::sync::records::{Record, SettingRec, SyncTombstone};
use ghi_store::sync::wipe::WipeReport;
use zeroize::Zeroizing;

use crate::audio::{OfferResult, STORAGE_HEADROOM_BYTES, TrackInfo};
use crate::wire::{RefuseReason, TrackOffer};

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
    fn accept_dek(&self, meeting_gid: &str, dek: &[u8; 32], from_device: &str) -> Result<()>;
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
    /// Compare-and-set of a lease's state: moves it to `to` only if it is in
    /// one of `from`; false when it was not (the commit or revoke that raced
    /// won).
    fn lease_transition(&self, job_uuid: &str, from: &[&str], to: &str) -> Result<bool>;
    /// The revoke side of the revoke/commit race: `granted -> revoked`. True
    /// means the revoke won (`Revoked`); false means the commit did
    /// (`AlreadyDone`).
    fn lease_revoke(&self, job_uuid: &str) -> Result<bool>;
    /// Every lease of a meeting, newest epoch first.
    fn leases_for_meeting(&self, meeting_gid: &str) -> Result<Vec<Lease>>;
    /// Every lease that is still open (both roles).
    fn leases_open(&self) -> Result<Vec<Lease>>;

    // --- mass-delete confirmation (D13, per peer device)
    fn mass_delete_confirmed(&self, device_gid: &str) -> Result<bool>;
    fn set_mass_delete_confirmed(&self, device_gid: &str, confirmed: bool) -> Result<()>;

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
    /// Sender: the peer holds the whole track (`tracks_to_send` stops listing
    /// it). Slice 15-F additive method; the default does nothing, so the
    /// real store must override it once `tracks_to_send` filters on it.
    fn mark_track_sent(&self, _device_gid: &str, _track_gid: &str) -> Result<()> {
        Ok(())
    }
    /// Spoke: whether a row has local changes the hub has not acknowledged.
    /// `mark_clean` re-logs a row, so the spoke's push feed skips clean ones.
    /// The default (everything is dirty) is for fakes that never re-log.
    fn sync_dirty(&self, _kind: &str, _gid: &str) -> Result<bool> {
        Ok(true)
    }
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
    fn accept_dek(&self, meeting_gid: &str, dek: &[u8; 32], from_device: &str) -> Result<()> {
        Store::accept_dek(self, meeting_gid, dek, from_device)
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

    fn mass_delete_confirmed(&self, device_gid: &str) -> Result<bool> {
        Store::mass_delete_confirmed(self, device_gid)
    }
    fn set_mass_delete_confirmed(&self, device_gid: &str, confirmed: bool) -> Result<()> {
        Store::set_mass_delete_confirmed(self, device_gid, confirmed)
    }

    fn lease_transition(&self, job_uuid: &str, from: &[&str], to: &str) -> Result<bool> {
        Store::lease_transition(self, job_uuid, from, to)
    }
    fn lease_revoke(&self, job_uuid: &str) -> Result<bool> {
        Store::lease_revoke(self, job_uuid)
    }
    fn leases_for_meeting(&self, meeting_gid: &str) -> Result<Vec<Lease>> {
        Store::leases_for_meeting(self, meeting_gid)
    }
    fn leases_open(&self) -> Result<Vec<Lease>> {
        Store::leases_open(self)
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

    fn tracks_to_send(&self, device_gid: &str) -> Result<Vec<TrackInfo>> {
        Ok(Store::tracks_to_send(self, device_gid)?
            .into_iter()
            .filter(|t| t.header.len() > 5)
            .map(|t| TrackInfo {
                magic: t.header[..4].to_vec(),
                version: t.header[4],
                prefix: t.header[5..].to_vec(),
                track_gid: t.track_gid,
                meeting_gid: t.meeting_gid,
                pages: t.records,
                bytes: t.bytes,
            })
            .collect())
    }
    fn track_read_pages(&self, track_gid: &str, first: u64, max: usize) -> Result<Vec<Vec<u8>>> {
        Store::track_read_records(self, track_gid, first, max)
    }
    fn mark_track_sent(&self, device_gid: &str, track_gid: &str) -> Result<()> {
        Store::mark_track_sent(self, device_gid, track_gid)
    }
    fn track_offer(&self, from_device: &str, offer: &TrackOffer) -> Result<OfferResult> {
        let h = &offer.header;
        let mut header = Vec::with_capacity(h.magic.0.len() + 1 + h.prefix.0.len());
        header.extend_from_slice(&h.magic.0);
        header.push(h.version);
        header.extend_from_slice(&h.prefix.0);
        let need = offer.bytes.saturating_add(STORAGE_HEADROOM_BYTES);
        let free = free_bytes(self.dir());
        Ok(
            match Store::raw_offer(
                self,
                from_device,
                &offer.meeting_gid,
                &offer.track_gid,
                &header,
                free,
                need,
            )? {
                RawOffer::Have(n) => OfferResult::Have(n),
                RawOffer::Complete => OfferResult::Complete,
                RawOffer::Deleted => OfferResult::Refuse(RefuseReason::Deleted),
                RawOffer::NoKey => OfferResult::Refuse(RefuseReason::NoKey),
                RawOffer::StorageFull => OfferResult::Refuse(RefuseReason::StorageFull),
            },
        )
    }
    fn track_push(
        &self,
        _from_device: &str,
        track_gid: &str,
        prefix: &[u8],
        first: u64,
        records: &[Vec<u8>],
    ) -> Result<u64> {
        Store::raw_push(self, track_gid, prefix, first, records)
    }
    fn sync_dirty(&self, kind: &str, gid: &str) -> Result<bool> {
        Store::sync_dirty(self, kind, gid)
    }
}

/// Free bytes on the volume holding `dir`; `None` where it is not known (the
/// offer is then not refused for space).
#[cfg(target_vendor = "apple")]
fn free_bytes(dir: &std::path::Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    // SAFETY: a zeroed statvfs is a valid out-parameter; `path` is NUL-terminated.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut st) } != 0 {
        return None;
    }
    #[allow(clippy::unnecessary_cast, clippy::useless_conversion)]
    Some(u64::from(st.f_bavail) * st.f_frsize as u64)
}

#[cfg(not(target_vendor = "apple"))]
fn free_bytes(_dir: &std::path::Path) -> Option<u64> {
    None
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
