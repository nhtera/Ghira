// SPDX-License-Identifier: Apache-2.0
//! Final-pass checkpoints: the sealed outputs of the parts of a final pass that
//! finished (one ASR chunk of a track, the diarization), so a pass that
//! yielded or was killed resumes instead of starting over.
//!
//! Derived from the meeting's audio, so like the waveform they are sealed
//! under the meeting DEK (AAD binds gid, stamp and part): a crypto-shred makes
//! them unreadable and the rows go with the meeting (cascade). They are also
//! removed with the audio ([`Store::delete_audio`], the retention sweep) and
//! never written for a sensitive meeting or one without audio.
//!
//! A `stamp` names what the rows were computed from (audio, engine, chunking;
//! the caller hashes it). Reading or writing with another stamp drops the rows
//! of the old one, so rows of two different computations are never mixed.

use std::collections::HashMap;

use rusqlite::params;

use crate::rowcrypt::{self, row_aad};
use crate::store::Store;
use crate::{Result, StoreError};

fn check_token(what: &str, s: &str) -> Result<()> {
    if s.is_empty()
        || s.len() > 96
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b':' | b'-'))
    {
        return Err(StoreError::Invalid(format!("bad checkpoint {what}")));
    }
    Ok(())
}

fn aad(gid: &str, stamp: &str, part: &str) -> Vec<u8> {
    row_aad(
        "final_pass_ckpt",
        "data_ct",
        &format!("{gid}:{stamp}:{part}"),
    )
}

impl Store {
    /// Saves one finished part of a meeting's final pass (replacing the part's
    /// earlier row). Rows of another `stamp` are dropped in the same step.
    /// Does nothing without audio or for a sensitive meeting; returns whether
    /// a row was written.
    pub fn put_pass_checkpoint(
        &self,
        meeting_gid: &str,
        stamp: &str,
        part: &str,
        data: &[u8],
    ) -> Result<bool> {
        check_token("stamp", stamp)?;
        check_token("part", part)?;
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let ct = rowcrypt::seal(&dek, data, &aad(meeting_gid, stamp, part));
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM final_pass_ckpt WHERE meeting_id = ?1 AND stamp <> ?2",
            params![m.id, stamp],
        )?;
        let n = tx.execute(
            "INSERT INTO final_pass_ckpt (meeting_id, part, stamp, data_ct)
             SELECT ?1, ?2, ?3, ?4 WHERE EXISTS (SELECT 1 FROM tracks WHERE meeting_id = ?1)
               AND NOT EXISTS (SELECT 1 FROM meetings WHERE id = ?1 AND sensitive = 1)
             ON CONFLICT (meeting_id, part) DO UPDATE SET stamp = excluded.stamp, data_ct = excluded.data_ct",
            params![m.id, part, stamp, ct],
        )?;
        tx.commit()?;
        Ok(n > 0)
    }

    /// The parts saved under `stamp`, by part name. Rows of another stamp are
    /// dropped; a row that does not open (a wrong key, damage) is dropped and
    /// left out: the part is simply computed again.
    pub fn pass_checkpoints(
        &self,
        meeting_gid: &str,
        stamp: &str,
    ) -> Result<HashMap<String, Vec<u8>>> {
        check_token("stamp", stamp)?;
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        conn.execute(
            "DELETE FROM final_pass_ckpt WHERE meeting_id = ?1 AND stamp <> ?2",
            params![m.id, stamp],
        )?;
        let rows: Vec<(String, Vec<u8>)> = conn
            .prepare("SELECT part, data_ct FROM final_pass_ckpt WHERE meeting_id = ?1")?
            .query_map([m.id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        if rows.is_empty() {
            return Ok(HashMap::new());
        }
        let dek = self.dek(&conn, m.id)?;
        let mut out = HashMap::new();
        for (part, ct) in rows {
            match rowcrypt::open(&dek, &ct, &aad(meeting_gid, stamp, &part)) {
                Ok(d) => {
                    out.insert(part, d);
                }
                Err(_) => {
                    conn.execute(
                        "DELETE FROM final_pass_ckpt WHERE meeting_id = ?1 AND part = ?2",
                        params![m.id, part],
                    )?;
                }
            }
        }
        Ok(out)
    }

    /// Drops every checkpoint of a meeting (the pass finished, or its inputs
    /// changed). Returns how many rows went.
    pub fn clear_pass_checkpoints(&self, meeting_gid: &str) -> Result<usize> {
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        Ok(conn.execute("DELETE FROM final_pass_ckpt WHERE meeting_id = ?1", [m.id])?)
    }
}
