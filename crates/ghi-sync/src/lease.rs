// SPDX-License-Identifier: Apache-2.0
//! Job leases (doc 07 §8; slice 15-F): the phone (grantor) hands a meeting's
//! final pass and notes to the desktop (holder), with fencing so exactly one
//! result is kept.
//!
//! Grantor states: `offered -> granted -> done | revoking -> self_taken |
//! expired -> self_taken(epoch + 1)` (if capable and N hours have passed
//! since `G`). Holder: `H = recv + ttl`, renewed on every `LeaseStatus`; it
//! checks `now < H` before claiming, at each checkpoint and before the
//! commit (`H - 1 min`). Deadlines are durations on the injected
//! [`crate::clock::Clock`]; after a holder reboot the lease is suspended
//! until renewed; the grantor uses wall time plus an hour of grace after its
//! own reboot. `Revoked` and `AlreadyDone` are decided by a compare-and-set on
//! the lease row (the same one the result commit runs in its transaction), so
//! exactly one is ever sent.
//!
//! All lease state lives in the store (`leases` table): these types hold none.

use ghi_store::StoreError;
use ghi_store::sync::leases::{Lease, LeaseRole};

use crate::clock::Clock;
use crate::session::Rpc;
use crate::store::SyncStore;
use crate::transport::Transport;
use crate::wire::{LeaseInfo, LeaseQuery, Message, ProcessRequest, RefuseReason};
use crate::{Result, SyncError};

/// Default `ttl_ms`: 12 hours.
pub const DEFAULT_TTL_MS: i64 = 12 * 3600 * 1000;
/// The grantor's extra wait past `ttl` before it may self-take.
pub const GRACE_MS: i64 = 15 * 60 * 1000;
/// The holder stops this long before `H` (it must not commit inside it).
pub const COMMIT_MARGIN_MS: i64 = 60 * 1000;
/// Extra grace the grantor uses after its own reboot (it has only wall time).
pub const REBOOT_GRACE_MS: i64 = 3600 * 1000;
/// Longest ttl a holder accepts.
pub const MAX_TTL_MS: i64 = 7 * 24 * 3600 * 1000;
/// Job kinds a lease may carry.
pub const KINDS: [&str; 3] = ["final_pass", "notes_live", "notes_final"];
/// Holder states in which the job may still run.
const HOLDER_OPEN: [&str; 2] = ["granted", "running"];

/// The grantor's view of a lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantorState {
    Offered,
    Granted,
    Done,
    Revoking,
    SelfTaken,
    Expired,
}

impl GrantorState {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "offered" => Self::Offered,
            "granted" | "running" => Self::Granted,
            "done" => Self::Done,
            "revoking" => Self::Revoking,
            "self_taken" => Self::SelfTaken,
            "expired" => Self::Expired,
            _ => return None,
        })
    }
}

/// A job the phone took back and must now run itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakeBack {
    pub meeting_gid: String,
    pub job_uuid: String,
    /// The fencing epoch the local result carries (`epoch + 1`).
    pub epoch: i64,
    pub kinds: Vec<String>,
}

/// What the grantor decides when polled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantorAction {
    /// Take the job back with this epoch (`epoch + 1`).
    SelfTake(TakeBack),
}

/// What one pass of the `Leases` phase did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LeaseExchange {
    /// The desktop's state of each granted lease (the "Final pass on desktop
    /// %" chip).
    pub infos: Vec<LeaseInfo>,
    /// Jobs the phone took back after a revoke.
    pub take_back: Vec<TakeBack>,
}

fn cont_now(clock: &dyn Clock) -> i64 {
    i64::try_from(clock.now_cont_ns()).unwrap_or(i64::MAX)
}

fn ms_to_ns(ms: i64) -> i64 {
    ms.saturating_mul(1_000_000)
}

fn unexpected(what: &str) -> SyncError {
    SyncError::Wire(format!("unexpected reply to {what}"))
}

fn request_of(l: &Lease) -> ProcessRequest {
    ProcessRequest {
        job_uuid: l.job_uuid.clone(),
        meeting_gid: l.meeting_gid.clone(),
        epoch: l.epoch,
        kinds: l.kinds.clone(),
        ttl_ms: l.ttl_ms,
    }
}

/// The phone's side.
#[derive(Debug, Default)]
pub struct Grantor;

impl Grantor {
    /// Records an offer for a meeting whose rows, key and audio are acked by
    /// `peer_gid`. The epoch is one above every earlier lease of the meeting.
    pub fn offer(
        &self,
        store: &dyn SyncStore,
        peer_gid: &str,
        meeting_gid: &str,
        kinds: &[String],
        ttl_ms: i64,
    ) -> Result<ProcessRequest> {
        let invalid = |what: &str| SyncError::Store(StoreError::Invalid(what.to_string()));
        if kinds.is_empty() || kinds.iter().any(|k| !KINDS.contains(&k.as_str())) {
            return Err(invalid("unknown job kind"));
        }
        if ttl_ms <= 0 || ttl_ms > MAX_TTL_MS {
            return Err(invalid("lease ttl out of range"));
        }
        // The desktop needs everything before it can work: the meeting's rows,
        // its key and its audio (doc 07 §8).
        let known = store
            .peer_meetings(peer_gid)?
            .iter()
            .any(|m| m == meeting_gid);
        let key_pending = store.meeting_dek_for_peer(peer_gid, meeting_gid)?.is_some();
        let audio_pending = store
            .tracks_to_send(peer_gid)?
            .iter()
            .any(|t| t.meeting_gid == meeting_gid);
        if !known || key_pending || audio_pending {
            return Err(invalid("rows, key and audio must be acked first"));
        }
        let earlier = store.leases_for_meeting(meeting_gid)?;
        if earlier.iter().any(|l| {
            l.role == LeaseRole::Grantor
                && matches!(
                    l.state.as_str(),
                    "offered" | "granted" | "running" | "revoking"
                )
        }) {
            return Err(invalid("the meeting already has an open lease"));
        }
        // A self-take ran locally under `epoch + 1`, so that one is spent too.
        let spent = |l: &Lease| l.epoch + i64::from(l.state == "self_taken");
        let epoch = earlier.iter().map(spent).max().unwrap_or(0) + 1;
        let lease = Lease {
            job_uuid: uuid::Uuid::new_v4().to_string(),
            meeting_gid: meeting_gid.to_string(),
            role: LeaseRole::Grantor,
            epoch,
            kinds: kinds.to_vec(),
            state: "offered".to_string(),
            ttl_ms,
            deadline_cont_ns: None,
            boot_id: None,
            wall_deadline_ms: None,
            progress: 0.0,
            peer_gid: Some(peer_gid.to_string()),
        };
        Ok(request_of(&store.lease_open(&lease)?))
    }

