// SPDX-License-Identifier: Apache-2.0
//! PII redaction before a cloud send (doc 05 §1.6): deterministic patterns, spoken forms, known entities; reversible.
//!
//! What leaves the device is the redacted text; the [`Redactor`] keeps the map
//! from placeholders (`<<EMAIL_1>>`) back to the original values and
//! [`Redactor::restore`] puts them back into the model's output.
//!
//! 1. **Spoken forms** are normalised first (RT-13): "không chín một hai ..."
//!    or "oh nine one ..." become digits, "a còng" / "at" become `@` and
//!    "chấm" / "dot" become `.` inside email-like runs. Conservative: a span is
//!    only rewritten when the result is something layer 2 would redact.
//! 2. **Layer 1**, patterns: emails, URLs, phone numbers (VN and `+CC`),
//!    CCCD/CMND, payment cards (Luhn), account numbers.
//! 3. **Layer 2**, [`KnownEntities`]: people, organisations and terms the
//!    caller knows about, matched without case or diacritics, whole words.
//!
//! The same value always gets the same placeholder. Restoring is forgiving
//! about how a model mangles one (`<EMAIL_1>`, `[email 1]`, bare `EMAIL_1`) and
//! counts the ones it cannot resolve.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use crate::Transcript;
use crate::transcript::Segment;

/// Names to redact with layer 2. Matching is case- and diacritic-insensitive
/// and needs whole words; give the forms people say (full name, first name).
#[derive(Debug, Clone, Default)]
pub struct KnownEntities {
    pub people: Vec<String>,
    pub orgs: Vec<String>,
    pub terms: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Email,
    Url,
    Phone,
    Id,
    Card,
    Account,
    Person,
    Org,
    Term,
}

impl Kind {
    const ALL: [Kind; 9] = [
        Kind::Email,
        Kind::Url,
        Kind::Phone,
        Kind::Id,
        Kind::Card,
        Kind::Account,
        Kind::Person,
        Kind::Org,
        Kind::Term,
    ];

    fn tag(self) -> &'static str {
        match self {
            Kind::Email => "EMAIL",
            Kind::Url => "URL",
            Kind::Phone => "PHONE",
            Kind::Id => "ID",
            Kind::Card => "CARD",
            Kind::Account => "ACCOUNT",
            Kind::Person => "PERSON",
            Kind::Org => "ORG",
            Kind::Term => "TERM",
        }
    }

    fn from_tag(tag: &str) -> Option<Kind> {
        Kind::ALL
            .into_iter()
            .find(|k| k.tag().eq_ignore_ascii_case(tag))
    }
}

#[derive(Debug, Clone)]
struct Entity {
    kind: Kind,
    canonical: String,
    folded: String,
}

/// Redacts text and remembers how to undo it. One per cloud request: the
/// placeholder numbering is shared across every text it redacts.
#[derive(Debug, Clone)]
pub struct Redactor {
    /// Longest first, so "Nguyễn Văn An" wins over "An".
    entities: Vec<Entity>,
    counts: HashMap<Kind, usize>,
    by_value: HashMap<(Kind, String), usize>,
    originals: HashMap<(Kind, usize), String>,
}

/// Model output with placeholders put back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    pub text: String,
    /// Placeholders in the text with no entry in the map (the model invented
    /// or garbled one); they are left as they were.
    pub unresolved: usize,
}

impl Redactor {
    pub fn new(known: KnownEntities) -> Redactor {
        let mut entities = Vec::new();
        for (kind, list) in [
            (Kind::Person, known.people),
            (Kind::Org, known.orgs),
            (Kind::Term, known.terms),
        ] {
            for name in list {
                let canonical = ghi_text::nfc(name.trim());
                let folded = ghi_text::fold(&canonical);
                if !folded.is_empty() && !entities.iter().any(|e: &Entity| e.folded == folded) {
                    entities.push(Entity {
                        kind,
                        canonical,
                        folded,
                    });
                }
            }
        }
        entities.sort_by(|a, b| {
            b.folded
                .chars()
                .count()
                .cmp(&a.folded.chars().count())
                .then_with(|| a.folded.cmp(&b.folded))
        });
        Redactor {
            entities,
            counts: HashMap::new(),
            by_value: HashMap::new(),
            originals: HashMap::new(),
        }
    }

    /// `text` with spoken forms normalised and every match replaced by a
    /// placeholder. The output is NFC.
    pub fn redact(&mut self, text: &str) -> String {
        let text = ghi_text::nfc(text);
        let text = normalise_spoken(&text);
        let text = self.replace_emails_and_urls(&text);
        let text = self.replace_numbers(&text);
        self.replace_entities(&text)
    }

