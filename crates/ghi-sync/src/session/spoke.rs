// SPDX-License-Identifier: Apache-2.0
//! The spoke (phone) side of a session.

use std::sync::Arc;

use ghi_store::StoreError;
use ghi_store::sync::devices::Device;
use ghi_store::sync::records::{Bytes, Record};

use super::{MassDeleteGuard, Rpc, SessionReport, Timing, default_protos, send_msg};
use crate::audio;
use crate::clock::Clock;
use crate::control::{self, ControlOutcome};
use crate::lease::Grantor;
use crate::store::SyncStore;
use crate::transport::Transport;
use crate::wire::{
    self, Ack, ApplyOutcome, Control, ErrorCode, Hello, Message, Proto, PushRows, PushTombs,
    Record as WireRecord,
};
use crate::{Result, SyncError};

/// App version put in `Hello` (display only).
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

fn not_found(gid: &str) -> SyncError {
    SyncError::Store(StoreError::NotFound {
        kind: "device",
        gid: gid.to_string(),
    })
}

fn unexpected(what: &str) -> SyncError {
    SyncError::Wire(format!("unexpected reply to {what}"))
}

/// One session towards the paired hub.
pub struct SpokeSession<T: Transport> {
    pub(crate) store: Arc<dyn SyncStore>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) transport: T,
    pub(crate) hub_device: String,
    pub(crate) rpc: Rpc,
    timing: Timing,
    protos: Vec<Proto>,
    caps: Vec<String>,
    greeted: bool,
    /// The hub's feed id from `HelloOk`.
    hub_feed: Option<String>,
    guard: MassDeleteGuard,
    report: SessionReport,
    /// Set once a command ended the session.
    closed: bool,
}

impl<T: Transport> SpokeSession<T> {
    pub fn new(
        store: Arc<dyn SyncStore>,
        clock: Arc<dyn Clock>,
        transport: T,
        hub_device: String,
    ) -> Self {
        Self {
            store,
            clock,
            transport,
            hub_device,
            rpc: Rpc::default(),
            timing: Timing::default(),
            protos: default_protos(),
            caps: Vec::new(),
            greeted: false,
            hub_feed: None,
            guard: MassDeleteGuard::default(),
            report: SessionReport::default(),
            closed: false,
        }
    }

    /// Replaces the ping, silence and idle limits (tests).
    pub fn with_timing(mut self, timing: Timing) -> Self {
        self.timing = timing;
        self
    }

    /// A flag that, once set, ends a pass after the request in flight (the
    /// current batch or audio chunk): the report says `interrupted` and the
    /// caller sends `Bye`.
    pub fn with_stop(mut self, stop: Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.rpc.with_stop(stop);
        self
    }

    /// Replaces the versions offered in `Hello` (tests of version skew).
    pub fn with_protos(mut self, protos: Vec<Proto>) -> Self {
        self.protos = protos;
        self
    }

    /// What the session did so far (also after an error).
    pub fn report(&self) -> &SessionReport {
        &self.report
    }

    /// The last `Error` the hub sent.
    pub fn last_peer_error(&self) -> Option<&wire::ErrorBody> {
        self.rpc.last_error.as_ref()
    }

    pub(crate) fn hub_device_row(&self) -> Result<Device> {
        self.store
            .device(&self.hub_device)?
            .ok_or_else(|| not_found(&self.hub_device))
    }

    fn call(&mut self, msg: &Message) -> Result<Message> {
        self.rpc.call(&mut self.transport, msg)
    }

    /// Runs one full pass (Hello through PullRows) and returns to idle. The
    /// first call also does `Hello` and `Control`; later ones (from `Idle`)
    /// start at `PushTombs`.
    pub fn run_once(&mut self) -> Result<SessionReport> {
        if self.closed {
            return Err(SyncError::Closed);
        }
        self.transport.set_recv_timeout(Some(self.timing.silence))?;
        if !self.greeted {
            self.hello()?;
            if self.closed {
                return Ok(self.report.clone());
            }
        }
        self.report.interrupted = false;
        self.rpc.arm(true);
        let ran = self.phases();
        self.rpc.arm(false);
        match ran {
            Err(_) if self.rpc.interrupted() => self.report.interrupted = true,
            other => other?,
        }
        Ok(self.report.clone())
    }

