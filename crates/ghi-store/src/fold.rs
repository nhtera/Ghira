// SPDX-License-Identifier: Apache-2.0
//! Accent-insensitive folding for Vietnamese (and any Latin) search text.
//!
//! `fold(s)` = NFC → NFD → drop combining marks → `đ→d`, `Đ→D` → lowercase
//! (doc 05 §2.1). FTS5's own `remove_diacritics` does not fold `đ`, so the
//! index is built over `fold(text)` with `remove_diacritics 0`, and queries are
//! folded the same way.
//!
//! Text is stored and shown as NFC. Folding works one NFC char at a time, so
//! for Vietnamese the folded string has exactly as many chars as the NFC
//! original and match offsets map 1:1 for highlighting. The exceptions (a
//! stray combining mark, or a lowercase mapping that expands) are handled by an
//! offset map ([`Folded::map`]) instead of being assumed away.

use std::ops::Range;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::{canonical_combining_class, is_combining_mark};

/// A folded string plus the way back to the NFC original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folded {
    pub text: String,
    /// `map[i]` is the NFC char index that folded char `i` came from. `None`
    /// when folding was 1:1 (the normal case), where the index is `i` itself.
    pub map: Option<Vec<usize>>,
}

impl Folded {
    /// Maps a char range of `self.text` to the char range of the NFC original.
    pub fn to_original(&self, r: Range<usize>) -> Range<usize> {
        match &self.map {
            None => r,
            Some(map) if r.start < r.end && r.end <= map.len() => map[r.start]..map[r.end - 1] + 1,
            Some(map) => {
                let end = map.last().map_or(0, |l| l + 1);
                end..end
            }
        }
    }
}

/// True for the combining marks Vietnamese uses (tone, hat, breve, horn, dot
/// below, ...). Marks with combining class 0 (spacing vowel signs in Indic
/// scripts) are kept.
fn is_dropped_mark(c: char) -> bool {
    is_combining_mark(c) && canonical_combining_class(c) != 0
}

/// Folds one NFC char into `out`; returns how many chars it produced.
fn fold_char(c: char, out: &mut String) -> usize {
    if c.is_ascii() {
        out.push(c.to_ascii_lowercase());
        return 1;
    }
    let mut n = 0;
    for d in std::iter::once(c).nfd() {
        if is_dropped_mark(d) {
            continue;
        }
        match d {
            'đ' | 'Đ' => {
                out.push('d');
                n += 1;
            }
            _ => {
                for l in d.to_lowercase() {
                    out.push(l);
                    n += 1;
                }
            }
        }
    }
    n
}

/// `fold(s)`, with the offset map when it is not 1:1.
pub fn fold_mapped(s: &str) -> Folded {
    let nfc = nfc(s);
    let mut text = String::with_capacity(nfc.len());
    let mut map: Vec<usize> = Vec::new();
    let mut one_to_one = true;
    for (i, c) in nfc.chars().enumerate() {
        let produced = fold_char(c, &mut text);
        if produced != 1 {
            one_to_one = false;
        }
        map.extend(std::iter::repeat_n(i, produced));
    }
    Folded {
        text,
        map: if one_to_one { None } else { Some(map) },
    }
}

/// The folded form of `s` (what goes into the FTS index and queries).
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.nfc() {
        fold_char(c, &mut out);
    }
    out
}

/// NFC form, the canonical stored text.
pub fn nfc(s: &str) -> String {
    s.nfc().collect()
}

/// Splits into FTS-style tokens (runs of alphanumeric chars) with their char
/// ranges. Matches how `unicode61` splits folded text.
pub fn tokens(s: &str) -> Vec<(Range<usize>, String)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut cur = String::new();
    let mut n = 0;
    for (i, c) in s.chars().enumerate() {
        n = i + 1;
        if c.is_alphanumeric() {
            if start.is_none() {
                start = Some(i);
            }
            cur.push(c);
        } else if let Some(st) = start.take() {
            out.push((st..i, std::mem::take(&mut cur)));
        }
    }
    if let Some(st) = start {
        out.push((st..n, cur));
    }
    out
}

/// True if `s` carries diacritics or `đ` that folding would remove.
pub fn has_diacritics(s: &str) -> bool {
    let lower: String = s.nfc().flat_map(char::to_lowercase).collect();
    lower != fold(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_vietnamese() {
        assert_eq!(fold("Đồng"), "dong");
        assert_eq!(fold("Đà Nẵng"), "da nang");
        assert_eq!(fold("chốt"), "chot");
        assert_eq!(fold("họp"), "hop");
        assert_eq!(fold("Nguyễn Thị Hương"), "nguyen thi huong");
    }

    #[test]
    fn nfd_input_is_composed_first() {
        let nfd: String = "đồng chốt".nfd().collect();
        assert_eq!(fold(&nfd), "dong chot");
    }

    #[test]
    fn one_to_one_has_no_map() {
        let f = fold_mapped("Đà Nẵng họp");
        assert_eq!(f.text.chars().count(), nfc("Đà Nẵng họp").chars().count());
        assert!(f.map.is_none());
    }

    #[test]
    fn map_fallback_when_length_changes() {
        // A lone combining mark folds away, so offsets need the map.
        let s = "a\u{0301}\u{0301}b"; // á + extra acute (does not compose)
        let f = fold_mapped(s);
        assert_eq!(f.text, "ab");
        let m = f.map.as_ref().unwrap();
        assert_eq!(f.to_original(1..2), m[1]..m[1] + 1);
    }

    #[test]
    fn tokens_have_char_ranges() {
        let t = tokens("da nang, hop!");
        assert_eq!(t[0], (0..2, "da".into()));
        assert_eq!(t[1], (3..7, "nang".into()));
        assert_eq!(t[2], (9..12, "hop".into()));
    }
}
