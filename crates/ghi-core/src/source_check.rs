// SPDX-License-Identifier: Apache-2.0
//! "Sources linked n, k to check": how well each AI claim is backed by the
//! lines it cites, computed when notes are read (nothing is stored).
//!
//! A claim's citations are time anchors ([`Anchor`]); the lines they point at
//! are the segments of the current transcript that overlap them. The score is
//! the share of the claim's content words found in those lines
//! ([`ghi_llm::validate::source_score`]); a claim below
//! [`ghi_llm::validate::WEAK_BELOW`] is weak. "Linked" is honest wording: a
//! claim whose words match is linked to its source, not proven by it.
//! Claims are scored only in the language of the transcript lines; a claim
//! whose lines are in another language (or whose notes language is unknown)
//! counts as linked but is never flagged.

use std::collections::{HashMap, HashSet};

use ghi_llm::validate::{claim_words, is_weak, source_score};
use ghi_store::anchors::Anchor;
use ghi_store::store::Segment;

/// A claim to check: its text and the time anchors it cites.
pub struct Claim<'a> {
    pub text: &'a str,
    pub anchors: &'a [Anchor],
}

/// What was found for one claim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClaimCheck {
    /// At least one line of the transcript overlaps a citation.
    pub linked: bool,
    /// The share (0 to 1) of the claim's words found in those lines; `None`
    /// when it was not scored (other language, no words to compare, no line).
    pub score: Option<f32>,
}

impl ClaimCheck {
    /// Scored and below the threshold: worth a second look.
    pub fn weak(&self) -> bool {
        self.score.is_some_and(is_weak)
    }
}

/// Segments longer than this are not looked for before an anchor (the walk
/// back from the anchor's end stops there).
const LONGEST_SEGMENT_MS: i64 = 120_000;

/// Checks every claim. `segments` are the current transcript in time order;
/// `notes_lang` is the language the notes are written in (`en` / `vi`), or
/// `None` when it was not recorded. The words of each line are worked out at
/// most once per call.
pub fn check_claims(
    claims: &[Claim],
    segments: &[Segment],
    notes_lang: Option<&str>,
) -> Vec<ClaimCheck> {
    let mut words: HashMap<usize, HashSet<String>> = HashMap::new();
    claims
        .iter()
        .map(|c| {
            let lines = lines_of(c.anchors, segments);
            let same_lang = notes_lang.is_some_and(|l| {
                lines
                    .iter()
                    .all(|&i| segments[i].lang.as_deref().is_none_or(|sl| sl == l))
            });
            let score = if same_lang && !lines.is_empty() {
                for &i in &lines {
                    words
                        .entry(i)
                        .or_insert_with(|| claim_words(&segments[i].text));
                }
                let cited: Vec<&HashSet<String>> = lines.iter().map(|i| &words[i]).collect();
                source_score(&claim_words(c.text), &cited)
            } else {
                None
            };
            ClaimCheck {
                linked: !lines.is_empty(),
                score,
            }
        })
        .collect()
}

/// `(linked, to check)` over all checks.
pub fn totals(checks: &[ClaimCheck]) -> (usize, usize) {
    (
        checks.iter().filter(|c| c.linked).count(),
        checks.iter().filter(|c| c.weak()).count(),
    )
}

