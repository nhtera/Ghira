// SPDX-License-Identifier: Apache-2.0
//! An in-memory [`SyncStore`] for the session tests: a feed log, rows with
//! versions, tombstones, pins, keys, leases and audio parts. It follows the
//! contracts the sessions rely on (doc 07 §7), not the real store's SQL.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard};

use ghi_store::StoreError;
use ghi_store::sync::apply::{ApplyOutcome, ApplyResult, TombResult};
use ghi_store::sync::devices::{Device, DeviceState, NewDevice};
use ghi_store::sync::feed::{ChangeBatch, FeedChange, TombBatch};
use ghi_store::sync::leases::Lease;
use ghi_store::sync::records::{Record, SettingRec, SyncTombstone, TombCause, Version};
use ghi_store::sync::wipe::WipeReport;
use zeroize::Zeroizing;

use crate::audio::{OfferResult, TrackInfo};
use crate::store::SyncStore;
use crate::wire::{RefuseReason, TrackOffer};

type Res<T> = ghi_store::Result<T>;

#[derive(Clone)]
enum Entry {
    Row(String),
    Tomb(SyncTombstone),
}

/// A track recorded here (the sender's copy).
#[derive(Clone)]
pub struct LocalTrack {
    pub info: TrackInfo,
    pub pages: Vec<Vec<u8>>,
}

/// A page of the fake bundle: index, payload, checksum. Verifying a page
/// checks both, like `stream_open` checks the index in its nonce.
pub fn make_page(index: u64, payload: &[u8]) -> Vec<u8> {
    let mut p = index.to_be_bytes().to_vec();
    p.extend_from_slice(payload);
    p.push(payload.iter().fold(0u8, |a, b| a.wrapping_add(*b)));
    p
}

fn page_ok(index: u64, page: &[u8]) -> bool {
    if page.len() < 9 || page[..8] != index.to_be_bytes() {
        return false;
    }
    let (payload, sum) = page[8..].split_at(page.len() - 9);
    payload.iter().fold(0u8, |a, b| a.wrapping_add(*b)) == sum[0]
}

#[derive(Default, Clone)]
struct Part {
    prefix: Vec<u8>,
    pages: Vec<Vec<u8>>,
    total: u64,
}

#[derive(Default)]
struct Inner {
    feed_id: String,
    log: Vec<(i64, Entry)>,
    rows: BTreeMap<String, Record>,
    dirty: BTreeSet<String>,
    tombs: BTreeMap<String, SyncTombstone>,
    lamport: i64,
    devices: BTreeMap<String, (Device, [u8; 32])>,
    next_dev_id: i64,
    deks: BTreeMap<String, [u8; 32]>,
    key_sent: BTreeSet<(String, String)>,
    peer_meetings: BTreeSet<(String, String)>,
    audio_origins: BTreeMap<String, String>,
    /// Tracks with an open import (the real store's open `.part` handle).
    open_imports: BTreeSet<String>,
    leases: BTreeMap<String, Lease>,
    confirmed: BTreeSet<String>,
    // audio
    local_tracks: BTreeMap<String, LocalTrack>,
    tracks_acked: BTreeSet<(String, String)>,
    parts: BTreeMap<String, Part>,
    complete: BTreeMap<String, (Vec<u8>, Vec<Vec<u8>>)>,
    free_bytes: u64,
    // counters for assertions
    pub applied_rows: usize,
}

/// See the module docs. `relay` is true on the hub (it logs what it applies
/// so other spokes pull it).
pub struct FakeSyncStore {
    gid: String,
    relay: bool,
    /// Gives lease renewals a wall time (the real store reads its own).
    clock: Mutex<Option<std::sync::Arc<crate::clock::FakeClock>>>,
    inner: Mutex<Inner>,
}

impl FakeSyncStore {
    pub fn new(gid: &str, relay: bool) -> Self {
        let inner = Inner {
            feed_id: format!("feed-{gid}"),
            free_bytes: u64::MAX / 2,
            ..Inner::default()
        };
        Self {
            gid: gid.to_string(),
            relay,
            clock: Mutex::new(None),
            inner: Mutex::new(inner),
        }
    }

    /// Lets `lease_renew` stamp `wall_deadline_ms` from this clock.
    pub fn attach_clock(&self, clock: std::sync::Arc<crate::clock::FakeClock>) {
        *self.clock.lock().unwrap_or_else(|p| p.into_inner()) = Some(clock);
    }