    /// The same transcript with every segment's text redacted; ids, times,
    /// speakers and languages are kept.
    pub fn redact_transcript(&mut self, t: &Transcript) -> Transcript {
        let segments: Vec<Segment> = t
            .segments()
            .iter()
            .map(|s| Segment {
                text: self.redact(&s.text),
                ..s.clone()
            })
            .collect();
        Transcript::new(segments)
            .expect("ids of a valid transcript stay unique")
            .with_speaker_names(t.speaker_names().clone())
    }

    /// Puts originals back for every placeholder in `text`.
    pub fn restore(&self, text: &str) -> Restored {
        let mut unresolved = 0;
        let out = RESTORE.replace_all(text, |c: &regex::Captures| {
            let tag = c.get(1).or(c.get(3)).map_or("", |m| m.as_str());
            let num = c.get(2).or(c.get(4)).map_or("", |m| m.as_str());
            let found = Kind::from_tag(tag)
                .zip(num.parse::<usize>().ok())
                .and_then(|key| self.originals.get(&key));
            match found {
                Some(original) => original.clone(),
                None => {
                    unresolved += 1;
                    c[0].to_owned()
                }
            }
        });
        Restored {
            text: out.into_owned(),
            unresolved,
        }
    }

    /// How many distinct values were replaced, by placeholder kind (`EMAIL`,
    /// `PHONE`, ...), for the preview.
    pub fn summary(&self) -> Vec<(&'static str, usize)> {
        Kind::ALL
            .into_iter()
            .filter_map(|k| self.counts.get(&k).map(|n| (k.tag(), *n)))
            .collect()
    }

    /// The placeholder for `value` (new or seen before).
    fn placeholder(&mut self, kind: Kind, key: String, original: &str) -> String {
        let n = match self.by_value.get(&(kind, key.clone())) {
            Some(n) => *n,
            None => {
                let n = self.counts.entry(kind).or_default();
                *n += 1;
                let n = *n;
                self.by_value.insert((kind, key), n);
                self.originals.insert((kind, n), original.to_owned());
                n
            }
        };
        format!("<<{}_{}>>", kind.tag(), n)
    }

    fn replace_emails_and_urls(&mut self, text: &str) -> String {
        let text = EMAIL.replace_all(text, |c: &regex::Captures| {
            self.placeholder(Kind::Email, c[0].to_lowercase(), &c[0])
        });
        URL.replace_all(&text, |c: &regex::Captures| {
            self.placeholder(Kind::Url, c[0].to_lowercase(), &c[0])
        })
        .into_owned()
    }

    fn replace_numbers(&mut self, text: &str) -> String {
        let chars: Vec<char> = text.chars().collect();
        let mut spans: Vec<(Range<usize>, String)> = Vec::new();
        for run in scan_runs(&chars) {
            let mut i = 0;
            while i < run.groups.len() {
                match classify_groups(&chars, &run, i) {
                    Some((kind, end)) => {
                        let span =
                            run.groups[i].start_with_prefix(&run, i)..run.groups[end - 1].end;
                        let original: String = chars[span.clone()].iter().collect();
                        let digits: String = run.groups[i..end]
                            .iter()
                            .flat_map(|g| chars[g.start..g.end].iter())
                            .collect();
                        let key = if run.plus && i == 0 {
                            format!("+{digits}")
                        } else {
                            digits
                        };
                        spans.push((span, self.placeholder(kind, key, &original)));
                        i = end;
                    }
                    None => i += 1,
                }
            }
        }
        replace_ranges(&chars, spans)
    }

    fn replace_entities(&mut self, text: &str) -> String {
        if self.entities.is_empty() {
            return text.to_owned();
        }
        let chars: Vec<char> = text.chars().collect();
        let folded = ghi_text::fold_mapped(text);
        let protected: Vec<Range<usize>> = PLACEHOLDER
            .find_iter(&folded.text)
            .map(|m| m.range())
            .collect();
        let mut taken: Vec<Range<usize>> = Vec::new();
        let mut found: Vec<(Range<usize>, usize)> = Vec::new();
        for (i, e) in self.entities.iter().enumerate() {
            for (at, _) in folded.text.match_indices(&e.folded) {
                let r = at..at + e.folded.len();
                if !whole_word(&folded.text, &r)
                    || protected.iter().chain(&taken).any(|p| overlaps(p, &r))
                {
                    continue;
                }
                taken.push(r.clone());
                found.push((r, i));
            }
        }
        // Number placeholders in reading order.
        found.sort_by_key(|(r, _)| r.start);
        let mut spans: Vec<(Range<usize>, String)> = Vec::new();
        for (r, i) in found {
            let (kind, folded_name, canonical) = {
                let e = &self.entities[i];
                (e.kind, e.folded.clone(), e.canonical.clone())
            };
            let ph = self.placeholder(kind, folded_name, &canonical);
            spans.push((span_to_orig(&folded, &r), ph));
        }
        replace_ranges(&chars, spans)
    }
}

