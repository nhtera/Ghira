// SPDX-License-Identifier: Apache-2.0
//! Transcript-chunk embeddings for semantic search.
//!
//! A meeting's transcript is cut into ~60 s chunks, each embedded by one
//! model. The vectors encode what was said, so they are sealed under the
//! meeting DEK (AAD binds the meeting gid, chunk and model): a crypto-shred
//! makes them unreadable, and the rows go with the meeting (cascade). They
//! are derived data, tied to a `transcript_version` and a model id; rows of
//! an older transcript version are never served and are rebuilt by the
//! indexer ([`Store::meetings_needing_embeddings`]), as are rows built from an
//! older `index_gen` (the chunk text changed: a rename, a line edit).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use rusqlite::{OptionalExtension, params};
use zeroize::Zeroize;

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

/// What a cached meeting was read at: its gid (row ids can be reused),
/// transcript version, index generation, row count and the sum of the chunk
/// numbers (the last two come from the `embeddings_model` index alone, so a
/// check never reads the sealed vectors). Any edit or re-index that changes
/// the rows changes this ([`Store::put_embeddings`] also drops the entry, for
/// a rebuild that lands on the same stamp).
type Stamp = (String, i64, i64, i64, i64);

/// The most decrypted vector bytes [`Store::embedding_index`] keeps (512 MB,
/// ~125k chunks of 1024 floats). Past it a call still works, reading the
/// remaining meetings as the uncached path did, and keeps nothing more.
/// (Semantic search needs the `embeddings` feature, which the iOS app does
/// not build, so there is no smaller phone budget.)
const CACHE_BUDGET_BYTES: usize = 512 * 1024 * 1024;

/// A meeting's decrypted chunk vectors; zeroed when the last holder drops it.
pub struct CachedRows(Vec<StoredEmbedding>);

impl CachedRows {
    fn bytes(&self) -> usize {
        self.0.iter().map(|e| e.vec.len() * 4).sum()
    }
}

impl std::ops::Deref for CachedRows {
    type Target = [StoredEmbedding];
    fn deref(&self) -> &[StoredEmbedding] {
        &self.0
    }
}

impl Drop for CachedRows {
    fn drop(&mut self) {
        for e in &mut self.0 {
            e.vec.zeroize();
        }
    }
}

/// Decrypted vectors by (meeting rowid, model).
pub(crate) type EmbeddingCache = HashMap<(i64, String), (Stamp, Arc<CachedRows>)>;

/// Drops a meeting's cached vectors (its key is gone, or its rows changed).
/// A query that already holds the `Arc` keeps its copy until it is done.
pub(crate) fn forget(store: &Store, meeting_id: i64) {
    store
        .emb_cache
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .retain(|(id, _), _| *id != meeting_id);
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

/// Marks the meeting's chunk text as changed (speaker names, line text, who
/// spoke): vectors built from an older generation are stale, and an indexer
/// that read the older one can't store ([`Store::put_embeddings`]).
pub(crate) fn bump_index_gen(conn: &rusqlite::Connection, meeting_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE meetings SET index_gen = index_gen + 1 WHERE id = ?1",
        [meeting_id],
    )?;
    Ok(())
}

/// Statuses of a meeting whose transcript is complete enough to index.
const INDEXABLE: &str = "m.status IN ('ready', 'done')";