/// Indexes of the segments overlapping any of `anchors` (a zero-length anchor
/// is the instant it names), in time order, each once.
fn lines_of(anchors: &[Anchor], segments: &[Segment]) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for a in anchors {
        let end = a.t1_ms.max(a.t0_ms + 1);
        // Segments starting before the anchor ends; walk back to those that
        // can still reach it.
        let upper = segments.partition_point(|s| s.t0_ms < end);
        for i in (0..upper).rev() {
            let s = &segments[i];
            if s.t0_ms < a.t0_ms - LONGEST_SEGMENT_MS {
                break;
            }
            if s.t1_ms.max(s.t0_ms + 1) > a.t0_ms {
                out.push(i);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn seg(t0: i64, t1: i64, text: &str, lang: Option<&str>) -> Segment {
        Segment {
            gid: format!("g{t0}"),
            version: 1,
            speaker_gid: None,
            t0_ms: t0,
            t1_ms: t1,
            text: text.into(),
            lang: lang.map(str::to_string),
            confidence: None,
            edited: false,
            overlap: false,
        }
    }

    fn at(t0: i64, t1: i64) -> Anchor {
        Anchor {
            meeting_gid: "m".into(),
            t0_ms: t0,
            t1_ms: t1,
            transcript_version: 1,
        }
    }

    fn meeting() -> Vec<Segment> {
        vec![
            seg(0, 4000, "Chốt lịch beta vào thứ Sáu nhé", Some("vi")),
            seg(
                4000,
                8000,
                "Em sẽ gửi tài liệu thiết kế mã hóa cho bên kiểm toán",
                Some("vi"),
            ),
            seg(
                8000,
                12_000,
                "Hộp thư hỗ trợ còn bốn mươi yêu cầu chưa trả lời",
                Some("vi"),
            ),
        ]
    }

    #[test]
    fn a_claim_is_scored_against_the_lines_its_anchors_overlap() {
        let segs = meeting();
        let a_good = [at(4100, 7900)];
        let a_wrong = [at(8100, 11_000)];
        let a_none = [at(50_000, 51_000)];
        let claims = [
            Claim {
                text: "Gửi tài liệu thiết kế mã hóa cho bên kiểm toán",
                anchors: &a_good,
            },
            Claim {
                text: "Gửi tài liệu thiết kế mã hóa cho bên kiểm toán",
                anchors: &a_wrong,
            },
            Claim {
                text: "Gửi tài liệu thiết kế mã hóa cho bên kiểm toán",
                anchors: &a_none,
            },
        ];
        let c = check_claims(&claims, &segs, Some("vi"));
        assert!(c[0].linked && !c[0].weak());
        assert!(c[1].linked && c[1].weak(), "{:?}", c[1]);
        // No line there any more: not linked, not flagged.
        assert!(!c[2].linked && c[2].score.is_none() && !c[2].weak());
        assert_eq!(totals(&c), (2, 1));
    }

    #[test]
    fn other_language_or_unknown_language_is_linked_but_never_flagged() {
        let segs = meeting();
        let a = [at(8100, 11_000)];
        let claims = [Claim {
            text: "Send the design document to the auditors",
            anchors: &a,
        }];
        let en = check_claims(&claims, &segs, Some("en"));
        assert!(en[0].linked && en[0].score.is_none());
        let unknown = check_claims(&claims, &segs, None);
        assert!(unknown[0].linked && !unknown[0].weak());
    }

    #[test]
    fn a_point_anchor_and_several_anchors() {
        let segs = meeting();
        let a = [at(5000, 5000), at(9000, 9000)];
        let claims = [Claim {
            text: "Tài liệu thiết kế cho kiểm toán, hộp thư hỗ trợ bốn mươi yêu cầu",
            anchors: &a,
        }];
        let c = check_claims(&claims, &segs, Some("vi"));
        assert!(c[0].linked && !c[0].weak(), "{:?}", c[0]);
    }

    /// A 2-hour meeting (2,400 lines of 3 s) and 150 claims with two anchors each.
    #[test]
    fn scoring_a_two_hour_meeting_is_fast() {
        let segs: Vec<Segment> = (0..2400)
            .map(|i| {
                seg(
                    i * 3000,
                    i * 3000 + 2800,
                    &format!(
                        "line {i} we talked about the release plan budget vendor contract topic{} for the quarter",
                        i % 50
                    ),
                    Some("en"),
                )
            })
            .collect();
        let anchors: Vec<[Anchor; 2]> = (0..150)
            .map(|k| {
                let t = k * 47_000;
                [at(t, t + 3000), at(t + 9000, t + 12_000)]
            })
            .collect();
        let texts: Vec<String> = (0..150)
            .map(|k| {
                format!(
                    "The team covered the release plan and vendor contract topic{}",
                    k % 50
                )
            })
            .collect();
        let claims: Vec<Claim> = texts
            .iter()
            .zip(&anchors)
            .map(|(t, a)| Claim {
                text: t,
                anchors: a,
            })
            .collect();
        let started = Instant::now();
        let c = check_claims(&claims, &segs, Some("en"));
        let took = started.elapsed();
        assert_eq!(c.len(), 150);
        assert!(c.iter().all(|c| c.linked));
        // The target is 15 ms in a release build; unoptimized code is allowed 10x.
        let budget = if cfg!(debug_assertions) { 150 } else { 15 };
        assert!(
            took.as_millis() <= budget,
            "{took:?} for 150 claims over 2,400 lines"
        );
        eprintln!("source check, 150 claims over 2,400 lines: {took:?}");
    }
}