    fn phases(&mut self) -> Result<()> {
        let tomb_pos = self.push_tombs()?;
        self.push_rows(tomb_pos)?;
        self.leases()?;
        self.audio()?;
        self.pull()
    }

    /// `Hello` / `HelloOk`, then any pending `Control` the hub carries.
    fn hello(&mut self) -> Result<()> {
        let dev = self.hub_device_row()?;
        let hello = Hello {
            proto: self.protos.clone(),
            app_version: APP_VERSION.to_string(),
            device_gid: self.store.device_gid()?,
            feed_id: self.store.feed_id()?,
            pull_cursor: dev.pull_seq,
            caps: self.caps.clone(),
        };
        let reply = self.call(&Message::Hello(hello))?;
        let Message::HelloOk(ok) = reply else {
            return Err(unexpected("Hello"));
        };
        if ok.device_gid != self.hub_device {
            return Err(SyncError::Wire("hub identity mismatch".into()));
        }
        // The hub picked one version: it must be one we offered.
        if wire::negotiate(&self.protos, &[ok.proto]).is_none() {
            return Err(SyncError::Peer(ErrorCode::UpgradeRequired));
        }
        self.greeted = true;
        // A new feed id means the hub's log was replaced: pull from 0.
        if dev.pull_feed_id.as_deref() != Some(ok.feed_id.as_str()) {
            self.store
                .set_cursors(&self.hub_device, dev.push_seq, Some(&ok.feed_id), 0)?;
        }
        self.hub_feed = Some(ok.feed_id);
        self.store.touch_device(&self.hub_device, None)?;
        for ctl in &ok.pending {
            self.deliver(ctl)?;
        }
        Ok(())
    }

    /// Applies a command the hub carried, tells it, and ends the session.
    fn deliver(&mut self, ctl: &Control) -> Result<()> {
        let outcome = control::apply_control(self.store.as_ref(), &self.hub_device, ctl)?;
        let done = match ctl {
            Control::Wipe { .. } => Message::WipeDone,
            Control::Unpair => Message::Ok,
        };
        // The hub may already be gone; the local effect is what counts.
        let _ = send_msg(&mut self.transport, 0, &done);
        self.report.closed_by = Some(outcome);
        self.closed = true;
        Ok(())
    }

    /// Persists the push position, never moving it backwards.
    fn persist_push(&self, seq: i64) -> Result<()> {
        let d = self.hub_device_row()?;
        if seq > d.push_seq {
            self.store
                .set_cursors(&self.hub_device, seq, d.pull_feed_id.as_deref(), d.pull_seq)?;
        }
        Ok(())
    }

    /// Persists the pull position (it may move back only on a feed change,
    /// which `hello` already handled).
    fn persist_pull(&self, seq: i64) -> Result<()> {
        let d = self.hub_device_row()?;
        self.store
            .set_cursors(&self.hub_device, d.push_seq, self.hub_feed.as_deref(), seq)?;
        Ok(())
    }

    /// Pushes tombstones; returns the log position they cover.
    fn push_tombs(&mut self) -> Result<i64> {
        let own = self.store.device_gid()?;
        let mut cur = self.hub_device_row()?.push_seq;
        loop {
            let mut batch = self.store.tombs_since(cur, wire::MAX_BATCH_RECORDS)?;
            // Tombstones applied from the hub (or relayed by it) are in the
            // log too: only the ones this device made go back.
            batch.tombs.retain(|t| t.origin == own);
            if !batch.tombs.is_empty() {
                let n = batch.tombs.len();
                let reply = match self.call(&Message::PushTombs(PushTombs {
                    tombs: batch.tombs,
                    upto_seq: batch.upto_seq,
                })) {
                    Ok(r) => r,
                    // The hub's user has to confirm this batch. Only it
                    // waits: the position stays below it (the rows still go),
                    // and a later session offers it again.
                    Err(SyncError::Peer(ErrorCode::NeedsConfirm)) => {
                        self.report.tombs_held = n;
                        return Ok(cur);
                    }
                    Err(e) => return Err(e),
                };
                match reply {
                    Message::Ack(Ack { upto_seq, .. }) if upto_seq == batch.upto_seq => {}
                    _ => return Err(unexpected("PushTombs")),
                }
                self.report.tombs_pushed += n;
            }
            if batch.more && batch.upto_seq <= cur {
                return Err(SyncError::Wire("feed did not advance".into()));
            }
            cur = cur.max(batch.upto_seq);
            if !batch.more {
                return Ok(cur);
            }
        }
    }