    /// `G = now + ttl + grace`. Written before an offer is sent, so an offer
    /// whose reply is lost still counts as granted from here; after a status
    /// reply that says the desktop still holds the lease, so `G` follows `H`.
    fn stamp(&self, store: &dyn SyncStore, clock: &dyn Clock, l: &Lease) -> Result<()> {
        let g_ns = cont_now(clock).saturating_add(ms_to_ns(l.ttl_ms.saturating_add(GRACE_MS)));
        Ok(store.lease_renew(&l.job_uuid, l.ttl_ms, g_ns, &clock.boot_id())?)
    }

    fn mine(&self, store: &dyn SyncStore, peer: &str) -> Result<Vec<Lease>> {
        Ok(store
            .leases_open()?
            .into_iter()
            .filter(|l| l.role == LeaseRole::Grantor && l.peer_gid.as_deref() == Some(peer))
            .collect())
    }

    /// The `Leases` phase: sends pending offers, revokes and status queries
    /// and applies the answers.
    pub fn exchange(
        &self,
        store: &dyn SyncStore,
        clock: &dyn Clock,
        rpc: &mut Rpc,
        transport: &mut dyn Transport,
        peer: &str,
    ) -> Result<LeaseExchange> {
        let mut out = LeaseExchange::default();
        let waiting_audio: Vec<String> = store
            .tracks_to_send(peer)?
            .into_iter()
            .map(|t| t.meeting_gid)
            .collect();
        for l in self.mine(store, peer)? {
            match l.state.as_str() {
                "offered" if !waiting_audio.contains(&l.meeting_gid) => {
                    self.send_offer(store, clock, rpc, transport, &l)?;
                }
                "revoking" => self.send_revoke(store, rpc, transport, &l, &mut out)?,
                "granted" | "running" => {
                    self.send_status(store, clock, rpc, transport, &l, &mut out)?;
                }
                _ => {}
            }
        }
        Ok(out)
    }

    fn send_offer(
        &self,
        store: &dyn SyncStore,
        clock: &dyn Clock,
        rpc: &mut Rpc,
        transport: &mut dyn Transport,
        l: &Lease,
    ) -> Result<()> {
        self.stamp(store, clock, l)?;
        match rpc.call(transport, &Message::ProcessRequest(request_of(l)))? {
            Message::ProcessAccept { job_uuid, epoch }
                if job_uuid == l.job_uuid && epoch == l.epoch =>
            {
                store.lease_transition(&l.job_uuid, &["offered"], "granted")?;
            }
            // The meeting is gone on the desktop: nothing to hand over.
            Message::Refuse(RefuseReason::Deleted) => {
                store.lease_transition(&l.job_uuid, &["offered"], "revoked")?;
            }
            // Not accepted now: it stays offered, and G keeps running.
            Message::Refuse(_) => {}
            _ => return Err(unexpected("ProcessRequest")),
        }
        Ok(())
    }

    fn send_revoke(
        &self,
        store: &dyn SyncStore,
        rpc: &mut Rpc,
        transport: &mut dyn Transport,
        l: &Lease,
        out: &mut LeaseExchange,
    ) -> Result<()> {
        let q = LeaseQuery {
            meeting_gid: l.meeting_gid.clone(),
            epoch: l.epoch,
        };
        match rpc.call(transport, &Message::LeaseRevoke(q))? {
            Message::Revoked { epoch } if epoch == l.epoch => {
                if store.lease_transition(&l.job_uuid, &["revoking"], "self_taken")? {
                    out.take_back.push(take_back(l));
                }
            }
            // The desktop finished first: its result is the one; take nothing.
            Message::AlreadyDone { job_uuid } if job_uuid == l.job_uuid => {
                store.lease_transition(&l.job_uuid, &["revoking"], "done")?;
            }
            _ => return Err(unexpected("LeaseRevoke")),
        }
        Ok(())
    }

    fn send_status(
        &self,
        store: &dyn SyncStore,
        clock: &dyn Clock,
        rpc: &mut Rpc,
        transport: &mut dyn Transport,
        l: &Lease,
        out: &mut LeaseExchange,
    ) -> Result<()> {
        let q = LeaseQuery {
            meeting_gid: l.meeting_gid.clone(),
            epoch: l.epoch,
        };
        let Message::LeaseStatusReply(infos) =
            rpc.call(transport, &Message::LeaseStatus(vec![q]))?
        else {
            return Err(unexpected("LeaseStatus"));
        };
        // Only the answer about this lease counts.
        let Some(info) = infos.into_iter().find(|i| i.meeting_gid == l.meeting_gid) else {
            return Ok(());
        };
        match info.state.as_str() {
            "done" => {
                store.lease_transition(&l.job_uuid, &["granted", "running"], "done")?;
            }
            // The desktop's deadline passed (it slept, or it never got to the
            // job): it cannot commit under this lease any more, so `G` is now.
            // `poll` then has the phone take the job back, or (a phone that
            // cannot run it) close the lease so the meeting is offered again
            // at the next epoch.
            "expired" => {
                let now = cont_now(clock);
                store.lease_renew(&l.job_uuid, l.ttl_ms, now, &clock.boot_id())?;
            }
            "revoked" => {}
            // queued / running: the desktop renewed H, so G moves too.
            _ => self.stamp(store, clock, l)?,
        }
        out.infos.push(info);
        Ok(())
    }

