// SPDX-License-Identifier: Apache-2.0
//! Transcript-chunk embeddings for semantic search.
//!
//! A meeting's transcript is cut into ~60 s chunks, each embedded by one
//! model. The vectors encode what was said, so they are sealed under the
//! meeting DEK (AAD binds the meeting gid, chunk and model): a crypto-shred
//! makes them unreadable, and the rows go with the meeting (cascade). They
//! are derived data, tied to a `transcript_version` and a model id; rows of
//! an older transcript version are never served and are rebuilt by the
//! indexer ([`Store::meetings_needing_embeddings`]).

use rusqlite::params;

use crate::rowcrypt::{self, Dek};
use crate::store::Store;
use crate::{Result, StoreError};

/// One embedded chunk of a meeting's transcript.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingChunk {
    pub chunk: u32,
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub vec: Vec<f32>,
}

/// A chunk with the meeting it belongs to ([`Store::all_embeddings`]).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEmbedding {
    pub meeting_gid: String,
    pub chunk: u32,
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub vec: Vec<f32>,
}

fn aad(meeting_gid: &str, chunk: u32, model: &str) -> Vec<u8> {
    format!("embeddings.vec_ct:{meeting_gid}:{chunk}:{model}").into_bytes()
}

fn encode(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn decode(bytes: &[u8], dim: usize) -> Result<Vec<f32>> {
    if bytes.len() != dim * 4 {
        return Err(StoreError::Decrypt);
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

fn open_vec(
    dek: &Dek,
    gid: &str,
    chunk: u32,
    model: &str,
    dim: i64,
    ct: &[u8],
) -> Result<Vec<f32>> {
    let plain = rowcrypt::open(
        &dek.subkey(rowcrypt::ROWS_INFO),
        ct,
        &aad(gid, chunk, model),
    )?;
    decode(
        &plain,
        usize::try_from(dim).map_err(|_| StoreError::Decrypt)?,
    )
}

/// Statuses of a meeting whose transcript is complete enough to index.
const INDEXABLE: &str = "m.status IN ('ready', 'done')";

impl Store {
    /// Replaces the meeting's rows for `model` with `chunks`, in one
    /// transaction. `version` is the transcript version the chunks were cut
    /// from. Empty `chunks` just removes the model's rows.
    pub fn put_embeddings(
        &self,
        meeting_gid: &str,
        model: &str,
        version: i64,
        chunks: Vec<EmbeddingChunk>,
    ) -> Result<()> {
        let dim = chunks.first().map_or(0, |c| c.vec.len());
        if chunks.iter().any(|c| c.vec.len() != dim || dim == 0) {
            return Err(StoreError::Invalid(
                "embedding vectors must be non-empty and of one size".into(),
            ));
        }
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let key = dek.subkey(rowcrypt::ROWS_INFO);
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM embeddings WHERE meeting_id = ?1 AND model = ?2",
            params![m.id, model],
        )?;
        for c in &chunks {
            let ct = rowcrypt::seal(&key, &encode(&c.vec), &aad(meeting_gid, c.chunk, model));
            tx.execute(
                "INSERT INTO embeddings
                     (meeting_id, chunk, t0_ms, t1_ms, transcript_version, model, dim, vec_ct)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    m.id, c.chunk, c.t0_ms, c.t1_ms, version, model, dim as i64, ct
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The meeting's chunks for `model` of its current transcript version,
    /// in order. A shredded meeting gives [`StoreError::Decrypt`].
    pub fn embeddings(&self, meeting_gid: &str, model: &str) -> Result<Vec<EmbeddingChunk>> {
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let mut stmt = conn.prepare_cached(
            "SELECT chunk, t0_ms, t1_ms, dim, vec_ct FROM embeddings
             WHERE meeting_id = ?1 AND model = ?2 AND transcript_version = ?3
             ORDER BY chunk",
        )?;
        let rows: Vec<(u32, i64, i64, i64, Vec<u8>)> = stmt
            .query_map(params![m.id, model, m.version], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let dek = self.dek(&conn, m.id)?;
        rows.into_iter()
            .map(|(chunk, t0_ms, t1_ms, dim, ct)| {
                Ok(EmbeddingChunk {
                    chunk,
                    t0_ms,
                    t1_ms,
                    vec: open_vec(&dek, meeting_gid, chunk, model, dim, &ct)?,
                })
            })
            .collect()
    }

    /// Every chunk of every meeting for `model` (current transcript versions
    /// only), for the search index. Meetings whose key is gone, and rows that
    /// do not authenticate, are skipped.
    pub fn all_embeddings(&self, model: &str) -> Result<Vec<StoredEmbedding>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT m.id, m.gid, e.chunk, e.t0_ms, e.t1_ms, e.dim, e.vec_ct
             FROM embeddings e JOIN meetings m ON m.id = e.meeting_id
             WHERE e.model = ?1 AND e.transcript_version = m.transcript_version
             ORDER BY m.id, e.chunk",
        )?;
        let rows = stmt.query_map([model], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, Vec<u8>>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        // The key of the meeting being read; `None`: shredded, skip its rows.
        let mut current: Option<(i64, Option<Dek>)> = None;
        for row in rows {
            let (id, gid, chunk, t0_ms, t1_ms, dim, ct) = row?;
            if current.as_ref().is_none_or(|(cid, _)| *cid != id) {
                current = Some((id, self.dek(&conn, id).ok()));
            }
            let Some((_, Some(dek))) = &current else {
                continue;
            };
            if let Ok(vec) = open_vec(dek, &gid, chunk, model, dim, &ct) {
                out.push(StoredEmbedding {
                    meeting_gid: gid,
                    chunk,
                    t0_ms,
                    t1_ms,
                    vec,
                });
            }
        }
        Ok(out)
    }

    /// Gids of finished meetings with a transcript but no embeddings of
    /// `model` for its current version, newest first.
    pub fn meetings_needing_embeddings(&self, model: &str, limit: usize) -> Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT m.gid FROM meetings m
             WHERE {INDEXABLE}
               AND EXISTS (SELECT 1 FROM segments s
                           WHERE s.meeting_id = m.id AND s.version = m.transcript_version)
               AND NOT EXISTS (SELECT 1 FROM embeddings e
                               WHERE e.meeting_id = m.id AND e.model = ?1
                                 AND e.transcript_version = m.transcript_version)
             ORDER BY m.started_at DESC, m.id DESC
             LIMIT ?2"
        ))?;
        let gids = stmt
            .query_map(params![model, limit as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(gids)
    }
}
