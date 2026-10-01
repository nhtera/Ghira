// SPDX-License-Identifier: Apache-2.0
//! Edits made while a meeting is recorded or reviewed (phase 8): speaker
//! rename/merge/split/"not a person", moving lines between speakers, discard
//! [RT-1], and the imported-file hash for duplicate detection.
//!
//! Every deletion writes tombstones in the same transaction, so sync never
//! brings a discarded row back.

use rusqlite::{OptionalExtension, params};

use crate::rowcrypt::{row_aad, seal_text};
use crate::store::{Store, TrackKind, id_of};
use crate::{Result, StoreError, new_gid, tombstones};

/// What a discard removed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiscardReport {
    /// The `discards` row; complete it with [`Store::discard_audio_done`].
    pub id: i64,
    pub segments: usize,
    pub marks: usize,
    pub notes: usize,
    pub action_items: usize,
    pub speakers: usize,
}

/// The audio side of a discard for one track: keep `pages` records of the
/// bundle whose nonce prefix is `prefix` (the file before rotation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepPages {
    pub kind: TrackKind,
    pub pages: u32,
    pub prefix: Option<String>,
}

/// A discard whose audio side has not been applied yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDiscard {
    pub id: i64,
    pub meeting_gid: String,
    pub keep: Vec<KeepPages>,
}

fn kind_name(k: TrackKind) -> &'static str {
    k.as_str()
}

fn kind_from(s: &str) -> Option<TrackKind> {
    match s {
        "mic" => Some(TrackKind::Mic),
        "system" => Some(TrackKind::System),
        "file" => Some(TrackKind::File),
        _ => None,
    }
}

impl Store {
    /// Sets (or with `None`/blank, clears) a speaker's display name.
    pub fn rename_speaker(&self, speaker_gid: &str, name: Option<&str>) -> Result<()> {
        let mut conn = self.conn();
        let (sid, meeting_id): (i64, i64) = conn
            .query_row(
                "SELECT id, meeting_id FROM speakers WHERE gid = ?1",
                [speaker_gid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "speaker",
                gid: speaker_gid.to_string(),
            })?;
        let dek = self.dek(&conn, meeting_id)?;
        let name = name.map(str::trim).filter(|n| !n.is_empty());
        let ct = name.map(|n| {
            seal_text(
                &dek,
                n,
                &row_aad("speakers", "display_name_ct", speaker_gid),
            )
        });
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "UPDATE speakers SET display_name_ct = ?1, lamport = ?2 WHERE id = ?3",
            params![ct, lamport, sid],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Merges `from` into `into` (same meeting): `from`'s lines and action
    /// items move to `into`, `from` points at `into`, and Me carries over.
    pub fn merge_speakers(&self, from_gid: &str, into_gid: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let speaker = |gid: &str| -> Result<(i64, i64, bool)> {
            tx.query_row(
                "SELECT id, meeting_id, is_me FROM speakers WHERE gid = ?1",
                [gid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "speaker",
                gid: gid.to_string(),
            })
        };
        let (from, m1, from_me) = speaker(from_gid)?;
        let (into, m2, _) = speaker(into_gid)?;
        if m1 != m2 || from == into {
            return Err(StoreError::Invalid(
                "speakers must differ and share a meeting".into(),
            ));
        }
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "UPDATE segments SET speaker_id = ?1, lamport = ?2 WHERE speaker_id = ?3",
            params![into, lamport, from],
        )?;
        tx.execute(
            "UPDATE action_items SET owner_speaker_id = ?1, lamport = ?2 WHERE owner_speaker_id = ?3",
            params![into, lamport, from],
        )?;
        tx.execute(
            "UPDATE speakers SET merged_into = ?1, is_me = 0, lamport = ?2 WHERE id = ?3",
            params![into, lamport, from],
        )?;
        if from_me {
            tx.execute(
                "UPDATE speakers SET is_me = 1, lamport = ?1 WHERE id = ?2",
                params![lamport, into],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// A new speaker split off `from_gid`, taking the given lines. Returns its gid.
    pub fn split_speaker(
        &self,
        from_gid: &str,
        segment_gids: &[String],
        color_slot: i64,
    ) -> Result<String> {
        let gid = new_gid();
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (from, meeting_id): (i64, i64) = tx
            .query_row(
                "SELECT id, meeting_id FROM speakers WHERE gid = ?1",
                [from_gid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "speaker",
                gid: from_gid.to_string(),
            })?;
        let label_idx: i64 = tx.query_row(
            "SELECT COALESCE(MAX(label_idx), 0) + 1 FROM speakers WHERE meeting_id = ?1",
            [meeting_id],
            |r| r.get(0),
        )?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "INSERT INTO speakers (gid, meeting_id, label_idx, color_slot, is_me, lamport)
             VALUES (?1, ?2, ?3, ?4, 0, ?5)",
            params![gid, meeting_id, label_idx, color_slot, lamport],
        )?;
        let new_id = tx.last_insert_rowid();
        for seg in segment_gids {
            let n = tx.execute(
                "UPDATE segments SET speaker_id = ?1, lamport = ?2
                 WHERE gid = ?3 AND speaker_id = ?4 AND meeting_id = ?5",
                params![new_id, lamport, seg, from, meeting_id],
            )?;
            if n == 0 {
                return Err(StoreError::Invalid(format!(
                    "segment {seg} is not a line of {from_gid}"
                )));
            }
        }
        tx.commit()?;
        Ok(gid)
    }

    /// Sets the "Speaker N" index (0-based: shown as N = index + 1) once a
    /// provisional speaker is confirmed; -1 while provisional.
    pub fn set_speaker_label_idx(&self, speaker_gid: &str, label_idx: i64) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        let n = tx.execute(
            "UPDATE speakers SET label_idx = ?1, lamport = ?2 WHERE gid = ?3",
            params![label_idx, lamport, speaker_gid],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound {
                kind: "speaker",
                gid: speaker_gid.to_string(),
            });
        }
        tx.commit()?;
        Ok(())
    }

