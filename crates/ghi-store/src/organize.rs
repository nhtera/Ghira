// SPDX-License-Identifier: Apache-2.0
//! Organizing meetings (phase 14d): folders, tags, and the per-meeting facts an
//! import or a calendar event adds.
//!
//! - Folders: one optional folder per meeting (`meetings.folder_id`). Tags:
//!   many per meeting (`tags` + `meeting_tags`). Names are plaintext under
//!   SQLCipher (like persons) and unique by [`name_key`] (NFC, trimmed,
//!   lowercase; accents count: "Họp" and "Hộp" are two names). To keep typing
//!   easy, a typed name *without* accents that matches exactly one existing
//!   name once accents are ignored stands for it: "hop" finds "Họp" (with two
//!   such names, "hop" is a new name). Link rows carry their own gid, lamport
//!   and tombstones, fresh each time a tag is added again, so adding or removing
//!   a tag does not touch the meeting's lamport (a folder is a meeting column,
//!   so changing it does).
//! - `meetings.source_app`: which app an import came from.
//! - `calendar_ct`: JSON sealed under the meeting key (crypto-shredded with
//!   it). `track_speakers_ct`: the participants' speech spans as a compact
//!   binary (delta-encoded varints), sealed the same way, up to
//!   [`MAX_TRACK_SPEAKERS_BYTES`].
//! - `segments.overlap`: set by [`Store::mark_overlaps`].
//!
//! Every deletion writes tombstones; names are never logged.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::rowcrypt::{self, open_text, row_aad, seal_text};
use crate::store::{NewSegment, Segment, Store, insert_segments, now_ms};
use crate::tombstones::Cause;
use crate::{Result, StoreError, db, fold, new_gid, tombstones};

pub const MAX_FOLDER_NAME: usize = 60;
pub const MAX_TAG_NAME: usize = 40;
pub const MAX_TAGS_PER_MEETING: usize = 20;
pub const MAX_FOLDERS: usize = 200;
pub const MAX_TAGS: usize = 200;
/// The values of `meetings.source_app`.
pub const SOURCE_APPS: [&str; 5] = ["zoom", "teams", "meet", "plaud", "voice_memos"];
/// Most meetings a bulk call takes at once.
const MAX_BULK: usize = 5_000;
/// Largest sealed JSON value (calendar info, track speakers), in bytes.
pub const MAX_SEALED_JSON: usize = 256 * 1024;
/// Largest encoded size of a meeting's participant spans, in bytes (a 49-track
/// recording of many hours stays far below it; see [`track_speakers_bound`]).
pub const MAX_TRACK_SPEAKERS_BYTES: usize = 32 * 1024 * 1024;

/// An upper bound of the encoded size of `tracks` participants' spans over
/// `duration_ms`, given that consecutive spans start at least
/// `min_period_ms` apart: lets an importer refuse a recording too long to
/// store before decoding it.
pub fn track_speakers_bound(tracks: usize, duration_ms: i64, min_period_ms: i64) -> usize {
    let per_track = (duration_ms.max(0) / min_period_ms.max(1) + 1) as usize;
    // Two varints per span, each at most 4 bytes while a recording is under
    // 74 hours; plus the names and counts.
    tracks.saturating_mul(per_track.saturating_mul(8).saturating_add(512))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub gid: String,
    pub name: String,
    pub created_at: i64,
    /// Meetings in the folder.
    pub meetings: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub gid: String,
    pub name: String,
    pub created_at: i64,
    /// Meetings with the tag.
    pub meetings: i64,
}

/// Speech spans of one participant of a multi-track import (D9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackSpeaker {
    /// The participant as named in the file (e.g. a Zoom display name).
    #[serde(default)]
    pub label: String,
    /// The meeting speaker created for them (one of the meeting's speakers).
    #[serde(default)]
    pub speaker_gid: String,
    /// `[t0_ms, t1_ms]` pairs, in time order.
    #[serde(default)]
    pub spans: Vec<[i64; 2]>,
}

/// Zero-width, bidi and other invisible format characters (Unicode Cf).
fn is_format(c: char) -> bool {
    matches!(c, '\u{ad}' | '\u{61c}' | '\u{180e}' | '\u{200b}'..='\u{200f}'
        | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206f}'
        | '\u{feff}')
}

