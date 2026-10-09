// SPDX-License-Identifier: Apache-2.0
//! Marks (the user's "this matters" presses while recording) against the
//! transcript and the notes.
//!
//! [`mark_lines`] puts each mark on a line (the rule of the transcript view's
//! `marksOf`, plus a limit for silence); [`hints`] feeds the local notes
//! prompt; [`coverage`] says which marks the written notes cover. Nothing
//! here is stored, and marks never go to a cloud send.

use ghi_llm::notes::{MarkHint, MarkKind};
use ghi_store::anchors::Anchor;
use ghi_store::store::{Mark, MarkTag, Segment};

/// A mark further than this past the end of the line before it is in silence.
pub const MAX_GAP_MS: i64 = 10_000;

/// A mark and the line it falls on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarkLine {
    pub t_ms: i64,
    pub tag: MarkTag,
    /// Index into the meeting's segments (also the line id the notes prompt
    /// uses); `None` for a mark in silence, which keeps its time only.
    pub segment: Option<usize>,
    /// The line's span, or `(t_ms, t_ms)` for a mark in silence.
    pub range: (i64, i64),
}

/// Each mark on a line; this is the one rule for the notes prompt, the
/// coverage check and (through `MeetingNotes.marks`) the apps.
///
/// A mark belongs to the last line starting at or before it, if that line ends
/// within [`MAX_GAP_MS`] of the mark. When that line is too far back (speech
/// overlapped: a longer line started earlier and still runs), the last line
/// whose span contains the mark is used instead. Otherwise the mark is in
/// silence and keeps its time only. `segments` are in time order; marks keep
/// their order.
pub fn mark_lines(marks: &[Mark], segments: &[Segment]) -> Vec<MarkLine> {
    marks
        .iter()
        .map(|m| {
            // The last one wins a tie, as in the transcript view.
            let at = segments.iter().rposition(|s| s.t0_ms <= m.t_ms);
            let line = at
                .filter(|&i| m.t_ms - segments[i].t1_ms <= MAX_GAP_MS)
                .or_else(|| {
                    segments
                        .iter()
                        .rposition(|s| s.t0_ms <= m.t_ms && m.t_ms <= s.t1_ms)
                });
            MarkLine {
                t_ms: m.t_ms,
                tag: m.tag,
                segment: line,
                range: line.map_or((m.t_ms, m.t_ms), |i| (segments[i].t0_ms, segments[i].t1_ms)),
            }
        })
        .collect()
}

fn kind(tag: MarkTag) -> MarkKind {
    match tag {
        MarkTag::Star => MarkKind::Star,
        MarkTag::Decision => MarkKind::Decision,
        MarkTag::Action => MarkKind::Action,
        MarkTag::Question => MarkKind::Question,
    }
}

/// The meeting's marks as prompt hints. Marks only steer the local model, so a
/// store that cannot be read costs the hint, never the notes.
pub fn load_hints(
    store: &ghi_store::store::Store,
    meeting: &str,
    segments: &[Segment],
) -> Vec<MarkHint> {
    match store.marks(meeting) {
        Ok(marks) => hints(&mark_lines(&marks, segments)),
        Err(e) => {
            log::warn!("marks not read, notes written without them: {e}");
            Vec::new()
        }
    }
}

/// The marks that sit on a line, for `Options::marks`.
pub fn hints(lines: &[MarkLine]) -> Vec<MarkHint> {
    lines
        .iter()
        .filter_map(|m| {
            Some(MarkHint {
                id: m.segment? as u64,
                kind: kind(m.tag),
                t_ms: m.t_ms,
            })
        })
        .collect()
}

