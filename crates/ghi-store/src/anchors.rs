// SPDX-License-Identifier: Apache-2.0
//! Citation anchors (doc 05 §2.3).
//!
//! The final pass replaces live segments, so a citation can't point at a
//! segment id. It is anchored to audio time instead:
//! `(meeting gid, t0_ms, t1_ms, transcript_version)`. Rendering resolves the
//! anchor to the overlapping segments of the *current* version; when the
//! anchor's version is older the result is flagged `stale` (the text is
//! still found by time). If retention removed the audio, the anchor still
//! resolves to text and `audio_available` is false (the UI shows a dashed chip).

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::store::{Segment, Store};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub meeting_gid: String,
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub transcript_version: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAnchor {
    /// Segments of the current version overlapping the anchor, in time order.
    pub segments: Vec<Segment>,
    /// The anchor was made against an older transcript version.
    pub stale: bool,
    /// At least one audio track still exists for the meeting.
    pub audio_available: bool,
}

impl Store {
    /// An anchor for `[t0_ms, t1_ms]` on the meeting's current transcript version.
    pub fn anchor_for_range(&self, meeting_gid: &str, t0_ms: i64, t1_ms: i64) -> Result<Anchor> {
        let m = Store::meeting_ref(&self.conn(), meeting_gid)?;
        Ok(Anchor {
            meeting_gid: meeting_gid.to_string(),
            t0_ms,
            t1_ms,
            transcript_version: m.version,
        })
    }

    /// An anchor spanning one segment.
    pub fn anchor_for_segment(&self, meeting_gid: &str, segment: &Segment) -> Result<Anchor> {
        self.anchor_for_range(meeting_gid, segment.t0_ms, segment.t1_ms)
    }

    /// Resolves an anchor against the current transcript version. A zero-length
    /// anchor (`t1 <= t0`) is a point and matches the segment containing it.
    pub fn resolve_anchor(&self, anchor: &Anchor) -> Result<ResolvedAnchor> {
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, &anchor.meeting_gid)?;
        let audio_available: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM tracks WHERE meeting_id = ?1)",
            [m.id],
            |r| r.get(0),
        )?;
        let segments = if anchor.t1_ms <= anchor.t0_ms {
            self.read_segments(
                &conn,
                &m,
                "AND s.t0_ms <= ?3 AND s.t1_ms > ?3",
                params![m.id, m.version, anchor.t0_ms],
            )?
        } else {
            self.read_segments(
                &conn,
                &m,
                "AND s.t0_ms < ?4 AND s.t1_ms > ?3",
                params![m.id, m.version, anchor.t0_ms, anchor.t1_ms],
            )?
        };
        Ok(ResolvedAnchor {
            segments,
            stale: anchor.transcript_version != m.version,
            audio_available,
        })
    }
}