/// Strips control and invisible format characters, turns whitespace runs into
/// one space, trims, NFC. Errors if empty or over `max` characters.
fn clean_name(name: &str, max: usize, what: &'static str) -> Result<String> {
    let visible: String = name
        .chars()
        .filter_map(|c| {
            if c.is_whitespace() {
                Some(' ')
            } else if c.is_control() || is_format(c) {
                None
            } else {
                Some(c)
            }
        })
        .collect();
    let name = fold::nfc(&visible.split_whitespace().collect::<Vec<_>>().join(" "));
    if name.is_empty() {
        return Err(StoreError::Invalid(format!("a {what} needs a name")));
    }
    if name.chars().count() > max {
        return Err(StoreError::Limit {
            kind: if what == "folder" {
                "characters in a folder name"
            } else {
                "characters in a tag name"
            },
            max,
        });
    }
    Ok(name)
}

/// What names are unique by: NFC, trimmed, lowercase (not accent-folded).
pub fn name_key(name: &str) -> String {
    crate::people::name_key(name)
}

/// The row the name stands for: an exact [`name_key`] match, else the lone
/// existing name that equals it once accents are ignored (only when the typed
/// name has none). Returns the row id.
fn resolve(conn: &Connection, table: &'static str, name: &str) -> Result<Option<i64>> {
    let exact: Option<i64> = conn
        .query_row(
            &format!("SELECT id FROM {table} WHERE name_key = ?1"),
            [name_key(name)],
            |r| r.get(0),
        )
        .optional()?;
    if exact.is_some() || fold::has_diacritics(name) {
        return Ok(exact);
    }
    let want = fold::fold(name);
    let mut stmt = conn.prepare_cached(&format!("SELECT id, name FROM {table}"))?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut like = rows.into_iter().filter(|(_, n)| fold::fold(n) == want);
    Ok(match (like.next(), like.next()) {
        (Some((id, _)), None) => Some(id),
        _ => None,
    })
}

/// A unique-constraint failure on a name is a [`StoreError::Duplicate`].
fn name_taken(e: rusqlite::Error, kind: &'static str) -> StoreError {
    match &e {
        rusqlite::Error::SqliteFailure(f, _)
            if f.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
        {
            StoreError::Duplicate { kind }
        }
        _ => e.into(),
    }
}

/// Rowid of `gid` in `table`; `kind` is the singular name for errors.
fn find(conn: &Connection, table: &'static str, kind: &'static str, gid: &str) -> Result<i64> {
    conn.query_row(
        &format!("SELECT id FROM {table} WHERE gid = ?1"),
        [gid],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| StoreError::NotFound {
        kind,
        gid: gid.to_string(),
    })
}

fn check_bulk(gids: &[String]) -> Result<()> {
    if gids.len() > MAX_BULK {
        return Err(StoreError::Limit {
            kind: "meetings per call",
            max: MAX_BULK,
        });
    }
    Ok(())
}

fn count(conn: &Connection, table: &'static str) -> Result<usize> {
    Ok(
        conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
            r.get::<_, i64>(0)
        })? as usize,
    )
}

fn check_json_size(json: &str) -> Result<()> {
    if json.len() > MAX_SEALED_JSON {
        return Err(StoreError::Limit {
            kind: "bytes of meeting data",
            max: MAX_SEALED_JSON,
        });
    }
    Ok(())
}

fn tag_row(r: &rusqlite::Row) -> rusqlite::Result<Tag> {
    Ok(Tag {
        gid: r.get(0)?,
        name: r.get(1)?,
        created_at: r.get(2)?,
        meetings: r.get(3)?,
    })
}

impl Store {
    // ------------------------------------------------------------ folders

