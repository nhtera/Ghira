// SPDX-License-Identifier: Apache-2.0
//! Custom vocabulary (doc 02 §B, P1): a post-correction pass in the final
//! pass. Words that come close to one of the user's terms (names, products,
//! jargon) are replaced by the term as the user wrote it.
//!
//! Matching is on folded text (case, Vietnamese diacritics and đ ignored), over
//! windows of the term's word count, with a small edit distance that grows
//! with the term's length. Exact folded matches are re-cased/re-accented too
//! ("le minh" → "Lê Minh").

/// At most this many of a meeting's attendees become terms.
const MAX_ATTENDEE_TERMS: usize = 30;

/// At most this many terms (doc 02 §B).
pub const MAX_TERMS: usize = 200;

/// Store setting: the user's own terms (a JSON list of strings).
pub const TERMS_SETTING: &str = "vocabulary";
/// Store setting: learned names the user removed (a JSON list of strings).
pub const IGNORED_SETTING: &str = "vocabulary.ignored";

fn list(store: &ghi_store::store::Store, key: &str) -> Result<Vec<String>, String> {
    Ok(store
        .get_setting(key)
        .map_err(|e| e.to_string())?
        .and_then(|v| serde_json::from_value::<Vec<String>>(v).ok())
        .unwrap_or_default())
}

/// The user's own terms.
pub fn user_terms(store: &ghi_store::store::Store) -> Result<Vec<String>, String> {
    list(store, TERMS_SETTING)
}

/// Names learned from the speakers the user named (RT-14), minus the ones
/// they removed and the ones already among their terms; sorted.
pub fn learned_terms(store: &ghi_store::store::Store) -> Result<Vec<String>, String> {
    let fold = |s: &str| ghi_text::fold(s);
    let ignored: Vec<String> = list(store, IGNORED_SETTING)?
        .iter()
        .map(|s| fold(s))
        .collect();
    let own: Vec<String> = user_terms(store)?.iter().map(|s| fold(s)).collect();
    let gids: Vec<String> = store
        .list_meetings(100_000, 0)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|m| m.gid)
        .collect();
    let mut names: Vec<String> = store
        .named_speakers(&gids)
        .map_err(|e| e.to_string())?
        .into_values()
        .flatten()
        .map(|(n, _)| n.trim().to_string())
        .filter(|n| n.chars().count() >= 2)
        .filter(|n| !ignored.contains(&fold(n)) && !own.contains(&fold(n)))
        .collect();
    names.sort_by_key(|n| fold(n));
    names.dedup_by(|a, b| fold(a) == fold(b));
    Ok(names)
}

/// What the final pass corrects towards: the user's terms, then learned names,
/// at most [`MAX_TERMS`].
pub fn effective_terms(store: &ghi_store::store::Store) -> Result<Vec<String>, String> {
    let mut t = user_terms(store)?;
    t.extend(learned_terms(store)?);
    t.truncate(MAX_TERMS);
    Ok(t)
}

/// What the final pass corrects towards for one meeting: the names of the
/// people in its calendar event (phase 14d), then [`effective_terms`], at most
/// [`MAX_TERMS`].
pub fn meeting_terms(
    store: &ghi_store::store::Store,
    meeting: &str,
) -> Result<Vec<String>, String> {
    let mut terms: Vec<String> = crate::calendar::info(store, meeting)
        .map(|i| i.attendees)
        .unwrap_or_default();
    terms.retain(|n| n.chars().count() >= 2);
    terms.truncate(MAX_ATTENDEE_TERMS);
    for t in effective_terms(store)? {
        let folded = ghi_text::fold(&t);
        if !terms.iter().any(|x| ghi_text::fold(x) == folded) {
            terms.push(t);
        }
    }
    terms.truncate(MAX_TERMS);
    Ok(terms)
}

#[derive(Debug, Clone)]
pub struct Vocabulary {
    /// (term as written, folded words).
    terms: Vec<(String, Vec<String>)>,
}