    /// "Process on this phone now": starts a revoke (sent in the next
    /// session's `Leases` phase).
    pub fn revoke(&self, store: &dyn SyncStore, meeting_gid: &str) -> Result<()> {
        let open = store
            .leases_for_meeting(meeting_gid)?
            .into_iter()
            .find(|l| {
                l.role == LeaseRole::Grantor && matches!(l.state.as_str(), "granted" | "running")
            });
        match open {
            Some(l)
                if store.lease_transition(&l.job_uuid, &["granted", "running"], "revoking")? =>
            {
                Ok(())
            }
            _ => Err(SyncError::Store(StoreError::Invalid(
                "no granted lease to revoke".into(),
            ))),
        }
    }

    /// Whether anything is due: a lease whose `G` passed is expired
    /// ([`Grantor::state_of`]); the phone takes the job back (`epoch + 1`)
    /// only if it is `capable` of running the pass and `G + offline_after_ms`
    /// has passed too; a phone that is not capable closes the lease as
    /// `expired` so the meeting can be offered again. After a reboot
    /// only wall time is left, so an hour of grace is added.
    pub fn poll(
        &self,
        store: &dyn SyncStore,
        clock: &dyn Clock,
        offline_after_ms: i64,
        capable: bool,
    ) -> Result<Vec<GrantorAction>> {
        let mut actions = Vec::new();
        for l in store.leases_open()? {
            if l.role != LeaseRole::Grantor
                || !matches!(l.state.as_str(), "offered" | "granted" | "running")
            {
                continue;
            }
            let Some(past_g_ms) = self.ms_past_g(clock, &l) else {
                continue;
            };
            // `Expired` is derived, not stored: G passed and the lease is still
            // open (see `state_of`). Take the job back only if allowed to.
            if capable {
                if past_g_ms >= offline_after_ms.max(0)
                    && store.lease_transition(
                        &l.job_uuid,
                        &["offered", "granted", "running"],
                        "self_taken",
                    )?
                {
                    actions.push(GrantorAction::SelfTake(take_back(&l)));
                }
            } else if past_g_ms >= 0 {
                // A phone below the final-pass tier cannot run it: the lease
                // is closed (`expired`) and the meeting offered again, at
                // `epoch + 1`, when the desktop is next reachable (it may
                // have been asleep). The newer epoch fences the old holder.
                store.lease_transition(
                    &l.job_uuid,
                    &["offered", "granted", "running"],
                    "expired",
                )?;
            }
        }
        Ok(actions)
    }

    /// The grantor's state of a lease, with `Expired` derived: `G` passed and
    /// the phone has not taken the job back yet.
    pub fn state_of(&self, clock: &dyn Clock, l: &Lease) -> Option<GrantorState> {
        let state = GrantorState::parse(&l.state)?;
        let open = matches!(state, GrantorState::Offered | GrantorState::Granted);
        if open && self.ms_past_g(clock, l).is_some_and(|ms| ms >= 0) {
            return Some(GrantorState::Expired);
        }
        Some(state)
    }

    /// Milliseconds since `G` (negative: not yet), `None` if it was never
    /// stamped.
    fn ms_past_g(&self, clock: &dyn Clock, l: &Lease) -> Option<i64> {
        if l.boot_id.as_deref() == Some(clock.boot_id().as_str()) {
            let g = l.deadline_cont_ns?;
            return Some((cont_now(clock) - g) / 1_000_000);
        }
        // Rebooted: the sleep-inclusive baseline is gone. Wall time, plus the
        // grace; the stored wall deadline is `ttl` past the last stamp.
        let g_wall = l
            .wall_deadline_ms?
            .saturating_add(GRACE_MS)
            .saturating_add(REBOOT_GRACE_MS);
        Some(clock.wall_ms() - g_wall)
    }
}

fn take_back(l: &Lease) -> TakeBack {
    TakeBack {
        meeting_gid: l.meeting_gid.clone(),
        job_uuid: l.job_uuid.clone(),
        epoch: l.epoch + 1,
        kinds: l.kinds.clone(),
    }
}

/// What the holder does with a `ProcessRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestOutcome {
    Accepted {
        job_uuid: String,
        epoch: i64,
        /// First time this `job_uuid` was seen: the caller enqueues the local
        /// job (payload: the lease's `job_uuid` only).
        newly_opened: bool,
    },
    Refused(RefuseReason),
}

/// The desktop's side.
#[derive(Debug, Default)]
pub struct Holder;

