// SPDX-License-Identifier: Apache-2.0
//! Accent-insensitive full-text search over transcripts and notes.
//!
//! The indexes are contentless FTS5 tables over [`fold`]ed text, so a query is
//! folded the same way (`dong` finds `đồng`, `da nang` finds `Đà Nẵng`). The
//! index has no text, so every hit is decrypted with its meeting's DEK (cached
//! in memory, zeroized on drop) and highlights are computed on the decrypted
//! original: char offsets into the stored (NFC) text, mapped through
//! [`fold::Folded`] if folding ever changes the length.
//!
//! # Query safety
//!
//! User input never reaches FTS5 as syntax. The query is folded, cut into
//! alphanumeric tokens, and each token is emitted as a quoted string
//! (`"tok"`), the last one as a prefix (`"tok"*`) for search-as-you-type.
//! Tokens can't contain a quote, `*`, `:`, `-`, `NEAR`, `AND`... those are
//! just separators or quoted words.
//!
//! # Ranking
//!
//! BM25 over the candidates FTS returns. If the query itself has diacritics
//! (`đồng`), hits whose original text also contains the exact accented words
//! are ranked ahead of accent-folded matches (an exact-diacritic tier).
//! Candidates are the best [`CANDIDATE_WINDOW`] matches per index by BM25,
//! independent of the page requested, so paging is stable.

use std::collections::HashMap;
use std::ops::Range;

use rusqlite::params_from_iter;
use rusqlite::types::Value;

use crate::fold;
use crate::rowcrypt::{open_text, row_aad};
use crate::store::Store;
use crate::{Result, StoreError};

const MAX_QUERY_TOKENS: usize = 16;
const SNIPPET_CHARS: usize = 240;
const SNIPPET_LEAD: usize = 80;

#[derive(Debug, Clone, Default)]
pub struct SearchFilter {
    /// Only segments spoken by these persons (notes have no speaker, so a
    /// person filter excludes them).
    pub person_gids: Vec<String>,
    /// `live` or `file`.
    pub source: Option<String>,
    pub template: Option<String>,
    /// Meeting start time range, unix ms, inclusive.
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    /// Restrict to one meeting.
    pub meeting_gid: Option<String>,
    /// Restrict to any of these meetings (empty: no restriction).
    pub meeting_gids: Vec<String>,
    /// Transcript lines only (no note blocks).
    pub segments_only: bool,
}

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: String,
    pub filter: SearchFilter,
    pub limit: usize,
    pub offset: usize,
}

impl SearchQuery {
    pub fn new(text: impl Into<String>) -> SearchQuery {
        SearchQuery {
            text: text.into(),
            filter: SearchFilter::default(),
            limit: 20,
            offset: 0,
        }
    }
}

/// Per index, how many best matches a search considers (see [`Store::search_page`]).
pub const CANDIDATE_WINDOW: usize = 500;

#[derive(Debug, Clone)]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    /// More matches exist than the candidate window holds.
    pub truncated: bool,
    /// Matches found (at most the candidate window per index), across pages.
    pub matches: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitKind {
    Segment,
    Note,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub kind: HitKind,
    pub meeting_gid: String,
    pub meeting_title: String,
    pub meeting_started_at: i64,
    /// Segment or note block gid.
    pub item_gid: String,
    pub speaker_gid: Option<String>,
    /// Time range (segments only).
    pub t0_ms: Option<i64>,
    pub t1_ms: Option<i64>,
    /// The original text, cut to a window for long bodies.
    pub snippet: String,
    /// Char offset of `snippet` in the full original text.
    pub snippet_start: usize,
    /// Match ranges as char offsets into the full ORIGINAL text (subtract
    /// `snippet_start` for the snippet). Ranges outside the window are dropped.
    pub highlights: Vec<Range<usize>>,
    /// Higher is better.
    pub score: f64,
    /// The exact accented form of the query matched (only when the query has accents).
    pub exact: bool,
}

/// A folded query, ready for FTS5.
struct ParsedQuery {
    /// Folded tokens.
    tokens: Vec<String>,
    /// Lowercased NFC tokens of the raw query, when it has diacritics.
    exact_tokens: Option<Vec<String>>,
    fts: String,
}