/// `<<EMAIL_1>>` and the ways a model mangles it: other brackets, spaces or a
/// dash instead of the underscore, lowercase; bare `EMAIL_1` with an
/// underscore.
static RESTORE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)(?:<<|<|\[|\(\()\s*({k})[\s_\-]*(\d+)\s*(?:>>|>|\]|\)\))|\b({k})_(\d+)\b",
        k = "EMAIL|PHONE|URL|ID|CARD|ACCOUNT|PERSON|ORG|TERM"
    ))
    .expect("valid regex")
});
static PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<<[a-z]+_[0-9]+>>").expect("valid regex"));

static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\w.%+\-]+@[\w\-]+(?:\.[\w\-]+)*\.[A-Za-z]{2,}").expect("valid regex")
});
/// A scheme or `www.` URL, or a bare domain on a common TLD. Trailing
/// punctuation is not part of it.
static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)(?:\b(?:https?://|www\.)[^\s<>\x22']*[^\s<>\x22'.,;:!?)\]]",
        r"|\b(?:[a-z0-9\-]+\.)+(?:com|net|org|vn|io|co|edu|gov|ai|app|dev|info|biz|me|us|uk|xyz|cloud|tech)",
        r"(?:\.[a-z]{2})?\b(?:/[^\s<>\x22']*[^\s<>\x22'.,;:!?)\]])?)",
    ))
    .expect("valid regex")
});

/// Whether the folded match `r` is bounded by non-alphanumerics (or the ends).
fn whole_word(folded: &str, r: &Range<usize>) -> bool {
    let before = folded[..r.start].chars().next_back();
    let after = folded[r.end..].chars().next();
    !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
}

