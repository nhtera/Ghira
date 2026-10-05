// SPDX-License-Identifier: Apache-2.0
//! The change feed (slice 15-C1): `sync_log` joined to its rows.
//!
//! `sync_log.seq` is the cursor for both rows and tombstones (Lamport values
//! can't be: relayed rows keep old ones). Rows of a meeting that is still
//! `recording` are held back; [`Store::relog_meeting`] re-logs a meeting's rows
//! when it finishes. A meeting's own record comes before its children in a
//! batch. `feed_id` identifies this database's log: a peer that sees a new one
//! starts over from sequence 0.
//!
//! Me is a device-local person row (empty name): it is not in the feed, and a
//! speaker linked to it travels with `is_me` and no `person_gid`.
//!
//! Cursors: a session keeps the lower of the two `upto_seq` values it got
//! (rows, tombstones) as its push cursor. A tombstone batch that is drained
//! reports the highest sequence number at read time, so a tombstone written
//! after it is never skipped by a later, higher row cursor.

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::records::{
    ActionItemRec, Bytes, ConflictCopyRec, FolderRec, MarkRec, MeetingRec, MeetingTagRec, NoteRec,
    PersonRec, Record, SegmentRec, SettingRec, SpeakerRec, SyncTombstone, TagRec, TombCause,
    TrackRec, Version, WordRec,
};
use crate::migrate::{SYNC_TABLES, TOMBSTONE_LOG_KIND};
use crate::store::Store;
use crate::{Result, StoreError};

/// Most records in one batch.
pub const MAX_BATCH_RECORDS: usize = 256;
/// Most (estimated) bytes in one batch; one larger record still goes alone.
pub const MAX_BATCH_BYTES: usize = 1024 * 1024;
/// `sync_log` entries read per query while scanning.
const SCAN_PAGE: usize = 512;
/// Fixed cost assumed per record on top of its blobs.
const RECORD_OVERHEAD: usize = 256;

/// One changed row and its position in the log.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedChange {
    pub seq: i64,
    pub record: Record,
}

/// Up to `max` changes after a sequence number.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChangeBatch {
    pub changes: Vec<FeedChange>,
    /// The highest sequence number this batch covers (acked as `upto_seq`);
    /// it may exceed the last change's when entries were skipped.
    pub upto_seq: i64,
    pub more: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TombBatch {
    pub tombs: Vec<SyncTombstone>,
    pub upto_seq: i64,
    pub more: bool,
}

impl Store {
    /// This database's feed id (`settings['sync.feed_id']`).
    pub fn feed_id(&self) -> Result<String> {
        self.setting_string("sync.feed_id")
    }

    /// Draws a new feed id (after a restore, an import or a wipe). Every
    /// peer's cursors are reset too: they counted positions in the old log.
    pub fn regen_feed_id(&self) -> Result<String> {
        self.reset_feed_id()?;
        self.conn().execute(
            "UPDATE devices SET push_seq = 0, pull_seq = 0, pull_feed_id = NULL",
            [],
        )?;
        self.feed_id()
    }

    /// This device's own gid (`settings['sync.device_gid']`).
    pub fn sync_device_gid(&self) -> Result<String> {
        self.setting_string("sync.device_gid")
    }

    /// A restored archive is a new device (doc 07 §3.1): a new gid and a new
    /// feed, and the cursors start over.
    pub(crate) fn reset_sync_identity(&self) -> Result<()> {
        self.set_setting(
            "sync.device_gid",
            &serde_json::Value::String(crate::new_gid()),
        )?;
        self.regen_feed_id().map(|_| ())
    }

    fn setting_string(&self, key: &'static str) -> Result<String> {
        match self.get_setting(key)? {
            Some(serde_json::Value::String(s)) => Ok(s),
            _ => Err(StoreError::NotFound {
                kind: "setting",
                gid: key.to_string(),
            }),
        }
    }

