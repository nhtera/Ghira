// SPDX-License-Identifier: Apache-2.0
//! The hub (desktop) side of a session: answers one spoke's requests.

use std::collections::HashMap;
use std::sync::Arc;

use ghi_store::StoreError;
use ghi_store::sync::devices::{Device, DeviceState};
use ghi_store::sync::records::Record;

use super::spoke::attach_deks;
use super::{
    MassDeleteGuard, SessionReport, Timing, code_for, default_protos, error_msg, recv_msg, send_msg,
};
use crate::audio;
use crate::clock::Clock;
use crate::control::{self};
use crate::lease::{Holder, RequestOutcome};
use crate::store::SyncStore;
use crate::transport::Transport;
use crate::wire::{
    self, Ack, Decoded, ErrorCode, HelloOk, Message, Proto, Record as WireRecord, RowsBatch,
    TombsBatch,
};
use crate::{Result, SyncError};

/// One session from an already authenticated spoke.
pub struct HubSession<T: Transport> {
    store: Arc<dyn SyncStore>,
    clock: Arc<dyn Clock>,
    transport: T,
    timing: Timing,
    protos: Vec<Proto>,
    guard: MassDeleteGuard,
    report: SessionReport,
    /// The spoke, once `Hello` identified it.
    spoke: Option<Device>,
    /// A `Wipe` went out in `HelloOk`; only `WipeDone` may follow.
    wipe_delivered: bool,
    /// Rows sent with a key: `(upto_seq, meeting gids)` until the spoke's
    /// next request shows it applied them.
    pending_keys: Vec<(i64, Vec<String>)>,
    /// Tracks offered in this session (`track_gid` -> pages).
    offers: HashMap<String, u64>,
    last_activity_ns: u64,
}

/// How one request ended.
enum Flow {
    Continue,
    Close,
}

impl<T: Transport> HubSession<T> {
    pub fn new(store: Arc<dyn SyncStore>, clock: Arc<dyn Clock>, transport: T) -> Self {
        let now = clock.now_cont_ns();
        Self {
            store,
            clock,
            transport,
            timing: Timing::default(),
            protos: default_protos(),
            guard: MassDeleteGuard::default(),
            report: SessionReport::default(),
            spoke: None,
            wipe_delivered: false,
            pending_keys: Vec::new(),
            offers: HashMap::new(),
            last_activity_ns: now,
        }
    }

    /// Replaces the ping, silence and idle limits (tests).
    pub fn with_timing(mut self, timing: Timing) -> Self {
        self.timing = timing;
        self
    }

    /// Replaces the versions this hub speaks (tests of version skew).
    pub fn with_protos(mut self, protos: Vec<Proto>) -> Self {
        self.protos = protos;
        self
    }

    /// What the session did so far (also after an error).
    pub fn report(&self) -> &SessionReport {
        &self.report
    }

    /// Answers requests until the spoke says `Bye`, the connection ends, or an
    /// idle or silence limit is hit.
    pub fn serve(&mut self) -> Result<SessionReport> {
        self.transport.set_recv_timeout(Some(self.timing.silence))?;
        loop {
            let (id, decoded) = match recv_msg(&mut self.transport) {
                Ok(m) => m,
                // The spoke went away between requests: a normal end.
                Err(SyncError::Closed) => return Ok(self.report.clone()),
                Err(e @ SyncError::Wire(_)) => {
                    let _ = send_msg(
                        &mut self.transport,
                        0,
                        &error_msg(ErrorCode::BadRecord, None),
                    );
                    return Err(e);
                }
                Err(e) => return Err(e),
            };
            let msg = match decoded {
                Decoded::Known(m) => m,
                Decoded::Unknown(_) => {
                    send_msg(
                        &mut self.transport,
                        id,
                        &error_msg(ErrorCode::Unsupported, None),
                    )?;
                    continue;
                }
            };
            match self.handle(id, msg) {
                Ok(Flow::Continue) => {}
                Ok(Flow::Close) => return Ok(self.report.clone()),
                Err(SyncError::Closed) => return Ok(self.report.clone()),
                Err(e) => {
                    // Codes only, never content.
                    let _ = send_msg(&mut self.transport, id, &error_msg(code_for(&e), None));
                    return Err(e);
                }
            }
        }
    }