impl Holder {
    /// Handles a `ProcessRequest` (idempotent by `job_uuid`): opens the lease
    /// with `H = now + ttl`. Peer data is validated; it never becomes a job
    /// payload.
    pub fn on_request(
        &self,
        store: &dyn SyncStore,
        clock: &dyn Clock,
        from_device: &str,
        req: &ProcessRequest,
    ) -> Result<RequestOutcome> {
        let valid = !req.job_uuid.is_empty()
            && req.job_uuid.len() <= 64
            && !req.meeting_gid.is_empty()
            && req.meeting_gid.len() <= 64
            && req.epoch >= 0
            && req.ttl_ms > 0
            && req.ttl_ms <= MAX_TTL_MS
            && !req.kinds.is_empty()
            && req.kinds.iter().all(|k| KINDS.contains(&k.as_str()));
        if !valid {
            return Ok(RequestOutcome::Refused(RefuseReason::Unsupported));
        }
        let accepted = |newly_opened| RequestOutcome::Accepted {
            job_uuid: req.job_uuid.clone(),
            epoch: req.epoch,
            newly_opened,
        };
        if store.lease_state(&req.job_uuid)?.is_some() {
            return Ok(accepted(false));
        }
        // No meeting exchanged with this device: no key, nothing to process.
        if !store.peer_meetings(from_device)?.contains(&req.meeting_gid) {
            return Ok(RequestOutcome::Refused(RefuseReason::NoKey));
        }
        // A higher epoch is the phone's answer to a lease it saw expire: the
        // older one must not commit any more.
        for old in store.leases_for_meeting(&req.meeting_gid)? {
            if old.role == LeaseRole::Holder
                && old.epoch < req.epoch
                && HOLDER_OPEN.contains(&old.state.as_str())
            {
                store.lease_transition(&old.job_uuid, &HOLDER_OPEN, "expired")?;
            }
        }
        let lease = Lease {
            job_uuid: req.job_uuid.clone(),
            meeting_gid: req.meeting_gid.clone(),
            role: LeaseRole::Holder,
            epoch: req.epoch,
            kinds: req.kinds.clone(),
            state: "granted".to_string(),
            ttl_ms: req.ttl_ms,
            deadline_cont_ns: Some(cont_now(clock).saturating_add(ms_to_ns(req.ttl_ms))),
            boot_id: Some(clock.boot_id()),
            wall_deadline_ms: Some(clock.wall_ms().saturating_add(req.ttl_ms)),
            progress: 0.0,
            peer_gid: Some(from_device.to_string()),
        };
        match store.lease_open(&lease) {
            Ok(_) => Ok(accepted(true)),
            Err(StoreError::Tombstoned { .. }) => {
                Ok(RequestOutcome::Refused(RefuseReason::Deleted))
            }
            Err(e) => Err(e.into()),
        }
    }

    fn find(&self, store: &dyn SyncStore, q: &LeaseQuery) -> Result<Option<Lease>> {
        Ok(store
            .leases_for_meeting(&q.meeting_gid)?
            .into_iter()
            .find(|l| l.role == LeaseRole::Holder && l.epoch == q.epoch))
    }

    /// `LeaseRevoke`: exactly one of `Revoked` / `AlreadyDone`, decided by the
    /// compare-and-set the result commit also runs.
    pub fn on_revoke(&self, store: &dyn SyncStore, q: &LeaseQuery) -> Result<Message> {
        let Some(l) = self.find(store, q)? else {
            // Nothing is running here for it.
            return Ok(Message::Revoked { epoch: q.epoch });
        };
        let done = || Message::AlreadyDone {
            job_uuid: l.job_uuid.clone(),
        };
        Ok(match l.state.as_str() {
            "done" => done(),
            s if HOLDER_OPEN.contains(&s) => {
                if store.lease_revoke(&l.job_uuid)? {
                    Message::Revoked { epoch: q.epoch }
                } else {
                    done()
                }
            }
            _ => Message::Revoked { epoch: q.epoch },
        })
    }

    /// `LeaseStatus`: renews (`H` moves to now + ttl) every lease that is
    /// still valid, and reports each. A lease past `H` is expired for good; a
    /// lease from before a reboot (suspended) resumes when renewed.
    pub fn on_status(
        &self,
        store: &dyn SyncStore,
        clock: &dyn Clock,
        queries: &[LeaseQuery],
    ) -> Result<Vec<LeaseInfo>> {
        let mut out = Vec::with_capacity(queries.len());
        for q in queries {
            let info = |state: &str, progress: f64, job: &str| LeaseInfo {
                meeting_gid: q.meeting_gid.clone(),
                state: state.to_string(),
                progress,
                job_uuid: job.to_string(),
            };
            let Some(l) = self.find(store, q)? else {
                out.push(info("expired", 0.0, ""));
                continue;
            };
            let state = match l.state.as_str() {
                s if HOLDER_OPEN.contains(&s) => {
                    let same_boot = l.boot_id.as_deref() == Some(clock.boot_id().as_str());
                    let past_h = l.deadline_cont_ns.is_none_or(|h| cont_now(clock) >= h);
                    if same_boot && past_h {
                        store.lease_transition(&l.job_uuid, &HOLDER_OPEN, "expired")?;
                        "expired"
                    } else {
                        let h = cont_now(clock).saturating_add(ms_to_ns(l.ttl_ms));
                        match store.lease_renew(&l.job_uuid, l.ttl_ms, h, &clock.boot_id()) {
                            // Lost the race to a revoke or a commit: report it.
                            Err(StoreError::Fenced) => "expired",
                            Err(e) => return Err(e.into()),
                            Ok(()) if l.progress > 0.0 => "running",
                            Ok(()) => "queued",
                        }
                    }
                }
                "done" => "done",
                "revoked" => "revoked",
                _ => "expired",
            };
            out.push(info(state, l.progress, &l.job_uuid));
        }
        Ok(out)
    }

    /// The fence the job runner calls: may the job still run, with `margin`
    /// before the deadline?
    pub fn fence_ok(
        &self,
        store: &dyn SyncStore,
        clock: &dyn Clock,
        job_uuid: &str,
        margin_ms: i64,
    ) -> Result<bool> {
        Ok(store.lease_fence_ok(job_uuid, cont_now(clock), &clock.boot_id(), margin_ms)?)
    }