/// Edits allowed for a folded term of `len` characters.
fn budget(len: usize) -> usize {
    match len {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

fn words(s: &str) -> Vec<String> {
    ghi_text::tokens(&ghi_text::fold(s))
        .into_iter()
        .map(|(_, t)| t)
        .collect()
}

impl Vocabulary {
    /// Builds the list (blank and duplicate terms dropped, at most [`MAX_TERMS`]).
    pub fn new<S: AsRef<str>>(terms: &[S]) -> Vocabulary {
        let mut out: Vec<(String, Vec<String>)> = Vec::new();
        for t in terms {
            let t = t.as_ref().trim();
            let w = words(t);
            if w.is_empty() || out.iter().any(|(_, o)| *o == w) {
                continue;
            }
            out.push((t.to_string(), w));
            if out.len() == MAX_TERMS {
                break;
            }
        }
        // Longer terms first, so "Lê Minh Anh" wins over "Minh Anh".
        out.sort_by_key(|(_, w)| std::cmp::Reverse(w.len()));
        Vocabulary { terms: out }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// `text` with near-misses of the terms replaced. Returns `None` when
    /// nothing changed.
    pub fn correct(&self, text: &str) -> Option<String> {
        if self.terms.is_empty() {
            return None;
        }
        // Work on the NFC text (what the store keeps); byte ranges of its
        // words (the fold maps char ranges back), and their folded forms.
        let nfc = ghi_text::nfc(text);
        let text = nfc.as_str();
        let bytes: Vec<usize> = text
            .char_indices()
            .map(|(b, _)| b)
            .chain([text.len()])
            .collect();
        let folded = ghi_text::fold_mapped(text);
        let toks: Vec<(std::ops::Range<usize>, String)> = ghi_text::tokens(&folded.text)
            .into_iter()
            .map(|(r, w)| {
                let c = folded.to_original(r);
                (bytes[c.start]..bytes[c.end], w)
            })
            .collect();
        let mut out = String::with_capacity(text.len());
        let mut last = 0usize;
        let mut changed = false;
        let mut i = 0usize;
        'outer: while i < toks.len() {
            for (term, tw) in &self.terms {
                let n = tw.len();
                if i + n > toks.len() {
                    continue;
                }
                let window: Vec<char> = toks[i..i + n]
                    .iter()
                    .map(|(_, w)| w.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .collect();
                let target: Vec<char> = tw.join(" ").chars().collect();
                if levenshtein(&window, &target) > budget(target.len()) {
                    continue;
                }
                let span = toks[i].0.start..toks[i + n - 1].0.end;
                // Words already written with diacritics are real Vietnamese
                // words ("mình" is not "Minh"): only an exact match counts.
                let src = &text[span.clone()];
                if ghi_text::has_diacritics(src) && src.to_lowercase() != term.to_lowercase() {
                    continue;
                }
                if src != term {
                    out.push_str(&text[last..span.start]);
                    out.push_str(term);
                    last = span.end;
                    changed = true;
                }
                i += n;
                continue 'outer;
            }
            i += 1;
        }
        changed.then(|| {
            out.push_str(&text[last..]);
            out
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_misses_and_unaccented_forms_become_the_term() {
        let v = Vocabulary::new(&["Lê Minh Anh", "Kubernetes", "Ghira", "AWS"]);
        assert_eq!(
            v.correct("anh le minh anh deploy lên kubernetis rồi")
                .as_deref(),
            Some("anh Lê Minh Anh deploy lên Kubernetes rồi")
        );
        assert_eq!(
            v.correct("mở app Ghira đi").as_deref(),
            None,
            "already right"
        );
        assert_eq!(
            v.correct("gira and ghirra").as_deref(),
            Some("Ghira and Ghira")
        );
        // Short terms need an exact (folded) match: "aws" yes, "awe" no.
        assert_eq!(v.correct("on aws today").as_deref(), Some("on AWS today"));
        assert_eq!(v.correct("I awe you"), None);
    }

    #[test]
    fn accented_words_are_never_rewritten_into_a_term() {
        let v = Vocabulary::new(&["Minh", "Lê"]);
        assert_eq!(v.correct("mình đi lễ nhé"), None);
        assert_eq!(
            v.correct("anh minh và chị le").as_deref(),
            Some("anh Minh và chị Lê")
        );
        assert_eq!(v.correct("MINH").as_deref(), Some("Minh"));
    }

    #[test]
    fn list_is_cleaned_and_capped() {
        let many: Vec<String> = (0..300).map(|i| format!("term{i}")).collect();
        assert_eq!(Vocabulary::new(&many).terms.len(), MAX_TERMS);
        assert!(Vocabulary::new(&["  ", ""]).is_empty());
        assert_eq!(Vocabulary::new(&["Đà Nẵng", "da nang"]).terms.len(), 1);
    }
}