fn parse_query(text: &str) -> Option<ParsedQuery> {
    let mut tokens: Vec<String> = fold::tokens(&fold::fold(text))
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    tokens.truncate(MAX_QUERY_TOKENS);
    if tokens.is_empty() {
        return None;
    }
    let mut fts = String::new();
    for (i, t) in tokens.iter().enumerate() {
        if i > 0 {
            fts.push(' ');
        }
        fts.push('"');
        fts.push_str(t); // alphanumeric only: nothing to escape
        fts.push('"');
        if i + 1 == tokens.len() && t.chars().count() >= 2 {
            fts.push('*');
        }
    }
    let exact_tokens = fold::has_diacritics(text).then(|| {
        let mut v: Vec<String> = fold::tokens(&fold::nfc(text).to_lowercase())
            .into_iter()
            .map(|(_, t)| t)
            .collect();
        v.truncate(MAX_QUERY_TOKENS);
        v
    });
    Some(ParsedQuery {
        tokens,
        exact_tokens,
        fts,
    })
}

fn token_matches(tok: &str, q: &[String]) -> bool {
    let last = q.len().saturating_sub(1);
    q.iter().enumerate().any(|(i, q)| {
        tok == q || (i == last && q.chars().count() >= 2 && tok.starts_with(q.as_str()))
    })
}

/// Match ranges (char offsets into `original`, which must be NFC). Matches
/// separated only by whitespace are merged (`Đà Nẵng` is one range).
pub fn highlight_ranges(original: &str, folded_query_tokens: &[String]) -> Vec<Range<usize>> {
    let folded = fold::fold_mapped(original);
    let chars: Vec<char> = original.chars().collect();
    let mut out: Vec<Range<usize>> = Vec::new();
    for (r, _) in fold::tokens(&folded.text)
        .into_iter()
        .filter(|(_, t)| token_matches(t, folded_query_tokens))
    {
        let r = folded.to_original(r);
        match out.last_mut() {
            Some(prev) if chars[prev.end..r.start].iter().all(|c| c.is_whitespace()) => {
                prev.end = r.end
            }
            _ => out.push(r),
        }
    }
    out
}

/// Whether every exact (accented) query word occurs in `original`.
fn exact_match(original: &str, exact: &[String]) -> bool {
    if exact.is_empty() {
        return false;
    }
    let words: Vec<String> = fold::tokens(&original.to_lowercase())
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    let last = exact.len() - 1;
    exact.iter().enumerate().all(|(i, q)| {
        words
            .iter()
            .any(|w| w == q || (i == last && q.chars().count() >= 2 && w.starts_with(q.as_str())))
    })
}

fn snippet(original: &str, highlights: &[Range<usize>]) -> (String, usize, Vec<Range<usize>>) {
    let chars: Vec<char> = original.chars().collect();
    if chars.len() <= SNIPPET_CHARS {
        return (original.to_string(), 0, highlights.to_vec());
    }
    let first = highlights.first().map_or(0, |h| h.start);
    let start = first
        .saturating_sub(SNIPPET_LEAD)
        .min(chars.len() - SNIPPET_CHARS);
    let end = start + SNIPPET_CHARS;
    let kept = highlights
        .iter()
        .filter(|h| h.start >= start && h.end <= end)
        .cloned()
        .collect();
    (chars[start..end].iter().collect(), start, kept)
}

/// A raw FTS candidate before decryption.
struct Candidate {
    kind: HitKind,
    item_gid: String,
    meeting_id: i64,
    meeting_gid: String,
    started_at: i64,
    speaker_gid: Option<String>,
    t0_ms: Option<i64>,
    t1_ms: Option<i64>,
    ct: Vec<u8>,
    rank: f64,
}

impl Store {
    /// Runs a search over segments (current transcript version) and notes.
    /// An empty or punctuation-only query returns no hits. See
    /// [`Store::search_page`] for the paging guarantees.
    pub fn search(&self, q: &SearchQuery) -> Result<Vec<SearchHit>> {
        Ok(self.search_page(q)?.hits)
    }