/// What covers a mark: an index into the block or action-item anchors given
/// to [`coverage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cover {
    Block(usize),
    Action(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkCoverage {
    pub mark: MarkLine,
    pub covered_by: Vec<Cover>,
}

impl MarkCoverage {
    pub fn is_covered(&self) -> bool {
        !self.covered_by.is_empty()
    }
}

/// Whether two spans share time. A point (empty or one ms) counts as the
/// millisecond it is on, so lines that merely touch do not overlap.
fn overlaps((a0, a1): (i64, i64), (b0, b1): (i64, i64)) -> bool {
    let (a1, b1) = (a1.max(a0 + 1), b1.max(b0 + 1));
    a0 < b1 && b0 < a1
}

/// Which marks the notes cover: a mark is covered by every block or action
/// item that has an anchor overlapping the mark's line (anchors are times, so
/// this survives a new transcript version). `blocks` and `actions` hold the
/// anchors of each AI block and each action item.
pub fn coverage(
    marks: &[MarkLine],
    blocks: &[&[Anchor]],
    actions: &[&[Anchor]],
) -> Vec<MarkCoverage> {
    let hit = |anchors: &[Anchor], m: &MarkLine| {
        anchors
            .iter()
            .any(|a| overlaps((a.t0_ms, a.t1_ms), m.range))
    };
    marks
        .iter()
        .map(|m| MarkCoverage {
            mark: *m,
            covered_by: blocks
                .iter()
                .enumerate()
                .filter(|(_, a)| hit(a, m))
                .map(|(i, _)| Cover::Block(i))
                .chain(
                    actions
                        .iter()
                        .enumerate()
                        .filter(|(_, a)| hit(a, m))
                        .map(|(i, _)| Cover::Action(i)),
                )
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(t0: i64, t1: i64) -> Segment {
        Segment {
            gid: format!("g{t0}"),
            version: 1,
            speaker_gid: None,
            t0_ms: t0,
            t1_ms: t1,
            text: "x".into(),
            lang: None,
            confidence: None,
            edited: false,
            overlap: false,
        }
    }

    fn mark(t: i64, tag: MarkTag) -> Mark {
        Mark {
            gid: format!("m{t}"),
            t_ms: t,
            tag,
        }
    }

    fn anchor(t0: i64, t1: i64) -> Anchor {
        Anchor {
            meeting_gid: "m".into(),
            t0_ms: t0,
            t1_ms: t1,
            transcript_version: 1,
        }
    }

    #[test]
    fn a_mark_belongs_to_the_last_line_at_or_before_it() {
        let segs = [seg(0, 4000), seg(4000, 8000), seg(20_000, 24_000)];
        let lines = mark_lines(
            &[
                mark(0, MarkTag::Star),
                mark(4000, MarkTag::Decision),
                mark(7999, MarkTag::Action),
                mark(26_000, MarkTag::Question),
            ],
            &segs,
        );
        let at: Vec<_> = lines.iter().map(|l| l.segment).collect();
        assert_eq!(at, [Some(0), Some(1), Some(1), Some(2)]);
        assert_eq!(lines[1].range, (4000, 8000));
    }

    #[test]
    fn overlapping_speech_falls_back_to_the_line_that_holds_the_mark() {
        // A long line (0-60 s) is still running when two short ones start.
        let segs = [seg(0, 60_000), seg(20_000, 22_000), seg(25_000, 26_000)];
        let lines = mark_lines(
            &[mark(50_000, MarkTag::Star), mark(70_000, MarkTag::Star)],
            &segs,
        );
        // The last line to start ended 24 s ago, but line 0 still holds 50 s.
        assert_eq!(lines[0].segment, Some(0));
        assert_eq!(lines[0].range, (0, 60_000));
        // Past every line by more than 10 s: silence.
        assert_eq!(lines[1].segment, None);
    }

    #[test]
    fn a_mark_in_silence_keeps_its_time_only() {
        let segs = [seg(0, 4000), seg(30_000, 34_000)];
        let lines = mark_lines(
            &[
                mark(14_000, MarkTag::Star),
                mark(14_001, MarkTag::Star),
                mark(-5, MarkTag::Star),
            ],
            &segs,
        );
        // Exactly 10 s after the line ended is still on it; 1 ms more is not.
        assert_eq!(lines[0].segment, Some(0));
        assert_eq!(lines[1].segment, None);
        assert_eq!(lines[1].range, (14_001, 14_001));
        // Before the first line there is none to belong to.
        assert_eq!(lines[2].segment, None);
        assert_eq!(hints(&lines).len(), 1);
    }

    #[test]
    fn hints_carry_line_ids_and_tags() {
        let segs = [seg(0, 4000), seg(4000, 8000)];
        let lines = mark_lines(
            &[mark(5000, MarkTag::Decision), mark(100_000, MarkTag::Star)],
            &segs,
        );
        assert_eq!(
            hints(&lines),
            [MarkHint {
                id: 1,
                kind: MarkKind::Decision,
                t_ms: 5000
            }]
        );
    }

    #[test]
    fn coverage_is_by_time_overlap_across_blocks_and_actions() {
        let segs = [seg(0, 4000), seg(4000, 8000), seg(8000, 12_000)];
        let lines = mark_lines(
            &[
                mark(1000, MarkTag::Star),
                mark(5000, MarkTag::Action),
                mark(9000, MarkTag::Question),
                mark(500_000, MarkTag::Star),
            ],
            &segs,
        );
        let b0 = [anchor(0, 4000)];
        // Touches line 1 at 4000 only: not an overlap.
        let b1 = [anchor(0, 4000), anchor(6000, 6500)];
        let a0 = [anchor(4500, 5500)];
        let a1: [Anchor; 0] = [];
        let cov = coverage(&lines, &[&b0, &b1], &[&a0, &a1]);
        assert_eq!(
            cov[0].covered_by,
            [Cover::Block(0), Cover::Block(1)],
            "both blocks cite line 0"
        );
        assert_eq!(
            cov[1].covered_by,
            [Cover::Block(1), Cover::Action(0)],
            "an action item anchor counts; 4000 is not 4000..8000"
        );
        assert!(!cov[2].is_covered());
        assert!(!cov[3].is_covered(), "silence far from any anchor");
    }

    #[test]
    fn a_point_anchor_covers_the_line_holding_it_and_silence_needs_the_instant() {
        let segs = [seg(0, 4000)];
        let lines = mark_lines(
            &[mark(2000, MarkTag::Star), mark(20_000, MarkTag::Star)],
            &segs,
        );
        let p = [anchor(2000, 2000)];
        let q = [anchor(20_000, 20_000)];
        let cov = coverage(&lines, &[&p, &q], &[]);
        assert_eq!(cov[0].covered_by, [Cover::Block(0)]);
        assert_eq!(cov[1].covered_by, [Cover::Block(1)]);
    }
}
