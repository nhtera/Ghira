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

/// At most this many glossary-pack terms (8 packs of up to 150): their own cap,
/// apart from [`MAX_TERMS`], so enabling packs never cuts the user's terms
/// and the user's list never cuts a pack's. Packs only help the notes spell
/// terms that were said ([`pack_terms_seen`]); they never rewrite the transcript.
pub const MAX_PACK_TERMS: usize = 1_200;

/// Store setting: the user's own terms (a JSON list of strings).
pub const TERMS_SETTING: &str = "vocabulary";
/// Store setting: learned names the user removed (a JSON list of strings).
pub const IGNORED_SETTING: &str = "vocabulary.ignored";
/// Store setting (synced): the enabled glossary packs (a JSON list of pack ids).
pub const PACKS_SETTING: &str = "vocabulary.packs";

/// A bundled glossary pack (`glossaries/<domain>.<lang>.toml`).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    /// `<domain>-<lang>`, one of [`ghi_store::sync::settings::PACK_IDS`].
    pub id: String,
    pub domain: String,
    pub lang: String,
    /// Drafted in-house and not yet read by a domain expert (the owner reviews
    /// the Vietnamese medical and legal packs before they ship).
    pub needs_owner_review: bool,
    pub terms: Vec<String>,
}

macro_rules! packs_src {
    ($($file:literal),* $(,)?) => {
        &[$(include_str!(concat!("../glossaries/", $file))),*]
    };
}

const PACK_SOURCES: &[&str] = packs_src![
    "medical.en.toml",
    "medical.vi.toml",
    "legal.en.toml",
    "legal.vi.toml",
    "finance.en.toml",
    "finance.vi.toml",
    "tech.en.toml",
    "tech.vi.toml",
];

/// Every bundled pack, in [`ghi_store::sync::settings::PACK_IDS`] order.
pub fn packs() -> &'static [Pack] {
    static PACKS: std::sync::OnceLock<Vec<Pack>> = std::sync::OnceLock::new();
    PACKS.get_or_init(|| {
        PACK_SOURCES
            .iter()
            .map(|src| toml::from_str(src).expect("bundled glossary pack parses"))
            .collect()
    })
}

/// The enabled packs' ids (known ids only, in pack order).
pub fn enabled_packs(store: &ghi_store::store::Store) -> Result<Vec<String>, String> {
    let on = list(store, PACKS_SETTING)?;
    Ok(packs()
        .iter()
        .filter(|p| on.contains(&p.id))
        .map(|p| p.id.clone())
        .collect())
}

/// Of the packs named in `ids`, the terms the lines actually say, in the
/// order first said. Glossary packs only ever help the notes spell terms that
/// were said; they never change the transcript. So a term counts only on an
/// exact whole-word match (case and, for terms without accents, diacritics
/// ignored), on a line in the pack's own language ([`crate::live::line_language`]):
/// nothing fuzzy, and a Vietnamese term is never looked for in an English line.
pub fn pack_terms_seen<S: AsRef<str>>(lines: &[S], ids: &[String]) -> Vec<String> {
    use std::collections::HashMap;
    // (language, folded words joined by one space) → the terms written so.
    let mut index: HashMap<(&str, String), Vec<&'static str>> = HashMap::new();
    let mut sizes: Vec<usize> = Vec::new();
    for p in packs().iter().filter(|p| ids.contains(&p.id)) {
        for t in &p.terms {
            let w = words(t);
            if w.is_empty() {
                continue;
            }
            if !sizes.contains(&w.len()) {
                sizes.push(w.len());
            }
            let slot = index.entry((p.lang.as_str(), w.join(" "))).or_default();
            if !slot.contains(&t.as_str()) {
                slot.push(t);
            }
        }
    }
    let mut seen: Vec<String> = Vec::new();
    if index.is_empty() {
        return seen;
    }
    for line in lines {
        let nfc = ghi_text::nfc(line.as_ref());
        let Some(lang) = crate::live::line_language(&nfc, None) else {
            continue;
        };
        let toks = tokenize(&nfc);
        for &n in &sizes {
            for w in toks.windows(n) {
                let key = w
                    .iter()
                    .map(|t| t.folded.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                let Some(terms) = index.get(&(lang.as_str(), key)) else {
                    continue;
                };
                let src = nfc[w[0].range.start..w[n - 1].range.end].to_lowercase();
                for t in terms {
                    // A term written with accents is said with them.
                    let said = !ghi_text::has_diacritics(t) || src == t.to_lowercase();
                    if said && !seen.iter().any(|x| x == t) {
                        seen.push((*t).to_string());
                    }
                }
            }
        }
    }
    seen
}

/// The terms of the packs named in `ids`, at most [`MAX_PACK_TERMS`].
pub fn pack_terms(ids: &[String]) -> Vec<&'static str> {
    packs()
        .iter()
        .filter(|p| ids.contains(&p.id))
        .flat_map(|p| p.terms.iter().map(String::as_str))
        .take(MAX_PACK_TERMS)
        .collect()
}

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