    /// Every folder by name, with its meeting count.
    pub fn folders(&self) -> Result<Vec<Folder>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT f.gid, f.name, f.created_at,
                    (SELECT count(*) FROM meetings m WHERE m.folder_id = f.id)
             FROM folders f ORDER BY f.name_key, f.gid",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Folder {
                gid: r.get(0)?,
                name: r.get(1)?,
                created_at: r.get(2)?,
                meetings: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A new folder. [`StoreError::Duplicate`] if the name is taken (exactly,
    /// or as the lone accented name an unaccented one stands for).
    pub fn create_folder(&self, name: &str) -> Result<Folder> {
        let name = clean_name(name, MAX_FOLDER_NAME, "folder")?;
        let key = name_key(&name);
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if resolve(&tx, "folders", &name)?.is_some() {
            return Err(StoreError::Duplicate { kind: "folder" });
        }
        if count(&tx, "folders")? >= MAX_FOLDERS {
            return Err(StoreError::Limit {
                kind: "folders",
                max: MAX_FOLDERS,
            });
        }
        let (gid, now) = (new_gid(), now_ms());
        tombstones::assert_live(&tx, &gid)?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "INSERT INTO folders (gid, name, name_key, created_at, lamport)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![gid, name, key, now, lamport],
        )
        .map_err(|e| name_taken(e, "folder"))?;
        tx.commit()?;
        Ok(Folder {
            gid,
            name,
            created_at: now,
            meetings: 0,
        })
    }