    pub fn set_speaker_not_person(&self, speaker_gid: &str, not_person: bool) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        let n = tx.execute(
            "UPDATE speakers SET not_person = ?1, lamport = ?2 WHERE gid = ?3",
            params![not_person, lamport, speaker_gid],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound {
                kind: "speaker",
                gid: speaker_gid.to_string(),
            });
        }
        tx.commit()?;
        Ok(())
    }

    /// Moves one line to another speaker of the same meeting (`None`: unknown).
    pub fn set_segment_speaker(&self, segment_gid: &str, speaker_gid: Option<&str>) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let speaker = speaker_gid.map(|g| id_of(&tx, "speakers", g)).transpose()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        let n = tx.execute(
            "UPDATE segments SET speaker_id = ?1, lamport = ?2 WHERE gid = ?3
             AND (?1 IS NULL OR meeting_id = (SELECT meeting_id FROM speakers WHERE id = ?1))",
            params![speaker, lamport, segment_gid],
        )?;
        if n == 0 {
            return Err(StoreError::Invalid(format!(
                "cannot move segment {segment_gid}"
            )));
        }
        tx.commit()?;
        Ok(())
    }

    /// Discard [RT-1], text side: in one transaction removes every segment
    /// ending after `t_cut_ms` (a line straddling the cut goes entirely),
    /// marks from the cut on, and notes and action items citing the window;
    /// with `drop_orphans`, also speakers left without lines (unless named or
    /// linked to a person — a live session passes `false` and cleans up at
    /// stop, since its speakers may talk again). Records a pending `discards`
    /// row with the pages to keep per track. Apply the audio side, then
    /// [`Store::discard_audio_done`].
    pub fn discard_after(
        &self,
        meeting_gid: &str,
        t_cut_ms: i64,
        now_ms: i64,
        keep_pages: &[KeepPages],
        drop_orphans: bool,
    ) -> Result<DiscardReport> {
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        let mut rep = DiscardReport::default();

        tombstones::write_where(
            &tx,
            "segment",
            "SELECT gid FROM segments WHERE meeting_id = ?1 AND t1_ms > ?2",
            params![m.id, t_cut_ms],
            lamport,
        )?;
        tx.execute(
            "DELETE FROM segments_fts WHERE rowid IN
             (SELECT id FROM segments WHERE meeting_id = ?1 AND t1_ms > ?2)",
            params![m.id, t_cut_ms],
        )?;
        rep.segments = tx.execute(
            "DELETE FROM segments WHERE meeting_id = ?1 AND t1_ms > ?2",
            params![m.id, t_cut_ms],
        )?;

        tombstones::write_where(
            &tx,
            "mark",
            "SELECT gid FROM marks WHERE meeting_id = ?1 AND t_ms >= ?2",
            params![m.id, t_cut_ms],
            lamport,
        )?;
        rep.marks = tx.execute(
            "DELETE FROM marks WHERE meeting_id = ?1 AND t_ms >= ?2",
            params![m.id, t_cut_ms],
        )?;

        // Notes and action items citing anything after the cut.
        tombstones::write_where(&tx, "note", NOTES_CITING, params![m.id, t_cut_ms], lamport)?;
        tx.execute(
            &format!("DELETE FROM notes_fts WHERE rowid IN (SELECT id FROM notes_blocks WHERE gid IN ({NOTES_CITING}))"),
            params![m.id, t_cut_ms],
        )?;
        rep.notes = tx.execute(
            &format!("DELETE FROM notes_blocks WHERE gid IN ({NOTES_CITING})"),
            params![m.id, t_cut_ms],
        )?;
        tombstones::write_where(
            &tx,
            "action_item",
            ACTIONS_CITING,
            params![m.id, t_cut_ms],
            lamport,
        )?;
        rep.action_items = tx.execute(
            &format!("DELETE FROM action_items WHERE gid IN ({ACTIONS_CITING})"),
            params![m.id, t_cut_ms],
        )?;

        if drop_orphans {
            rep.speakers = remove_orphans(&tx, m.id, lamport)?;
        }

        let keep: serde_json::Map<String, serde_json::Value> = keep_pages
            .iter()
            .map(|k| {
                (
                    kind_name(k.kind).to_string(),
                    serde_json::json!({"pages": k.pages, "prefix": k.prefix}),
                )
            })
            .collect();
        tx.execute(
            "INSERT INTO discards (meeting_id, t0_ms, t1_ms, keep_pages_json, audio_state, created_at)
             VALUES (?1, ?2, ?3, ?4, 'pending', ?5)",
            params![
                m.id,
                t_cut_ms,
                now_ms.max(t_cut_ms),
                serde_json::Value::Object(keep).to_string(),
                crate::store::now_ms()
            ],
        )?;
        rep.id = tx.last_insert_rowid();
        tx.execute(
            "UPDATE meetings SET lamport = ?1 WHERE id = ?2",
            params![lamport, m.id],
        )?;
        tx.commit()?;
        Ok(rep)
    }

    /// Deletes the meeting's speakers that have no line, no name and no
    /// person (after a discard, or provisional ones that never confirmed).
    pub fn remove_orphan_speakers(&self, meeting_gid: &str) -> Result<usize> {
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        let n = remove_orphans(&tx, m.id, lamport)?;
        tx.commit()?;
        Ok(n)
    }

    /// The audio side of a discard is applied.
    pub fn discard_audio_done(&self, discard_id: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE discards SET audio_state = 'done' WHERE id = ?1",
            [discard_id],
        )?;
        Ok(())
    }

    /// Discards whose audio side is not applied (a crash in between).
    pub fn pending_discards(&self) -> Result<Vec<PendingDiscard>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT d.id, m.gid, d.keep_pages_json FROM discards d
             JOIN meetings m ON m.id = d.meeting_id WHERE d.audio_state = 'pending' ORDER BY d.id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .map(|(id, meeting_gid, json)| {
                let map: serde_json::Map<String, serde_json::Value> =
                    serde_json::from_str(&json).unwrap_or_default();
                let keep = map
                    .iter()
                    .filter_map(|(k, v)| {
                        Some(KeepPages {
                            kind: kind_from(k)?,
                            pages: u32::try_from(v["pages"].as_u64()?).ok()?,
                            prefix: v["prefix"].as_str().map(str::to_string),
                        })
                    })
                    .collect();
                PendingDiscard {
                    id,
                    meeting_gid,
                    keep,
                }
            })
            .collect())
    }

    /// Spans removed by discards, `(t0_ms, t1_ms)` in time order (the final
    /// pass skips them; the UI marks them).
    pub fn discarded_spans(&self, meeting_gid: &str) -> Result<Vec<(i64, i64)>> {
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let mut stmt = conn.prepare_cached(
            "SELECT t0_ms, t1_ms FROM discards WHERE meeting_id = ?1 ORDER BY t0_ms",
        )?;
        Ok(stmt
            .query_map([m.id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Completes a discard's audio side after a crash: cuts the track's
    /// bundle to `keep.pages` records (see [`crate::bundle::truncate`]),
    /// unless the file no longer has `keep.prefix` (the rotation already
    /// happened; cutting it again would drop audio recorded since). Returns
    /// the pages kept, or `None` when nothing was done.
    pub fn complete_discard_audio(
        &self,
        meeting_gid: &str,
        keep: &KeepPages,
    ) -> Result<Option<u32>> {
        let path = self.bundle_path(meeting_gid, keep.kind)?;
        if !path.exists() {
            return Ok(None);
        }
        if let Some(p) = &keep.prefix
            && crate::bundle::file_prefix_hex(&path)? != *p
        {
            return Ok(None);
        }
        self.truncate_track(meeting_gid, keep.kind, keep.pages)
            .map(Some)
    }

    /// Cuts a closed track bundle to `keep` pages.
    pub fn truncate_track(&self, meeting_gid: &str, kind: TrackKind, keep: u32) -> Result<u32> {
        let path = self.bundle_path(meeting_gid, kind)?;
        if !path.exists() {
            return Ok(0);
        }
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let track_gid: String = conn.query_row(
            "SELECT gid FROM tracks WHERE meeting_id = ?1 AND kind = ?2",
            params![m.id, kind.as_str()],
            |r| r.get(0),
        )?;
        drop(conn);
        let kept = crate::bundle::truncate(&path, &dek, &Store::bundle_aad(&track_gid), keep)?;
        // The recorded page count guards against a cut-off file; it is now shorter on purpose.
        self.conn().execute(
            "UPDATE tracks SET page_count = ?1 WHERE gid = ?2 AND page_count > ?1",
            params![kept, track_gid],
        )?;
        Ok(kept)
    }

    /// Records the SHA-256 (hex) of an imported file.
    pub fn set_source_hash(&self, meeting_gid: &str, sha256_hex: &str) -> Result<()> {
        if sha256_hex.len() != 64 || !sha256_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(StoreError::Invalid(
                "source hash must be SHA-256 hex".into(),
            ));
        }
        let n = self.conn().execute(
            "UPDATE meetings SET source_hash = ?1 WHERE gid = ?2",
            params![sha256_hex.to_ascii_lowercase(), meeting_gid],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound {
                kind: "meeting",
                gid: meeting_gid.to_string(),
            });
        }
        Ok(())
    }

    /// The meeting already imported from a file with this SHA-256, if any.
    pub fn meeting_by_source_hash(&self, sha256_hex: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT gid FROM meetings WHERE source_hash = ?1",
                [sha256_hex.to_ascii_lowercase()],
                |r| r.get(0),
            )
            .optional()?)
    }
}