/// One term: as written, its folded words and the char length of the words
/// joined by single spaces.
#[derive(Debug, Clone)]
struct Term {
    text: String,
    words: Vec<String>,
    /// The folded words joined by single spaces, as chars.
    chars: Vec<char>,
    len: usize,
}

/// The terms with `n` words, indexed by folded length so a window only meets
/// the terms whose length is within the edit budget of its own.
#[derive(Debug, Clone)]
struct Group {
    n: usize,
    /// Term length → indexes into `Vocabulary::terms`, ascending.
    by_len: std::collections::HashMap<usize, Vec<usize>>,
}

#[derive(Debug, Clone)]
pub struct Vocabulary {
    /// Longest (in words) first, then in the order given.
    terms: Vec<Term>,
    /// Longest first, like `terms`.
    groups: Vec<Group>,
}

/// Edits allowed for a folded term of `len` characters.
fn budget(len: usize) -> usize {
    match len {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

/// The most edits any term can allow.
const MAX_BUDGET: usize = 2;

/// The edit distance of `a` and `b` if it is at most `k`, else `None`; gives up
/// as soon as no alignment can stay within `k`.
fn within(a: &[char], b: &[char], k: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > k {
        return None;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        let mut row_min = cur[0];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
            row_min = row_min.min(cur[j + 1]);
        }
        if row_min > k {
            return None;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    Some(prev[b.len()]).filter(|&d| d <= k)
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
        Self::build(terms, MAX_TERMS)
    }

    fn build<S: AsRef<str>>(terms: &[S], cap: usize) -> Vocabulary {
        let mut out: Vec<Term> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for t in terms {
            let t = t.as_ref().trim();
            let w = words(t);
            if w.is_empty() || !seen.insert(w.clone()) {
                continue;
            }
            let chars: Vec<char> = w.join(" ").chars().collect();
            out.push(Term {
                text: t.to_string(),
                words: w,
                len: chars.len(),
                chars,
            });
            if out.len() == cap {
                break;
            }
        }
        // Longer terms first, so "Lê Minh Anh" wins over "Minh Anh".
        out.sort_by_key(|t| std::cmp::Reverse(t.words.len()));
        let mut groups: Vec<Group> = Vec::new();
        for (i, t) in out.iter().enumerate() {
            let n = t.words.len();
            if groups.last().is_none_or(|g| g.n != n) {
                groups.push(Group {
                    n,
                    by_len: Default::default(),
                });
            }
            if let Some(g) = groups.last_mut() {
                g.by_len.entry(t.len).or_default().push(i);
            }
        }
        Vocabulary { terms: out, groups }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// The term the window of `toks[i..i + g.n]` is a near-miss of, if any:
    /// the first in list order.
    fn find(&self, text: &str, toks: &[Tok], i: usize, g: &Group) -> Option<&Term> {
        let n = g.n;
        let wlen = toks[i..i + n].iter().map(|t| t.len).sum::<usize>() + n - 1;
        let mut buckets = (wlen.saturating_sub(MAX_BUDGET)..=wlen + MAX_BUDGET)
            .filter_map(|l| g.by_len.get(&l))
            .peekable();
        buckets.peek()?;
        let folded = toks[i..i + n]
            .iter()
            .map(|t| t.folded.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let window: Vec<char> = folded.chars().collect();
        let span = toks[i].range.start..toks[i + n - 1].range.end;
        let src = &text[span];
        let accented = ghi_text::has_diacritics(src);
        let src_lower = src.to_lowercase();
        let mut best: Option<usize> = None;
        for &c in buckets.flatten() {
            let t = &self.terms[c];
            if best.is_some_and(|b| b < c) {
                continue;
            }
            if within(&window, &t.chars, budget(t.len)).is_none() {
                continue;
            }
            // Words already written with diacritics are real Vietnamese
            // words ("mình" is not "Minh"): only an exact match counts.
            if accented && src_lower != t.text.to_lowercase() {
                continue;
            }
            best = Some(c);
        }
        best.map(|c| &self.terms[c])
    }

    /// `text` with near-misses of the terms replaced. Returns `None` when
    /// nothing changed.
    pub fn correct(&self, text: &str) -> Option<String> {
        if self.terms.is_empty() {
            return None;
        }
        // Work on the NFC text (what the store keeps).
        let nfc = ghi_text::nfc(text);
        let text = nfc.as_str();
        let toks = tokenize(text);
        let mut out = String::with_capacity(text.len());
        let mut last = 0usize;
        let mut changed = false;
        let mut i = 0usize;
        'outer: while i < toks.len() {
            for g in &self.groups {
                if i + g.n > toks.len() {
                    continue;
                }
                let Some(term) = self.find(text, &toks, i, g) else {
                    continue;
                };
                let span = toks[i].range.start..toks[i + g.n - 1].range.end;
                let src = &text[span.clone()];
                if src != term.text {
                    out.push_str(&text[last..span.start]);
                    out.push_str(&term.text);
                    last = span.end;
                    changed = true;
                }
                i += g.n;
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

/// The words of NFC `text`: byte ranges in it (the fold maps char ranges
/// back), folded forms and their lengths.
fn tokenize(text: &str) -> Vec<Tok> {
    let bytes: Vec<usize> = text
        .char_indices()
        .map(|(b, _)| b)
        .chain([text.len()])
        .collect();
    let folded = ghi_text::fold_mapped(text);
    ghi_text::tokens(&folded.text)
        .into_iter()
        .map(|(r, w)| {
            let c = folded.to_original(r);
            Tok {
                range: bytes[c.start]..bytes[c.end],
                len: w.chars().count(),
                folded: w,
            }
        })
        .collect()
}

/// A word of the text: byte range in the NFC text, folded form and its length.
struct Tok {
    range: std::ops::Range<usize>,
    folded: String,
    len: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The correction as it was before the length-bucket index: every term
    /// tried at every position, in list order. The reference for `correct`.
    fn naive_correct(v: &Vocabulary, text: &str) -> Option<String> {
        if v.terms.is_empty() {
            return None;
        }
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
        let mut out = String::new();
        let mut last = 0usize;
        let mut changed = false;
        let mut i = 0usize;
        'outer: while i < toks.len() {
            for t in &v.terms {
                let n = t.words.len();
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
                let target: Vec<char> = t.words.join(" ").chars().collect();
                if levenshtein(&window, &target) > budget(target.len()) {
                    continue;
                }
                let span = toks[i].0.start..toks[i + n - 1].0.end;
                let src = &text[span.clone()];
                if ghi_text::has_diacritics(src) && src.to_lowercase() != t.text.to_lowercase() {
                    continue;
                }
                if src != t.text {
                    out.push_str(&text[last..span.start]);
                    out.push_str(&t.text);
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

    /// A small deterministic generator (no dependency for tests).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) as usize
        }
        fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
            &v[self.next() % v.len()]
        }
    }

    /// One random edit (drop, double, swap, replace) of a word.
    fn mutate(r: &mut Rng, w: &str) -> String {
        let mut c: Vec<char> = w.chars().collect();
        if c.len() < 2 {
            return w.to_string();
        }
        let i = r.next() % c.len();
        match r.next() % 4 {
            0 => {
                c.remove(i);
            }
            1 => c.insert(i, c[i]),
            2 => {
                let j = (i + 1) % c.len();
                c.swap(i, j);
            }
            _ => c[i] = *r.pick(&['a', 'e', 'i', 'o', 'u', 'n', 't']),
        }
        c.into_iter().collect()
    }

    const POOL: &[&str] = &[
        "Lê Minh Anh",
        "Minh Anh",
        "Kubernetes",
        "Ghira",
        "AWS",
        "Đà Nẵng",
        "Nguyễn Văn An",
        "Văn An",
        "PostgreSQL",
        "huyết áp",
        "tăng huyết áp",
        "OAuth",
        "Hà Nội",
        "Trần",
        "an",
        "Lê",
        "Minh",
        "zebra",
        "Zebra Crossing",
        "kubectl",
        "metformin",
        "xin chào",
    ];
    const FILLER: &[&str] = &[
        "anh", "và", "đi", "the", "meeting", "mình", "tôi", "nói", "rằng", "we", "need", "an",
        "chị", "lên", "deploy", "today", "ok", "huyet", "ap", "da", "nang", "le", "tran",
    ];

    #[test]
    fn bucket_index_equals_the_naive_scan() {
        let mut r = Rng(7);
        for round in 0..400 {
            // A random subset of the pool, in a random order.
            let terms: Vec<&str> = (0..2 + r.next() % POOL.len())
                .map(|_| *r.pick(POOL))
                .collect();
            let v = Vocabulary::new(&terms);
            for _ in 0..8 {
                let mut words: Vec<String> = Vec::new();
                for _ in 0..4 + r.next() % 14 {
                    let w = if r.next().is_multiple_of(3) {
                        // A term's words, sometimes with an edit.
                        let t = *r.pick(&terms);
                        let t = if r.next().is_multiple_of(2) {
                            mutate(&mut r, t)
                        } else {
                            t.to_string()
                        };
                        if r.next().is_multiple_of(2) {
                            t.to_lowercase()
                        } else {
                            t
                        }
                    } else {
                        r.pick(FILLER).to_string()
                    };
                    words.push(w);
                }
                let text = words.join(" ");
                assert_eq!(
                    v.correct(&text),
                    naive_correct(&v, &text),
                    "round {round}: {terms:?} on {text:?}"
                );
            }
        }
    }

    #[test]
    fn all_packs_fit_under_their_own_cap() {
        let all: Vec<String> = packs().iter().flat_map(|p| p.terms.clone()).collect();
        assert!(all.len() <= MAX_PACK_TERMS, "{}", all.len());
        let ids: Vec<String> = packs().iter().map(|p| p.id.clone()).collect();
        assert_eq!(pack_terms(&ids).len(), all.len());
        // Apart from the user's cap: 200 user terms and every pack at once.
        assert!(MAX_TERMS + all.len() > MAX_TERMS);
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_pack_term_is_seen_only_when_said_exactly_in_its_language() {
        let seen = |lines: &[&str], packs: &[&str]| pack_terms_seen(lines, &ids(packs));
        // Positive, EN and VI (case ignored, order of first mention).
        assert_eq!(
            seen(
                &["My hypertension is high.", "Take Metformin daily."],
                &["medical-en"]
            ),
            ["hypertension", "metformin"]
        );
        assert_eq!(
            seen(&["Bác sĩ nói huyết áp của tôi hơi cao."], &["medical-vi"]),
            ["huyết áp"]
        );
        assert_eq!(
            seen(
                &["we run it on kubernetes", "the Kubernetes pod"],
                &["tech-en"]
            ),
            ["Kubernetes"],
            "once"
        );
        // Nothing fuzzy: common EN pairs the old corrector rewrote, inflections
        // and near-misses do not count.
        let common = [
            "I can",
            "I don't",
            "to do",
            "so to",
            "So can we",
            "pay me",
            "do the",
            "sharing the screen",
            "an evaluation",
            "he trusted me",
            "stating the facts",
            "the box was contained",
            "two diagnoses",
            "aspiring to learn",
            "next is lunch",
            "my hypertensin",
            "kubernetis",
        ];
        let all = [
            "medical-en",
            "medical-vi",
            "legal-en",
            "legal-vi",
            "finance-en",
            "finance-vi",
            "tech-en",
            "tech-vi",
        ];
        for line in common {
            assert!(
                seen(&[line], &all).is_empty(),
                "{line}: {:?}",
                seen(&[line], &all)
            );
        }
        // A Vietnamese term is not looked for in an English line, nor an English
        // term in a Vietnamese pack's language.
        assert!(seen(&["sổ đỏ"], &["legal-en"]).is_empty());
        assert!(seen(&["it is a so do"], &["legal-vi"]).is_empty());
        assert!(seen(&["the hypertension"], &["medical-vi"]).is_empty());
        // Accents belong to the term: "huyet ap" on a line that has accents elsewhere.
        assert!(seen(&["tôi bị huyet ap"], &["medical-vi"]).is_empty());
        // A pack that is off sees nothing.
        assert!(seen(&["hypertension"], &["tech-en"]).is_empty());
    }

    #[test]
    fn bundled_packs_are_well_formed() {
        let ids: Vec<&str> = packs().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ghi_store::sync::settings::PACK_IDS, "ids and order");
        for p in packs() {
            assert_eq!(p.id, format!("{}-{}", p.domain, p.lang));
            assert!(matches!(p.lang.as_str(), "en" | "vi"), "{}", p.id);
            assert!(
                (60..=150).contains(&p.terms.len()),
                "{}: {} terms",
                p.id,
                p.terms.len()
            );
            assert_eq!(
                p.needs_owner_review,
                matches!(p.id.as_str(), "medical-vi" | "legal-vi"),
                "{}",
                p.id
            );
            let folded: Vec<String> = p.terms.iter().map(|t| words(t).join(" ")).collect();
            for (i, t) in p.terms.iter().enumerate() {
                assert_eq!(t.trim(), t, "{}: `{t}`", p.id);
                assert!(!folded[i].is_empty(), "{}: blank term", p.id);
                assert!(!folded[..i].contains(&folded[i]), "{}: `{t}` twice", p.id);
            }
            if p.lang == "vi" {
                assert!(
                    p.terms.iter().any(|t| ghi_text::has_diacritics(t)),
                    "{}",
                    p.id
                );
            }
        }
    }

    #[test]
    fn enabled_packs_come_from_the_setting_and_unknown_ids_are_ignored() {
        use ghi_store::keys::{MemoryKeyStore, Protection};
        let dir = tempfile::tempdir().unwrap();
        let store = ghi_store::store::Store::open(
            dir.path(),
            std::sync::Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        assert!(enabled_packs(&store).unwrap().is_empty());
        store
            .set_setting(
                PACKS_SETTING,
                &serde_json::json!(["tech-vi", "nope", "medical-en"]),
            )
            .unwrap();
        assert_eq!(enabled_packs(&store).unwrap(), ["medical-en", "tech-vi"]);
    }

    /// Ten minutes of speech (150 words a minute, 15-word lines) with a term
    /// misspelled now and then.
    fn ten_minutes(terms: &[String]) -> Vec<String> {
        let mut r = Rng(99);
        (0..100)
            .map(|_| {
                (0..15)
                    .map(|_| {
                        if r.next().is_multiple_of(12) {
                            let t = r.pick(terms);
                            mutate(&mut r, t)
                        } else {
                            r.pick(FILLER).to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    /// The length-bucket index keeps the user's 200 terms well under the cost of
    /// the old scan (it was O(words x terms)); wall-clock, so best of 3.
    #[test]
    fn the_index_is_faster_than_the_full_scan_on_two_hundred_terms() {
        let user: Vec<String> = (0..MAX_TERMS)
            .map(|i| format!("{}{}", ["Tran", "Nguyen", "Pham", "Vo"][i % 4], i))
            .collect();
        let lines = ten_minutes(&user);
        let v = Vocabulary::new(&user);
        let best = |f: &dyn Fn(&str) -> Option<String>| {
            (0..3)
                .map(|_| {
                    let t = std::time::Instant::now();
                    for l in &lines {
                        std::hint::black_box(f(l));
                    }
                    t.elapsed()
                })
                .min()
                .unwrap()
        };
        let before = best(&|l| naive_correct(&v, l));
        let after = best(&|l| v.correct(l));
        eprintln!("200 terms, 10 min: naive scan {before:?}, indexed {after:?}");
        assert!(after <= before, "{after:?} > {before:?}");
    }
}