    fn reply(&mut self, id: u32, msg: &Message) -> Result<Flow> {
        send_msg(&mut self.transport, id, msg)?;
        Ok(Flow::Continue)
    }

    fn refuse(&mut self, id: u32, code: ErrorCode, detail: Option<String>) -> Result<Flow> {
        send_msg(&mut self.transport, id, &error_msg(code, detail))?;
        Ok(Flow::Close)
    }

    fn spoke(&self) -> Result<&Device> {
        self.spoke
            .as_ref()
            .ok_or_else(|| SyncError::Wire("request before hello".into()))
    }

    fn handle(&mut self, id: u32, msg: Message) -> Result<Flow> {
        let now = self.clock.now_cont_ns();
        let is_ping = matches!(msg, Message::Ping | Message::Pong { .. });
        if is_ping {
            let idle = u64::try_from(self.timing.idle.as_nanos()).unwrap_or(u64::MAX);
            if now.saturating_sub(self.last_activity_ns) > idle {
                return Ok(Flow::Close);
            }
        } else {
            self.last_activity_ns = now;
        }
        if !matches!(msg, Message::Hello(_)) && self.spoke.is_none() {
            return Err(SyncError::Wire("request before hello".into()));
        }
        if self.wipe_delivered {
            // A session that delivers a wipe does nothing else.
            return match msg {
                Message::WipeDone | Message::Ok => self.wipe_done(),
                Message::Bye => Ok(Flow::Close),
                Message::Ping => self.reply(id, &Message::Pong { dirty: false }),
                _ => self.refuse(id, ErrorCode::Busy, Some("wipe_pending".into())),
            };
        }
        match msg {
            Message::Hello(h) => self.on_hello(id, h),
            Message::Control(ctl) => {
                let from = self.spoke()?.gid.clone();
                let outcome = control::apply_control(self.store.as_ref(), &from, &ctl)?;
                self.report.closed_by = Some(outcome);
                let done = match ctl {
                    wire::Control::Wipe { .. } => Message::WipeDone,
                    wire::Control::Unpair => Message::Ok,
                };
                send_msg(&mut self.transport, id, &done)?;
                Ok(Flow::Close)
            }
            Message::PushTombs(b) => self.on_push_tombs(id, b),
            Message::PushRows(b) => self.on_push_rows(id, b),
            Message::PullTombs(p) => self.on_pull_tombs(id, p),
            Message::PullRows(p) => self.on_pull_rows(id, p),
            Message::TrackOffer(o) => self.on_track_offer(id, o),
            Message::TrackPages(p) => self.on_track_pages(id, p),
            Message::ProcessRequest(r) => self.on_process_request(id, r),
            Message::LeaseRevoke(q) => {
                let reply = Holder.on_revoke(self.store.as_ref(), &q)?;
                self.reply(id, &reply)
            }
            Message::LeaseStatus(qs) => {
                let infos = Holder.on_status(self.store.as_ref(), self.clock.as_ref(), &qs)?;
                self.reply(id, &Message::LeaseStatusReply(infos))
            }
            Message::Ping => self.on_ping(id),
            Message::Bye => Ok(Flow::Close),
            _ => self.refuse_unsupported(id),
        }
    }

    fn refuse_unsupported(&mut self, id: u32) -> Result<Flow> {
        send_msg(
            &mut self.transport,
            id,
            &error_msg(ErrorCode::Unsupported, None),
        )?;
        Ok(Flow::Continue)
    }

    fn on_hello(&mut self, id: u32, h: wire::Hello) -> Result<Flow> {
        if self.spoke.is_some() {
            return Err(SyncError::Wire("second hello".into()));
        }
        // The handshake proved a key; the pin says who owns it.
        let dev = self
            .store
            .device_by_key(&self.transport.peer_static())?
            .ok_or_else(|| SyncError::Wire("unknown device".into()))?;
        if dev.gid != h.device_gid {
            return Err(SyncError::Wire("device identity mismatch".into()));
        }
        let Some(proto) = wire::negotiate(&self.protos, &h.proto) else {
            let side = wire::upgrade_side(&self.protos, &h.proto, true);
            return self.refuse(id, ErrorCode::UpgradeRequired, Some(side.to_string()));
        };
        // A new feed id on the spoke means its log was replaced.
        if dev.pull_feed_id.as_deref() != Some(h.feed_id.as_str()) {
            self.store
                .set_cursors(&dev.gid, dev.push_seq, Some(&h.feed_id), 0)?;
        }
        self.store.touch_device(&dev.gid, None)?;
        let pending = control::pending_for(self.store.as_ref(), &dev.gid)?;
        self.wipe_delivered = !pending.is_empty();
        let ok = HelloOk {
            proto,
            device_gid: self.store.device_gid()?,
            feed_id: self.store.feed_id()?,
            pending,
        };
        self.spoke = Some(dev);
        self.reply(id, &Message::HelloOk(ok))
    }