/// Speakers with no line left, no name, no person and nobody merged into them.
const ORPHANS: &str = "SELECT gid FROM speakers s WHERE s.meeting_id = ?1
     AND s.display_name_ct IS NULL AND s.person_id IS NULL AND s.is_me = 0
     AND NOT EXISTS (SELECT 1 FROM segments g WHERE g.speaker_id = s.id)
     AND NOT EXISTS (SELECT 1 FROM speakers o WHERE o.merged_into = s.id)";

fn remove_orphans(tx: &rusqlite::Transaction, meeting_id: i64, lamport: i64) -> Result<usize> {
    tombstones::write_where(tx, "speaker", ORPHANS, params![meeting_id], lamport)?;
    Ok(tx.execute(
        &format!("DELETE FROM speakers WHERE gid IN ({ORPHANS})"),
        params![meeting_id],
    )?)
}

/// Notes and action items citing anything after the cut (`?1` meeting id,
/// `?2` cut). Literals, because `tombstones::write_where` takes `&'static str`.
const NOTES_CITING: &str = "SELECT gid FROM notes_blocks WHERE meeting_id = ?1 AND EXISTS
     (SELECT 1 FROM json_each(anchors_json) a WHERE json_extract(a.value, '$.t1_ms') > ?2)";
const ACTIONS_CITING: &str = "SELECT gid FROM action_items WHERE meeting_id = ?1 AND EXISTS
     (SELECT 1 FROM json_each(anchors_json) a WHERE json_extract(a.value, '$.t1_ms') > ?2)";