    fn g(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Writes a local row (dirty, logged), bumping the Lamport clock.
    pub fn put_local(&self, mut rec: Record) -> Record {
        let mut g = self.g();
        g.lamport += 1;
        set_version(
            &mut rec,
            Version {
                lamport: g.lamport,
                origin: self.gid.clone(),
            },
        );
        let gid = rec.gid().to_string();
        g.rows.insert(gid.clone(), rec.clone());
        g.dirty.insert(gid.clone());
        let seq = g.log.len() as i64 + 1;
        g.log.push((seq, Entry::Row(gid)));
        rec
    }

    /// A row that never reaches the feed (a meeting still recording, say).
    pub fn put_unsynced(&self, rec: Record) {
        let mut g = self.g();
        g.rows.insert(rec.gid().to_string(), rec);
    }

    /// Deletes locally: a tombstone, logged; children of a meeting go too.
    pub fn delete_local(&self, gid: &str, kind: &str, cause: Option<TombCause>) {
        let mut g = self.g();
        g.lamport += 1;
        let t = SyncTombstone {
            gid: gid.into(),
            kind: kind.into(),
            lamport: g.lamport,
            origin: self.gid.clone(),
            cause,
        };
        remove_row(&mut g, gid);
        g.tombs.insert(gid.into(), t.clone());
        let seq = g.log.len() as i64 + 1;
        g.log.push((seq, Entry::Tomb(t)));
    }

    pub fn row(&self, gid: &str) -> Option<Record> {
        self.g().rows.get(gid).cloned()
    }

    pub fn row_gids(&self) -> Vec<String> {
        self.g().rows.keys().cloned().collect()
    }

    pub fn is_dirty(&self, gid: &str) -> bool {
        self.g().dirty.contains(gid)
    }

    pub fn is_tombstoned(&self, gid: &str) -> bool {
        self.g().tombs.contains_key(gid)
    }

    pub fn applied_rows(&self) -> usize {
        self.g().applied_rows
    }

    pub fn set_dek(&self, meeting_gid: &str, dek: [u8; 32]) {
        self.g().deks.insert(meeting_gid.into(), dek);
    }

    pub fn forget_dek(&self, meeting_gid: &str) {
        self.g().deks.remove(meeting_gid);
    }

    pub fn dek(&self, meeting_gid: &str) -> Option<[u8; 32]> {
        self.g().deks.get(meeting_gid).copied()
    }

    pub fn key_sent(&self, device: &str, meeting_gid: &str) -> bool {
        self.g()
            .key_sent
            .contains(&(device.to_string(), meeting_gid.to_string()))
    }

    /// Cursor view for assertions: `(push_seq, pull_feed_id, pull_seq)`.
    pub fn cursors(&self, device: &str) -> Option<(i64, Option<String>, i64)> {
        self.g()
            .devices
            .get(device)
            .map(|(d, _)| (d.push_seq, d.pull_feed_id.clone(), d.pull_seq))
    }

    pub fn confirm_mass_delete(&self, device: &str) {
        self.g().confirmed.insert(device.into());
    }

    pub fn head(&self) -> i64 {
        self.g().log.len() as i64
    }

    // --- audio helpers
    pub fn add_local_track(&self, track: LocalTrack) {
        let mut g = self.g();
        g.local_tracks.insert(track.info.track_gid.clone(), track);
    }

    pub fn set_free_bytes(&self, n: u64) {
        self.g().free_bytes = n;
    }

    /// Tracks whose import is still open (a leaked handle in the real store).
    pub fn open_imports(&self) -> usize {
        self.g().open_imports.len()
    }

    pub fn part_pages(&self, track_gid: &str) -> Option<usize> {
        self.g().parts.get(track_gid).map(|p| p.pages.len())
    }

    pub fn complete_track(&self, track_gid: &str) -> Option<(Vec<u8>, Vec<Vec<u8>>)> {
        self.g().complete.get(track_gid).cloned()
    }

    pub fn track_acked(&self, device: &str, track_gid: &str) -> bool {
        self.g()
            .tracks_acked
            .contains(&(device.to_string(), track_gid.to_string()))
    }

    /// Seeds a stale `.part` (a previous attempt).
    pub fn seed_part(&self, track_gid: &str, prefix: &[u8], pages: Vec<Vec<u8>>, total: u64) {
        self.g().parts.insert(
            track_gid.into(),
            Part {
                prefix: prefix.to_vec(),
                pages,
                total,
            },
        );
    }

    pub fn lease(&self, job_uuid: &str) -> Option<Lease> {
        self.g().leases.get(job_uuid).cloned()
    }

    pub fn all_leases(&self) -> Vec<Lease> {
        self.g().leases.values().cloned().collect()
    }

    pub fn device_ids(&self) -> Vec<String> {
        self.g().devices.keys().cloned().collect()
    }
}

fn set_version(rec: &mut Record, v: Version) {
    match rec {
        Record::Meeting(r) => r.version = v,
        Record::Track(r) => r.version = v,
        Record::Person(r) => r.version = v,
        Record::Speaker(r) => r.version = v,
        Record::Segment(r) => r.version = v,
        Record::Note(r) => r.version = v,
        Record::ActionItem(r) => r.version = v,
        Record::Mark(r) => r.version = v,
        Record::Folder(r) => r.version = v,
        Record::Tag(r) => r.version = v,
        Record::MeetingTag(r) => r.version = v,
        Record::VoiceProfile(r) => r.version = v,
        Record::ConflictCopy(r) => r.version = v,
        Record::Setting(r) => r.version = v,
    }
}

fn note_epoch(rec: &Record) -> Option<i64> {
    match rec {
        Record::Note(n) => n.epoch,
        _ => None,
    }
}

fn remove_row(g: &mut Inner, gid: &str) {
    g.rows.remove(gid);
    g.dirty.remove(gid);
    // A meeting takes its children with it.
    let children: Vec<String> = g
        .rows
        .iter()
        .filter(|(_, r)| r.meeting_gid() == Some(gid))
        .map(|(k, _)| k.clone())
        .collect();
    for c in children {
        g.rows.remove(&c);
        g.dirty.remove(&c);
    }
}

fn not_found(kind: &'static str, gid: &str) -> StoreError {
    StoreError::NotFound {
        kind,
        gid: gid.to_string(),
    }
}

impl SyncStore for FakeSyncStore {
    fn feed_id(&self) -> Res<String> {
        Ok(self.g().feed_id.clone())
    }
    fn regen_feed_id(&self) -> Res<String> {
        let mut g = self.g();
        g.feed_id = format!("{}-regen", g.feed_id);
        Ok(g.feed_id.clone())
    }
    fn device_gid(&self) -> Res<String> {
        Ok(self.gid.clone())
    }

