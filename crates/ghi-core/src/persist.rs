// SPDX-License-Identifier: Apache-2.0
//! The persist thread: the only writer of a live session's text to the store.
//!
//! Lines are batched and written at least every [`FLUSH_EVERY`] (crash loss of
//! text ≤5 s; the audio is in the bundles anyway and the final pass redoes
//! the transcript). Speakers are created, renamed, merged and split as the
//! engine reports them. Messages are applied in order, so a discard sees
//! every line produced before it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError};
use ghi_store::store::{MarkTag, NewSegment, NewSpeaker, Store, Word};

use crate::events::{ErrorKind, Event, EventTx};
use crate::live::{LineOut, PersistMsg, SpeakerOut};
use crate::speakers::{Change, SpeakerId};

/// Longest time a final line waits before it is written.
pub const FLUSH_EVERY: Duration = Duration::from_secs(5);
const FLUSH_LINES: usize = 20;

/// Stored speaker index: "Speaker N" is index N − 1; -1 while provisional
/// (and for Me, who is shown as "Me").
fn label_idx(s: &SpeakerOut) -> i64 {
    i64::from(s.number) - 1
}

pub struct Persist {
    store: Arc<Store>,
    meeting: String,
    events: EventTx,
    speakers: HashMap<SpeakerId, String>,
    pending: Vec<LineOut>,
    since: Option<Instant>,
}

impl Persist {
    pub fn new(store: Arc<Store>, meeting: String, events: EventTx) -> Persist {
        Persist {
            store,
            meeting,
            events,
            speakers: HashMap::new(),
            pending: Vec::new(),
            since: None,
        }
    }

    fn error(&self, message: String) {
        self.events.emit(Event::Error {
            meeting: Some(self.meeting.clone()),
            kind: ErrorKind::Storage,
            message,
        });
    }

    /// Runs until every sender is gone, flushing on the way out.
    pub fn run(mut self, rx: Receiver<PersistMsg>) {
        loop {
            let wait = match self.since {
                Some(t) => FLUSH_EVERY.saturating_sub(t.elapsed()),
                None => Duration::from_secs(3600),
            };
            match rx.recv_timeout(wait) {
                Ok(msg) => self.handle(msg),
                Err(RecvTimeoutError::Timeout) => self.flush(),
                Err(RecvTimeoutError::Disconnected) => {
                    self.flush();
                    return;
                }
            }
            if self.pending.len() >= FLUSH_LINES
                || self.since.is_some_and(|t| t.elapsed() >= FLUSH_EVERY)
            {
                self.flush();
            }
        }
    }

    fn handle(&mut self, msg: PersistMsg) {
        match msg {
            PersistMsg::Lines(lines) => {
                if self.since.is_none() {
                    self.since = Some(Instant::now());
                }
                self.pending.extend(lines);
            }
            PersistMsg::Speaker(change, s) => self.speaker(change, s),
            PersistMsg::Split { from, new, lines } => {
                self.flush();
                let Some(from_gid) = self.speakers.get(&from).cloned() else {
                    return;
                };
                match self
                    .store
                    .split_speaker(&from_gid, &lines, i64::from(new.color_slot))
                    .and_then(|gid| {
                        self.store.set_speaker_label_idx(&gid, label_idx(&new))?;
                        Ok(gid)
                    }) {
                    Ok(gid) => {
                        self.speakers.insert(new.id, gid);
                    }
                    Err(e) => {
                        // The new speaker's later lines stay with the
                        // original one rather than losing their speaker.
                        self.speakers.insert(new.id, from_gid);
                        self.error(format!("split: {e}"));
                    }
                }
            }
            PersistMsg::Mark { t_ms } => {
                if let Err(e) = self.store.add_mark(&self.meeting, t_ms, MarkTag::Star) {
                    self.error(format!("mark: {e}"));
                }
            }
            PersistMsg::Discard {
                t_cut_ms,
                now_ms,
                keep,
                reply,
            } => {
                self.flush();
                // Speakers stay: they may talk again; orphans go at stop.
                let r = self
                    .store
                    .discard_after(&self.meeting, t_cut_ms, now_ms, &keep, false)
                    .map(|rep| rep.id)
                    .map_err(|e| e.to_string());
                let _ = reply.send(r);
            }
            PersistMsg::Flush(done) => {
                self.flush();
                let _ = done.send(());
            }
            PersistMsg::SpeakerGids(reply) => {
                let mut v: Vec<_> = self.speakers.iter().map(|(k, g)| (*k, g.clone())).collect();
                v.sort();
                let _ = reply.send(v);
            }
        }
    }

    fn speaker(&mut self, change: Change, s: SpeakerOut) {
        let r = match change {
            Change::Arrived(id) => self
                .store
                .add_speaker(
                    &self.meeting,
                    NewSpeaker {
                        label_idx: label_idx(&s),
                        color_slot: i64::from(s.color_slot),
                        is_me: s.is_me,
                        ..Default::default()
                    },
                )
                .map(|gid| {
                    self.speakers.insert(id, gid);
                }),
            Change::Renamed(id) => match self.speakers.get(&id) {
                Some(gid) => self.store.rename_speaker(gid, s.name.as_deref()),
                None => Ok(()),
            },
            Change::Merged { from, into } => {
                // Lines still waiting must land on the right speaker first.
                self.flush();
                match (self.speakers.get(&from), self.speakers.get(&into)) {
                    (Some(f), Some(i)) => self.store.merge_speakers(f, i),
                    _ => Ok(()),
                }
            }
            Change::NotAPerson(id) => match self.speakers.get(&id) {
                Some(gid) => self.store.set_speaker_not_person(gid, true),
                None => Ok(()),
            },
            Change::Confirmed(id) => match self.speakers.get(&id) {
                Some(gid) => self.store.set_speaker_label_idx(gid, label_idx(&s)),
                None => Ok(()),
            },
            Change::Split { .. } => Ok(()),
        };
        if let Err(e) = r {
            self.error(format!("speaker: {e}"));
        }
    }

    fn flush(&mut self) {
        self.since = None;
        if self.pending.is_empty() {
            return;
        }
        let segs: Vec<NewSegment> = self
            .pending
            .drain(..)
            .map(|l| NewSegment {
                gid: Some(l.gid),
                speaker_gid: l.speaker.and_then(|id| self.speakers.get(&id).cloned()),
                t0_ms: l.t0_ms,
                t1_ms: l.t1_ms,
                text: l.text,
                lang: l.lang,
                confidence: l.confidence,
                words: l
                    .words
                    .into_iter()
                    .map(|(t0_ms, t1_ms, conf)| Word { t0_ms, t1_ms, conf })
                    .collect(),
                edited: false,
            })
            .collect();
        if let Err(e) = self.store.add_segments(&self.meeting, segs) {
            self.error(format!("saving the transcript: {e}"));
        }
    }

    /// Store gid of a session speaker (tests, snapshots).
    pub fn speaker_gid(&self, id: SpeakerId) -> Option<&String> {
        self.speakers.get(&id)
    }
}
