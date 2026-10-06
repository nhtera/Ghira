// SPDX-License-Identifier: Apache-2.0
//! Conflict copies (slice 15-C2, doc 07 §7.5): the losing side of a concurrent
//! free-text edit, sealed under the meeting key, with a deterministic gid.
//!
//! The copies are made by the merge ([`super::apply`]); this module lists and
//! resolves them.

use rusqlite::{OptionalExtension, params};

use super::records::origin_gid;
use super::records::own_gid;
use crate::rowcrypt::{open_text, row_aad, seal_text};
use crate::store::Store;
use crate::tombstones::{self, Cause};
use crate::{Result, StoreError, fold};

/// A conflict copy, decrypted for the UI.
#[derive(Debug, Clone, PartialEq)]
pub struct ConflictCopy {
    pub gid: String,
    pub meeting_gid: String,
    pub target_kind: String,
    pub target_gid: String,
    pub field: String,
    /// Device gid it was written on.
    pub origin: String,
    pub text: String,
}

impl Store {
    /// The open conflict copies of a meeting, oldest first.
    pub fn conflict_copies(&self, meeting_gid: &str) -> Result<Vec<ConflictCopy>> {
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let own = own_gid(&conn)?;
        #[allow(clippy::type_complexity)]
        let rows: Vec<(String, String, String, String, Vec<u8>, Option<i64>)> = conn
            .prepare(
                "SELECT gid, target_kind, target_gid, field, value_ct, origin
                 FROM conflict_copies WHERE meeting_id = ?1 ORDER BY created_at, gid",
            )?
            .query_map([m.id], |r| {
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
        let mut out = Vec::with_capacity(rows.len());
        for (gid, target_kind, target_gid, field, ct, origin) in rows {
            let text = open_text(&dek, &ct, &row_aad("conflict_copies", "value_ct", &gid))?;
            out.push(ConflictCopy {
                origin: origin_gid(&conn, origin, &own)?,
                gid,
                meeting_gid: meeting_gid.to_string(),
                target_kind,
                target_gid,
                field,
                text,
            });
        }
        Ok(out)
    }

    /// "Use this" (`use_it`) writes the copy's text into its target (as a new
    /// local edit); either way the copy is tombstoned.
    pub fn resolve_conflict(&self, gid: &str, use_it: bool) -> Result<()> {
        let mut conn = self.conn();
        let row: Option<(i64, String, String, String, Vec<u8>)> = conn
            .query_row(
                "SELECT meeting_id, target_kind, target_gid, field, value_ct
                 FROM conflict_copies WHERE gid = ?1",
                [gid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some((meeting_id, kind, target, field, ct)) = row else {
            return Err(StoreError::NotFound {
                kind: "conflict copy",
                gid: gid.to_string(),
            });
        };
        let dek = self.dek(&conn, meeting_id)?;
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        if use_it {
            let text = fold::nfc(&open_text(
                &dek,
                &ct,
                &row_aad("conflict_copies", "value_ct", gid),
            )?);
            let (table, fts) = match kind.as_str() {
                "meeting" => ("meetings", None),
                "speaker" => ("speakers", None),
                "segment" => ("segments", Some(("segments_fts", "text_norm"))),
                "note" => ("notes_blocks", Some(("notes_fts", "body_norm"))),
                "action_item" => ("action_items", None),
                _ => return Err(StoreError::Invalid("unknown conflict target".into())),
            };
            if !super::apply::copy_target_ok(&kind, &field) {
                return Err(StoreError::Invalid("unknown conflict field".into()));
            }
            let sealed = seal_text(&dek, &text, &row_aad(table, &field, &target));
            // `field` is one of the fixed names checked above.
            let n = tx.execute(
                &format!(
                    "UPDATE {table} SET {field} = ?1, lamport = ?2, origin = NULL
                         {} WHERE gid = ?3",
                    match kind.as_str() {
                        "note" => ", provenance = CASE provenance WHEN 'ai' THEN 'ai_edited' ELSE provenance END",
                        "segment" => ", edited = 1",
                        _ => "",
                    }
                ),
                params![sealed, lamport, target],
            )?;
            if n == 0 {
                // The target was superseded or deleted: keep the copy so the
                // chosen text is not lost silently.
                return Err(StoreError::NotFound {
                    kind: "conflict target",
                    gid: target,
                });
            }
            if let Some((fts, col)) = fts {
                let id: i64 = tx.query_row(
                    &format!("SELECT id FROM {table} WHERE gid = ?1"),
                    [&target],
                    |r| r.get(0),
                )?;
                tx.execute(&format!("DELETE FROM {fts} WHERE rowid = ?1"), [id])?;
                let norm = fold::fold(&text);
                if !norm.is_empty() {
                    tx.execute(
                        &format!("INSERT INTO {fts} (rowid, {col}) VALUES (?1, ?2)"),
                        params![id, norm],
                    )?;
                }
            }
            crate::embeddings::bump_index_gen(&tx, meeting_id)?;
        }
        tombstones::write(&tx, gid, "conflict_copy", lamport, Cause::User)?;
        tx.execute("DELETE FROM conflict_copies WHERE gid = ?1", [gid])?;
        tx.commit()?;
        Ok(())
    }
}