    fn changes_since(&self, seq: i64, max: usize) -> Res<ChangeBatch> {
        let g = self.g();
        let max = max.max(1);
        let mut changes = Vec::new();
        let mut upto = seq;
        let mut more = false;
        for (s, e) in g.log.iter().filter(|(s, _)| *s > seq) {
            let Entry::Row(gid) = e else {
                upto = *s;
                continue;
            };
            let Some(rec) = g.rows.get(gid) else {
                upto = *s;
                continue;
            };
            if changes.len() == max {
                more = true;
                break;
            }
            changes.push(FeedChange {
                seq: *s,
                record: rec.clone(),
            });
            upto = *s;
        }
        if !more {
            upto = upto.max(g.log.len() as i64);
        }
        Ok(ChangeBatch {
            changes,
            upto_seq: upto,
            more,
        })
    }

    fn tombs_since(&self, seq: i64, max: usize) -> Res<TombBatch> {
        let g = self.g();
        let max = max.max(1);
        let mut tombs = Vec::new();
        let mut upto = seq;
        let mut more = false;
        for (s, e) in g.log.iter().filter(|(s, _)| *s > seq) {
            let Entry::Tomb(t) = e else {
                upto = *s;
                continue;
            };
            if tombs.len() == max {
                more = true;
                break;
            }
            tombs.push(t.clone());
            upto = *s;
        }
        if !more {
            upto = upto.max(g.log.len() as i64);
        }
        Ok(TombBatch {
            tombs,
            upto_seq: upto,
            more,
        })
    }

    fn relog_meeting(&self, _meeting_gid: &str) -> Res<()> {
        Ok(())
    }

