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
/// and the user's list never cuts a pack's.
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

/// The terms of the packs named in `ids`, at most [`MAX_PACK_TERMS`].
pub fn pack_terms(ids: &[String]) -> Vec<&'static str> {
    packs()
        .iter()
        .filter(|p| ids.contains(&p.id))
        .flat_map(|p| p.terms.iter().map(String::as_str))
        .take(MAX_PACK_TERMS)
        .collect()
}

/// The strict vocabulary of the enabled packs, if any are on.
pub fn pack_vocabulary(store: &ghi_store::store::Store) -> Result<Option<Vocabulary>, String> {
    let ids = enabled_packs(store)?;
    Ok(Some(Vocabulary::pack(&pack_terms(&ids))).filter(|v| !v.is_empty()))
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
    /// The folded words joined by single spaces, and as chars.
    folded: String,
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
    /// Pack terms: see [`Vocabulary::pack`].
    strict: bool,
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

/// Pack terms are corrected by one edit at most, and only when the term has
/// this many characters.
const PACK_MIN_LEN: usize = 5;

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

/// `s` with its first letter upper-cased.
fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

impl Vocabulary {
    /// Builds the list (blank and duplicate terms dropped, at most [`MAX_TERMS`]).
    pub fn new<S: AsRef<str>>(terms: &[S]) -> Vocabulary {
        Self::build(terms, MAX_TERMS, false)
    }

    /// A pack's terms (at most [`MAX_PACK_TERMS`], a cap of their own so the
    /// user's list never crowds them out). Stricter than [`Vocabulary::new`]:
    /// one edit at most and only for terms of 5+ characters, a plural is not
    /// a near-miss, and a difference of case alone is left as spoken.
    pub fn pack<S: AsRef<str>>(terms: &[S]) -> Vocabulary {
        Self::build(terms, MAX_PACK_TERMS, true)
    }

    fn build<S: AsRef<str>>(terms: &[S], cap: usize, strict: bool) -> Vocabulary {
        let mut out: Vec<Term> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for t in terms {
            let t = t.as_ref().trim();
            let w = words(t);
            if w.is_empty() || !seen.insert(w.clone()) {
                continue;
            }
            let folded = w.join(" ");
            let chars: Vec<char> = folded.chars().collect();
            out.push(Term {
                text: t.to_string(),
                words: w,
                len: chars.len(),
                chars,
                folded,
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
        Vocabulary {
            terms: out,
            groups,
            strict,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Edits allowed for a term of `len` characters.
    fn allowed(&self, len: usize) -> usize {
        if self.strict {
            usize::from(len >= PACK_MIN_LEN)
        } else {
            budget(len)
        }
    }

    /// The term the window of `toks[i..i + g.n]` is a near-miss of, if any:
    /// the first in list order, or for a pack the closest (then first).
    fn find(&self, text: &str, toks: &[Tok], i: usize, g: &Group) -> Option<&Term> {
        let n = g.n;
        let wlen = toks[i..i + n].iter().map(|t| t.len).sum::<usize>() + n - 1;
        let reach = if self.strict { 1 } else { MAX_BUDGET };
        let mut buckets = (wlen.saturating_sub(reach)..=wlen + reach)
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
        let mut best: Option<(usize, usize)> = None; // (distance, index)
        for &c in buckets.flatten() {
            let t = &self.terms[c];
            if !self.strict && best.is_some_and(|(_, bi)| bi < c) {
                continue;
            }
            let Some(d) = within(&window, &t.chars, self.allowed(t.len)) else {
                continue;
            };
            // Words already written with diacritics are real Vietnamese
            // words ("mình" is not "Minh"): only an exact match counts.
            if accented && src_lower != t.text.to_lowercase() {
                continue;
            }
            // "tendons" is not a near-miss of "tendon".
            if self.strict && d == 1 {
                let (w, term) = (folded.as_str(), t.folded.as_str());
                if w.strip_suffix('s') == Some(term) || term.strip_suffix('s') == Some(w) {
                    continue;
                }
            }
            let key = (if self.strict { d } else { 0 }, c);
            if best.is_none_or(|b| key < b) {
                best = Some(key);
            }
        }
        best.map(|(_, c)| &self.terms[c])
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
        let toks: Vec<Tok> = ghi_text::tokens(&folded.text)
            .into_iter()
            .map(|(r, w)| {
                let c = folded.to_original(r);
                Tok {
                    range: bytes[c.start]..bytes[c.end],
                    len: w.chars().count(),
                    folded: w,
                }
            })
            .collect();
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
                let replacement = if self.strict {
                    // Case alone is the speaker's (or the sentence's).
                    if src.to_lowercase() == term.text.to_lowercase() {
                        None
                    } else if src.chars().next().is_some_and(char::is_uppercase)
                        && term.text.chars().next().is_some_and(char::is_lowercase)
                    {
                        Some(capitalize(&term.text))
                    } else {
                        Some(term.text.clone())
                    }
                } else {
                    (src != term.text).then(|| term.text.clone())
                };
                if let Some(r) = replacement {
                    out.push_str(&text[last..span.start]);
                    out.push_str(&r);
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
    fn packs_have_their_own_cap() {
        let many: Vec<String> = (0..1_500).map(|i| format!("glossaryterm{i}")).collect();
        assert_eq!(Vocabulary::new(&many).terms.len(), MAX_TERMS);
        assert_eq!(Vocabulary::pack(&many).terms.len(), MAX_PACK_TERMS);
        // Every pack at once fits.
        let all: Vec<String> = packs().iter().flat_map(|p| p.terms.clone()).collect();
        assert!(all.len() <= MAX_PACK_TERMS, "{}", all.len());
        let ids: Vec<String> = packs().iter().map(|p| p.id.clone()).collect();
        assert_eq!(pack_terms(&ids).len(), all.len());
        // The user's 200 terms and the packs' do not share a budget.
        let user = Vocabulary::new(&many[..MAX_TERMS]);
        assert_eq!(user.terms.len(), MAX_TERMS);
        assert_eq!(Vocabulary::pack(&all).terms.len(), {
            let mut f: Vec<_> = all.iter().map(|t| words(t)).collect();
            f.sort();
            f.dedup();
            f.len()
        });
    }

    #[test]
    fn pack_terms_are_corrected_by_one_edit_and_five_letters() {
        let v = Vocabulary::pack(&["tendon", "MRI", "lien", "bệnh viện"]);
        assert_eq!(
            v.correct("the tendom is sore").as_deref(),
            Some("the tendon is sore")
        );
        // Two edits: no. Under five letters: only exact.
        assert_eq!(v.correct("the tendxm is sore"), None);
        assert_eq!(v.correct("a lier and a mri"), None);
        // Case alone is left as spoken; a plural is not a near-miss.
        assert_eq!(v.correct("Tendon first, TENDON"), None);
        assert_eq!(v.correct("two tendons"), None);
        assert_eq!(
            v.correct("Tendom first").as_deref(),
            Some("Tendon first"),
            "sentence case kept"
        );
        // Accents come back on an unaccented exact match, but accented words stay.
        assert_eq!(v.correct("vao benh vien").as_deref(), Some("vao bệnh viện"));
        assert_eq!(v.correct("bênh viện"), None);
        // The user's list keeps its wider reach on the same text.
        assert!(
            Vocabulary::new(&["cardiology"])
                .correct("cardiolxgx")
                .is_some()
        );
        assert_eq!(
            Vocabulary::pack(&["cardiology"]).correct("cardiolxgx"),
            None
        );
    }

    const COMMON_EN: &[&str] = &[
        "Thanks for joining the call today, let's start with the agenda and the last action items.",
        "I think the contract is fine but the contact person changed, so please confirm the cause.",
        "We moved the meeting to Thursday because the revenge of the weather was too much to handle.",
        "The patient parent said the station was closed, so they took a different route home.",
        "Please share your screen, the audio is breaking up and the video is frozen again.",
        "He felt a tension in the room and lost his balance for a moment, then kept walking.",
        "The report is divided into three parts: background, results and a short conclusion.",
        "Our team will print the documents, then reach out to the other office about the lease.",
        "Can you hear me? I will send the schema, the scheme and the plan after lunch tomorrow.",
        "It is a nice day, the sky is clear, and we should probably go for a walk after the call.",
        "Inflation is high this year and the price of coffee and fuel has gone up again.",
        "The doctor's office called to say that the appointment is on Tuesday at nine in the morning.",
    ];
    const COMMON_VI: &[&str] = &[
        "Chào mọi người, hôm nay chúng ta họp về kế hoạch quý sau và các việc còn tồn đọng.",
        "Mình nghĩ là hợp đồng này ổn, nhưng cần hỏi lại bên kia về thời hạn thanh toán.",
        "Anh ấy bị đau đầu từ hôm qua nên đã nghỉ làm, chiều nay sẽ đi khám.",
        "Chị Lan nói rằng cuối tuần này cả nhà sẽ về quê thăm ông bà và ăn cơm cùng nhau.",
        "Giá xăng tăng nên chi phí đi lại của công ty cũng tăng theo, phải tính lại ngân sách.",
        "Em gửi lại file cho anh nhé, nếu có gì chưa rõ thì mình trao đổi thêm vào buổi chiều.",
        "Hom nay troi dep qua, chung ta di an trua roi quay lai lam tiep nhe moi nguoi.",
    ];

    #[test]
    fn packs_do_not_rewrite_everyday_speech() {
        let all: Vec<String> = packs().iter().flat_map(|p| p.terms.clone()).collect();
        let v = Vocabulary::pack(&all);
        for s in COMMON_EN.iter().chain(COMMON_VI) {
            assert_eq!(v.correct(s), None, "{s}");
        }
    }

    #[test]
    fn packs_fix_near_misses_of_their_terms() {
        let on = |ids: &[&str]| {
            let ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
            Vocabulary::pack(&pack_terms(&ids))
        };
        assert_eq!(
            on(&["medical-en"])
                .correct("my hypertensin and the metformn")
                .as_deref(),
            Some("my hypertension and the metformin")
        );
        assert_eq!(
            on(&["medical-vi"]).correct("bị tang huyet ap").as_deref(),
            Some("bị tăng huyết áp")
        );
        assert_eq!(
            on(&["tech-en"]).correct("deploy to kubernetis").as_deref(),
            Some("deploy to Kubernetes")
        );
        // A pack that is off does nothing.
        assert_eq!(on(&["finance-en"]).correct("hypertensin"), None);
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
            // Two terms one edit apart would correct each other's spelling.
            for (i, a) in folded.iter().enumerate() {
                for b in &folded[..i] {
                    // (In Vietnamese the accents of real words guard them.)
                    if p.lang == "en"
                        && a.chars().count() >= PACK_MIN_LEN
                        && b.chars().count() >= PACK_MIN_LEN
                    {
                        let (ac, bc): (Vec<char>, Vec<char>) =
                            (a.chars().collect(), b.chars().collect());
                        let plural =
                            a.strip_suffix('s') == Some(b) || b.strip_suffix('s') == Some(a);
                        assert!(
                            plural || levenshtein(&ac, &bc) > 1,
                            "{}: `{a}` ~ `{b}`",
                            p.id
                        );
                    }
                }
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
        assert!(pack_vocabulary(&store).unwrap().is_none());
        store
            .set_setting(
                PACKS_SETTING,
                &serde_json::json!(["tech-vi", "nope", "medical-en"]),
            )
            .unwrap();
        assert_eq!(enabled_packs(&store).unwrap(), ["medical-en", "tech-vi"]);
        let v = pack_vocabulary(&store).unwrap().unwrap();
        assert!(v.correct("hypertensin").is_some());
    }

    /// A 60-minute transcript (150 words a minute, 15-word lines) with a term
    /// misspelled now and then.
    fn hour(terms: &[String]) -> Vec<String> {
        let mut r = Rng(99);
        (0..600)
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

    #[test]
    fn two_hundred_user_and_six_hundred_pack_terms_are_no_slower_than_two_hundred_naive() {
        let user: Vec<String> = (0..MAX_TERMS)
            .map(|i| format!("{}{}", ["Tran", "Nguyen", "Pham", "Vo"][i % 4], i))
            .collect();
        let pack: Vec<String> = packs()
            .iter()
            .flat_map(|p| p.terms.clone())
            .take(600)
            .collect();
        assert_eq!(pack.len(), 600);
        let mut say = user.clone();
        say.extend(pack.iter().cloned());
        let lines = hour(&say);
        let (uv, pv) = (Vocabulary::new(&user), Vocabulary::pack(&pack));
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
        let before = best(&|l| naive_correct(&uv, l));
        let after = best(&|l| {
            let a = uv.correct(l);
            pv.correct(a.as_deref().unwrap_or(l)).or(a)
        });
        eprintln!("200 terms, naive scan: {before:?}; 200 + 600 terms, indexed: {after:?}");
        assert!(after <= before, "{after:?} > {before:?}");
    }
}