    /// The spoke confirmed it shredded what it held: the pin can go.
    fn wipe_done(&mut self) -> Result<Flow> {
        let gid = self.spoke()?.gid.clone();
        self.store.unpin_device(&gid)?;
        self.report.closed_by = Some(control::ControlOutcome::Wiped);
        Ok(Flow::Close)
    }

    fn on_push_tombs(&mut self, id: u32, b: wire::PushTombs) -> Result<Flow> {
        let from = self.spoke()?.gid.clone();
        if !self.guard.allows(self.store.as_ref(), &from, &b.tombs)? {
            self.report.needs_confirm = self.guard.held();
            return self.refuse(
                id,
                ErrorCode::NeedsConfirm,
                Some(self.guard.held().to_string()),
            );
        }
        let res = self.store.apply_tombs(&from, &b.tombs)?;
        if res.needs_confirm > 0 {
            // The store holds its own guard too: same outcome.
            self.report.needs_confirm = res.needs_confirm;
            return self.refuse(
                id,
                ErrorCode::NeedsConfirm,
                Some(res.needs_confirm.to_string()),
            );
        }
        self.guard.settle(self.store.as_ref(), &from)?;
        self.report.tombs_pushed += b.tombs.len();
        self.reply(
            id,
            &Message::Ack(Ack {
                upto_seq: b.upto_seq,
                rejected: res.rejected,
                results: Vec::new(),
            }),
        )
    }

    fn on_push_rows(&mut self, id: u32, b: wire::PushRows) -> Result<Flow> {
        let from = self.spoke()?.gid.clone();
        let rows: Vec<Record> = b.rows.into_iter().map(|r| r.0).collect();
        let res = self.store.apply_rows(&from, &rows)?;
        self.store.retry_pending()?;
        self.report.rows_pushed += rows.len();
        self.reply(
            id,
            &Message::Ack(Ack {
                upto_seq: b.upto_seq,
                rejected: Vec::new(),
                results: res.results,
            }),
        )
    }

    /// The spoke's `since_seq` acks what it applied before: the keys carried
    /// by those batches arrived.
    fn ack_keys(&mut self, since: i64) -> Result<()> {
        let gid = self.spoke()?.gid.clone();
        let (done, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending_keys)
            .into_iter()
            .partition(|(upto, _)| *upto <= since);
        self.pending_keys = keep;
        for (_, meetings) in done {
            for m in meetings {
                self.store.mark_key_sent(&gid, &m)?;
            }
        }
        Ok(())
    }

    fn check_feed(&mut self, id: u32, p: &wire::Pull) -> Result<Option<Flow>> {
        if p.feed_id != self.store.feed_id()? {
            return self
                .refuse(id, ErrorCode::Internal, Some("feed_changed".into()))
                .map(Some);
        }
        Ok(None)
    }

    fn on_pull_tombs(&mut self, id: u32, p: wire::Pull) -> Result<Flow> {
        if let Some(flow) = self.check_feed(id, &p)? {
            return Ok(flow);
        }
        let max = (p.max as usize).min(wire::MAX_BATCH_RECORDS);
        let batch = if max == 0 {
            Default::default()
        } else {
            self.store.tombs_since(p.since_seq, max)?
        };
        let upto = if max == 0 {
            p.since_seq
        } else {
            batch.upto_seq
        };
        self.report.tombs_pulled += batch.tombs.len();
        self.reply(
            id,
            &Message::Tombs(TombsBatch {
                tombs: batch.tombs,
                upto_seq: upto,
                more: batch.more,
            }),
        )
    }