    fn apply_tombs(&self, from_device: &str, tombs: &[SyncTombstone]) -> Res<TombResult> {
        let mut g = self.g();
        let mut res = TombResult::default();
        for t in tombs {
            if t.gid.is_empty() {
                res.rejected.push(t.gid.clone());
                continue;
            }
            if g.tombs.contains_key(&t.gid) {
                continue;
            }
            remove_row(&mut g, &t.gid);
            if t.kind == "meeting" {
                g.deks.remove(&t.gid);
            }
            g.tombs.insert(t.gid.clone(), t.clone());
            if self.relay {
                let seq = g.log.len() as i64 + 1;
                g.log.push((seq, Entry::Tomb(t.clone())));
            }
            res.applied.push(t.gid.clone());
        }
        let _ = from_device;
        Ok(res)
    }

    fn apply_rows(&self, from_device: &str, rows: &[Record]) -> Res<ApplyResult> {
        let mut g = self.g();
        let mut out = ApplyResult::default();
        for rec in rows {
            let gid = rec.gid().to_string();
            let dead = g.tombs.contains_key(&gid)
                || rec.meeting_gid().is_some_and(|m| g.tombs.contains_key(m));
            if dead {
                out.results.push((gid, ApplyOutcome::Tombstoned));
                continue;
            }
            if rec.gid().is_empty() || rec.version().origin.is_empty() {
                return Err(StoreError::Invalid("bad record".into()));
            }
            if let Record::Meeting(m) = rec {
                if let Some(dek) = &m.dek {
                    let dek: [u8; 32] = dek
                        .0
                        .as_slice()
                        .try_into()
                        .map_err(|_| StoreError::Invalid("bad key".into()))?;
                    if g.deks.get(&gid).is_some_and(|have| *have != dek) {
                        return Err(StoreError::Invalid("different key".into()));
                    }
                    g.deks.insert(gid.clone(), dek);
                }
                g.peer_meetings.insert((from_device.into(), gid.clone()));
                let origin = m.audio_origin.clone().unwrap_or_else(|| from_device.into());
                g.audio_origins.entry(gid.clone()).or_insert(origin);
            }
            let outcome = match g.rows.get(&gid) {
                None => ApplyOutcome::Accepted,
                Some(cur) => match (note_epoch(cur), note_epoch(rec)) {
                    (Some(a), Some(b)) if b < a => ApplyOutcome::Tombstoned,
                    (Some(a), Some(b)) if b > a => ApplyOutcome::Accepted,
                    _ if rec.version() > cur.version() => ApplyOutcome::Accepted,
                    _ if rec.version() == cur.version() => {
                        out.results.push((gid, ApplyOutcome::Accepted));
                        continue;
                    }
                    _ => ApplyOutcome::Merged,
                },
            };
            if outcome == ApplyOutcome::Accepted {
                let mut stored = rec.clone();
                if let Record::Meeting(m) = &mut stored {
                    m.dek = None;
                }
                g.rows.insert(gid.clone(), stored);
                g.dirty.remove(&gid);
                g.applied_rows += 1;
                if self.relay {
                    let seq = g.log.len() as i64 + 1;
                    g.log.push((seq, Entry::Row(gid.clone())));
                }
            }
            if outcome == ApplyOutcome::Tombstoned {
                // The lower epoch lost: it never lands.
            }
            out.results.push((gid, outcome));
        }
        Ok(out)
    }

    fn mark_clean(&self, gid: &str, lamport: i64) -> Res<()> {
        let mut g = self.g();
        if g.rows
            .get(gid)
            .is_some_and(|r| r.version().lamport == lamport)
        {
            g.dirty.remove(gid);
        }
        Ok(())
    }
    fn retry_pending(&self) -> Res<usize> {
        Ok(0)
    }
    fn observe_lamport(&self, remote: i64) -> Res<()> {
        let mut g = self.g();
        g.lamport = g.lamport.max(remote);
        Ok(())
    }