impl Store {
    /// The meeting's index generation (see [`Store::put_embeddings`]).
    pub fn index_gen(&self, meeting_gid: &str) -> Result<i64> {
        self.conn()
            .query_row(
                "SELECT index_gen FROM meetings WHERE gid = ?1",
                [meeting_gid],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "meeting",
                gid: meeting_gid.to_string(),
            })
    }

    /// Replaces the meeting's rows for `model` with `chunks`, in one
    /// transaction. `version` is the transcript version the chunks were cut
    /// from, and `index_gen` the [`Store::index_gen`] read *before* the text
    /// was read: if the meeting's generation moved on (a rename, an edit, a
    /// name removal), nothing is stored and the result is
    /// [`StoreError::IndexStale`]. Empty `chunks` just removes the model's rows.
    pub fn put_embeddings(
        &self,
        meeting_gid: &str,
        model: &str,
        version: i64,
        index_gen: i64,
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
        let current: i64 = tx.query_row(
            "SELECT index_gen FROM meetings WHERE id = ?1",
            [m.id],
            |r| r.get(0),
        )?;
        if current != index_gen {
            return Err(StoreError::IndexStale);
        }
        tx.execute(
            "DELETE FROM embeddings WHERE meeting_id = ?1 AND model = ?2",
            params![m.id, model],
        )?;
        for c in &chunks {
            let ct = rowcrypt::seal(&key, &encode(&c.vec), &aad(meeting_gid, c.chunk, model));
            tx.execute(
                "INSERT INTO embeddings
                     (meeting_id, chunk, t0_ms, t1_ms, transcript_version, model, dim, vec_ct,
                      index_gen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    m.id, c.chunk, c.t0_ms, c.t1_ms, version, model, dim as i64, ct, index_gen
                ],
            )?;
        }
        tx.commit()?;
        forget(self, m.id);
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

    /// [`Store::all_embeddings`] per meeting, kept decrypted in memory so a
    /// query pays one cheap stamp query instead of decrypting every vector:
    /// only meetings whose rows changed since the last call are read again.
    /// Entries go with the meeting's key ([`Store::shred_key`]) and with
    /// meetings that no longer have rows. Meetings whose key is gone are
    /// skipped, as in [`Store::all_embeddings`]. At most
    /// [`CACHE_BUDGET_BYTES`] stay cached, and the cache is emptied by
    /// [`Store::clear_embedding_cache`] (the app lock).
    pub fn embedding_index(&self, model: &str) -> Result<Vec<Arc<CachedRows>>> {
        let conn = self.conn();
        let metas = Self::embedding_stamps(&conn, model)?;
        let live: HashSet<i64> = metas.iter().map(|m| m.0).collect();
        let mut cache = self.emb_cache.lock().unwrap_or_else(|p| p.into_inner());
        let epoch = self.emb_epoch.load(Ordering::SeqCst);
        cache.retain(|(id, m), _| m == model && live.contains(id));
        let mut used: usize = cache.values().map(|(_, r)| r.bytes()).sum();
        let mut out = Vec::with_capacity(metas.len());
        for (id, gid, stamp) in metas {
            let key = (id, model.to_string());
            if let Some((at, rows)) = cache.get(&key)
                && *at == stamp
            {
                out.push(rows.clone());
                continue;
            }
            cache.remove(&key);
            let Some(rows) = self.read_rows(&conn, id, &gid, &stamp, model)? else {
                continue;
            };
            if used + rows.bytes() <= CACHE_BUDGET_BYTES {
                used += rows.bytes();
                cache.insert(key, (stamp, rows.clone()));
            }
            out.push(rows);
        }
        // The app locked while this read: the query keeps its rows, nothing stays.
        if self.emb_epoch.load(Ordering::SeqCst) != epoch {
            cache.clear();
        }
        Ok(out)
    }

    /// Each meeting with vectors of `model`, with what they were read at.
    fn embedding_stamps(
        conn: &rusqlite::Connection,
        model: &str,
    ) -> Result<Vec<(i64, String, Stamp)>> {
        let mut stmt = conn.prepare_cached(
            "SELECT m.id, m.gid, m.transcript_version, m.index_gen, e.n, e.s
             FROM (SELECT meeting_id, count(*) AS n, sum(chunk) AS s
                   FROM embeddings WHERE model = ?1 GROUP BY meeting_id) e
             JOIN meetings m ON m.id = e.meeting_id
             ORDER BY m.id",
        )?;
        let mut metas: Vec<(i64, String, Stamp)> = stmt
            .query_map([model], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    (String::new(), r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?),
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        for m in &mut metas {
            m.2.0 = m.1.clone();
        }
        Ok(metas)
    }

    /// One meeting's decrypted rows at `stamp`; `None` when its key is gone.
    fn read_rows(
        &self,
        conn: &rusqlite::Connection,
        id: i64,
        gid: &str,
        stamp: &Stamp,
        model: &str,
    ) -> Result<Option<Arc<CachedRows>>> {
        let Ok(dek) = self.dek(conn, id) else {
            return Ok(None);
        };
        let mut rows_stmt = conn.prepare_cached(
            "SELECT chunk, t0_ms, t1_ms, dim, vec_ct FROM embeddings
             WHERE meeting_id = ?1 AND model = ?2 AND transcript_version = ?3
             ORDER BY chunk",
        )?;
        let raw: Vec<(u32, i64, i64, i64, Vec<u8>)> = rows_stmt
            .query_map(params![id, model, stamp.1], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let rows: Vec<StoredEmbedding> = raw
            .into_iter()
            .filter_map(|(chunk, t0_ms, t1_ms, dim, ct)| {
                Some(StoredEmbedding {
                    meeting_gid: gid.to_string(),
                    chunk,
                    t0_ms,
                    t1_ms,
                    vec: open_vec(&dek, gid, chunk, model, dim, &ct).ok()?,
                })
            })
            .collect();
        Ok(Some(Arc::new(CachedRows(rows))))
    }

    /// Fills the cache [`Store::embedding_index`] serves from, off the query
    /// path: the connection is taken per meeting, never held across the
    /// whole read, so a recording's writes and a search interleave with it.
    /// Stops (cache kept as far as it got) once `keep_going` is false; that
    /// runs without the connection, so it may look at anything. `allowed`
    /// runs after the connection and the cache are taken, right before each
    /// meeting is read: it must only read a flag (the app lock), never take a
    /// lock of its own. A [`Store::clear_embedding_cache`] that lands
    /// meanwhile empties what this call put in. Returns how many meetings it
    /// read.
    pub fn warm_embedding_index(
        &self,
        model: &str,
        keep_going: &dyn Fn() -> bool,
        allowed: &dyn Fn() -> bool,
    ) -> Result<usize> {
        let metas = Self::embedding_stamps(&self.conn(), model)?;
        let live: HashSet<i64> = metas.iter().map(|m| m.0).collect();
        // Entries of other models and gone meetings don't count against the budget.
        let mut used: usize = {
            let mut cache = self.emb_cache.lock().unwrap_or_else(|p| p.into_inner());
            cache.retain(|(id, m), _| m == model && live.contains(id));
            cache.values().map(|(_, r)| r.bytes()).sum()
        };
        let mut read = 0;
        for (id, gid, stamp) in metas {
            if !keep_going() {
                break;
            }
            let conn = self.conn();
            let mut cache = self.emb_cache.lock().unwrap_or_else(|p| p.into_inner());
            let epoch = self.emb_epoch.load(Ordering::SeqCst);
            if !allowed() {
                break;
            }
            let key = (id, model.to_string());
            if cache.get(&key).is_some_and(|(at, _)| *at == stamp) {
                continue;
            }
            if let Some((_, old)) = cache.remove(&key) {
                used -= old.bytes();
            }
            let Some(rows) = self.read_rows(&conn, id, &gid, &stamp, model)? else {
                continue;
            };
            read += 1;
            if self.emb_epoch.load(Ordering::SeqCst) != epoch {
                cache.clear();
                break;
            }
            if used + rows.bytes() <= CACHE_BUDGET_BYTES {
                used += rows.bytes();
                cache.insert(key, (stamp, rows));
            }
        }
        Ok(read)
    }

    /// How many meetings' vectors are cached right now.
    pub fn cached_embedding_meetings(&self) -> usize {
        self.emb_cache
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }

    /// Empties the decrypted-vector cache (the app locks). Never waits for
    /// the connection, and waits for the cache only if it is free: a read
    /// that holds it empties it again when it finishes (see `emb_epoch`), so
    /// this is safe on the phone's main thread. A query already running
    /// keeps the `Arc`s it holds until it finishes.
    pub fn clear_embedding_cache(&self) {
        self.emb_epoch.fetch_add(1, Ordering::SeqCst);
        match self.emb_cache.try_lock() {
            Ok(mut c) => c.clear(),
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner().clear(),
            Err(std::sync::TryLockError::WouldBlock) => {}
        }
    }

    /// Returns SQLite's page cache (decrypted pages) to the allocator. Takes
    /// the connection, so call it off the UI thread. The pages are wiped as
    /// they are freed (`cipher_memory_security`, set in `db.rs`).
    pub fn release_page_cache(&self) {
        let conn = self.conn();
        let _ = conn.execute_batch("PRAGMA shrink_memory;");
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
                                 AND e.transcript_version = m.transcript_version
                                 AND e.index_gen = m.index_gen)
             ORDER BY m.started_at DESC, m.id DESC
             LIMIT ?2"
        ))?;
        let gids = stmt
            .query_map(params![model, limit as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(gids)
    }
}