fn overlaps(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

/// Maps a byte range of the folded text to a char range of the NFC original.
fn span_to_orig(folded: &ghi_text::Folded, r: &Range<usize>) -> Range<usize> {
    let start = folded.text[..r.start].chars().count();
    let end = start + folded.text[r.clone()].chars().count();
    folded.to_original(start..end)
}

/// Replaces char ranges (non-overlapping) of `chars` with strings.
fn replace_ranges(chars: &[char], mut spans: Vec<(Range<usize>, String)>) -> String {
    spans.sort_by_key(|(r, _)| r.start);
    let mut out = String::with_capacity(chars.len());
    let mut at = 0;
    for (r, s) in spans {
        if r.start < at {
            continue;
        }
        out.extend(&chars[at..r.start]);
        out.push_str(&s);
        at = r.end;
    }
    out.extend(&chars[at..]);
    out
}

// ---------------------------------------------------------------- numbers

/// A digit group inside a [`Run`], as a char range.
#[derive(Debug, Clone)]
struct Group {
    start: usize,
    end: usize,
    /// Start of a `+` or `(` just before the group, if any.
    lead: Option<usize>,
}

impl Group {
    /// Where a match starting at this group begins (a leading `+`/`(` belongs
    /// to the run's first group).
    fn start_with_prefix(&self, _run: &Run, index: usize) -> usize {
        if index == 0 {
            self.lead.unwrap_or(self.start)
        } else {
            self.start
        }
    }
}

/// Digit groups joined by single separators (space, `.`, `-`, parentheses): a
/// candidate phone number, id or card.
#[derive(Debug)]
struct Run {
    groups: Vec<Group>,
    plus: bool,
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn scan_runs(chars: &[char]) -> Vec<Run> {
    let len = chars.len();
    let digit = |i: usize| i < len && chars[i].is_ascii_digit();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < len {
        let lead_at = i;
        let mut j = i;
        let mut plus = false;
        if chars[j] == '(' {
            j += 1;
        }
        if j < len && chars[j] == '+' {
            plus = true;
            j += 1;
        }
        let starts_ok = lead_at == 0 || !is_word_char(chars[lead_at - 1]);
        if !digit(j) || !starts_ok {
            i += 1;
            continue;
        }
        let mut groups = Vec::new();
        let mut lead = (j > lead_at).then_some(lead_at);
        loop {
            let gs = j;
            while digit(j) {
                j += 1;
            }
            groups.push(Group {
                start: gs,
                end: j,
                lead: lead.take(),
            });
            let mut k = j;
            if k < len && chars[k] == ')' {
                k += 1;
            }
            let after_paren = k > j;
            if k < len && matches!(chars[k], ' ' | '.' | '-') {
                k += 1;
            } else if !after_paren {
                break;
            }
            if k < len && chars[k] == '(' {
                k += 1;
            }
            if !digit(k) {
                break;
            }
            j = k;
        }
        i = j.max(i + 1);
        // A number glued to letters ("ID123456789", "12345678abc") is not one.
        if i < len && is_word_char(chars[i]) {
            continue;
        }
        runs.push(Run { groups, plus });
    }
    runs
}

/// The kind of the first valid match starting at group `from`, and the group
/// index after it.
fn classify_groups(chars: &[char], run: &Run, from: usize) -> Option<(Kind, usize)> {
    let plus = run.plus && from == 0;
    let digits_of = |to: usize| -> String {
        run.groups[from..to]
            .iter()
            .flat_map(|g| chars[g.start..g.end].iter())
            .collect()
    };
    let contiguous_ok = |to: usize| to == from + 1;
    let classify = |to: usize| -> Option<Kind> {
        let d = digits_of(to);
        let n = d.len();
        if plus {
            // International: every later group has 2+ digits (a stray "2"
            // after a number is not part of it).
            let groups_ok = run.groups[from + 1..to]
                .iter()
                .all(|g| g.end - g.start >= 2);
            return ((8..=15).contains(&n) && groups_ok).then_some(Kind::Phone);
        }
        if contiguous_ok(to) {
            return match n {
                12 | 9 => Some(Kind::Id),
                10 | 11 if d.starts_with('0') => Some(Kind::Phone),
                11 if d.starts_with("84") => Some(Kind::Phone),
                13..=19 if luhn(&d) => Some(Kind::Card),
                8..=19 => Some(Kind::Account),
                _ => None,
            };
        }
        let groups_ok = run.groups[from..to].iter().all(|g| g.end - g.start >= 2);
        if (13..=19).contains(&n) && luhn(&d) && groups_ok {
            return Some(Kind::Card);
        }
        let vn = (d.starts_with('0') && (10..=11).contains(&n))
            || (d.starts_with("84") && (11..=12).contains(&n));
        if vn && groups_ok {
            return Some(Kind::Phone);
        }
        // Grouped IDs and account numbers ("001 099 012 345", "1903-6789-0123"):
        // groups of 3–4 digits. A last group of "000" is an amount
        // ("100.000.000 đồng"), not an ID.
        let len = |g: &Group| g.end - g.start;
        let blocks = run.groups[from..to - 1]
            .iter()
            .all(|g| (3..=4).contains(&len(g)))
            && (1..=4).contains(&len(&run.groups[to - 1]));
        let last = &run.groups[to - 1];
        let amount = chars[last.start..last.end].iter().all(|c| *c == '0');
        if !blocks || amount {
            return None;
        }
        match n {
            9 | 12 if len(last) >= 3 => Some(Kind::Id),
            10..=19 => Some(Kind::Account),
            _ => None,
        }
    };
    // A long first group ("0987654321 0912 345 678") is a number of its own.
    let first = &run.groups[from];
    if !plus && first.end - first.start >= 8 {
        return classify(from + 1).map(|k| (k, from + 1));
    }
    // Otherwise the longest match: "+84 912 345 678" must not stop at
    // "+84 912 345", nor a 16-digit card or a 12-digit ID at 9 digits.
    (from + 1..=run.groups.len())
        .rev()
        .find_map(|to| classify(to).map(|k| (k, to)))
}

fn luhn(digits: &str) -> bool {
    let mut sum = 0;
    for (i, c) in digits.chars().rev().enumerate() {
        let Some(mut d) = c.to_digit(10) else {
            return false;
        };
        if i % 2 == 1 {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
    }
    sum % 10 == 0
}

// ---------------------------------------------------------- spoken forms

const DIGIT_WORDS: &str = "khong|mot|hai|ba|bon|nam|sau|bay|tam|chin|zero|oh|one|two|three|four|five|six|seven|eight|nine";
const DOT_WORDS: &str = "cham|dot";

fn digit_of(word: &str) -> Option<char> {
    Some(match word {
        "khong" | "zero" | "oh" => '0',
        "mot" | "one" => '1',
        "hai" | "two" => '2',
        "ba" | "three" => '3',
        "bon" | "four" => '4',
        "nam" | "five" => '5',
        "sau" | "six" => '6',
        "bay" | "seven" => '7',
        "tam" | "eight" => '8',
        "chin" | "nine" => '9',
        _ => return None,
    })
}

/// "nam a cong gmail cham com", "nam at gmail dot com": a local part, an
/// at-word or `@`, then a dotted domain. Run on folded text.
static SPOKEN_EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"\b([a-z0-9_]+(?:(?:\.|\s+(?:{d})\s+)[a-z0-9_]+)*)(?:\s*@\s*|\s+(?:a\s?cong|at)\s+)([a-z0-9\-]+(?:(?:\.|\s+(?:{d})\s+)[a-z0-9\-]+)+)\b",
        d = DOT_WORDS
    ))
    .expect("valid regex")
});
static SPOKEN_DOT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"\s+(?:{DOT_WORDS})\s+")).expect("valid regex"));
/// 8+ digit words/digits in a row, optionally after "cộng"/"plus".
static SPOKEN_DIGITS: LazyLock<Regex> = LazyLock::new(|| {
    let tok = format!(r"(?:{DIGIT_WORDS}|\d+)");
    Regex::new(&format!(
        r"\b(?:(?:cong|plus)\s+)?{tok}(?:(?:[\s,.\-]+|\s+(?:{DOT_WORDS})\s+){tok}){{7,}}\b"
    ))
    .expect("valid regex")
});
static SPOKEN_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"{DIGIT_WORDS}|\d+")).expect("valid regex"));