    /// Renames a folder (case or accents may change; another folder's exact
    /// [`name_key`] is [`StoreError::Duplicate`]).
    pub fn rename_folder(&self, folder_gid: &str, name: &str) -> Result<()> {
        let name = clean_name(name, MAX_FOLDER_NAME, "folder")?;
        let key = name_key(&name);
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let id = find(&tx, "folders", "folder", folder_gid)?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "UPDATE folders SET name = ?1, name_key = ?2, lamport = ?3 WHERE id = ?4",
            params![name, key, lamport, id],
        )
        .map_err(|e| name_taken(e, "folder"))?;
        tx.commit()?;
        Ok(())
    }

    /// Deletes a folder; its meetings stay, without a folder (and their
    /// lamport moves, as `folder_id` is a meeting column). Returns how many
    /// meetings that touched.
    pub fn delete_folder(&self, folder_gid: &str) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let id = find(&tx, "folders", "folder", folder_gid)?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tombstones::write(&tx, folder_gid, "folder", lamport, Cause::User)?;
        // Explicit, so the meetings' lamport moves (sync sees the change).
        let cleared = tx.execute(
            "UPDATE meetings SET folder_id = NULL, lamport = ?1 WHERE folder_id = ?2",
            params![lamport, id],
        )?;
        tx.execute("DELETE FROM folders WHERE id = ?1", [id])?;
        tx.commit()?;
        // Best effort: the delete is done; this folds the WAL into the file.
        let _ = db::checkpoint(&conn);
        Ok(cleared)
    }

    /// Moves the meetings into `folder` (`None`: out of any folder). Returns
    /// how many meetings changed. An unknown folder or meeting fails the whole
    /// call with no change.
    pub fn set_meeting_folder(
        &self,
        meeting_gids: &[String],
        folder_gid: Option<&str>,
    ) -> Result<usize> {
        check_bulk(meeting_gids)?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let folder = folder_gid
            .map(|g| find(&tx, "folders", "folder", g))
            .transpose()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        let mut changed = 0;
        for g in meeting_gids {
            find(&tx, "meetings", "meeting", g)?;
            changed += tx.execute(
                "UPDATE meetings SET folder_id = ?1, lamport = ?2
                 WHERE gid = ?3 AND folder_id IS NOT ?1",
                params![folder, lamport, g],
            )?;
        }
        tx.commit()?;
        Ok(changed)
    }

    // --------------------------------------------------------------- tags

    /// Every tag by name, with its meeting count.
    pub fn tags(&self) -> Result<Vec<Tag>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT t.gid, t.name, t.created_at, COALESCE(c.n, 0)
             FROM tags t
             LEFT JOIN (SELECT tag_id, count(*) AS n FROM meeting_tags GROUP BY tag_id) c
                    ON c.tag_id = t.id
             ORDER BY t.name_key, t.gid",
        )?;
        let rows = stmt.query_map([], tag_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The tag with this name (see the module docs for how accents are
    /// matched), or a new one.
    pub fn create_tag(&self, name: &str) -> Result<Tag> {
        let name = clean_name(name, MAX_TAG_NAME, "tag")?;
        let key = name_key(&name);
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if let Some(id) = resolve(&tx, "tags", &name)? {
            return Ok(tx.query_row(
                "SELECT t.gid, t.name, t.created_at,
                        (SELECT count(*) FROM meeting_tags mt WHERE mt.tag_id = t.id)
                 FROM tags t WHERE t.id = ?1",
                [id],
                tag_row,
            )?);
        }
        if count(&tx, "tags")? >= MAX_TAGS {
            return Err(StoreError::Limit {
                kind: "tags",
                max: MAX_TAGS,
            });
        }
        let (gid, now) = (new_gid(), now_ms());
        tombstones::assert_live(&tx, &gid)?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "INSERT INTO tags (gid, name, name_key, created_at, lamport)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![gid, name, key, now, lamport],
        )
        .map_err(|e| name_taken(e, "tag"))?;
        tx.commit()?;
        Ok(Tag {
            gid,
            name,
            created_at: now,
            meetings: 0,
        })
    }

    /// Renames a tag ([`StoreError::Duplicate`] if another has that exact
    /// [`name_key`]).
    pub fn rename_tag(&self, tag_gid: &str, name: &str) -> Result<()> {
        let name = clean_name(name, MAX_TAG_NAME, "tag")?;
        let key = name_key(&name);
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let id = find(&tx, "tags", "tag", tag_gid)?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "UPDATE tags SET name = ?1, name_key = ?2, lamport = ?3 WHERE id = ?4",
            params![name, key, lamport, id],
        )
        .map_err(|e| name_taken(e, "tag"))?;
        tx.commit()?;
        Ok(())
    }

    /// Deletes a tag and its links (tombstones for both). Returns how many
    /// meetings had it.
    pub fn delete_tag(&self, tag_gid: &str) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let id = find(&tx, "tags", "tag", tag_gid)?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tombstones::write_where(
            &tx,
            "meeting_tag",
            "SELECT gid FROM meeting_tags WHERE tag_id = ?1",
            [id],
            lamport,
            Cause::User,
        )?;
        tombstones::write(&tx, tag_gid, "tag", lamport, Cause::User)?;
        let links: usize = tx.query_row(
            "SELECT count(*) FROM meeting_tags WHERE tag_id = ?1",
            [id],
            |r| r.get::<_, i64>(0),
        )? as usize;
        tx.execute("DELETE FROM tags WHERE id = ?1", [id])?;
        tx.commit()?;
        // Best effort: the delete is done; this folds the WAL into the file.
        let _ = db::checkpoint(&conn);
        Ok(links)
    }

    /// Adds the tag to the meetings (ones that have it are skipped). Returns
    /// how many were tagged. A meeting holds at most [`MAX_TAGS_PER_MEETING`];
    /// going over, or one unknown meeting, fails the whole call with no change.
    /// The meetings' own lamport does not move (the link rows carry theirs).
    pub fn tag_meetings(&self, meeting_gids: &[String], tag_gid: &str) -> Result<usize> {
        check_bulk(meeting_gids)?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let tag = find(&tx, "tags", "tag", tag_gid)?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        let mut added = 0;
        for g in meeting_gids {
            let m = find(&tx, "meetings", "meeting", g)?;
            let has: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM meeting_tags WHERE meeting_id = ?1 AND tag_id = ?2)",
                params![m, tag],
                |r| r.get(0),
            )?;
            if has {
                continue;
            }
            let n: i64 = tx.query_row(
                "SELECT count(*) FROM meeting_tags WHERE meeting_id = ?1",
                [m],
                |r| r.get(0),
            )?;
            if n as usize >= MAX_TAGS_PER_MEETING {
                return Err(StoreError::Limit {
                    kind: "tags per meeting",
                    max: MAX_TAGS_PER_MEETING,
                });
            }
            let link_gid = new_gid();
            tombstones::assert_live(&tx, &link_gid)?;
            tx.execute(
                "INSERT INTO meeting_tags (meeting_id, tag_id, gid, lamport)
                 VALUES (?1, ?2, ?3, ?4)",
                params![m, tag, link_gid, lamport],
            )?;
            added += 1;
        }
        tx.commit()?;
        Ok(added)
    }

    /// Removes the tag from the meetings (a tombstone per link; the meetings'
    /// own lamport does not move). Returns how many had it; one unknown
    /// meeting fails the whole call with no change.
    pub fn untag_meetings(&self, meeting_gids: &[String], tag_gid: &str) -> Result<usize> {
        check_bulk(meeting_gids)?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let tag = find(&tx, "tags", "tag", tag_gid)?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        let mut removed = 0;
        for g in meeting_gids {
            let m = find(&tx, "meetings", "meeting", g)?;
            let link: Option<String> = tx
                .query_row(
                    "SELECT gid FROM meeting_tags WHERE meeting_id = ?1 AND tag_id = ?2",
                    params![m, tag],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(link) = link {
                tombstones::write(&tx, &link, "meeting_tag", lamport, Cause::User)?;
                tx.execute(
                    "DELETE FROM meeting_tags WHERE meeting_id = ?1 AND tag_id = ?2",
                    params![m, tag],
                )?;
                removed += 1;
            }
        }
        tx.commit()?;
        Ok(removed)
    }

    /// The tags of each of `meeting_gids` (by name; meetings without tags are
    /// absent), in one query.
    pub fn meeting_tags(&self, meeting_gids: &[String]) -> Result<HashMap<String, Vec<Tag>>> {
        check_bulk(meeting_gids)?;
        let want =
            serde_json::to_string(meeting_gids).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT m.gid, t.gid, t.name, t.created_at, COALESCE(c.n, 0)
             FROM meeting_tags mt
             JOIN meetings m ON m.id = mt.meeting_id
             JOIN tags t ON t.id = mt.tag_id
             LEFT JOIN (SELECT tag_id, count(*) AS n FROM meeting_tags GROUP BY tag_id) c
                    ON c.tag_id = t.id
             WHERE m.gid IN (SELECT value FROM json_each(?1))
             ORDER BY m.gid, t.name_key, t.gid",
        )?;
        let rows = stmt.query_map([want], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Tag {
                    gid: r.get(1)?,
                    name: r.get(2)?,
                    created_at: r.get(3)?,
                    meetings: r.get(4)?,
                },
            ))
        })?;
        let mut out: HashMap<String, Vec<Tag>> = HashMap::new();
        for row in rows {
            let (m, tag) = row?;
            out.entry(m).or_default().push(tag);
        }
        Ok(out)
    }

    // --------------------------------------------- facts about a meeting

    /// Records which app an import came from (one of [`SOURCE_APPS`]; `None`
    /// clears it).
    pub fn set_source_app(&self, meeting_gid: &str, source_app: Option<&str>) -> Result<()> {
        if let Some(a) = source_app
            && !SOURCE_APPS.contains(&a)
        {
            return Err(StoreError::Invalid("unknown source app".into()));
        }
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        let n = tx.execute(
            "UPDATE meetings SET source_app = ?1, lamport = ?2 WHERE gid = ?3",
            params![source_app, lamport, meeting_gid],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound {
                kind: "meeting",
                gid: meeting_gid.to_string(),
            });
        }
        tx.commit()?;
        Ok(())
    }

    /// Stores (or with `None` clears) the calendar facts of the meeting
    /// (event, attendees, calendar: the caller's JSON, at most
    /// [`MAX_SEALED_JSON`] bytes), sealed under the meeting key.
    pub fn set_calendar_info(
        &self,
        meeting_gid: &str,
        info: Option<&serde_json::Value>,
    ) -> Result<()> {
        self.set_sealed_json(meeting_gid, "calendar_ct", info)
    }

    /// The meeting's calendar facts. A shredded meeting is
    /// [`StoreError::Decrypt`].
    pub fn calendar_info(&self, meeting_gid: &str) -> Result<Option<serde_json::Value>> {
        self.get_sealed_json(meeting_gid, "calendar_ct")
    }

    /// Stores the speech spans per participant of a multi-track import
    /// (replacing earlier ones; an empty list clears them), compactly encoded
    /// and sealed. Every `speaker_gid` must be a speaker of this meeting; the
    /// spans of each participant must be in time order and not overlap; the
    /// encoding may not exceed [`MAX_TRACK_SPEAKERS_BYTES`].
    pub fn set_track_speakers(&self, meeting_gid: &str, speakers: &[TrackSpeaker]) -> Result<()> {
        let plain = (!speakers.is_empty())
            .then(|| encode_track_speakers(speakers))
            .transpose()?;
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        for s in speakers {
            let ok: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM speakers WHERE gid = ?1 AND meeting_id = ?2)",
                params![s.speaker_gid, m.id],
                |r| r.get(0),
            )?;
            if !ok {
                return Err(StoreError::Invalid(
                    "a track speaker is not a speaker of this meeting".into(),
                ));
            }
        }
        let ct = match &plain {
            Some(p) => {
                let dek = self.dek(&conn, m.id)?;
                Some(rowcrypt::seal(
                    &dek.subkey(rowcrypt::ROWS_INFO),
                    p,
                    &row_aad("meetings", "track_speakers_ct", meeting_gid),
                ))
            }
            None => None,
        };
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "UPDATE meetings SET track_speakers_ct = ?1, lamport = ?2 WHERE id = ?3",
            params![ct, lamport, m.id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The stored participant spans (empty if none). A shredded meeting is
    /// [`StoreError::Decrypt`].
    pub fn track_speakers(&self, meeting_gid: &str) -> Result<Vec<TrackSpeaker>> {
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let ct: Option<Vec<u8>> = conn.query_row(
            "SELECT track_speakers_ct FROM meetings WHERE id = ?1",
            [m.id],
            |r| r.get(0),
        )?;
        let Some(ct) = ct else {
            return Ok(Vec::new());
        };
        let dek = self.dek(&conn, m.id)?;
        let plain = rowcrypt::open(
            &dek.subkey(rowcrypt::ROWS_INFO),
            &ct,
            &row_aad("meetings", "track_speakers_ct", meeting_gid),
        )?;
        decode_track_speakers(&plain)
    }

    /// [`Store::replace_transcript`] that also marks `overlap_gids` (gids among
    /// `segs`) as talked over, in the same transaction: a crash never leaves
    /// the new transcript without its marks. Returns the new version.
    pub fn replace_transcript_marked(
        &self,
        meeting_gid: &str,
        segs: Vec<NewSegment>,
        overlap_gids: &[String],
    ) -> Result<i64> {
        self.replace_transcript_inner(meeting_gid, segs, overlap_gids, None, None)
    }

    /// [`Store::replace_transcript_marked`] at a fencing `epoch` (doc 07 §8).
    /// With `lease` (a job uuid) the same transaction moves that lease
    /// `granted -> done`; if it is no longer granted (revoked, expired),
    /// nothing is written and the result is [`StoreError::Fenced`].
    pub fn replace_transcript_marked_epoch(
        &self,
        meeting_gid: &str,
        segs: Vec<NewSegment>,
        overlap_gids: &[String],
        epoch: i64,
        lease: Option<&str>,
    ) -> Result<i64> {
        self.replace_transcript_inner(meeting_gid, segs, overlap_gids, Some(epoch), lease)
    }

    /// The one replace-transcript transaction. `epoch` `None` keeps the
    /// meeting's current one; otherwise the meeting takes `max(current, epoch)`
    /// and the new segments carry it.
    pub(crate) fn replace_transcript_inner(
        &self,
        meeting_gid: &str,
        segs: Vec<NewSegment>,
        overlap_gids: &[String],
        epoch: Option<i64>,
        lease: Option<&str>,
    ) -> Result<i64> {
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let new_version = m.version + 1;
        let tx = conn.transaction()?;
        if let Some(job) = lease {
            crate::store::finish_lease(&tx, job)?;
        }
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tombstones::write_where(
            &tx,
            "segment",
            "SELECT gid FROM segments WHERE meeting_id = ?1 AND version < ?2",
            params![m.id, new_version],
            lamport,
            Cause::Transcript,
        )?;
        tx.execute(
            "DELETE FROM segments_fts WHERE rowid IN (SELECT id FROM segments WHERE meeting_id = ?1)",
            [m.id],
        )?;
        tx.execute("DELETE FROM segments WHERE meeting_id = ?1", [m.id])?;
        tx.execute(
            "UPDATE meetings SET transcript_version = ?1, lamport = ?2,
                 transcript_epoch = MAX(transcript_epoch, COALESCE(?4, transcript_epoch))
             WHERE id = ?3",
            params![new_version, lamport, m.id, epoch],
        )?;
        insert_segments(&tx, &dek, m.id, new_version, &segs)?;
        mark_in(&tx, m.id, overlap_gids)?;
        tx.commit()?;
        Ok(new_version)
    }

    /// [`Store::add_segments`] that also marks `overlap_gids` as talked over,
    /// in the same transaction.
    pub fn add_segments_marked(
        &self,
        meeting_gid: &str,
        segs: Vec<NewSegment>,
        overlap_gids: &[String],
    ) -> Result<Vec<Segment>> {
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let tx = conn.transaction()?;
        let out = insert_segments(&tx, &dek, m.id, m.version, &segs)?;
        mark_in(&tx, m.id, overlap_gids)?;
        tx.commit()?;
        Ok(out)
    }

    fn set_sealed_json(
        &self,
        meeting_gid: &str,
        column: &'static str,
        value: Option<&serde_json::Value>,
    ) -> Result<()> {
        let json = value.map(|v| v.to_string());
        if let Some(j) = &json {
            check_json_size(j)?;
        }
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let ct = match &json {
            Some(j) => {
                let dek = self.dek(&conn, m.id)?;
                Some(seal_text(
                    &dek,
                    j,
                    &row_aad("meetings", column, meeting_gid),
                ))
            }
            None => None,
        };
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            &format!("UPDATE meetings SET {column} = ?1, lamport = ?2 WHERE id = ?3"),
            params![ct, lamport, m.id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// `Decrypt` if the key or the AAD is wrong; `Invalid` if what decrypted
    /// is not JSON.
    fn get_sealed_json(
        &self,
        meeting_gid: &str,
        column: &'static str,
    ) -> Result<Option<serde_json::Value>> {
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let ct: Option<Vec<u8>> = conn.query_row(
            &format!("SELECT {column} FROM meetings WHERE id = ?1"),
            [m.id],
            |r| r.get(0),
        )?;
        let Some(ct) = ct else { return Ok(None) };
        let dek = self.dek(&conn, m.id)?;
        let text = open_text(&dek, &ct, &row_aad("meetings", column, meeting_gid))?;
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|_| StoreError::Invalid("stored meeting data is malformed".into()))
    }

    /// Marks the lines (segment gids of this meeting, any transcript version)
    /// as talked over. Returns how many were marked. Lines of other meetings
    /// are ignored.
    pub fn mark_overlaps(&self, meeting_gid: &str, segment_gids: &[String]) -> Result<usize> {
        if segment_gids.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let tx = conn.transaction()?;
        let n = mark_in(&tx, m.id, segment_gids)?;
        tx.commit()?;
        Ok(n)
    }
}

/// Marks lines of meeting `meeting_id` as talked over, in the caller's
/// transaction.
fn mark_in(tx: &rusqlite::Transaction, meeting_id: i64, segment_gids: &[String]) -> Result<usize> {
    if segment_gids.is_empty() {
        return Ok(0);
    }
    let want =
        serde_json::to_string(segment_gids).map_err(|e| StoreError::Invalid(e.to_string()))?;
    let lamport = Store::alloc_lamport(tx, 1)?;
    Ok(tx.execute(
        "UPDATE segments SET overlap = 1, lamport = ?1
         WHERE meeting_id = ?2 AND overlap = 0
           AND gid IN (SELECT value FROM json_each(?3))",
        params![lamport, meeting_id, want],
    )?)
}

// -------------------------------------------------- track speakers encoding

const TS_VERSION: u8 = 1;

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn take_varint(b: &[u8], pos: &mut usize) -> Result<u64> {
    let mut v = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *b.get(*pos).ok_or_else(malformed)?;
        *pos += 1;
        v |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(malformed())
}

fn take_str(b: &[u8], pos: &mut usize) -> Result<String> {
    let n = take_varint(b, pos)? as usize;
    let end = pos
        .checked_add(n)
        .filter(|e| *e <= b.len())
        .ok_or_else(malformed)?;
    let s = std::str::from_utf8(&b[*pos..end]).map_err(|_| malformed())?;
    *pos = end;
    Ok(s.to_string())
}

fn malformed() -> StoreError {
    StoreError::Invalid("stored track speakers are malformed".into())
}

/// Version, count, then per participant: label, speaker gid, span count and
/// the spans as (gap since the previous span's end, length) varints, in ms.
fn encode_track_speakers(speakers: &[TrackSpeaker]) -> Result<Vec<u8>> {
    let mut out = vec![TS_VERSION];
    put_varint(&mut out, speakers.len() as u64);
    for s in speakers {
        for text in [&s.label, &s.speaker_gid] {
            put_varint(&mut out, text.len() as u64);
            out.extend_from_slice(text.as_bytes());
        }
        put_varint(&mut out, s.spans.len() as u64);
        let mut end = 0i64;
        for [t0, t1] in &s.spans {
            if *t0 < end || *t1 < *t0 {
                return Err(StoreError::Invalid(
                    "spans must be in time order and not overlap".into(),
                ));
            }
            put_varint(&mut out, (*t0 - end) as u64);
            put_varint(&mut out, (*t1 - *t0) as u64);
            end = *t1;
        }
        if out.len() > MAX_TRACK_SPEAKERS_BYTES {
            return Err(StoreError::Limit {
                kind: "bytes of participant spans",
                max: MAX_TRACK_SPEAKERS_BYTES,
            });
        }
    }
    Ok(out)
}

fn decode_track_speakers(b: &[u8]) -> Result<Vec<TrackSpeaker>> {
    // The first release stored these as JSON; both read.
    if b.first() == Some(&b'[') {
        return serde_json::from_slice(b).map_err(|_| malformed());
    }
    if b.first() != Some(&TS_VERSION) {
        return Err(malformed());
    }
    let mut pos = 1;
    let n = take_varint(b, &mut pos)? as usize;
    let mut out = Vec::with_capacity(n.min(1024));
    for _ in 0..n {
        let label = take_str(b, &mut pos)?;
        let speaker_gid = take_str(b, &mut pos)?;
        let count = take_varint(b, &mut pos)? as usize;
        let mut spans = Vec::with_capacity(count.min(1 << 20));
        let mut end = 0i64;
        for _ in 0..count {
            let t0 = end + take_varint(b, &mut pos)? as i64;
            let t1 = t0 + take_varint(b, &mut pos)? as i64;
            spans.push([t0, t1]);
            end = t1;
        }
        out.push(TrackSpeaker {
            label,
            speaker_gid,
            spans,
        });
    }
    Ok(out)
}

impl Store {
    /// The paired device that recorded the meeting (`devices.id`); `None`: this
    /// device (or an import). A meeting with a peer origin is processed only
    /// under a lease (doc 07 §8).
    pub fn meeting_audio_origin(&self, meeting_gid: &str) -> Result<Option<i64>> {
        let conn = self.conn();
        Store::meeting_ref(&conn, meeting_gid)?;
        Ok(conn.query_row(
            "SELECT audio_origin FROM meetings WHERE gid = ?1",
            [meeting_gid],
            |r| r.get(0),
        )?)
    }

    /// Marks a meeting as recorded by a peer. Tests only (sync apply sets it
    /// in production).
    #[doc(hidden)]
    pub fn set_meeting_audio_origin_for_tests(
        &self,
        meeting_gid: &str,
        origin: Option<i64>,
    ) -> Result<()> {
        self.conn().execute(
            "UPDATE meetings SET audio_origin = ?1 WHERE gid = ?2",
            params![origin, meeting_gid],
        )?;
        Ok(())
    }

    /// Inserts a holder lease row in `state`. Tests only (the sync layer
    /// opens leases in production).
    #[doc(hidden)]
    pub fn insert_lease_for_tests(
        &self,
        job_uuid: &str,
        meeting_gid: &str,
        epoch: i64,
        state: &str,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO leases (job_uuid, meeting_gid, role, epoch, state, ttl_ms)
             VALUES (?1, ?2, 'holder', ?3, ?4, 1000)",
            params![job_uuid, meeting_gid, epoch, state],
        )?;
        Ok(())
    }
}
