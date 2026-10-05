// SPDX-License-Identifier: Apache-2.0
//! Audio transfer, phone to desktop (doc 07 §7.7): the store side.
//!
//! The sender reads a finished track's records verbatim
//! ([`crate::bundle::raw_records`]); the receiver verifies each one while
//! appending it to `<bundle>.part` ([`crate::bundle::RawImport`]) and renames
//! the file into place after the final record. "Records" count the audio
//! pages plus the final record, so a track of `n` pages is `n + 1` records.
//!
//! Which tracks a peer holds is kept per device and meeting in
//! `peer_meetings.tracks_sent` (a bit per track kind), so it goes with the
//! pin and the meeting.

use rusqlite::{Connection, OptionalExtension, params};

use crate::bundle::{self, RawBegin, raw_header, raw_records};
use crate::store::{Store, TrackKind, check_gid};
use crate::{Result, StoreError, tombstones};

/// A finished local track that a peer does not hold yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendTrack {
    pub track_gid: String,
    pub meeting_gid: String,
    /// The bundle file header: magic, version and the 19-byte nonce prefix.
    pub header: Vec<u8>,
    /// Records in the bundle: audio pages plus the final record.
    pub records: u64,
    /// Bytes of record data (what the receiver needs room for).
    pub bytes: u64,
}

/// The receiver's decision on a track offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawOffer {
    /// Continue from this many verified records.
    Have(u64),
    /// The whole track is already here, whatever the sender's prefix.
    Complete,
    Deleted,
    /// No key, no row for the track, or the peer never exchanged the meeting.
    NoKey,
    StorageFull,
}

fn kind_from(s: &str) -> Result<TrackKind> {
    match s {
        "mic" => Ok(TrackKind::Mic),
        "system" => Ok(TrackKind::System),
        "file" => Ok(TrackKind::File),
        other => Err(StoreError::Invalid(format!("track kind {other:?}"))),
    }
}

fn kind_bit(kind: TrackKind) -> i64 {
    match kind {
        TrackKind::Mic => 1,
        TrackKind::System => 2,
        TrackKind::File => 4,
    }
}

/// `(meeting gid, kind)` of a track gid.
fn track_meta(conn: &Connection, track_gid: &str) -> Result<Option<(String, TrackKind)>> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT m.gid, t.kind FROM tracks t JOIN meetings m ON m.id = t.meeting_id
             WHERE t.gid = ?1",
            [track_gid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(m, k)| Ok((m, kind_from(&k)?))).transpose()
}