/// Rewrites spoken emails and digit runs as written ones, so layer 1 sees
/// them. Only spans that turn into an email or an 8-19 digit number change.
fn normalise_spoken(text: &str) -> String {
    let text = rewrite_folded(text, &SPOKEN_EMAIL, |c| {
        let m = &c[0];
        // Plain `a@b.com` (no spoken words) is layer 1's job.
        if !m.contains(char::is_whitespace) {
            return None;
        }
        let despoken = |s: &str| {
            SPOKEN_DOT
                .replace_all(s, ".")
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
        };
        Some(format!("{}@{}", despoken(&c[1]), despoken(&c[2])))
    });
    rewrite_folded(&text, &SPOKEN_DIGITS, |c| {
        let m = &c[0];
        let mut digits = String::new();
        let mut words = 0;
        for t in SPOKEN_TOKEN.find_iter(m) {
            match digit_of(t.as_str()) {
                Some(d) => {
                    digits.push(d);
                    words += 1;
                }
                None => digits.push_str(t.as_str()),
            }
        }
        // Real digits alone are layer 1's job.
        if words == 0 || !(8..=19).contains(&digits.len()) {
            return None;
        }
        let plus = m.starts_with("cong") || m.starts_with("plus");
        Some(if plus { format!("+{digits}") } else { digits })
    })
}

