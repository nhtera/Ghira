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
        let epoch = earlier.iter().map(|l| l.epoch).max().unwrap_or(0) + 1;
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
            // The desktop gave it up: nothing to do until `G` passes and the
            // phone may take the job back (`poll`).
            "revoked" | "expired" => {}
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
    /// has passed too. After a reboot
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
            if capable
                && past_g_ms >= offline_after_ms.max(0)
                && store.lease_transition(
                    &l.job_uuid,
                    &["offered", "granted", "running"],
                    "self_taken",
                )?
            {
                actions.push(GrantorAction::SelfTake(take_back(&l)));
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