    fn push_rows(&mut self, tomb_pos: i64) -> Result<()> {
        let mut cur = self.hub_device_row()?.push_seq;
        loop {
            let batch = self.store.changes_since(cur, wire::MAX_BATCH_RECORDS)?;
            // `mark_clean` and applying the hub's rows re-log them: only rows
            // changed here since the hub's version go out.
            let mut rows: Vec<Record> = Vec::with_capacity(batch.changes.len());
            for c in batch.changes {
                if self
                    .store
                    .sync_dirty(c.record.kind().log_kind(), c.record.gid())?
                {
                    rows.push(c.record);
                }
            }
            if !rows.is_empty() {
                let keyed = attach_deks(self.store.as_ref(), &self.hub_device, &mut rows)?;
                let versions: Vec<(String, i64)> = rows
                    .iter()
                    .map(|r| (r.gid().to_string(), r.version().lamport))
                    .collect();
                let n = rows.len();
                let reply = self.call(&Message::PushRows(PushRows {
                    rows: rows.into_iter().map(WireRecord).collect(),
                    upto_seq: batch.upto_seq,
                }))?;
                let Message::Ack(ack) = reply else {
                    return Err(unexpected("PushRows"));
                };
                if ack.upto_seq != batch.upto_seq || ack.results.len() != n {
                    return Err(unexpected("PushRows"));
                }
                for ((gid, outcome), (sent_gid, lamport)) in ack.results.iter().zip(&versions) {
                    if gid != sent_gid {
                        return Err(unexpected("PushRows"));
                    }
                    if matches!(outcome, ApplyOutcome::Accepted | ApplyOutcome::Merged) {
                        self.store.mark_clean(gid, *lamport)?;
                        if keyed.contains(gid) {
                            self.store.mark_key_sent(&self.hub_device, gid)?;
                        }
                    }
                }
                self.report.rows_pushed += n;
            }
            if batch.more && batch.upto_seq <= cur {
                return Err(SyncError::Wire("feed did not advance".into()));
            }
            cur = cur.max(batch.upto_seq);
            // Everything up to the lower of the two phases is on the hub.
            self.persist_push(cur.min(tomb_pos))?;
            if !batch.more {
                return Ok(());
            }
        }
    }

    /// The `Leases` phase.
    fn leases(&mut self) -> Result<()> {
        let ex = Grantor.exchange(
            self.store.as_ref(),
            self.clock.as_ref(),
            &mut self.rpc,
            &mut self.transport,
            &self.hub_device,
        )?;
        self.report.lease_infos = ex.infos;
        self.report.take_back.extend(ex.take_back);
        Ok(())
    }

    /// The `Audio` phase.
    fn audio(&mut self) -> Result<()> {
        self.report.tracks_sent += audio::send_tracks(
            self.store.as_ref(),
            &mut self.transport,
            &mut self.rpc,
            &self.hub_device,
        )?;
        Ok(())
    }