/// Runs `re` over the folded text and applies `f`'s replacement to the NFC
/// original at the same places.
fn rewrite_folded(
    text: &str,
    re: &Regex,
    f: impl Fn(&regex::Captures) -> Option<String>,
) -> String {
    let folded = ghi_text::fold_mapped(text);
    let mut spans = Vec::new();
    for c in re.captures_iter(&folded.text) {
        if let Some(rep) = f(&c) {
            let m = c.get(0).expect("group 0 always matches");
            spans.push((span_to_orig(&folded, &m.range()), rep));
        }
    }
    if spans.is_empty() {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    replace_ranges(&chars, spans)
}

// --------------------------------------------------------------- warnings

static DIGIT_RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d[\d \-.]{4,}\d").expect("valid regex"));

/// Things in already-redacted `text` that still look like personal data, for
/// the preview. Messages never quote the text.
pub fn warnings(text: &str) -> Vec<String> {
    let bare = PLACEHOLDER.replace_all(text, " ");
    let mut out = Vec::new();
    let mut add = |s: &str| {
        if !out.iter().any(|o: &String| o == s) {
            out.push(s.to_owned());
        }
    };
    if DIGIT_RUN
        .find_iter(&bare)
        .any(|m| m.as_str().chars().filter(char::is_ascii_digit).count() >= 6)
    {
        add("a long number (6+ digits) is still in the text: a phone, account or id?");
    }
    if bare.contains('@') {
        add("an @ is still in the text: an email or handle?");
    }
    let folded = ghi_text::fold(&bare);
    if SPOKEN_DIGITS.is_match(&folded) {
        add("spoken digits are still in the text: a number read aloud?");
    }
    if SPOKEN_EMAIL.is_match(&folded) {
        add("a spoken email (\"at ... dot ...\") may still be in the text");
    }
    if EMAIL.is_match(&bare) || URL.is_match(&bare) {
        add("an email or web address is still in the text");
    }
    out
}

#[cfg(test)]
mod tests {

    #[test]
    fn grouped_ids_and_accounts_are_redacted_but_amounts_are_not() {
        for (text, kind) in [
            ("CCCD 001 099 012 345 nhé", "ID"),
            ("CCCD 001.099.012.345 nhé", "ID"),
            ("CCCD 001-099-012-345 nhé", "ID"),
            ("CMND 123 456 789 nhé", "ID"),
            ("TK 1903 6789 0123 45 nhé", "ACCOUNT"),
        ] {
            let mut r = Redactor::new(KnownEntities::default());
            let out = r.redact(text);
            assert!(out.contains(&format!("<<{kind}_1>>")), "{text} -> {out}");
            let digits = out.chars().filter(char::is_ascii_digit).count();
            assert_eq!(digits, 1, "only the placeholder's number is left: {out}");
        }
        let mut r = Redactor::new(KnownEntities::default());
        for amount in [
            "giá 100.000.000 đồng",
            "giá 99.000 đồng",
            "giá 1.500.000 đồng",
        ] {
            assert_eq!(r.redact(amount), amount);
        }
    }

    use super::*;
    use proptest::prelude::*;

    fn redactor() -> Redactor {
        Redactor::new(KnownEntities::default())
    }

    fn redact(text: &str) -> String {
        redactor().redact(text)
    }

    #[test]
    fn written_patterns() {
        assert_eq!(
            redact("Mail nam.tran@acme.com.vn hoặc gọi 0912 345 678 nhé."),
            "Mail <<EMAIL_1>> hoặc gọi <<PHONE_1>> nhé."
        );
        assert_eq!(redact("Call +84 912 345 678."), "Call <<PHONE_1>>.");
        assert_eq!(redact("Gọi (+84) 912-345-678 nha"), "Gọi <<PHONE_1>> nha");
        assert_eq!(
            redact("call +1 (415) 555-2671 today"),
            "call <<PHONE_1>> today"
        );
        assert_eq!(redact("0912.345.678"), "<<PHONE_1>>");
        assert_eq!(redact("CCCD 001099012345 của anh"), "CCCD <<ID_1>> của anh");
        assert_eq!(redact("CMND 123456789"), "CMND <<ID_1>>");
        assert_eq!(
            redact("thẻ 4111 1111 1111 1111 hết hạn"),
            "thẻ <<CARD_1>> hết hạn"
        );
        assert_eq!(redact("card 4111-1111-1111-1111."), "card <<CARD_1>>.");
        assert_eq!(
            redact("tài khoản 19036789012345"),
            "tài khoản <<ACCOUNT_1>>"
        );
        assert_eq!(
            redact("xem https://acme.com/docs?id=7, và www.acme.vn/x."),
            "xem <<URL_1>>, và <<URL_2>>."
        );
        assert_eq!(redact("see docs.acme.io/start now"), "see <<URL_1>> now");
    }

    #[test]
    fn does_not_redact_ordinary_numbers() {
        for s in [
            "Họp lúc 09:30 ngày 2024-10-01 với 12 người.",
            "Ngân sách 10,000,000 đồng, tăng 15%.",
            "Version 1.2.3 ra ngày 01.10.2026",
            "chúng tôi có 3 con mèo và 12 tuổi",
            "room 12 34 56",
        ] {
            assert_eq!(redact(s), s);
        }
    }

    #[test]
    fn same_value_same_placeholder_and_adjacent_numbers_split() {
        let mut r = redactor();
        let out = r.redact("a@x.com, A@X.com, b@y.com; 0912345678 0987654321 0912 345 678");
        assert_eq!(
            out,
            "<<EMAIL_1>>, <<EMAIL_1>>, <<EMAIL_2>>; <<PHONE_1>> <<PHONE_2>> <<PHONE_1>>"
        );
        assert_eq!(r.summary(), vec![("EMAIL", 2), ("PHONE", 2)]);
    }

    #[test]
    fn vietnamese_spoken_digits() {
        assert_eq!(
            redact("số của em là không chín một hai ba bốn năm sáu bảy tám nhé"),
            "số của em là <<PHONE_1>> nhé"
        );
        // Diacritics are optional (ASR often drops them).
        assert_eq!(
            redact("khong chin mot hai ba bon nam sau bay tam"),
            "<<PHONE_1>>"
        );
        assert_eq!(
            redact("cộng tám bốn chín một hai ba bốn năm sáu bảy"),
            "<<PHONE_1>>"
        );
        // CCCD read aloud: 12 digits.
        assert_eq!(
            redact("không không một không chín chín không một hai ba bốn năm"),
            "<<ID_1>>"
        );
        // Words alone are not a number.
        assert_eq!(redact("hai ba năm sau"), "hai ba năm sau");
        assert_eq!(
            redact("năm nay mình có sáu bảy người"),
            "năm nay mình có sáu bảy người"
        );
    }

    #[test]
    fn english_spoken_digits() {
        assert_eq!(
            redact("my number is oh nine one two three four five six seven eight"),
            "my number is <<PHONE_1>>"
        );
        assert_eq!(
            redact("zero nine one two, three four five, six seven eight"),
            "<<PHONE_1>>"
        );
        assert_eq!(redact("one two three four"), "one two three four");
        assert_eq!(
            redact("plus eight four nine one two three four five six seven"),
            "<<PHONE_1>>"
        );
    }

    #[test]
    fn spoken_emails() {
        assert_eq!(
            redact("mail cho em qua nam a còng gmail chấm com nhé"),
            "mail cho em qua <<EMAIL_1>> nhé"
        );
        assert_eq!(
            redact("nguyen chấm van a còng acme chấm com chấm vn"),
            "<<EMAIL_1>>"
        );
        assert_eq!(
            redact("email john dot smith at gmail dot com please"),
            "email <<EMAIL_1>> please"
        );
        assert_eq!(redact("nam at gmail.com"), "<<EMAIL_1>>");
        // Ordinary prose with "at" or "chấm" is left alone.
        for s in [
            "let's meet at noon, the dot on the map",
            "chấm điểm xong rồi",
            "we arrive at the office",
        ] {
            assert_eq!(redact(s), s);
        }
        // Same address spoken and written shares a placeholder.
        let mut r = redactor();
        assert_eq!(
            r.redact("nam a còng gmail chấm com / nam@gmail.com"),
            "<<EMAIL_1>> / <<EMAIL_1>>"
        );
    }

    #[test]
    fn known_entities_ignore_case_and_diacritics() {
        let mut r = Redactor::new(KnownEntities {
            people: vec!["Nguyễn Văn An".into(), "An".into(), "Linh".into()],
            orgs: vec!["Acme Corp".into()],
            terms: vec!["Project Falcon".into()],
        });
        let out = r.redact(
            "nguyen van an gặp ANH An và linh ở Acme Corp về project falcon. Lan không bị.",
        );
        assert_eq!(
            out,
            "<<PERSON_1>> gặp ANH <<PERSON_2>> và <<PERSON_3>> ở <<ORG_1>> về <<TERM_1>>. Lan không bị."
        );
        // Whole words only: "Anh" is not "An", "Linhs" is not "Linh".
        assert_eq!(r.redact("Anh Linhs tan an"), "Anh Linhs tan <<PERSON_2>>");
        // Restored with the canonical spelling.
        assert_eq!(
            r.restore("<<PERSON_1>> / <<ORG_1>>").text,
            "Nguyễn Văn An / Acme Corp"
        );
    }

    #[test]
    fn entities_do_not_touch_placeholders() {
        let mut r = Redactor::new(KnownEntities {
            terms: vec!["email".into(), "phone".into(), "1".into()],
            ..Default::default()
        });
        let out = r.redact("email nam@gmail.com phone 0912345678 số 1");
        assert!(
            out.contains("<<EMAIL_1>>") && out.contains("<<PHONE_1>>"),
            "{out}"
        );
        assert!(!out.contains("<<TERM_1>>>"), "{out}");
        assert_eq!(
            r.restore(&out).text,
            "email nam@gmail.com phone 0912345678 số 1"
        );
    }

    #[test]
    fn restore_is_forgiving_and_counts_unresolved() {
        let mut r = redactor();
        r.redact("a@x.com và 0912345678");
        for form in [
            "<<EMAIL_1>>",
            "<EMAIL_1>",
            "[EMAIL_1]",
            "<<Email 1>>",
            "<< email_1 >>",
            "EMAIL_1",
            "email_1",
            "<<EMAIL-1>>",
            "((EMAIL_1))",
        ] {
            let got = r.restore(&format!("gửi tới {form}."));
            assert_eq!(
                got,
                Restored {
                    text: "gửi tới a@x.com.".into(),
                    unresolved: 0
                },
                "{form}"
            );
        }
        let got = r.restore("<<PHONE_1>>, <<PHONE_2>>, <EMAIL_7>, EMAIL 1, ID_1");
        assert_eq!(
            got.text,
            "0912345678, <<PHONE_2>>, <EMAIL_7>, EMAIL 1, ID_1"
        );
        assert_eq!(got.unresolved, 3);
        // Words that only look like a kind are untouched.
        assert_eq!(r.restore("our ORGANISATION and emails").unresolved, 0);
    }

    #[test]
    fn transcript_keeps_structure() {
        let t = Transcript::new(vec![
            Segment {
                id: 7,
                t0_ms: 1000,
                t1_ms: 2000,
                speaker: Some("Linh".into()),
                text: "mail a@x.com".into(),
                lang: Some("vi".into()),
            },
            Segment {
                id: 9,
                t0_ms: 2000,
                t1_ms: 3000,
                speaker: None,
                text: "ok".into(),
                lang: None,
            },
        ])
        .unwrap();
        let mut r = redactor();
        let out = r.redact_transcript(&t);
        assert_eq!(out.segments()[0].text, "mail <<EMAIL_1>>");
        assert_eq!(out.segments()[0].id, 7);
        assert_eq!(out.segments()[0].speaker.as_deref(), Some("Linh"));
        assert_eq!(
            (out.segments()[0].t0_ms, out.segments()[1].text.as_str()),
            (1000, "ok")
        );
    }

    #[test]
    fn warnings_flag_leftovers_without_quoting_them() {
        assert!(warnings("clean <<EMAIL_1>> <<PHONE_12>> text 2024").is_empty());
        let w = warnings("id 12345678901 and me@x.org");
        assert!(w.iter().any(|s| s.contains("long number")));
        assert!(w.iter().any(|s| s.contains('@')));
        assert!(
            w.iter()
                .all(|s| !s.contains("12345678901") && !s.contains("me@x.org"))
        );
        assert!(!warnings("một hai ba bốn năm sáu bảy tám chín").is_empty());
    }

    // ---- properties

    fn luhn_complete(mut body: Vec<u8>) -> String {
        // Check digit so the whole number passes Luhn.
        let sum: u32 = body
            .iter()
            .rev()
            .enumerate()
            .map(|(i, &d)| {
                let d = u32::from(d);
                if i % 2 == 0 {
                    let x = d * 2;
                    if x > 9 { x - 9 } else { x }
                } else {
                    d
                }
            })
            .sum();
        body.push(((10 - sum % 10) % 10) as u8);
        body.iter().map(|d| char::from(b'0' + d)).collect()
    }

    fn email() -> impl Strategy<Value = String> {
        (
            "[a-z][a-z0-9._]{2,10}",
            "[a-z]{3,8}",
            prop::sample::select(vec!["com", "vn", "org", "co.uk"]),
        )
            .prop_map(|(l, d, t)| format!("{}@{d}.{t}", l.trim_end_matches('.')))
    }

    fn vn_phone() -> impl Strategy<Value = String> {
        (
            prop::sample::select(vec!["090", "091", "093", "098", "032", "070"]),
            "[0-9]{7}",
            0..5usize,
        )
            .prop_map(|(p, rest, fmt)| {
                let d = format!("{p}{rest}");
                match fmt {
                    0 => d,
                    1 => format!("{} {} {}", &d[..4], &d[4..7], &d[7..]),
                    2 => format!("{}.{}.{}", &d[..4], &d[4..7], &d[7..]),
                    3 => format!("+84 {} {} {}", &d[1..4], &d[4..7], &d[7..]),
                    _ => format!("{}-{}-{}", &d[..3], &d[3..6], &d[6..]),
                }
            })
    }

    fn cccd() -> impl Strategy<Value = String> {
        "0[0-9]{2}[0-3][0-9]{8}"
    }

    fn card() -> impl Strategy<Value = String> {
        (prop::collection::vec(0u8..10, 15), 0..3usize).prop_map(|(body, fmt)| {
            let d = luhn_complete(body);
            match fmt {
                0 => d,
                1 => format!("{} {} {} {}", &d[..4], &d[4..8], &d[8..12], &d[12..]),
                _ => format!("{}-{}-{}-{}", &d[..4], &d[4..8], &d[8..12], &d[12..]),
            }
        })
    }

    fn pii() -> impl Strategy<Value = String> {
        prop_oneof![email(), vn_phone(), cccd(), card()]
    }

    fn words() -> impl Strategy<Value = String> {
        prop::sample::select(vec![
            "xin chào",
            "liên hệ",
            "please",
            "gọi",
            "số",
            "of",
            "the",
            "họp",
            "ok",
            "contact",
            "tại",
            "—",
        ])
        .prop_map(String::from)
    }

    proptest! {
        #[test]
        fn formatted_pii_never_survives(before in words(), item in pii(), after in words()) {
            let text = format!("{before} {item} {after}");
            let mut r = redactor();
            let out = r.redact(&text);
            prop_assert!(!out.contains(&item), "{out}");
            let digits: String = item.chars().filter(char::is_ascii_digit).collect();
            if digits.len() >= 8 {
                prop_assert!(!out.chars().filter(char::is_ascii_digit).collect::<String>().contains(&digits[..8]), "{out}");
            }
            prop_assert!(!out.contains('@'), "{out}");
            prop_assert!(warnings(&out).is_empty(), "{out}: {:?}", warnings(&out));
        }

        #[test]
        fn redact_then_restore_round_trips(parts in prop::collection::vec((words(), pii()), 1..4)) {
            let text = parts.iter().map(|(w, p)| format!("{w} {p}")).collect::<Vec<_>>().join(", ");
            let mut r = redactor();
            let out = r.redact(&text);
            let back = r.restore(&out);
            prop_assert_eq!(back.unresolved, 0);
            // Equal values in different formats share a placeholder and come
            // back in the first format, so compare on the value, not the text.
            let canon = |s: &str| s.chars().filter(|c| c.is_alphanumeric() || *c == '@').collect::<String>().to_lowercase();
            prop_assert_eq!(canon(&back.text), canon(&text));
        }

        #[test]
        fn plain_words_are_untouched(ws in prop::collection::vec(words(), 1..6)) {
            let text = ws.join(" ");
            prop_assert_eq!(redact(&text), text);
        }

        #[test]
        fn redact_never_panics(s in "\\PC{0,200}") {
            let mut r = redactor();
            let out = r.redact(&s);
            let _ = r.restore(&out);
            let _ = warnings(&out);
        }
    }
}