    fn devices(&self) -> Res<Vec<Device>> {
        Ok(self.g().devices.values().map(|(d, _)| d.clone()).collect())
    }
    fn device(&self, gid: &str) -> Res<Option<Device>> {
        Ok(self.g().devices.get(gid).map(|(d, _)| d.clone()))
    }
    fn device_by_key(&self, static_pub: &[u8; 32]) -> Res<Option<Device>> {
        Ok(self
            .g()
            .devices
            .values()
            .find(|(d, _)| &d.static_pub == static_pub)
            .map(|(d, _)| d.clone()))
    }
    fn pin_device(&self, device: &NewDevice, pair_psk: &[u8; 32]) -> Res<Device> {
        let mut g = self.g();
        g.next_dev_id += 1;
        let d = Device {
            id: g.next_dev_id,
            gid: device.gid.clone(),
            name: device.name.clone(),
            platform: device.platform.clone(),
            role: device.role,
            static_pub: device.static_pub,
            state: DeviceState::Paired,
            paired_at: 0,
            last_seen: None,
            last_addr: None,
            push_seq: 0,
            pull_feed_id: None,
            pull_seq: 0,
        };
        g.devices.insert(d.gid.clone(), (d.clone(), *pair_psk));
        Ok(d)
    }
    fn unpin_device(&self, gid: &str) -> Res<()> {
        self.g().devices.remove(gid);
        Ok(())
    }
    fn set_wipe_pending(&self, gid: &str) -> Res<()> {
        let mut g = self.g();
        let (d, _) = g
            .devices
            .get_mut(gid)
            .ok_or_else(|| not_found("device", gid))?;
        d.state = DeviceState::WipePending;
        Ok(())
    }
    fn set_unpair_pending(&self, gid: &str) -> Res<()> {
        let mut g = self.g();
        let (d, _) = g
            .devices
            .get_mut(gid)
            .ok_or_else(|| not_found("device", gid))?;
        if d.state == DeviceState::Paired {
            d.state = DeviceState::UnpairPending;
        }
        Ok(())
    }
    fn touch_device(&self, gid: &str, addr: Option<&str>) -> Res<()> {
        let mut g = self.g();
        let (d, _) = g
            .devices
            .get_mut(gid)
            .ok_or_else(|| not_found("device", gid))?;
        d.last_seen = Some(1);
        d.last_addr = addr.map(str::to_string);
        Ok(())
    }
    fn pair_psk(&self, gid: &str) -> Res<Zeroizing<[u8; 32]>> {
        let g = self.g();
        let (_, psk) = g.devices.get(gid).ok_or_else(|| not_found("device", gid))?;
        Ok(Zeroizing::new(*psk))
    }
    fn set_cursors(
        &self,
        gid: &str,
        push_seq: i64,
        pull_feed_id: Option<&str>,
        pull_seq: i64,
    ) -> Res<()> {
        let mut g = self.g();
        let (d, _) = g
            .devices
            .get_mut(gid)
            .ok_or_else(|| not_found("device", gid))?;
        d.push_seq = push_seq;
        d.pull_feed_id = pull_feed_id.map(str::to_string);
        d.pull_seq = pull_seq;
        Ok(())
    }

    fn meeting_dek_for_peer(
        &self,
        device_gid: &str,
        meeting_gid: &str,
    ) -> Res<Option<Zeroizing<[u8; 32]>>> {
        let g = self.g();
        if g.key_sent
            .contains(&(device_gid.into(), meeting_gid.into()))
        {
            return Ok(None);
        }
        Ok(g.deks.get(meeting_gid).map(|d| Zeroizing::new(*d)))
    }
    fn mark_key_sent(&self, device_gid: &str, meeting_gid: &str) -> Res<()> {
        let mut g = self.g();
        g.key_sent.insert((device_gid.into(), meeting_gid.into()));
        g.peer_meetings
            .insert((device_gid.into(), meeting_gid.into()));
        Ok(())
    }
    fn accept_dek(&self, meeting_gid: &str, dek: &[u8; 32], _from_device: &str) -> Res<()> {
        let mut g = self.g();
        if g.deks.get(meeting_gid).is_some_and(|d| d != dek) {
            return Err(StoreError::Invalid("different key".into()));
        }
        g.deks.insert(meeting_gid.into(), *dek);
        Ok(())
    }
    fn meeting_audio_origin_gid(&self, meeting_gid: &str) -> Res<Option<String>> {
        Ok(self.g().audio_origins.get(meeting_gid).cloned())
    }
    fn peer_meetings(&self, device_gid: &str) -> Res<Vec<String>> {
        Ok(self
            .g()
            .peer_meetings
            .iter()
            .filter(|(d, _)| d == device_gid)
            .map(|(_, m)| m.clone())
            .collect())
    }