    /// Row changes after `seq`, at most `max` records (and 256) and 1 MiB,
    /// parents first, without meetings that are recording.
    pub fn changes_since(&self, seq: i64, max: usize) -> Result<ChangeBatch> {
        let max = max.clamp(1, MAX_BATCH_RECORDS);
        let own = self.sync_device_gid()?;
        let mut conn = self.conn();
        // One snapshot for the whole scan.
        let tx = conn.transaction()?;
        let mut changes = Vec::new();
        let (mut bytes, mut upto, mut more) = (0usize, seq, false);
        let mut after = seq;
        'scan: loop {
            let page: Vec<(i64, String, String)> = {
                let mut stmt = tx.prepare_cached(
                    "SELECT seq, kind, gid FROM sync_log WHERE seq > ?1 ORDER BY seq LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![after, SCAN_PAGE as i64], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            let full = page.len() == SCAN_PAGE;
            for (s, kind, gid) in page {
                after = s;
                // Tombstones travel apart; voice profiles do not sync in v1.
                let record = if kind == TOMBSTONE_LOG_KIND || kind == "voice_profile" {
                    None
                } else {
                    load_record(&tx, &own, &kind, &gid)?
                };
                let Some(record) = record else {
                    upto = s;
                    continue;
                };
                let size = approx_size(&record);
                if !changes.is_empty() && (changes.len() >= max || bytes + size > MAX_BATCH_BYTES) {
                    more = true;
                    break 'scan;
                }
                bytes += size;
                changes.push(FeedChange { seq: s, record });
                upto = s;
            }
            if !full {
                break;
            }
        }
        tx.commit()?;
        // Parents before children (stable: log order otherwise).
        changes.sort_by_key(|c| kind_rank(c.record.kind().log_kind()));
        Ok(ChangeBatch {
            changes,
            upto_seq: upto,
            more,
        })
    }

    /// Tombstones after `seq`, at most `max`, by `sync_log.seq`.
    pub fn tombs_since(&self, seq: i64, max: usize) -> Result<TombBatch> {
        let max = max.max(1);
        let own = self.sync_device_gid()?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut rows: Vec<(i64, SyncTombstone)> = {
            let mut stmt = tx.prepare(
                "SELECT s.seq, t.gid, t.kind, t.lamport,
                        COALESCE((SELECT d.gid FROM devices d WHERE d.id = t.origin), ?1),
                        t.cause
                 FROM sync_log s JOIN tombstones t ON t.gid = s.gid
                 WHERE s.kind = ?2 AND s.seq > ?3
                 ORDER BY s.seq LIMIT ?4",
            )?;
            let rows =
                stmt.query_map(params![own, TOMBSTONE_LOG_KIND, seq, max as i64 + 1], |r| {
                    let cause: Option<String> = r.get(5)?;
                    Ok((
                        r.get::<_, i64>(0)?,
                        SyncTombstone {
                            gid: r.get(1)?,
                            kind: r.get(2)?,
                            lamport: r.get(3)?,
                            origin: r.get(4)?,
                            cause: cause.as_deref().and_then(TombCause::parse),
                        },
                    ))
                })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let more = rows.len() > max;
        rows.truncate(max);
        let upto = if more {
            rows.last().map_or(seq, |(s, _)| *s)
        } else {
            let top: Option<i64> =
                tx.query_row("SELECT max(seq) FROM sync_log", [], |r| r.get(0))?;
            top.unwrap_or(0).max(seq)
        };
        tx.commit()?;
        Ok(TombBatch {
            tombs: rows.into_iter().map(|(_, t)| t).collect(),
            upto_seq: upto,
            more,
        })
    }

    /// Logs every row of a meeting again (when it stops recording).
    pub fn relog_meeting(&self, meeting_gid: &str) -> Result<()> {
        self.relog_meeting_rows(meeting_gid)
    }
}

/// Position of a kind in the feed order (parents first).
fn kind_rank(kind: &str) -> usize {
    SYNC_TABLES
        .iter()
        .position(|(_, k)| *k == kind)
        .unwrap_or(usize::MAX)
}

fn blob_len(b: &Option<Bytes>) -> usize {
    b.as_ref().map_or(0, |b| b.0.len())
}

fn text_len(s: &Option<String>) -> usize {
    s.as_ref().map_or(0, String::len)
}

/// A cheap estimate of a record's encoded size: its blobs and long strings
/// plus a fixed overhead.
fn approx_size(r: &Record) -> usize {
    RECORD_OVERHEAD
        + match r {
            Record::Meeting(m) => {
                blob_len(&m.title_ct)
                    + blob_len(&m.calendar_ct)
                    + blob_len(&m.track_speakers_ct)
                    + blob_len(&m.dek)
            }
            Record::Speaker(s) => blob_len(&s.display_name_ct),
            Record::Segment(s) => blob_len(&s.text_ct) + 24 * s.words.as_ref().map_or(0, Vec::len),
            Record::Note(n) => blob_len(&n.body_ct) + text_len(&n.anchors_json),
            Record::ActionItem(a) => {
                blob_len(&a.text_ct) + blob_len(&a.due_text_ct) + text_len(&a.anchors_json)
            }
            Record::ConflictCopy(c) => blob_len(&c.value_ct),
            Record::Setting(s) => s.value_json.as_ref().map_or(0, String::len),
            _ => 0,
        }
}

/// `origin` column of alias `x` as a device gid (NULL = this device, `?1`).
fn org(col: &str) -> String {
    format!("COALESCE((SELECT d.gid FROM devices d WHERE d.id = x.{col}), ?1)")
}

/// The version columns every syncable row starts its SELECT with.
fn head_cols() -> String {
    format!(
        "x.lamport, {}, x.base_lamport,
         CASE WHEN x.base_lamport IS NULL THEN NULL ELSE {} END",
        org("origin"),
        org("base_origin")
    )
}

/// Reads the four version columns at positions 0..4.
fn head(r: &Row) -> rusqlite::Result<(Version, Option<Version>)> {
    let version = Version {
        lamport: r.get(0)?,
        origin: r.get(1)?,
    };
    let base = match r.get::<_, Option<i64>>(2)? {
        Some(lamport) => Some(Version {
            lamport,
            origin: r.get(3)?,
        }),
        None => None,
    };
    Ok((version, base))
}

fn bytes(r: &Row, i: usize) -> rusqlite::Result<Option<Bytes>> {
    Ok(r.get::<_, Option<Vec<u8>>>(i)?.map(Bytes))
}

/// The row of a `sync_log` entry as a record, or `None` when it is gone,
/// shredded or belongs to a meeting that is still recording.
fn load_record(conn: &Connection, own: &str, kind: &str, gid: &str) -> Result<Option<Record>> {
    let h = head_cols();
    // Children are held back with their recording meeting.
    let child = "JOIN meetings m ON m.id = x.meeting_id AND m.status <> 'recording'";
    let rec = match kind {
        "meeting" => conn
            .query_row(
                &format!(
                    "SELECT {h}, x.title_ct, x.started_at, x.duration_ms, x.source, x.mode, x.lang,
                            x.template, x.status, x.privacy_state, x.cloud_locked, x.sensitive,
                            x.consent_confirmed, x.cloud_used, x.transcript_version,
                            x.transcript_epoch, x.ai_epoch, x.audio_retained_until,
                            CASE WHEN x.audio_origin IS NOT NULL
                                 THEN (SELECT d.gid FROM devices d WHERE d.id = x.audio_origin)
                                 WHEN x.source <> 'file' AND x.origin IS NULL THEN ?1
                            END,
                            x.source_hash, x.source_app,
                            (SELECT f.gid FROM folders f WHERE f.id = x.folder_id),
                            x.calendar_ct, x.track_speakers_ct, x.created_at
                     FROM meetings x
                     WHERE x.gid = ?2 AND x.status <> 'recording'
                       AND x.dek_wrapped <> zeroblob(length(x.dek_wrapped))"
                ),
                params![own, gid],
                |r| {
                    let (version, base) = head(r)?;
                    Ok(Record::Meeting(MeetingRec {
                        gid: gid.to_string(),
                        version,
                        base,
                        title_ct: bytes(r, 4)?,
                        started_at: r.get(5)?,
                        duration_ms: r.get(6)?,
                        source: r.get(7)?,
                        mode: r.get(8)?,
                        lang: r.get(9)?,
                        template: r.get(10)?,
                        status: r.get(11)?,
                        privacy_state: r.get(12)?,
                        cloud_locked: r.get(13)?,
                        sensitive: r.get(14)?,
                        consent_confirmed: r.get(15)?,
                        cloud_used: r.get(16)?,
                        transcript_version: r.get(17)?,
                        transcript_epoch: r.get(18)?,
                        ai_epoch: r.get(19)?,
                        audio_retained_until: r.get(20)?,
                        audio_origin: r.get(21)?,
                        created_at: r.get(27)?,
                        source_hash: r.get(22)?,
                        source_app: r.get(23)?,
                        folder_gid: r.get(24)?,
                        calendar_ct: bytes(r, 25)?,
                        track_speakers_ct: bytes(r, 26)?,
                        dek: None,
                    }))
                },
            )
            .optional()?,
        "track" => conn
            .query_row(
                &format!(
                    "SELECT {h}, m.gid, x.kind, x.page_count, x.cut_pages
                     FROM tracks x {child} WHERE x.gid = ?2"
                ),
                params![own, gid],
                |r| {
                    let (version, base) = head(r)?;
                    Ok(Record::Track(TrackRec {
                        gid: gid.to_string(),
                        version,
                        base,
                        meeting_gid: r.get(4)?,
                        kind: r.get(5)?,
                        page_count: r.get(6)?,
                        cut_pages: r.get(7)?,
                    }))
                },
            )
            .optional()?,
        "person" => conn
            .query_row(
                &format!(
                    "SELECT {h}, x.name, x.color_slot, x.is_me, x.created_at
                     FROM persons x WHERE x.gid = ?2 AND x.is_me = 0"
                ),
                params![own, gid],
                |r| {
                    let (version, base) = head(r)?;
                    Ok(Record::Person(PersonRec {
                        gid: gid.to_string(),
                        version,
                        base,
                        name: r.get(4)?,
                        color_slot: r.get(5)?,
                        is_me: r.get(6)?,
                        created_at: r.get(7)?,
                    }))
                },
            )
            .optional()?,
        "speaker" => conn
            .query_row(
                &format!(
                    "SELECT {h}, m.gid, x.label_idx, x.display_name_ct,
                            (SELECT p.gid FROM persons p WHERE p.id = x.person_id AND p.is_me = 0),
                            x.color_slot, x.is_me, x.not_person,
                            (SELECT s.gid FROM speakers s WHERE s.id = x.merged_into)
                     FROM speakers x {child} WHERE x.gid = ?2"
                ),
                params![own, gid],
                |r| {
                    let (version, base) = head(r)?;
                    Ok(Record::Speaker(SpeakerRec {
                        gid: gid.to_string(),
                        version,
                        base,
                        meeting_gid: r.get(4)?,
                        label_idx: r.get(5)?,
                        display_name_ct: bytes(r, 6)?,
                        person_gid: r.get(7)?,
                        color_slot: r.get(8)?,
                        is_me: r.get(9)?,
                        not_person: r.get(10)?,
                        merged_into: r.get(11)?,
                    }))
                },
            )
            .optional()?,
        "segment" => {
            let found: Option<(i64, Record)> = conn
                .query_row(
                    &format!(
                        "SELECT {h}, m.gid, x.version, x.epoch,
                                (SELECT s.gid FROM speakers s WHERE s.id = x.speaker_id),
                                x.t0_ms, x.t1_ms, x.text_ct, x.lang, x.confidence, x.edited,
                                x.overlap, x.id
                         FROM segments x {child} WHERE x.gid = ?2"
                    ),
                    params![own, gid],
                    |r| {
                        let (version, base) = head(r)?;
                        Ok((
                            r.get::<_, i64>(15)?,
                            Record::Segment(SegmentRec {
                                gid: gid.to_string(),
                                version,
                                base,
                                meeting_gid: r.get(4)?,
                                transcript_version: r.get(5)?,
                                epoch: r.get(6)?,
                                speaker_gid: r.get(7)?,
                                t0_ms: r.get(8)?,
                                t1_ms: r.get(9)?,
                                text_ct: bytes(r, 10)?,
                                lang: r.get(11)?,
                                confidence: r.get(12)?,
                                edited: r.get(13)?,
                                overlap: r.get(14)?,
                                words: None,
                            }),
                        ))
                    },
                )
                .optional()?;
            match found {
                Some((id, Record::Segment(mut s))) => {
                    let mut stmt = conn.prepare_cached(
                        "SELECT t0_ms, t1_ms, conf FROM words WHERE segment_id = ?1 ORDER BY idx",
                    )?;
                    let words = stmt
                        .query_map([id], |r| {
                            Ok(WordRec {
                                t0_ms: r.get(0)?,
                                t1_ms: r.get(1)?,
                                conf: r.get(2)?,
                            })
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    s.words = Some(words);
                    Some(Record::Segment(s))
                }
                _ => None,
            }
        }
        "note" => conn
            .query_row(
                &format!(
                    "SELECT {h}, m.gid, x.kind, x.provenance, x.body_ct, x.anchors_json,
                            x.pinned, x.epoch, x.ord
                     FROM notes_blocks x {child} WHERE x.gid = ?2"
                ),
                params![own, gid],
                |r| {
                    let (version, base) = head(r)?;
                    Ok(Record::Note(NoteRec {
                        gid: gid.to_string(),
                        version,
                        base,
                        meeting_gid: r.get(4)?,
                        kind: r.get(5)?,
                        provenance: r.get(6)?,
                        body_ct: bytes(r, 7)?,
                        anchors_json: r.get(8)?,
                        pinned: r.get(9)?,
                        epoch: r.get(10)?,
                        ord: r.get(11)?,
                    }))
                },
            )
            .optional()?,
        "action_item" => conn
            .query_row(
                &format!(
                    "SELECT {h}, m.gid, x.text_ct, x.due_text_ct,
                            (SELECT s.gid FROM speakers s WHERE s.id = x.owner_speaker_id),
                            x.due, x.done, x.anchors_json, x.provenance, x.epoch, x.ord
                     FROM action_items x {child} WHERE x.gid = ?2"
                ),
                params![own, gid],
                |r| {
                    let (version, base) = head(r)?;
                    Ok(Record::ActionItem(ActionItemRec {
                        gid: gid.to_string(),
                        version,
                        base,
                        meeting_gid: r.get(4)?,
                        text_ct: bytes(r, 5)?,
                        due_text_ct: bytes(r, 6)?,
                        owner_speaker_gid: r.get(7)?,
                        due: r.get(8)?,
                        done: r.get(9)?,
                        anchors_json: r.get(10)?,
                        provenance: r.get(11)?,
                        epoch: r.get(12)?,
                        ord: r.get(13)?,
                    }))
                },
            )
            .optional()?,
        "mark" => conn
            .query_row(
                &format!("SELECT {h}, m.gid, x.t_ms, x.tag FROM marks x {child} WHERE x.gid = ?2"),
                params![own, gid],
                |r| {
                    let (version, base) = head(r)?;
                    Ok(Record::Mark(MarkRec {
                        gid: gid.to_string(),
                        version,
                        base,
                        meeting_gid: r.get(4)?,
                        t_ms: r.get(5)?,
                        tag: r.get(6)?,
                    }))
                },
            )
            .optional()?,
        "folder" => conn
            .query_row(
                &format!("SELECT {h}, x.name, x.created_at FROM folders x WHERE x.gid = ?2"),
                params![own, gid],
                |r| {
                    let (version, base) = head(r)?;
                    Ok(Record::Folder(FolderRec {
                        gid: gid.to_string(),
                        version,
                        base,
                        name: r.get(4)?,
                        created_at: r.get(5)?,
                    }))
                },
            )
            .optional()?,
        "tag" => conn
            .query_row(
                &format!("SELECT {h}, x.name, x.created_at FROM tags x WHERE x.gid = ?2"),
                params![own, gid],
                |r| {
                    let (version, base) = head(r)?;
                    Ok(Record::Tag(TagRec {
                        gid: gid.to_string(),
                        version,
                        base,
                        name: r.get(4)?,
                        created_at: r.get(5)?,
                    }))
                },
            )
            .optional()?,
        "meeting_tag" => conn
            .query_row(
                &format!(
                    "SELECT {h}, m.gid, (SELECT t.gid FROM tags t WHERE t.id = x.tag_id)
                     FROM meeting_tags x {child} WHERE x.gid = ?2"
                ),
                params![own, gid],
                |r| {
                    let (version, base) = head(r)?;
                    Ok(Record::MeetingTag(MeetingTagRec {
                        gid: gid.to_string(),
                        version,
                        base,
                        meeting_gid: r.get(4)?,
                        tag_gid: r.get(5)?,
                    }))
                },
            )
            .optional()?,
        "conflict_copy" => conn
            .query_row(
                &format!(
                    "SELECT {h}, m.gid, x.target_kind, x.target_gid, x.field, x.value_ct,
                            x.created_at
                     FROM conflict_copies x {child} WHERE x.gid = ?2"
                ),
                params![own, gid],
                |r| {
                    let (version, base) = head(r)?;
                    Ok(Record::ConflictCopy(ConflictCopyRec {
                        gid: gid.to_string(),
                        version,
                        base,
                        meeting_gid: r.get(4)?,
                        target_kind: r.get(5)?,
                        target_gid: r.get(6)?,
                        field: r.get(7)?,
                        value_ct: bytes(r, 8)?,
                        created_at: r.get(9)?,
                    }))
                },
            )
            .optional()?,
        "setting" => conn
            .query_row(
                &format!("SELECT {h}, x.key, x.value_json FROM synced_settings x WHERE x.gid = ?2"),
                params![own, gid],
                |r| {
                    let (version, base) = head(r)?;
                    Ok(Record::Setting(SettingRec {
                        gid: gid.to_string(),
                        version,
                        base,
                        key: r.get(4)?,
                        value_json: r.get(5)?,
                    }))
                },
            )
            .optional()?,
        // An unknown kind (a newer build's log) is skipped, not an error.
        _ => None,
    };
    Ok(rec)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::keys::{MemoryKeyStore, Protection};

    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(
            dir.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        (dir, store)
    }

    fn bury(store: &Store, gid: &str, lamport: i64) {
        store
            .conn()
            .execute(
                "INSERT INTO tombstones (gid, kind, lamport, deleted_at, cause)
                 VALUES (?1, 'meeting', ?2, 0, 'user')",
                params![gid, lamport],
            )
            .unwrap();
    }

    #[test]
    fn a_tombstone_with_an_old_lamport_still_follows_the_cursor() {
        let (_dir, store) = store();
        let (a, b) = (crate::new_gid(), crate::new_gid());
        bury(&store, &a, 500);
        let first = store.tombs_since(0, 10).unwrap();
        assert_eq!(first.tombs.len(), 1);
        // A relayed tombstone: old lamport, new log position.
        bury(&store, &b, 3);
        let next = store.tombs_since(first.upto_seq, 10).unwrap();
        assert_eq!(next.tombs.len(), 1);
        assert_eq!(
            (next.tombs[0].gid.as_str(), next.tombs[0].lamport),
            (b.as_str(), 3)
        );
        assert_eq!(next.tombs[0].cause, Some(TombCause::User));
        assert_eq!(next.tombs[0].origin, store.sync_device_gid().unwrap());
        assert!(next.upto_seq > first.upto_seq);
    }

    #[test]
    fn a_restore_is_a_new_device() {
        let (_dir, store) = store();
        let (feed, device) = (store.feed_id().unwrap(), store.sync_device_gid().unwrap());
        store.reset_sync_identity().unwrap();
        assert_ne!(store.feed_id().unwrap(), feed);
        assert_ne!(store.sync_device_gid().unwrap(), device);
    }
}