    /// The commit's side of the revoke race (the result transaction runs the
    /// same compare-and-set): `true` when the result may be kept.
    pub fn finish(&self, store: &dyn SyncStore, job_uuid: &str) -> Result<bool> {
        Ok(store.lease_transition(job_uuid, &HOLDER_OPEN, "done")?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    use ghi_store::sync::records::{NoteRec, Record, Version};

    use super::*;
    use crate::clock::FakeClock;
    use crate::mem::MemDuplex;
    use crate::session::fake::FakeSyncStore;
    use crate::session::tests::{
        HUB_KEY, hub_store, meeting, paired_spoke, run_with_clocks, spoke_key, sync_once,
    };

    const HOUR: Duration = Duration::from_secs(3600);

    struct Rig {
        hub: Arc<FakeSyncStore>,
        phone: Arc<FakeSyncStore>,
        hub_clock: Arc<FakeClock>,
        phone_clock: Arc<FakeClock>,
    }

    impl Rig {
        fn advance(&self, by: Duration) {
            self.hub_clock.advance(by);
            self.phone_clock.advance(by);
        }

        /// One session with the rig's clocks.
        fn sync(
            &self,
        ) -> (
            crate::Result<crate::session::SessionReport>,
            crate::Result<crate::session::SessionReport>,
        ) {
            let (a, b) = MemDuplex::pair(spoke_key(1), HUB_KEY);
            run_with_clocks(
                &self.hub,
                &self.phone,
                a,
                b,
                self.hub_clock.clone(),
                self.phone_clock.clone(),
            )
        }

        fn offer(&self) -> ProcessRequest {
            Grantor
                .offer(
                    self.phone.as_ref(),
                    "hub",
                    "m1",
                    &["final_pass".into()],
                    DEFAULT_TTL_MS,
                )
                .unwrap()
        }

        fn phone_lease(&self, job: &str) -> Lease {
            self.phone.lease(job).unwrap()
        }
    }

    /// A phone and hub that exchanged meeting `m1` and its key.
    fn rig() -> Rig {
        let hub = hub_store();
        let phone = paired_spoke(&hub, "phone-a", 1);
        phone.put_local(meeting("m1"));
        phone.set_dek("m1", [7; 32]);
        sync_once(&hub, &phone, 1).0.unwrap();
        let (hub_clock, phone_clock) = (
            Arc::new(FakeClock::new(1_700_000_000_000)),
            Arc::new(FakeClock::new(1_700_000_000_000)),
        );
        hub.attach_clock(hub_clock.clone());
        phone.attach_clock(phone_clock.clone());
        Rig {
            hub,
            phone,
            hub_clock,
            phone_clock,
        }
    }

    // The fake names the phone "phone-a"; the lease's peer on the phone is "hub".
    #[test]
    fn an_offer_needs_rows_key_and_audio_acked_and_one_open_lease_per_meeting() {
        let hub = hub_store();
        let phone = paired_spoke(&hub, "phone-a", 1);
        phone.put_local(meeting("m1"));
        phone.set_dek("m1", [7; 32]);
        let kinds = ["final_pass".to_string()];
        // Nothing acked yet.
        assert!(
            Grantor
                .offer(phone.as_ref(), "hub", "m1", &kinds, DEFAULT_TTL_MS)
                .is_err()
        );
        sync_once(&hub, &phone, 1).0.unwrap();
        // Audio still waiting.
        use crate::audio::TrackInfo;
        use crate::session::fake::{LocalTrack, make_page};
        phone.add_local_track(LocalTrack {
            info: TrackInfo {
                track_gid: "t1".into(),
                meeting_gid: "m1".into(),
                magic: b"GHB1".to_vec(),
                version: 1,
                prefix: vec![1; 19],
                pages: 1,
                bytes: 10,
            },
            pages: vec![make_page(0, b"x")],
        });
        assert!(
            Grantor
                .offer(phone.as_ref(), "hub", "m1", &kinds, DEFAULT_TTL_MS)
                .is_err()
        );
        sync_once(&hub, &phone, 1).0.unwrap();
        // Bad kinds and ttl are refused.
        assert!(
            Grantor
                .offer(
                    phone.as_ref(),
                    "hub",
                    "m1",
                    &["rm -rf".to_string()],
                    DEFAULT_TTL_MS
                )
                .is_err()
        );
        assert!(
            Grantor
                .offer(phone.as_ref(), "hub", "m1", &kinds, 0)
                .is_err()
        );
        let req = Grantor
            .offer(phone.as_ref(), "hub", "m1", &kinds, DEFAULT_TTL_MS)
            .unwrap();
        assert_eq!((req.epoch, req.ttl_ms), (1, DEFAULT_TTL_MS));
        assert!(
            Grantor
                .offer(phone.as_ref(), "hub", "m1", &kinds, DEFAULT_TTL_MS)
                .is_err()
        );
    }

    #[test]
    fn an_offer_becomes_a_lease_on_both_sides_and_a_duplicate_is_a_no_op() {
        let r = rig();
        let req = r.offer();
        assert_eq!(r.phone_lease(&req.job_uuid).state, "offered");
        let (mine, theirs) = r.sync();
        mine.unwrap();
        assert_eq!(
            theirs.unwrap().new_leases,
            std::slice::from_ref(&req.job_uuid)
        );
        let phone_side = r.phone_lease(&req.job_uuid);
        assert_eq!(phone_side.state, "granted");
        // G = send + ttl + 15 min, stamped before the offer went out.
        let g = phone_side.deadline_cont_ns.unwrap();
        assert_eq!(
            g,
            r.phone_clock.now_cont_ns() as i64 + (DEFAULT_TTL_MS + GRACE_MS) * 1_000_000
        );
        let hub_side = r.hub.lease(&req.job_uuid).unwrap();
        assert_eq!(
            (hub_side.role, hub_side.state.as_str()),
            (LeaseRole::Holder, "granted")
        );
        assert_eq!(
            hub_side.deadline_cont_ns.unwrap(),
            r.hub_clock.now_cont_ns() as i64 + DEFAULT_TTL_MS * 1_000_000,
            "H = recv + ttl, 15 min before G"
        );
        // The same request again: accepted, nothing new to enqueue.
        let again = Holder
            .on_request(r.hub.as_ref(), r.hub_clock.as_ref(), "phone-a", &req)
            .unwrap();
        assert!(matches!(
            again,
            RequestOutcome::Accepted {
                newly_opened: false,
                ..
            }
        ));
        assert_eq!(r.hub.all_leases().len(), 1);
    }

    #[test]
    fn the_holder_refuses_what_it_cannot_take() {
        let r = rig();
        let base = ProcessRequest {
            job_uuid: "j1".into(),
            meeting_gid: "m1".into(),
            epoch: 1,
            kinds: vec!["final_pass".into()],
            ttl_ms: 1000,
        };
        let ask = |req: &ProcessRequest, from: &str| {
            Holder
                .on_request(r.hub.as_ref(), r.hub_clock.as_ref(), from, req)
                .unwrap()
        };
        let refused = |o: RequestOutcome| match o {
            RequestOutcome::Refused(why) => why,
            other => panic!("{other:?}"),
        };
        let mut bad = base.clone();
        bad.kinds = vec!["shell".into()];
        assert_eq!(refused(ask(&bad, "phone-a")), RefuseReason::Unsupported);
        bad = base.clone();
        bad.ttl_ms = MAX_TTL_MS + 1;
        assert_eq!(refused(ask(&bad, "phone-a")), RefuseReason::Unsupported);
        bad = base.clone();
        bad.meeting_gid = "never-exchanged".into();
        assert_eq!(refused(ask(&bad, "phone-a")), RefuseReason::NoKey);
        // A meeting that was exchanged with another device only.
        assert_eq!(refused(ask(&base, "phone-b")), RefuseReason::NoKey);
        // A tombstoned meeting.
        r.hub.delete_local("m1", "meeting", None);
        assert_eq!(refused(ask(&base, "phone-a")), RefuseReason::Deleted);
        assert!(r.hub.all_leases().is_empty());
    }

    #[test]
    fn race_a_a_desktop_that_wakes_after_g_never_starts_and_the_phone_takes_over_once() {
        let r = rig();
        let req = r.offer();
        r.sync().0.unwrap();
        let (n, capable) = (12 * 3600 * 1000, true);
        // The desktop sleeps; the phone is away. Just before G + N: not yet.
        r.advance(
            Duration::from_millis((DEFAULT_TTL_MS + GRACE_MS) as u64)
                + 11 * HOUR
                + Duration::from_secs(59 * 60),
        );
        assert!(
            Grantor
                .poll(r.phone.as_ref(), r.phone_clock.as_ref(), n, capable)
                .unwrap()
                .is_empty()
        );
        r.advance(2 * HOUR);
        assert_eq!(
            Grantor.state_of(r.phone_clock.as_ref(), &r.phone_lease(&req.job_uuid)),
            Some(GrantorState::Expired)
        );
        // Capable: epoch + 1, exactly once.
        let acts = Grantor
            .poll(r.phone.as_ref(), r.phone_clock.as_ref(), n, capable)
            .unwrap();
        let [GrantorAction::SelfTake(t)] = acts.as_slice() else {
            panic!("{acts:?}")
        };
        assert_eq!((t.epoch, t.meeting_gid.as_str()), (req.epoch + 1, "m1"));
        assert!(
            Grantor
                .poll(r.phone.as_ref(), r.phone_clock.as_ref(), n, capable)
                .unwrap()
                .is_empty()
        );
        assert_eq!(r.phone_lease(&req.job_uuid).state, "self_taken");
        // The desktop wakes up long after H: its fence says no, so it never
        // starts or commits.
        assert!(
            !Holder
                .fence_ok(r.hub.as_ref(), r.hub_clock.as_ref(), &req.job_uuid, 0)
                .unwrap()
        );
        // A later offer for the meeting is above the epoch the take-back used.
        assert_eq!(r.offer().epoch, 3);
    }

    #[test]
    fn a_phone_that_cannot_run_the_pass_offers_again_after_the_lease_expired() {
        let r = rig();
        let first = r.offer();
        r.sync().0.unwrap();
        // The Mac's lid is shut for longer than the lease: its job is fenced.
        r.advance(Duration::from_millis(
            (DEFAULT_TTL_MS + GRACE_MS) as u64 + 1000,
        ));
        assert!(
            !Holder
                .fence_ok(r.hub.as_ref(), r.hub_clock.as_ref(), &first.job_uuid, 0)
                .unwrap()
        );
        // Not capable: nothing to run here, so the lease is closed as expired
        // (and stays so: a second poll does nothing).
        for _ in 0..2 {
            assert!(
                Grantor
                    .poll(r.phone.as_ref(), r.phone_clock.as_ref(), 0, false)
                    .unwrap()
                    .is_empty()
            );
        }
        assert_eq!(r.phone_lease(&first.job_uuid).state, "expired");
        // The Mac wakes up; the phone offers the meeting again at epoch + 1.
        let second = r.offer();
        assert_eq!(second.epoch, first.epoch + 1);
        assert_ne!(second.job_uuid, first.job_uuid);
        let (_, hub_rep) = r.sync();
        assert_eq!(hub_rep.unwrap().new_leases, vec![second.job_uuid.clone()]);
        // The newer epoch fences the older holder lease for good.
        assert_eq!(r.hub.lease(&first.job_uuid).unwrap().state, "expired");
        assert_eq!(r.hub.lease(&second.job_uuid).unwrap().state, "granted");
        let hub = r.hub.as_ref();
        assert!(!Holder.finish(hub, &first.job_uuid).unwrap());
        assert!(Holder.finish(hub, &second.job_uuid).unwrap(), "exactly one");
        // The phone learns of the result.
        let (mine, _) = r.sync();
        assert_eq!(mine.unwrap().lease_infos[0].state, "done");
        assert_eq!(r.phone_lease(&second.job_uuid).state, "done");
    }

    #[test]
    fn a_holder_that_says_expired_is_taken_at_its_word() {
        let r = rig();
        let req = r.offer();
        r.sync().0.unwrap();
        // The Mac slept past H, but G (H + grace) has not passed on the phone.
        r.advance(Duration::from_millis(DEFAULT_TTL_MS as u64 + 1000));
        let (mine, _) = r.sync();
        assert_eq!(mine.unwrap().lease_infos[0].state, "expired");
        let l = r.phone_lease(&req.job_uuid);
        assert_eq!(
            Grantor.state_of(r.phone_clock.as_ref(), &l),
            Some(GrantorState::Expired),
            "G is now: no waiting out the grace"
        );
        let acts = Grantor
            .poll(r.phone.as_ref(), r.phone_clock.as_ref(), 0, true)
            .unwrap();
        assert!(matches!(acts.as_slice(), [GrantorAction::SelfTake(_)]));
    }

    #[test]
    fn the_holder_stops_a_minute_before_h_and_a_status_renews_it() {
        let r = rig();
        let req = r.offer();
        r.sync().0.unwrap();
        let job = req.job_uuid.as_str();
        let ok = |margin| {
            Holder
                .fence_ok(r.hub.as_ref(), r.hub_clock.as_ref(), job, margin)
                .unwrap()
        };
        assert!(ok(COMMIT_MARGIN_MS));
        r.advance(Duration::from_millis(DEFAULT_TTL_MS as u64 - 90_000));
        assert!(ok(COMMIT_MARGIN_MS));
        r.advance(Duration::from_secs(40));
        assert!(!ok(COMMIT_MARGIN_MS), "inside the last minute: no commit");
        assert!(ok(0), "but it still holds the lease");
        // The phone is back: the next session renews H (and G).
        let before_g = r.phone_lease(job).deadline_cont_ns.unwrap();
        let (mine, _) = r.sync();
        let rep = mine.unwrap();
        assert_eq!(rep.lease_infos.len(), 1);
        assert_eq!(rep.lease_infos[0].state, "queued");
        assert!(ok(COMMIT_MARGIN_MS), "renewed");
        assert!(r.phone_lease(job).deadline_cont_ns.unwrap() > before_g);
    }

    #[test]
    fn an_expired_lease_is_not_revived_by_a_late_status() {
        let r = rig();
        let req = r.offer();
        r.sync().0.unwrap();
        r.advance(Duration::from_millis(DEFAULT_TTL_MS as u64 + 1000));
        let (mine, _) = r.sync();
        assert_eq!(mine.unwrap().lease_infos[0].state, "expired");
        assert!(
            !Holder
                .fence_ok(r.hub.as_ref(), r.hub_clock.as_ref(), &req.job_uuid, 0)
                .unwrap()
        );
        assert_eq!(r.hub.lease(&req.job_uuid).unwrap().state, "expired");
    }

    #[test]
    fn a_holder_reboot_suspends_the_lease_until_it_is_renewed() {
        let r = rig();
        let req = r.offer();
        r.sync().0.unwrap();
        r.advance(Duration::from_secs(600));
        r.hub_clock.reboot();
        assert!(
            !Holder
                .fence_ok(r.hub.as_ref(), r.hub_clock.as_ref(), &req.job_uuid, 0)
                .unwrap()
        );
        let (mine, _) = r.sync();
        assert_eq!(mine.unwrap().lease_infos[0].state, "queued");
        assert!(
            Holder
                .fence_ok(r.hub.as_ref(), r.hub_clock.as_ref(), &req.job_uuid, 0)
                .unwrap()
        );
    }

    #[test]
    fn a_wall_clock_step_in_the_same_boot_does_not_suspend_the_lease() {
        let r = rig();
        let req = r.offer();
        r.sync().0.unwrap();
        // NTP steps the wall clock after wake, forward and back: the boot
        // (and the sleep-inclusive clock) are the same.
        for wall in [1_800_000_000_000, 1_600_000_000_000] {
            r.hub_clock.set_wall_ms(wall);
            r.phone_clock.set_wall_ms(wall);
            assert!(
                Holder
                    .fence_ok(r.hub.as_ref(), r.hub_clock.as_ref(), &req.job_uuid, 0)
                    .unwrap()
            );
        }
        let (mine, _) = r.sync();
        assert_eq!(mine.unwrap().lease_infos[0].state, "queued");
        assert_eq!(r.phone_lease(&req.job_uuid).state, "granted");
    }

    #[test]
    fn a_grantor_reboot_uses_wall_time_with_an_hour_of_grace() {
        let r = rig();
        let req = r.offer();
        r.sync().0.unwrap(); // G stamped; wall deadline = now + ttl
        r.phone_clock.reboot();
        // Wall time only: G_wall = deadline + 15 min + 1 h.
        let to_g = Duration::from_millis((DEFAULT_TTL_MS + GRACE_MS + REBOOT_GRACE_MS) as u64);
        r.phone_clock.advance(to_g - Duration::from_secs(60));
        assert!(
            Grantor
                .poll(r.phone.as_ref(), r.phone_clock.as_ref(), 0, true)
                .unwrap()
                .is_empty()
        );
        r.phone_clock.advance(Duration::from_secs(120));
        let acts = Grantor
            .poll(r.phone.as_ref(), r.phone_clock.as_ref(), 0, true)
            .unwrap();
        assert_eq!(acts.len(), 1);
        assert_eq!(r.phone_lease(&req.job_uuid).state, "self_taken");
    }

    #[test]
    fn done_is_reported_once_and_closes_the_lease() {
        let r = rig();
        let req = r.offer();
        r.sync().0.unwrap();
        assert!(Holder.finish(r.hub.as_ref(), &req.job_uuid).unwrap());
        let (mine, _) = r.sync();
        assert_eq!(mine.unwrap().lease_infos[0].state, "done");
        assert_eq!(r.phone_lease(&req.job_uuid).state, "done");
        // Closed: no more status traffic for it.
        assert!(r.sync().0.unwrap().lease_infos.is_empty());
    }

    #[test]
    fn revoke_takes_the_job_back_with_the_next_epoch() {
        let r = rig();
        let req = r.offer();
        r.sync().0.unwrap();
        Grantor.revoke(r.phone.as_ref(), "m1").unwrap();
        assert_eq!(r.phone_lease(&req.job_uuid).state, "revoking");
        let (mine, _) = r.sync();
        let rep = mine.unwrap();
        assert_eq!(rep.take_back.len(), 1);
        assert_eq!(
            (
                rep.take_back[0].epoch,
                rep.take_back[0].meeting_gid.as_str()
            ),
            (2, "m1")
        );
        assert_eq!(r.phone_lease(&req.job_uuid).state, "self_taken");
        assert_eq!(r.hub.lease(&req.job_uuid).unwrap().state, "revoked");
        assert!(
            !Holder
                .fence_ok(r.hub.as_ref(), r.hub_clock.as_ref(), &req.job_uuid, 0)
                .unwrap()
        );
        assert!(
            Grantor.revoke(r.phone.as_ref(), "m1").is_err(),
            "nothing left to revoke"
        );
    }

    #[test]
    fn revoke_after_the_commit_takes_nothing() {
        let r = rig();
        let req = r.offer();
        r.sync().0.unwrap();
        // The desktop commits first; the phone does not know yet.
        assert!(Holder.finish(r.hub.as_ref(), &req.job_uuid).unwrap());
        Grantor.revoke(r.phone.as_ref(), "m1").unwrap();
        let (mine, _) = r.sync();
        assert!(mine.unwrap().take_back.is_empty());
        assert_eq!(r.phone_lease(&req.job_uuid).state, "done");
    }

    #[test]
    fn race_c_revoke_and_commit_in_the_same_instant_give_exactly_one_answer() {
        for round in 0..300 {
            let r = rig();
            let req = r.offer();
            r.sync().0.unwrap();
            let gate = Arc::new(Barrier::new(2));
            let (hub, job, g2) = (r.hub.clone(), req.job_uuid.clone(), gate.clone());
            let commit = thread::spawn(move || {
                g2.wait();
                Holder.finish(hub.as_ref(), &job).unwrap()
            });
            gate.wait();
            let reply = Holder
                .on_revoke(
                    r.hub.as_ref(),
                    &LeaseQuery {
                        meeting_gid: "m1".into(),
                        epoch: req.epoch,
                    },
                )
                .unwrap();
            let committed = commit.join().unwrap();
            match reply {
                Message::Revoked { .. } => assert!(!committed, "round {round}: both won"),
                Message::AlreadyDone { .. } => assert!(committed, "round {round}: neither won"),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn race_b_two_results_keep_the_higher_epoch_on_both_devices() {
        let r = rig();
        let req = r.offer();
        r.sync().0.unwrap();
        let note = |epoch: i64, body: u8, lamport: i64| {
            Record::Note(NoteRec {
                gid: "note-1".into(),
                version: Version {
                    lamport,
                    origin: "x".into(),
                },
                meeting_gid: "m1".into(),
                epoch: Some(epoch),
                body_ct: Some(ghi_store::sync::records::Bytes(vec![body])),
                ..Default::default()
            })
        };
        // The desktop finished under epoch 1 (and so has the higher Lamport
        // value); the phone self-took with epoch 2 meanwhile.
        r.hub.put_local(note(1, 1, 0));
        r.hub.put_local(note(1, 1, 0)); // bumps the Lamport clock further
        let acts = {
            r.advance(Duration::from_millis(
                (DEFAULT_TTL_MS + GRACE_MS) as u64 + 1,
            ));
            Grantor
                .poll(r.phone.as_ref(), r.phone_clock.as_ref(), 0, true)
                .unwrap()
        };
        let [GrantorAction::SelfTake(t)] = acts.as_slice() else {
            panic!("{acts:?}")
        };
        assert_eq!(t.epoch, 2);
        r.phone.put_local(note(t.epoch, 2, 0));
        r.sync().0.unwrap();
        r.sync().0.unwrap();
        for store in [&r.hub, &r.phone] {
            let Some(Record::Note(n)) = store.row("note-1") else {
                panic!("no note")
            };
            assert_eq!(n.epoch, Some(2));
            assert_eq!(n.body_ct.unwrap().0, vec![2]);
        }
        let _ = req;
    }

    #[test]
    fn a_refused_offer_for_a_deleted_meeting_ends_the_lease() {
        let r = rig();
        let req = r.offer();
        r.hub.delete_local("m1", "meeting", None);
        // The hub's tombstone and the offer cross in one session; the offer
        // goes first, the hub answers Deleted.
        r.sync().0.unwrap();
        assert_eq!(r.phone_lease(&req.job_uuid).state, "revoked");
        assert!(r.hub.all_leases().is_empty());
    }
}