    fn lease_open(&self, lease: &Lease) -> Res<Lease> {
        let mut g = self.g();
        if g.tombs.contains_key(&lease.meeting_gid) {
            return Err(StoreError::Tombstoned {
                gid: lease.meeting_gid.clone(),
            });
        }
        Ok(g.leases
            .entry(lease.job_uuid.clone())
            .or_insert_with(|| lease.clone())
            .clone())
    }
    fn lease_renew(
        &self,
        job_uuid: &str,
        ttl_ms: i64,
        deadline_cont_ns: i64,
        boot_id: &str,
    ) -> Res<()> {
        let wall = self
            .clock
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|c| crate::clock::Clock::wall_ms(c.as_ref()));
        let mut g = self.g();
        let l = g
            .leases
            .get_mut(job_uuid)
            .ok_or_else(|| not_found("lease", job_uuid))?;
        if !matches!(
            l.state.as_str(),
            "offered" | "granted" | "running" | "revoking"
        ) {
            return Err(StoreError::Fenced);
        }
        if let Some(w) = wall {
            l.wall_deadline_ms = Some(w + ttl_ms);
        }
        l.ttl_ms = ttl_ms;
        l.deadline_cont_ns = Some(deadline_cont_ns);
        l.boot_id = Some(boot_id.to_string());
        Ok(())
    }
    fn lease_state(&self, job_uuid: &str) -> Res<Option<Lease>> {
        Ok(self.g().leases.get(job_uuid).cloned())
    }
    fn lease_fence_ok(
        &self,
        job_uuid: &str,
        now_cont_ns: i64,
        boot_id: &str,
        margin_ms: i64,
    ) -> Res<bool> {
        let g = self.g();
        let Some(l) = g.leases.get(job_uuid) else {
            return Ok(false);
        };
        let open = l.state == "granted";
        let (Some(deadline), Some(boot)) = (l.deadline_cont_ns, l.boot_id.as_deref()) else {
            return Ok(false);
        };
        Ok(open && boot == boot_id && now_cont_ns + margin_ms * 1_000_000 < deadline)
    }
    fn lease_any_open_for(&self, meeting_gid: &str) -> Res<bool> {
        Ok(self.g().leases.values().any(|l| {
            l.meeting_gid == meeting_gid
                && !matches!(
                    l.state.as_str(),
                    "done" | "revoked" | "self_taken" | "fenced"
                )
        }))
    }
    fn lease_transition(&self, job_uuid: &str, from: &[&str], to: &str) -> Res<bool> {
        let mut g = self.g();
        let l = g
            .leases
            .get_mut(job_uuid)
            .ok_or_else(|| not_found("lease", job_uuid))?;
        if from.contains(&l.state.as_str()) {
            l.state = to.to_string();
            Ok(true)
        } else {
            Ok(false)
        }
    }
    fn lease_revoke(&self, job_uuid: &str) -> Res<bool> {
        self.lease_transition(job_uuid, &["granted"], "revoked")
    }
    fn leases_for_meeting(&self, meeting_gid: &str) -> Res<Vec<Lease>> {
        let mut v: Vec<Lease> = self
            .g()
            .leases
            .values()
            .filter(|l| l.meeting_gid == meeting_gid)
            .cloned()
            .collect();
        v.sort_by_key(|l| std::cmp::Reverse(l.epoch));
        Ok(v)
    }
    fn leases_open(&self) -> Res<Vec<Lease>> {
        Ok(self
            .g()
            .leases
            .values()
            .filter(|l| {
                matches!(
                    l.state.as_str(),
                    "offered" | "granted" | "running" | "revoking"
                )
            })
            .cloned()
            .collect())
    }

    fn mass_delete_confirmed(&self, device_gid: &str) -> Res<bool> {
        Ok(self.g().confirmed.contains(device_gid))
    }
    fn set_mass_delete_confirmed(&self, device_gid: &str, confirmed: bool) -> Res<()> {
        let mut g = self.g();
        if confirmed {
            g.confirmed.insert(device_gid.into());
        } else {
            g.confirmed.remove(device_gid);
        }
        Ok(())
    }

    fn wipe_peer(&self, device_gid: &str) -> Res<WipeReport> {
        let mut g = self.g();
        let meetings: Vec<String> = g
            .peer_meetings
            .iter()
            .filter(|(d, _)| d == device_gid)
            .map(|(_, m)| m.clone())
            .collect();
        for m in &meetings {
            // Local shred: no tombstones.
            remove_row(&mut g, m);
            g.deks.remove(m);
        }
        g.peer_meetings.retain(|(d, _)| d != device_gid);
        Ok(WipeReport {
            meetings: meetings.len(),
        })
    }
    fn put_synced(&self, _key: &str, _value_json: &str) -> Res<()> {
        Ok(())
    }
    fn apply_synced(&self, _rec: &SettingRec) -> Res<()> {
        Ok(())
    }