impl Store {
    /// Finished tracks recorded here that `device_gid` does not hold: the
    /// meeting's row and key are acked (`peer_meetings.key_sent`) and the
    /// track is not in `tracks_sent`.
    pub fn tracks_to_send(&self, device_gid: &str) -> Result<Vec<SendTrack>> {
        let rows: Vec<(String, String, String)> = {
            let conn = self.conn();
            let mut stmt = conn.prepare(
                "SELECT t.gid, m.gid, t.kind
                 FROM tracks t
                 JOIN meetings m ON m.id = t.meeting_id
                 JOIN devices d ON d.gid = ?1
                 JOIN peer_meetings p ON p.device_id = d.id AND p.meeting_gid = m.gid
                 WHERE p.key_sent = 1
                   AND m.audio_origin IS NULL AND m.source <> 'file'
                   AND m.status <> 'recording' AND t.page_count > 0
                   AND NOT EXISTS (SELECT 1 FROM tombstones x WHERE x.gid IN (t.gid, m.gid))
                   AND (p.tracks_sent & CASE t.kind WHEN 'mic' THEN 1 WHEN 'system' THEN 2
                                                    ELSE 4 END) = 0
                 ORDER BY m.started_at, t.gid",
            )?;
            let rows = stmt.query_map([device_gid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut out = Vec::new();
        for (track_gid, meeting_gid, kind) in rows {
            let path = self.bundle_path(&meeting_gid, kind_from(&kind)?)?;
            // A track whose file is gone or unreadable is skipped, not fatal.
            let (Ok(header), Ok(meta)) = (raw_header(&path), std::fs::metadata(&path)) else {
                continue;
            };
            let opened = {
                let conn = self.conn();
                let m = Store::meeting_ref(&conn, &meeting_gid)?;
                let dek = self.dek(&conn, m.id)?;
                bundle::BundleReader::open(&path, &dek, &Store::bundle_aad(&track_gid))
            };
            let Ok(reader) = opened else {
                continue;
            };
            if !reader.complete() {
                continue;
            }
            out.push(SendTrack {
                track_gid,
                meeting_gid,
                header,
                records: u64::from(reader.page_count()) + 1,
                bytes: meta.len(),
            });
        }
        Ok(out)
    }

    /// Up to `max` verbatim records of a local track from record `first`.
    pub fn track_read_records(
        &self,
        track_gid: &str,
        first: u64,
        max: usize,
    ) -> Result<Vec<Vec<u8>>> {
        check_gid(track_gid)?;
        let (meeting_gid, kind) = {
            let conn = self.conn();
            track_meta(&conn, track_gid)?.ok_or_else(|| StoreError::NotFound {
                kind: "track",
                gid: track_gid.to_string(),
            })?
        };
        let path = self.bundle_path(&meeting_gid, kind)?;
        raw_records(
            &path,
            u32::try_from(first).unwrap_or(u32::MAX),
            u32::try_from(max).unwrap_or(u32::MAX),
        )
    }

    /// The peer holds the whole track: [`Store::tracks_to_send`] stops
    /// listing it for that device.
    pub fn mark_track_sent(&self, device_gid: &str, track_gid: &str) -> Result<()> {
        let conn = self.conn();
        let (meeting_gid, kind) =
            track_meta(&conn, track_gid)?.ok_or_else(|| StoreError::NotFound {
                kind: "track",
                gid: track_gid.to_string(),
            })?;
        conn.execute(
            "UPDATE peer_meetings SET tracks_sent = tracks_sent | ?3
             WHERE meeting_gid = ?2 AND device_id = (SELECT id FROM devices WHERE gid = ?1)",
            params![device_gid, meeting_gid, kind_bit(kind)],
        )?;
        Ok(())
    }

    /// Receiver: decides an offer of `track_gid` from `from_device`.
    /// `header` is the sender's bundle header. `free` is the free space of
    /// the volume (None: unknown, never refuses) and `need` what the track
    /// must leave room for.
    pub fn raw_offer(
        &self,
        from_device: &str,
        meeting_gid: &str,
        track_gid: &str,
        header: &[u8],
        free: Option<u64>,
        need: u64,
    ) -> Result<RawOffer> {
        check_gid(meeting_gid)?;
        check_gid(track_gid)?;
        let kind = {
            let conn = self.conn();
            if tombstones::assert_live(&conn, meeting_gid).is_err()
                || tombstones::assert_live(&conn, track_gid).is_err()
            {
                return Ok(RawOffer::Deleted);
            }
            let exchanged: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM peer_meetings p JOIN devices d ON d.id = p.device_id
                                WHERE d.gid = ?1 AND p.meeting_gid = ?2)",
                params![from_device, meeting_gid],
                |r| r.get(0),
            )?;
            match track_meta(&conn, track_gid)? {
                Some((m, kind)) if exchanged && m == meeting_gid => kind,
                _ => return Ok(RawOffer::NoKey),
            }
        };
        // Replaces an earlier import of this track (a new session).
        self.raw_imports().remove(track_gid);
        let begin = match self.raw_import_begin(meeting_gid, kind, header) {
            Ok(b) => b,
            Err(StoreError::NotFound { .. }) => return Ok(RawOffer::NoKey),
            Err(StoreError::Tombstoned { .. }) => return Ok(RawOffer::Deleted),
            Err(e) => return Err(e),
        };
        match begin {
            RawBegin::Complete { .. } => Ok(RawOffer::Complete),
            RawBegin::Resume(imp) => {
                if imp.is_complete() {
                    // Everything arrived but the rename did not happen.
                    self.raw_import_finish(meeting_gid, kind, imp)?;
                    return Ok(RawOffer::Complete);
                }
                if free.is_some_and(|f| f < need) {
                    return Ok(RawOffer::StorageFull);
                }
                let have = u64::from(imp.have());
                self.raw_imports()
                    .insert(track_gid.to_string(), (header.to_vec(), imp));
                Ok(RawOffer::Have(have))
            }
        }
    }

    /// Receiver: verifies and appends records `first..` of the offered track;
    /// returns the number of verified records held. The record before a bad
    /// one stays, the bad one and the rest are not written. The final record
    /// completes the track (renamed into place, page count recorded).
    pub fn raw_push(
        &self,
        track_gid: &str,
        prefix: &[u8],
        first: u64,
        records: &[Vec<u8>],
    ) -> Result<u64> {
        check_gid(track_gid)?;
        let (meeting_gid, kind) = {
            let conn = self.conn();
            tombstones::assert_live(&conn, track_gid)?;
            track_meta(&conn, track_gid)?.ok_or_else(|| StoreError::NotFound {
                kind: "track",
                gid: track_gid.to_string(),
            })?
        };
        let mut imports = self.raw_imports();
        let Some((offered, mut imp)) = imports.remove(track_gid) else {
            return Err(StoreError::Invalid("pages without an offer".into()));
        };
        let held = u64::from(imp.have());
        if offered.get(5..) != Some(prefix) || first != held {
            // Put it back: the sender may retry in order.
            imports.insert(track_gid.to_string(), (offered, imp));
            return Err(StoreError::Invalid("pages out of order".into()));
        }
        // On a bad record the part is already cut back to the last good one;
        // the import is dropped and the next offer re-verifies it.
        imp.push(records)?;
        if imp.is_complete() {
            drop(imports);
            let n = u64::from(self.raw_import_finish(meeting_gid.as_str(), kind, imp)?);
            return Ok(n + 1);
        }
        let have = u64::from(imp.have());
        imports.insert(track_gid.to_string(), (offered, imp));
        Ok(have)
    }

    fn raw_imports(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, (Vec<u8>, bundle::RawImport)>>
    {
        self.raw_imports.lock().unwrap_or_else(|p| p.into_inner())
    }
}
