// SPDX-License-Identifier: Apache-2.0
//! People (phase 14c): persons linked from speaker names, Me, and what the
//! People screens show.
//!
//! A person is found by name: renaming a speaker links it to the person whose
//! [`name_key`] matches (NFC, trimmed, lowercased; accents count: "Minh" is
//! not "Mính"), or creates one. Me is a person row (`is_me`, empty name) every
//! Me speaker is linked to. A person's name is stored in plaintext (under
//! SQLCipher only), so persons left with no speaker and no voice profile are
//! removed, and names never go to logs.

use rusqlite::{Connection, OptionalExtension, params};

use crate::fold;
use crate::rowcrypt::{open_text, row_aad, seal_text};
use crate::store::{Store, id_of};
use crate::voice::VoiceConsent;
use crate::{Result, StoreError, new_gid, tombstones};

/// How person names are compared: NFC, trimmed, lowercased. Not accent-folded.
pub fn name_key(name: &str) -> String {
    fold::nfc(name.trim()).to_lowercase()
}

/// The voice profile of a person, as the People list needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceSummary {
    pub profile_gid: String,
    /// `self_checkbox` or `verbal_clip`.
    pub method: String,
    pub consent_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PersonOverview {
    pub gid: String,
    /// Empty for Me.
    pub name: String,
    pub is_me: bool,
    pub color_slot: i64,
    /// Meetings with a (not merged away) speaker linked to the person.
    pub meetings: i64,
    /// Open (not done) action items owned by the person's speakers.
    pub open_actions: i64,
    pub last_met_ms: Option<i64>,
    pub voice: Option<VoiceSummary>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PersonMeeting {
    pub gid: String,
    pub title: String,
    pub started_at: i64,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PersonAction {
    pub gid: String,
    pub meeting_gid: String,
    pub meeting_title: String,
    pub text: String,
    pub due: Option<i64>,
    pub due_text: Option<String>,
}

/// The Me person's row id.
pub(crate) fn me_id(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT id FROM persons WHERE is_me = 1", [], |r| r.get(0))
        .optional()?
        .ok_or_else(|| StoreError::NotFound {
            kind: "persons",
            gid: "me".into(),
        })
}

/// The person (not Me) with this name, or a new one. Returns its row id.
pub(crate) fn find_or_create_person(
    conn: &Connection,
    name: &str,
    color_slot: i64,
    lamport: i64,
) -> Result<i64> {
    let name = fold::nfc(name.trim());
    let key = name_key(&name);
    if let Some(id) = conn
        .query_row(
            "SELECT id FROM persons WHERE name_key = ?1 AND is_me = 0",
            [&key],
            |r| r.get(0),
        )
        .optional()?
    {
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO persons (gid, name, color_slot, lamport, is_me, created_at, name_key)
         VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6)",
        params![
            new_gid(),
            name,
            color_slot,
            lamport,
            crate::store::now_ms(),
            key
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Row ids of the persons linked to the meeting's speakers.
pub(crate) fn persons_of_meeting(conn: &Connection, meeting_id: i64) -> Result<Vec<i64>> {
    let ids = conn
        .prepare_cached(
            "SELECT DISTINCT person_id FROM speakers
             WHERE meeting_id = ?1 AND person_id IS NOT NULL",
        )?
        .query_map([meeting_id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

/// Removes those persons that are not Me and have no speaker (not merged
/// away) and no voice
/// profile (with a tombstone). Returns how many went.
pub(crate) fn gc_persons(conn: &Connection, ids: &[i64]) -> Result<usize> {
    let mut removed = 0;
    for id in ids {
        let gid: Option<String> = conn
            .query_row(
                "SELECT gid FROM persons p WHERE id = ?1 AND is_me = 0
                   AND NOT EXISTS (SELECT 1 FROM speakers s
                                   WHERE s.person_id = p.id AND s.merged_into IS NULL)
                   AND NOT EXISTS (SELECT 1 FROM voice_profiles v WHERE v.person_id = p.id)",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(gid) = gid {
            let lamport = Store::alloc_lamport(conn, 1)?;
            tombstones::write(conn, &gid, "person", lamport)?;
            conn.execute("DELETE FROM persons WHERE id = ?1", [id])?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// `text` (as NFC) with every whole-word, case-sensitive `needle` replaced, or
/// `None` if there was none. A word boundary is any character that is not a
/// letter, digit or combining mark.
fn replace_word(text: &str, needle: &str, with: &str) -> Option<String> {
    let text = fold::nfc(text);
    let needle = fold::nfc(needle);
    if needle.is_empty() {
        return None;
    }
    let in_word =
        |c: char| c.is_alphanumeric() || unicode_normalization::char::is_combining_mark(c);
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut changed = false;
    for (i, _) in text.match_indices(needle.as_str()) {
        let end = i + needle.len();
        if i < last
            || text[..i].chars().next_back().is_some_and(in_word)
            || text[end..].chars().next().is_some_and(in_word)
        {
            continue;
        }
        out.push_str(&text[last..i]);
        out.push_str(with);
        last = end;
        changed = true;
    }
    changed.then(|| {
        out.push_str(&text[last..]);
        out
    })
}

const OVERVIEW: &str = "SELECT p.gid, p.name, p.is_me, p.color_slot,
        (SELECT count(DISTINCT s.meeting_id) FROM speakers s
          WHERE s.person_id = p.id AND s.merged_into IS NULL),
        (SELECT count(*) FROM action_items a JOIN speakers s ON s.id = a.owner_speaker_id
          WHERE s.person_id = p.id AND s.merged_into IS NULL AND a.done = 0),
        (SELECT max(m.started_at) FROM speakers s JOIN meetings m ON m.id = s.meeting_id
          WHERE s.person_id = p.id AND s.merged_into IS NULL),
        v.gid, v.consent_json
     FROM persons p LEFT JOIN voice_profiles v ON v.person_id = p.id";

fn overview_row(r: &rusqlite::Row) -> rusqlite::Result<PersonOverview> {
    let profile: Option<String> = r.get(7)?;
    let consent: Option<String> = r.get(8)?;
    let voice = profile.map(|profile_gid| {
        let c = consent.and_then(|c| VoiceConsent::from_json(&c).ok());
        VoiceSummary {
            profile_gid,
            method: c.as_ref().map_or_else(String::new, |c| c.method.clone()),
            consent_at_ms: c.map_or(0, |c| c.at_ms),
        }
    });
    Ok(PersonOverview {
        gid: r.get(0)?,
        name: r.get(1)?,
        is_me: r.get(2)?,
        color_slot: r.get(3)?,
        meetings: r.get(4)?,
        open_actions: r.get(5)?,
        last_met_ms: r.get(6)?,
        voice,
    })
}

impl Store {
    /// Links every named, unlinked speaker (not Me, not "not a person", not
    /// merged away) to the person with its name. Idempotent; run at launch to
    /// fill `persons` for meetings named before people existed. Returns how
    /// many speakers were linked.
    pub fn link_named_speakers(&self) -> Result<usize> {
        let mut conn = self.conn();
        let rows: Vec<(i64, String, i64, Vec<u8>, i64)> = conn
            .prepare(
                "SELECT s.id, s.gid, s.meeting_id, s.display_name_ct, s.color_slot
                 FROM speakers s
                 WHERE s.display_name_ct IS NOT NULL AND s.person_id IS NULL
                   AND s.is_me = 0 AND s.not_person = 0 AND s.merged_into IS NULL
                 ORDER BY s.id",
            )?
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        if rows.is_empty() {
            return Ok(0);
        }
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        let mut linked = 0;
        for (id, gid, meeting_id, ct, color) in rows {
            // A shredded meeting is about to go: nothing to link.
            let Ok(dek) = self.dek(&tx, meeting_id) else {
                continue;
            };
            let Ok(name) = open_text(&dek, &ct, &row_aad("speakers", "display_name_ct", &gid))
            else {
                continue;
            };
            if name.trim().is_empty() {
                continue;
            }
            let person = find_or_create_person(&tx, &name, color, lamport)?;
            tx.execute(
                "UPDATE speakers SET person_id = ?1, lamport = ?2 WHERE id = ?3",
                params![person, lamport, id],
            )?;
            linked += 1;
        }
        tx.commit()?;
        Ok(linked)
    }

    /// Every person (Me first, then by last meeting) with counts and voice
    /// profile summary, in one query.
    pub fn people_overview(&self) -> Result<Vec<PersonOverview>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!(
            "{OVERVIEW} ORDER BY p.is_me DESC, 7 DESC, p.name COLLATE NOCASE, p.id"
        ))?;
        let rows = stmt.query_map([], overview_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One person's overview row.
    pub fn person(&self, person_gid: &str) -> Result<PersonOverview> {
        self.conn()
            .query_row(
                &format!("{OVERVIEW} WHERE p.gid = ?1"),
                [person_gid],
                overview_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "person",
                gid: person_gid.to_string(),
            })
    }

    /// The person's meetings, newest first.
    pub fn person_meetings(&self, person_gid: &str, limit: usize) -> Result<Vec<PersonMeeting>> {
        let conn = self.conn();
        let pid = id_of(&conn, "persons", person_gid)?;
        type Row = (i64, String, Option<Vec<u8>>, i64, i64);
        let rows: Vec<Row> = conn
            .prepare_cached(
                "SELECT m.id, m.gid, m.title_ct, m.started_at, m.duration_ms FROM meetings m
                 WHERE EXISTS (SELECT 1 FROM speakers s WHERE s.meeting_id = m.id
                               AND s.person_id = ?1 AND s.merged_into IS NULL)
                 ORDER BY m.started_at DESC, m.id DESC LIMIT ?2",
            )?
            .query_map(params![pid, limit as i64], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for (id, gid, ct, started_at, duration_ms) in rows {
            // Shredded meetings are on their way out.
            let Ok(dek) = self.dek(&conn, id) else {
                continue;
            };
            let title = match ct {
                Some(ct) => open_text(&dek, &ct, &row_aad("meetings", "title_ct", &gid))?,
                None => String::new(),
            };
            out.push(PersonMeeting {
                gid,
                title,
                started_at,
                duration_ms,
            });
        }
        Ok(out)
    }

    /// The person's open action items, newest meeting first.
    pub fn person_open_actions(&self, person_gid: &str) -> Result<Vec<PersonAction>> {
        let conn = self.conn();
        let pid = id_of(&conn, "persons", person_gid)?;
        type Row = (
            i64,
            String,
            Option<Vec<u8>>,
            String,
            Vec<u8>,
            Option<i64>,
            Option<Vec<u8>>,
        );
        let rows: Vec<Row> = conn
            .prepare_cached(
                "SELECT m.id, m.gid, m.title_ct, a.gid, a.text_ct, a.due, a.due_text_ct
                 FROM action_items a
                 JOIN speakers s ON s.id = a.owner_speaker_id
                 JOIN meetings m ON m.id = a.meeting_id
                 WHERE s.person_id = ?1 AND s.merged_into IS NULL AND a.done = 0
                 ORDER BY m.started_at DESC, m.id DESC, a.id",
            )?
            .query_map([pid], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for (mid, meeting_gid, title_ct, gid, text_ct, due, due_ct) in rows {
            let Ok(dek) = self.dek(&conn, mid) else {
                continue;
            };
            let meeting_title = match title_ct {
                Some(ct) => open_text(&dek, &ct, &row_aad("meetings", "title_ct", &meeting_gid))?,
                None => String::new(),
            };
            let text = open_text(&dek, &text_ct, &row_aad("action_items", "text_ct", &gid))?;
            let due_text = due_ct
                .map(|c| open_text(&dek, &c, &row_aad("action_items", "due_text_ct", &gid)))
                .transpose()?;
            out.push(PersonAction {
                gid,
                meeting_gid,
                meeting_title,
                text,
                due,
                due_text,
            });
        }
        Ok(out)
    }

    /// Merges person `from` into `into`: `from`'s speakers (and suggestions)
    /// move to `into`, their display names are re-sealed to `into`'s name, the
    /// voice profiles are merged (centroids recomputed; `from`'s is
    /// crypto-shredded when both have one), and `from` is tombstoned. Me can't
    /// be merged. Returns the gids of the meetings whose speaker names
    /// changed (their chunk embeddings carry the old names). Merging people
    /// who have a voice profile (always someone else's) needs `approval`.
    pub fn merge_persons(
        &self,
        from_gid: &str,
        into_gid: &str,
        approval: Option<crate::voice::ThirdPartyApproved>,
    ) -> Result<Vec<String>> {
        let person = |conn: &Connection, gid: &str| -> Result<(i64, String, bool)> {
            conn.query_row(
                "SELECT id, name, is_me FROM persons WHERE gid = ?1",
                [gid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "person",
                gid: gid.to_string(),
            })
        };
        let check = |conn: &Connection| -> Result<(i64, i64, String)> {
            let (from, _, from_me) = person(conn, from_gid)?;
            let (into, into_name, into_me) = person(conn, into_gid)?;
            if from == into {
                return Err(StoreError::Invalid("people must differ".into()));
            }
            if from_me || into_me {
                return Err(StoreError::Invalid("Me can't be merged".into()));
            }
            let has_profile: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM voice_profiles WHERE person_id IN (?1, ?2))",
                params![from, into],
                |r| r.get(0),
            )?;
            if has_profile && approval.is_none() {
                return Err(StoreError::Invalid(
                    "third-party voice profiles are not enabled".into(),
                ));
            }
            Ok((from, into, into_name))
        };
        // Validate first: a refused merge must not start a rotation. With two
        // profiles, the shredded one needs a wrap-secret rotation (started
        // before the change, so a crash leaves it to the next open).
        let both = {
            let conn = self.conn();
            let (from, into, _) = check(&conn)?;
            conn.query_row(
                "SELECT count(*) = 2 FROM voice_profiles WHERE person_id IN (?1, ?2)",
                params![from, into],
                |r| r.get::<_, bool>(0),
            )?
        };
        if both {
            self.begin_wrap_rotation()?;
        }
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (from, into, into_name) = check(&tx)?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        type Row = (i64, String, i64, Option<Vec<u8>>, String);
        let speakers: Vec<Row> = tx
            .prepare(
                "SELECT s.id, s.gid, s.meeting_id, s.display_name_ct, m.gid
                 FROM speakers s JOIN meetings m ON m.id = s.meeting_id
                 WHERE s.person_id = ?1 ORDER BY m.id, s.id",
            )?
            .query_map([from], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut affected: Vec<String> = Vec::new();
        for (id, gid, meeting_id, name_ct, meeting_gid) in speakers {
            let ct = match name_ct {
                Some(_) => match self.dek(&tx, meeting_id) {
                    Ok(dek) => Some(seal_text(
                        &dek,
                        &into_name,
                        &row_aad("speakers", "display_name_ct", &gid),
                    )),
                    // A shredded meeting: its names are gone anyway.
                    Err(StoreError::Decrypt) => None,
                    Err(e) => return Err(e),
                },
                None => None,
            };
            tx.execute(
                "UPDATE speakers SET person_id = ?1, lamport = ?2,
                        display_name_ct = COALESCE(?3, display_name_ct) WHERE id = ?4",
                params![into, lamport, ct, id],
            )?;
            if ct.is_some() && !affected.contains(&meeting_gid) {
                crate::embeddings::bump_index_gen(&tx, meeting_id)?;
                affected.push(meeting_gid);
            }
        }
        tx.execute(
            "UPDATE speakers SET suggest_person_id = ?1 WHERE suggest_person_id = ?2",
            params![into, from],
        )?;
        let profile = |p: i64| -> rusqlite::Result<Option<i64>> {
            tx.query_row(
                "SELECT id FROM voice_profiles WHERE person_id = ?1",
                [p],
                |r| r.get(0),
            )
            .optional()
        };
        let (from_profile, into_profile) = (profile(from)?, profile(into)?);
        let mut shredded = None;
        match (from_profile, into_profile) {
            (Some(f), Some(i)) => {
                self.merge_profile_tx(&tx, f, i, lamport)?;
                shredded = Some(f);
            }
            // Only `from` has one: it becomes `into`'s (same key).
            (Some(f), None) => {
                tx.execute(
                    "UPDATE voice_profiles SET person_id = ?1, lamport = ?2 WHERE id = ?3",
                    params![into, lamport, f],
                )?;
            }
            _ => {}
        }
        let from_person_gid: String =
            tx.query_row("SELECT gid FROM persons WHERE id = ?1", [from], |r| {
                r.get(0)
            })?;
        tombstones::write(&tx, &from_person_gid, "person", lamport)?;
        tx.execute("DELETE FROM persons WHERE id = ?1", [from])?;
        tx.commit()?;
        drop(conn);
        if let Some(f) = shredded {
            self.voice_keys().remove(&f);
            // The merge is done. If the rotation fails the ring stays
            // "rotating" and the next open finishes it, so don't fail the call.
            let _ = self.rotate_wraps(true);
        }
        Ok(affected)
    }

    /// "Remove name from notes" (D8): in every meeting, clears the display
    /// names and the person link of the person's speakers, replaces the name
    /// (whole word, case-sensitive; the person's name and each speaker's own
    /// display name) with that meeting's "Speaker N" in AI-written note blocks
    /// and action items, **without** flipping their provenance (user-written
    /// ones are not touched), and drops the meeting's chunk embeddings (their
    /// text carries the name). The person row stays only if it still has a
    /// voice profile; its voice profile is never touched. Me has no name to
    /// remove. Returns the gids of the meetings that had the person, so the
    /// caller can queue their indexing again.
    pub fn remove_person_name(&self, person_gid: &str) -> Result<Vec<String>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (pid, name, is_me): (i64, String, bool) = tx
            .query_row(
                "SELECT id, name, is_me FROM persons WHERE gid = ?1",
                [person_gid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "person",
                gid: person_gid.to_string(),
            })?;
        if is_me {
            return Err(StoreError::Invalid("Me has no name to remove".into()));
        }
        let lamport = Store::alloc_lamport(&tx, 1)?;
        // Speakers not merged away first: their number is the label.
        type Row = (i64, String, i64, String, i64, Option<Vec<u8>>);
        let speakers: Vec<Row> = tx
            .prepare(
                "SELECT s.id, s.gid, s.meeting_id, m.gid, s.label_idx, s.display_name_ct
                 FROM speakers s JOIN meetings m ON m.id = s.meeting_id
                 WHERE s.person_id = ?1
                 ORDER BY m.id, s.merged_into IS NOT NULL, s.label_idx, s.id",
            )?
            .query_map([pid], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        // meeting row id -> (gid, label, names to replace)
        let mut meetings: Vec<(i64, String, String, Vec<String>)> = Vec::new();
        for (sid, sgid, meeting_id, meeting_gid, label_idx, name_ct) in &speakers {
            let dek = self.dek(&tx, *meeting_id).ok();
            let display = match (&dek, name_ct) {
                (Some(dek), Some(ct)) => {
                    open_text(dek, ct, &row_aad("speakers", "display_name_ct", sgid)).ok()
                }
                _ => None,
            };
            let idx = meetings.iter().position(|m| m.0 == *meeting_id);
            let entry = match idx {
                Some(i) => &mut meetings[i],
                None => {
                    let label = if *label_idx >= 0 {
                        format!("Speaker {}", label_idx + 1)
                    } else {
                        "Speaker".to_string()
                    };
                    meetings.push((*meeting_id, meeting_gid.clone(), label, vec![name.clone()]));
                    meetings.last_mut().unwrap()
                }
            };
            if let Some(d) = display.filter(|d| !entry.3.contains(d)) {
                entry.3.push(d);
            }
            tx.execute(
                "UPDATE speakers SET display_name_ct = NULL, person_id = NULL, lamport = ?1
                 WHERE id = ?2",
                params![lamport, sid],
            )?;
        }
        tx.execute(
            "UPDATE speakers SET suggest_person_id = NULL, suggest_score = NULL
             WHERE suggest_person_id = ?1",
            [pid],
        )?;
        for (meeting_id, _, label, needles) in &mut meetings {
            // Longest first, so "An Nguyen" goes before "An".
            needles.sort_by_key(|n| std::cmp::Reverse(n.len()));
            if let Ok(dek) = self.dek(&tx, *meeting_id) {
                rewrite_ai_notes(&tx, &dek, *meeting_id, needles, label, lamport)?;
            }
            // The chunk text carries speaker names: they are stale now, and an
            // indexer that read the old names can't store its vectors.
            tx.execute(
                "DELETE FROM embeddings WHERE meeting_id = ?1",
                [*meeting_id],
            )?;
            crate::embeddings::bump_index_gen(&tx, *meeting_id)?;
        }
        gc_persons(&tx, &[pid])?;
        tx.commit()?;
        // The FTS5 index keeps the old note tokens (and positions) until
        // merged away: rewrite it now.
        crate::store::compact_locked(&conn)?;
        Ok(meetings.into_iter().map(|m| m.1).collect())
    }

    /// Marks the speaker as Me (and its person as Me): any other Me speaker of
    /// the meeting stops being Me, and a name link to another person is
    /// replaced by the link to Me (the display name stays as written). In a
    /// call with a system track only the mic speaker (`label_idx` -1) can be
    /// Me. When Me moves to another speaker, Me's voice exemplars taken from
    /// this meeting are dropped (they were somebody else's).
    pub fn set_speaker_me(&self, speaker_gid: &str) -> Result<()> {
        self.set_me(speaker_gid, false).map(|_| ())
    }

    /// [`set_speaker_me`](Store::set_speaker_me) for an automatic match: checked
    /// again inside the transaction, it applies only when the speaker is still
    /// unnamed, not "not a person", not merged, can be Me, and nobody else in
    /// the meeting is Me. Returns whether it applied.
    pub fn set_speaker_me_if_unclaimed(&self, speaker_gid: &str) -> Result<bool> {
        self.set_me(speaker_gid, true)
    }

    fn set_me(&self, speaker_gid: &str, only_if_unclaimed: bool) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        #[allow(clippy::type_complexity)]
        let (sid, meeting_id, merged, old_person, label_idx, is_me, not_person, named, call): (
            i64,
            i64,
            bool,
            Option<i64>,
            i64,
            bool,
            bool,
            bool,
            bool,
        ) = tx
            .query_row(
                "SELECT s.id, s.meeting_id, s.merged_into IS NOT NULL, s.person_id, s.label_idx,
                        s.is_me, s.not_person, s.display_name_ct IS NOT NULL,
                        m.mode = 'call' AND EXISTS (
                            SELECT 1 FROM tracks t WHERE t.meeting_id = m.id AND t.kind = 'system')
                 FROM speakers s JOIN meetings m ON m.id = s.meeting_id WHERE s.gid = ?1",
                [speaker_gid],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "speaker",
                gid: speaker_gid.to_string(),
            })?;
        let far_side = call && label_idx != -1 && !is_me;
        if merged {
            if only_if_unclaimed {
                return Ok(false);
            }
            return Err(StoreError::Invalid(
                "speaker was merged into another".into(),
            ));
        }
        if far_side {
            if only_if_unclaimed {
                return Ok(false);
            }
            return Err(StoreError::Invalid(
                "in a call only the mic speaker can be Me".into(),
            ));
        }
        let other_me: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM speakers WHERE meeting_id = ?1 AND is_me = 1
                            AND merged_into IS NULL AND id <> ?2)",
            params![meeting_id, sid],
            |r| r.get(0),
        )?;
        if only_if_unclaimed && (named || not_person || is_me || other_me) {
            return Ok(false);
        }
        let me = me_id(&tx)?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "UPDATE speakers SET is_me = 0, person_id = NULL, lamport = ?1
             WHERE meeting_id = ?2 AND is_me = 1 AND id <> ?3",
            params![lamport, meeting_id, sid],
        )?;
        tx.execute(
            "UPDATE speakers SET is_me = 1, not_person = 0, person_id = ?1,
                    suggest_person_id = NULL, suggest_score = NULL, lamport = ?2
             WHERE id = ?3",
            params![me, lamport, sid],
        )?;
        if let Some(old) = old_person.filter(|p| *p != me) {
            gc_persons(&tx, &[old])?;
        }
        crate::embeddings::bump_index_gen(&tx, meeting_id)?;
        let meeting_gid: String = tx.query_row(
            "SELECT gid FROM meetings WHERE id = ?1",
            [meeting_id],
            |r| r.get(0),
        )?;
        tx.commit()?;
        drop(conn);
        if other_me {
            self.drop_me_exemplars_from(&meeting_gid)?;
        }
        Ok(true)
    }

    /// "Not me": the speaker stops being Me (and its person link goes). Me's
    /// voice exemplars taken from this meeting are dropped (they were not
    /// Me's) and the meeting's index is stale. Refused in a call with a
    /// far-side track, where the mic speaker is Me by construction.
    pub fn clear_speaker_me(&self, speaker_gid: &str) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (sid, meeting_id, is_me, call, meeting_gid): (i64, i64, bool, bool, String) = tx
            .query_row(
                "SELECT s.id, s.meeting_id, s.is_me, m.gid,
                        m.mode = 'call' AND EXISTS (
                            SELECT 1 FROM tracks t WHERE t.meeting_id = m.id AND t.kind = 'system')
                 FROM speakers s JOIN meetings m ON m.id = s.meeting_id WHERE s.gid = ?1",
                [speaker_gid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(4)?, r.get(3)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "speaker",
                gid: speaker_gid.to_string(),
            })?;
        if !is_me {
            return Err(StoreError::Invalid("the speaker is not Me".into()));
        }
        if call {
            return Err(StoreError::Invalid(
                "in a call the mic speaker is always Me".into(),
            ));
        }
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "UPDATE speakers SET is_me = 0, person_id = NULL, lamport = ?1 WHERE id = ?2",
            params![lamport, sid],
        )?;
        crate::embeddings::bump_index_gen(&tx, meeting_id)?;
        tx.commit()?;
        drop(conn);
        self.drop_me_exemplars_from(&meeting_gid)?;
        Ok(())
    }

    /// Sets (or with `None` clears) the "sounds like ..." suggestion of a
    /// speaker. A suggestion is never applied by itself.
    pub fn set_speaker_suggestion(
        &self,
        speaker_gid: &str,
        suggestion: Option<(&str, f32)>,
    ) -> Result<()> {
        self.suggest(speaker_gid, suggestion, false).map(|_| ())
    }

    /// [`set_speaker_suggestion`](Store::set_speaker_suggestion) for an
    /// automatic match: only while the speaker is still unnamed, not Me, not
    /// "not a person" and not merged (checked inside the transaction).
    /// Returns whether it applied.
    pub fn set_speaker_suggestion_if_unnamed(
        &self,
        speaker_gid: &str,
        suggestion: Option<(&str, f32)>,
    ) -> Result<bool> {
        self.suggest(speaker_gid, suggestion, true)
    }

    fn suggest(
        &self,
        speaker_gid: &str,
        suggestion: Option<(&str, f32)>,
        only_if_unnamed: bool,
    ) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (person, score) = match suggestion {
            Some((gid, score)) => (Some(id_of(&tx, "persons", gid)?), Some(f64::from(score))),
            None => (None, None),
        };
        let lamport = Store::alloc_lamport(&tx, 1)?;
        let guard = if only_if_unnamed {
            " AND display_name_ct IS NULL AND is_me = 0 AND not_person = 0 AND merged_into IS NULL"
        } else {
            ""
        };
        let n = tx.execute(
            &format!(
                "UPDATE speakers SET suggest_person_id = ?1, suggest_score = ?2, lamport = ?3
                 WHERE gid = ?4{guard}"
            ),
            params![person, score, lamport, speaker_gid],
        )?;
        if n == 0 && !only_if_unnamed {
            return Err(StoreError::NotFound {
                kind: "speaker",
                gid: speaker_gid.to_string(),
            });
        }
        tx.commit()?;
        Ok(n > 0)
    }
}

/// Replaces `needles` with `label` in the meeting's AI-written (`ai`,
/// `ai_edited`) note blocks and action items, keeping provenance, and
/// re-indexes the changed blocks.
fn rewrite_ai_notes(
    tx: &Connection,
    dek: &crate::rowcrypt::Dek,
    meeting_id: i64,
    needles: &[String],
    label: &str,
    lamport: i64,
) -> Result<()> {
    let replace_all = |text: &str| -> Option<String> {
        let mut cur: Option<String> = None;
        for n in needles {
            if let Some(next) = replace_word(cur.as_deref().unwrap_or(text), n, label) {
                cur = Some(next);
            }
        }
        cur
    };
    let blocks: Vec<(i64, String, Vec<u8>)> = tx
        .prepare(
            "SELECT id, gid, body_ct FROM notes_blocks
             WHERE meeting_id = ?1 AND provenance IN ('ai', 'ai_edited')",
        )?
        .query_map([meeting_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, gid, ct) in blocks {
        let aad = row_aad("notes_blocks", "body_ct", &gid);
        let body = open_text(dek, &ct, &aad)?;
        let Some(new) = replace_all(&body) else {
            continue;
        };
        tx.execute(
            "UPDATE notes_blocks SET body_ct = ?1, lamport = ?2 WHERE id = ?3",
            params![seal_text(dek, &new, &aad), lamport, id],
        )?;
        tx.execute("DELETE FROM notes_fts WHERE rowid = ?1", [id])?;
        let norm = fold::fold(&new);
        if !norm.is_empty() {
            tx.execute(
                "INSERT INTO notes_fts (rowid, body_norm) VALUES (?1, ?2)",
                params![id, norm],
            )?;
        }
    }
    let actions: Vec<(i64, String, Vec<u8>)> = tx
        .prepare(
            "SELECT id, gid, text_ct FROM action_items
             WHERE meeting_id = ?1 AND provenance IN ('ai', 'ai_edited')",
        )?
        .query_map([meeting_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, gid, ct) in actions {
        let aad = row_aad("action_items", "text_ct", &gid);
        let text = open_text(dek, &ct, &aad)?;
        let Some(new) = replace_all(&text) else {
            continue;
        };
        tx.execute(
            "UPDATE action_items SET text_ct = ?1, lamport = ?2 WHERE id = ?3",
            params![seal_text(dek, &new, &aad), lamport, id],
        )?;
    }
    Ok(())
}

/// Per-person lookups for tests and callers that hold only a name.
impl Store {
    /// The person (not Me) whose name matches `name` by [`name_key`].
    pub fn find_person_by_name(&self, name: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT gid FROM persons WHERE name_key = ?1 AND is_me = 0",
                [name_key(name)],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The Me person's gid.
    pub fn me_person(&self) -> Result<String> {
        Ok(self
            .conn()
            .query_row("SELECT gid FROM persons WHERE is_me = 1", [], |r| r.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_word_case_sensitive_replace() {
        assert_eq!(
            replace_word("An said An's idea; Anh agreed. (An)", "An", "Speaker 2").as_deref(),
            Some("Speaker 2 said Speaker 2's idea; Anh agreed. (Speaker 2)")
        );
        assert_eq!(replace_word("an and Anh", "An", "X"), None);
        assert_eq!(replace_word("Mính", "Minh", "X"), None);
        assert_eq!(
            replace_word("Minh, Minh.", "Minh", "S").as_deref(),
            Some("S, S.")
        );
        assert_eq!(replace_word("MinhX", "Minh", "S"), None);
    }

    #[test]
    fn replace_works_on_nfc_and_treats_marks_as_word_characters() {
        // NFD body, NFC needle.
        assert_eq!(
            replace_word("Ha\u{0300} va Ha\u{0300}ng", "H\u{e0}", "S").as_deref(),
            Some("S va Hàng")
        );
        // A combining mark right after the name keeps it part of a longer word.
        assert_eq!(replace_word("Minh\u{0323} nói", "Minh", "S"), None);
    }

    #[test]
    fn names_compare_nfc_trimmed_lowercase_not_accent_folded() {
        assert_eq!(name_key("  MINH "), name_key("minh"));
        assert_ne!(name_key("Minh"), name_key("Mính"));
        // Precomposed vs decomposed.
        assert_eq!(name_key("Mi\u{0301}nh"), name_key("M\u{00ed}nh"));
        assert_eq!(name_key("Đức"), name_key("đức"));
    }
}