    fn tracks_to_send(&self, device_gid: &str) -> Res<Vec<TrackInfo>> {
        let g = self.g();
        Ok(g.local_tracks
            .values()
            .filter(|t| {
                !g.tracks_acked
                    .contains(&(device_gid.to_string(), t.info.track_gid.clone()))
                    && g.key_sent
                        .contains(&(device_gid.to_string(), t.info.meeting_gid.clone()))
            })
            .map(|t| t.info.clone())
            .collect())
    }
    fn track_read_pages(&self, track_gid: &str, first: u64, max: usize) -> Res<Vec<Vec<u8>>> {
        let g = self.g();
        let t = g
            .local_tracks
            .get(track_gid)
            .ok_or_else(|| not_found("track", track_gid))?;
        Ok(t.pages
            .iter()
            .skip(first as usize)
            .take(max)
            .cloned()
            .collect())
    }
    fn mark_track_sent(&self, device_gid: &str, track_gid: &str) -> Res<()> {
        self.g()
            .tracks_acked
            .insert((device_gid.to_string(), track_gid.to_string()));
        Ok(())
    }
    fn track_offer(&self, from_device: &str, offer: &TrackOffer) -> Res<OfferResult> {
        let g = self.g();
        if g.tombs.contains_key(&offer.meeting_gid) {
            return Ok(OfferResult::Refuse(RefuseReason::Deleted));
        }
        if !g.deks.contains_key(&offer.meeting_gid)
            || !g
                .peer_meetings
                .contains(&(from_device.to_string(), offer.meeting_gid.clone()))
        {
            return Ok(OfferResult::Refuse(RefuseReason::NoKey));
        }
        // A finished track is final whatever the prefix (advisor 3).
        if g.complete.contains_key(&offer.track_gid) {
            return Ok(OfferResult::Complete);
        }
        if g.free_bytes
            < offer
                .bytes
                .saturating_add(crate::audio::STORAGE_HEADROOM_BYTES)
        {
            return Ok(OfferResult::Refuse(RefuseReason::StorageFull));
        }
        drop(g);
        let mut g = self.g();
        let prefix = offer.header.prefix.0.clone();
        let have = match g.parts.get_mut(&offer.track_gid) {
            Some(p) if p.prefix == prefix => p.pages.len() as u64,
            Some(p) => {
                // A different prefix: the sender cut the track; start over.
                p.prefix = prefix;
                p.pages.clear();
                p.total = offer.pages;
                0
            }
            None => {
                g.parts.insert(
                    offer.track_gid.clone(),
                    Part {
                        prefix,
                        pages: Vec::new(),
                        total: offer.pages,
                    },
                );
                0
            }
        };
        g.open_imports.insert(offer.track_gid.clone());
        Ok(OfferResult::Have(have))
    }
    fn track_abandon(&self, track_gids: &[String]) {
        let mut g = self.g();
        for gid in track_gids {
            g.open_imports.remove(gid);
        }
    }
    fn track_push(
        &self,
        _from_device: &str,
        track_gid: &str,
        prefix: &[u8],
        first: u64,
        records: &[Vec<u8>],
    ) -> Res<u64> {
        let mut g = self.g();
        let part = g
            .parts
            .get_mut(track_gid)
            .ok_or_else(|| not_found("track", track_gid))?;
        if part.prefix != prefix || first != part.pages.len() as u64 {
            return Err(StoreError::Invalid("pages out of order".into()));
        }
        for (i, rec) in records.iter().enumerate() {
            if !page_ok(first + i as u64, rec) {
                // Truncated to the last good page (what is already stored).
                return Err(StoreError::Invalid("bad page".into()));
            }
            part.pages.push(rec.clone());
        }
        let have = part.pages.len() as u64;
        if have == part.total {
            g.open_imports.remove(track_gid);
            let done = g.parts.remove(track_gid).unwrap_or_default();
            g.complete
                .insert(track_gid.to_string(), (done.prefix, done.pages));
        }
        Ok(have)
    }
}