    fn on_pull_rows(&mut self, id: u32, p: wire::Pull) -> Result<Flow> {
        if let Some(flow) = self.check_feed(id, &p)? {
            return Ok(flow);
        }
        let gid = self.spoke()?.gid.clone();
        self.ack_keys(p.since_seq)?;
        // Record what the spoke has: its cursor into this feed.
        let dev = self
            .store
            .device(&gid)?
            .ok_or(SyncError::Store(StoreError::NotFound {
                kind: "device",
                gid: gid.clone(),
            }))?;
        if dev.push_seq != p.since_seq {
            self.store
                .set_cursors(&gid, p.since_seq, dev.pull_feed_id.as_deref(), dev.pull_seq)?;
        }
        let max = (p.max as usize).min(wire::MAX_BATCH_RECORDS);
        if max == 0 {
            return self.reply(
                id,
                &Message::Rows(RowsBatch {
                    rows: Vec::new(),
                    upto_seq: p.since_seq,
                    more: false,
                }),
            );
        }
        let batch = self.store.changes_since(p.since_seq, max)?;
        let mut rows: Vec<Record> = batch.changes.into_iter().map(|c| c.record).collect();
        let keyed = attach_deks(self.store.as_ref(), &gid, &mut rows)?;
        if !keyed.is_empty() {
            self.pending_keys.push((batch.upto_seq, keyed));
        }
        self.report.rows_pulled += rows.len();
        self.reply(
            id,
            &Message::Rows(RowsBatch {
                rows: rows.into_iter().map(WireRecord).collect(),
                upto_seq: batch.upto_seq,
                more: batch.more,
            }),
        )
    }

    fn on_track_offer(&mut self, id: u32, offer: wire::TrackOffer) -> Result<Flow> {
        let from = self.spoke()?.gid.clone();
        let result = audio::on_offer(self.store.as_ref(), &from, &offer)?;
        if matches!(result, audio::OfferResult::Complete) {
            // Nothing to receive; the sender marks it sent.
        } else if matches!(result, audio::OfferResult::Have(_)) {
            self.offers.insert(offer.track_gid.clone(), offer.pages);
        }
        self.reply(id, &audio::offer_reply(result, &offer))
    }

    fn on_track_pages(&mut self, id: u32, pages: wire::TrackPages) -> Result<Flow> {
        let from = self.spoke()?.gid.clone();
        // Pages only follow an offer of this session.
        let Some(&total) = self.offers.get(&pages.track_gid) else {
            return Err(SyncError::Wire("pages without an offer".into()));
        };
        let have = audio::on_pages(self.store.as_ref(), &from, &pages)?;
        if have >= total {
            self.offers.remove(&pages.track_gid);
            self.report.tracks_received.push(pages.track_gid.clone());
        }
        self.reply(id, &Message::TrackAck(wire::TrackAck { have }))
    }

    fn on_process_request(&mut self, id: u32, req: wire::ProcessRequest) -> Result<Flow> {
        let from = self.spoke()?.gid.clone();
        let reply =
            match Holder.on_request(self.store.as_ref(), self.clock.as_ref(), &from, &req)? {
                RequestOutcome::Accepted {
                    job_uuid,
                    epoch,
                    newly_opened,
                } => {
                    if newly_opened {
                        self.report.new_leases.push(job_uuid.clone());
                    }
                    Message::ProcessAccept { job_uuid, epoch }
                }
                RequestOutcome::Refused(reason) => Message::Refuse(reason),
            };
        self.reply(id, &reply)
    }

    fn on_ping(&mut self, id: u32) -> Result<Flow> {
        let gid = self.spoke()?.gid.clone();
        let dev = self.store.device(&gid)?;
        if dev
            .as_ref()
            .is_some_and(|d| d.state == DeviceState::WipePending)
        {
            // Marked while the spoke is connected: deliver it now.
            self.wipe_delivered = true;
            return self.reply(
                id,
                &Message::Control(wire::Control::Wipe {
                    reason: "wipe".into(),
                }),
            );
        }
        let since = dev.map_or(0, |d| d.push_seq);
        let dirty = !self.store.changes_since(since, 1)?.changes.is_empty()
            || !self.store.tombs_since(since, 1)?.tombs.is_empty();
        self.reply(id, &Message::Pong { dirty })
    }
}