    /// [`Store::search`] plus whether the result set was cut off.
    ///
    /// Each index contributes its best [`CANDIDATE_WINDOW`] matches by BM25,
    /// whatever `offset` is; all candidates are then put in one total order
    /// (exact-diacritic tier, BM25, newest meeting, gid) and the page is cut
    /// from that. So pages never overlap or skip. Matches beyond the window
    /// are unreachable, and `truncated` says the window was full.
    pub fn search_page(&self, q: &SearchQuery) -> Result<SearchPage> {
        let empty = SearchPage {
            hits: Vec::new(),
            truncated: false,
            matches: 0,
        };
        let Some(parsed) = parse_query(&q.text) else {
            return Ok(empty);
        };
        if q.limit == 0 {
            return Ok(empty);
        }
        let window = CANDIDATE_WINDOW as i64;
        let conn = self.conn();

        let mut cands = self.segment_candidates(&conn, &parsed, &q.filter, window)?;
        let mut truncated = cands.len() as i64 >= window;
        if q.filter.person_gids.is_empty() && !q.filter.segments_only {
            let notes = self.note_candidates(&conn, &parsed, &q.filter, window)?;
            truncated |= notes.len() as i64 >= window;
            cands.extend(notes);
        }

        let matches = cands.len();
        let mut titles: HashMap<i64, String> = HashMap::new();
        // (candidate, decrypted text, exact-diacritic match)
        let mut items: Vec<(Candidate, Option<String>, bool)> = Vec::with_capacity(cands.len());
        if let Some(exact) = parsed.exact_tokens.as_deref() {
            // The exact tier needs the text of every candidate.
            for c in cands {
                if let Some(text) = self.open_candidate(&conn, &c)? {
                    let is_exact = exact_match(&text, exact);
                    items.push((c, Some(text), is_exact));
                }
            }
        } else {
            items.extend(cands.into_iter().map(|c| (c, None, false)));
        }
        items.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then(b.0.rank.total_cmp(&a.0.rank).reverse())
                .then(b.0.started_at.cmp(&a.0.started_at))
                .then_with(|| a.0.item_gid.cmp(&b.0.item_gid))
        });

        let mut hits = Vec::new();
        for (c, text, exact) in items.into_iter().skip(q.offset).take(q.limit) {
            let text = match text {
                Some(t) => t,
                None => match self.open_candidate(&conn, &c)? {
                    Some(t) => t,
                    None => continue, // shredded
                },
            };
            if let std::collections::hash_map::Entry::Vacant(slot) = titles.entry(c.meeting_id) {
                let dek = self.dek(&conn, c.meeting_id)?;
                let title: Option<Vec<u8>> = conn.query_row(
                    "SELECT title_ct FROM meetings WHERE id = ?1",
                    [c.meeting_id],
                    |r| r.get(0),
                )?;
                slot.insert(match title {
                    Some(ct) => {
                        open_text(&dek, &ct, &row_aad("meetings", "title_ct", &c.meeting_gid))?
                    }
                    None => String::new(),
                });
            }
            let highlights = highlight_ranges(&text, &parsed.tokens);
            let (snip, snippet_start, highlights) = snippet(&text, &highlights);
            hits.push(SearchHit {
                kind: c.kind,
                meeting_title: titles[&c.meeting_id].clone(),
                meeting_gid: c.meeting_gid,
                meeting_started_at: c.started_at,
                item_gid: c.item_gid,
                speaker_gid: c.speaker_gid,
                t0_ms: c.t0_ms,
                t1_ms: c.t1_ms,
                snippet: snip,
                snippet_start,
                highlights,
                score: -c.rank,
                exact,
            });
        }
        Ok(SearchPage {
            hits,
            truncated,
            matches,
        })
    }

    /// Decrypts a candidate's text; `None` if its meeting key was shredded.
    fn open_candidate(&self, conn: &rusqlite::Connection, c: &Candidate) -> Result<Option<String>> {
        let Ok(dek) = self.dek(conn, c.meeting_id) else {
            return Ok(None);
        };
        let (table, column) = match c.kind {
            HitKind::Segment => ("segments", "text_ct"),
            HitKind::Note => ("notes_blocks", "body_ct"),
        };
        Ok(Some(open_text(
            &dek,
            &c.ct,
            &row_aad(table, column, &c.item_gid),
        )?))
    }

    fn segment_candidates(
        &self,
        conn: &rusqlite::Connection,
        parsed: &ParsedQuery,
        f: &SearchFilter,
        fetch: i64,
    ) -> Result<Vec<Candidate>> {
        let mut sql = String::from(
            "SELECT s.id, s.gid, s.meeting_id, m.gid, m.started_at, sp.gid, s.t0_ms, s.t1_ms, s.text_ct,
                    bm25(segments_fts)
             FROM segments_fts
             JOIN segments s ON s.id = segments_fts.rowid
             JOIN meetings m ON m.id = s.meeting_id AND s.version = m.transcript_version
             LEFT JOIN speakers sp ON sp.id = s.speaker_id
             WHERE segments_fts MATCH ?",
        );
        let mut args: Vec<Value> = vec![Value::Text(parsed.fts.clone())];
        push_meeting_filters(&mut sql, &mut args, f);
        if !f.person_gids.is_empty() {
            sql.push_str(" AND sp.person_id IN (SELECT id FROM persons WHERE gid IN (");
            for (i, g) in f.person_gids.iter().enumerate() {
                sql.push_str(if i == 0 { "?" } else { ",?" });
                args.push(Value::Text(g.clone()));
            }
            sql.push_str("))");
        }
        sql.push_str(" ORDER BY bm25(segments_fts), s.id LIMIT ?");
        args.push(Value::Integer(fetch));
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params_from_iter(args), |r| {
            Ok(Candidate {
                kind: HitKind::Segment,
                item_gid: r.get(1)?,
                meeting_id: r.get(2)?,
                meeting_gid: r.get(3)?,
                started_at: r.get(4)?,
                speaker_gid: r.get(5)?,
                t0_ms: Some(r.get(6)?),
                t1_ms: Some(r.get(7)?),
                ct: r.get(8)?,
                rank: r.get(9)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()
            .map_err(StoreError::from)
    }

    fn note_candidates(
        &self,
        conn: &rusqlite::Connection,
        parsed: &ParsedQuery,
        f: &SearchFilter,
        fetch: i64,
    ) -> Result<Vec<Candidate>> {
        let mut sql = String::from(
            "SELECT n.id, n.gid, n.meeting_id, m.gid, m.started_at, n.body_ct, bm25(notes_fts)
             FROM notes_fts
             JOIN notes_blocks n ON n.id = notes_fts.rowid
             JOIN meetings m ON m.id = n.meeting_id
             WHERE notes_fts MATCH ?",
        );
        let mut args: Vec<Value> = vec![Value::Text(parsed.fts.clone())];
        push_meeting_filters(&mut sql, &mut args, f);
        sql.push_str(" ORDER BY bm25(notes_fts), n.id LIMIT ?");
        args.push(Value::Integer(fetch));
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params_from_iter(args), |r| {
            Ok(Candidate {
                kind: HitKind::Note,
                item_gid: r.get(1)?,
                meeting_id: r.get(2)?,
                meeting_gid: r.get(3)?,
                started_at: r.get(4)?,
                speaker_gid: None,
                t0_ms: None,
                t1_ms: None,
                ct: r.get(5)?,
                rank: r.get(6)?,
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()
            .map_err(StoreError::from)
    }
}

fn push_meeting_filters(sql: &mut String, args: &mut Vec<Value>, f: &SearchFilter) {
    if let Some(g) = &f.meeting_gid {
        sql.push_str(" AND m.gid = ?");
        args.push(Value::Text(g.clone()));
    }
    if !f.meeting_gids.is_empty() {
        sql.push_str(" AND m.gid IN (");
        for (i, g) in f.meeting_gids.iter().enumerate() {
            sql.push_str(if i == 0 { "?" } else { ",?" });
            args.push(Value::Text(g.clone()));
        }
        sql.push(')');
    }
    if let Some(s) = &f.source {
        sql.push_str(" AND m.source = ?");
        args.push(Value::Text(s.clone()));
    }
    if let Some(t) = &f.template {
        sql.push_str(" AND m.template = ?");
        args.push(Value::Text(t.clone()));
    }
    if let Some(t) = f.from_ms {
        sql.push_str(" AND m.started_at >= ?");
        args.push(Value::Integer(t));
    }
    if let Some(t) = f.to_ms {
        sql.push_str(" AND m.started_at <= ?");
        args.push(Value::Integer(t));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_is_folded_quoted_and_prefixed() {
        let q = parse_query("Đà Nẵng").unwrap();
        assert_eq!(q.tokens, ["da", "nang"]);
        assert_eq!(q.fts, "\"da\" \"nang\"*");
        assert!(q.exact_tokens.is_some());
        assert!(parse_query("dong").unwrap().exact_tokens.is_none());
    }

    #[test]
    fn fts_syntax_cannot_be_injected() {
        for evil in [
            "\" OR 1",
            "a AND b NOT c",
            "col:x",
            "NEAR(a b)",
            "a*",
            "-x",
            "\"\"",
            "^a",
        ] {
            let q = parse_query(evil).map(|q| q.fts).unwrap_or_default();
            // Only quoted alphanumeric tokens and a trailing `*` may appear.
            assert!(
                q.chars()
                    .all(|c| c.is_alphanumeric() || c == '"' || c == ' ' || c == '*'),
                "{evil:?} -> {q}"
            );
            assert_eq!(q.matches('"').count() % 2, 0);
        }
        assert!(parse_query("  !!! ").is_none());
    }

    #[test]
    fn highlights_land_on_original_chars() {
        let text = "Tỷ giá đồng tăng, Đồng Nai";
        let h = highlight_ranges(text, &["dong".to_string()]);
        let got: Vec<String> = h
            .iter()
            .map(|r| text.chars().skip(r.start).take(r.len()).collect())
            .collect();
        assert_eq!(got, ["đồng", "Đồng"]);
    }
}