    /// Pulls tombstones then rows. A mass delete held for the user's
    /// confirmation ends the pull early (`report.needs_confirm`).
    fn pull(&mut self) -> Result<()> {
        let feed = self
            .hub_feed
            .clone()
            .ok_or_else(|| SyncError::Wire("pull before hello".into()))?;
        let dev = self.hub_device_row()?;
        let start = if dev.pull_feed_id.as_deref() == Some(feed.as_str()) {
            dev.pull_seq
        } else {
            0
        };
        // Tombstones first: they outrank every row in the same direction.
        let mut cur = start;
        let tomb_pos = loop {
            let reply = self.call(&Message::PullTombs(wire::Pull {
                feed_id: feed.clone(),
                since_seq: cur,
                max: wire::MAX_BATCH_RECORDS as u32,
            }))?;
            let Message::Tombs(batch) = reply else {
                return Err(unexpected("PullTombs"));
            };
            if !batch.tombs.is_empty() {
                if !self
                    .guard
                    .allows(self.store.as_ref(), &self.hub_device, &batch.tombs)?
                {
                    // Held for the user: this batch waits, the rows do not
                    // (the position stays below it; a later session asks
                    // again).
                    self.report.needs_confirm = self.guard.held();
                    break cur;
                }
                self.store.apply_tombs(&self.hub_device, &batch.tombs)?;
                self.guard.settle(self.store.as_ref(), &self.hub_device)?;
                self.report.tombs_pulled += batch.tombs.len();
            }
            if batch.more && batch.upto_seq <= cur {
                return Err(SyncError::Wire("feed did not advance".into()));
            }
            cur = cur.max(batch.upto_seq);
            if !batch.more {
                break cur;
            }
        };
        let mut cur = start;
        let last_had_rows = loop {
            let reply = self.call(&Message::PullRows(wire::Pull {
                feed_id: feed.clone(),
                since_seq: cur,
                max: wire::MAX_BATCH_RECORDS as u32,
            }))?;
            // This request told the hub everything up to `cur` is applied,
            // and its reply shows the hub got that: persist it now.
            self.persist_pull(cur.min(tomb_pos))?;
            let Message::Rows(batch) = reply else {
                return Err(unexpected("PullRows"));
            };
            let had_rows = !batch.rows.is_empty();
            if had_rows {
                let rows: Vec<Record> = batch.rows.into_iter().map(|r| r.0).collect();
                self.store.apply_rows(&self.hub_device, &rows)?;
                self.store.retry_pending()?;
                self.report.rows_pulled += rows.len();
            }
            if batch.more && batch.upto_seq <= cur {
                return Err(SyncError::Wire("feed did not advance".into()));
            }
            cur = cur.max(batch.upto_seq);
            if !batch.more {
                break had_rows;
            }
        };
        if last_had_rows {
            // The last batch is acked by a request of its own, so the hub can
            // mark the keys it carried as delivered; only then is it ours.
            let reply = self.call(&Message::PullRows(wire::Pull {
                feed_id: feed,
                since_seq: cur,
                max: 0,
            }))?;
            if !matches!(reply, Message::Rows(_)) {
                return Err(unexpected("PullRows"));
            }
        }
        self.persist_pull(cur.min(tomb_pos))?;
        Ok(())
    }

    /// `Ping` while idle. `true` when the hub says it has new changes (call
    /// [`SpokeSession::run_once`] again).
    pub fn ping(&mut self) -> Result<bool> {
        self.transport.set_recv_timeout(Some(self.timing.silence))?;
        match self.call(&Message::Ping)? {
            Message::Pong { dirty } => Ok(dirty),
            Message::Control(ctl) => {
                self.deliver(&ctl)?;
                Ok(false)
            }
            _ => Err(unexpected("Ping")),
        }
    }

    /// Asks the hub to unpair (`Control::Unpair`) or wipe what it holds of
    /// this phone (`Control::Wipe`), then drops the pin here. This device keeps
    /// all its meetings.
    pub fn send_control(&mut self, ctl: Control) -> Result<ControlOutcome> {
        self.transport.set_recv_timeout(Some(self.timing.silence))?;
        if !self.greeted {
            self.hello()?;
            if self.closed {
                return self.report.closed_by.ok_or(SyncError::Closed);
            }
        }
        let reply = self.call(&Message::Control(ctl.clone()))?;
        let want_done = matches!(ctl, Control::Wipe { .. });
        match (reply, want_done) {
            (Message::WipeDone, true) | (Message::Ok, false) => {}
            _ => return Err(unexpected("Control")),
        }
        let outcome = control::apply_sent(self.store.as_ref(), &self.hub_device, &ctl)?;
        self.report.closed_by = Some(outcome);
        self.closed = true;
        Ok(outcome)
    }

    /// Sends `Bye` (the app is going to the background) and ends the session.
    pub fn bye(&mut self) -> Result<()> {
        self.closed = true;
        send_msg(&mut self.transport, 0, &Message::Bye)
    }
}

/// Puts the meeting key into meeting records that have not carried it to
/// `peer` yet; returns their gids (marked sent after the ack).
pub(crate) fn attach_deks(
    store: &dyn SyncStore,
    peer: &str,
    rows: &mut [Record],
) -> Result<Vec<String>> {
    let mut keyed = Vec::new();
    for rec in rows.iter_mut() {
        let Record::Meeting(m) = rec else { continue };
        if m.dek.is_some() {
            continue;
        }
        if let Some(dek) = store.meeting_dek_for_peer(peer, &m.gid)? {
            m.dek = Some(Bytes(dek.to_vec()));
            keyed.push(m.gid.clone());
        }
    }
    Ok(keyed)
}
