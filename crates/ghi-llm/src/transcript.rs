// SPDX-License-Identifier: Apache-2.0
//! The engine's input: a final (or live) transcript with segment ids.
//!
//! Prompts show each segment as `[s<id>] (mm:ss) <speaker>: <text>`; the model
//! cites segments by those ids, and callers turn ids into time anchors when
//! they save (doc 05 §2.3).

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use serde::{Deserialize, Serialize};

use crate::{LlmError, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    /// Unique within the transcript; citations refer to it.
    pub id: u64,
    pub t0_ms: i64,
    pub t1_ms: i64,
    /// Diarization label (`S1`) or a person's name; `None` if unknown.
    pub speaker: Option<String>,
    pub text: String,
    /// `vi`, `en` or `None`.
    pub lang: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Transcript {
    segments: Vec<Segment>,
    by_id: HashMap<u64, usize>,
    /// Display names by speaker value (e.g. a stored speaker's gid → "Linh").
    names: HashMap<String, String>,
}

impl Transcript {
    /// Checks ids are unique and drops segments with no text. Segments are
    /// kept in the given order (time order for real transcripts; the eval's
    /// gold transcripts have all times at 0).
    pub fn new(segments: Vec<Segment>) -> Result<Transcript> {
        let segments: Vec<Segment> = segments
            .into_iter()
            .filter(|s| !s.text.trim().is_empty())
            .collect();
        let mut by_id = HashMap::with_capacity(segments.len());
        for (i, s) in segments.iter().enumerate() {
            if by_id.insert(s.id, i).is_some() {
                return Err(LlmError::Invalid(format!("duplicate segment id {}", s.id)));
            }
        }
        Ok(Transcript {
            segments,
            by_id,
            names: HashMap::new(),
        })
    }

    /// Names to show for speakers in the notes' text, by `Segment::speaker`
    /// value. Speakers without one are shown as their value.
    pub fn with_speaker_names(mut self, names: HashMap<String, String>) -> Transcript {
        self.names = names;
        self
    }

    pub fn speaker_names(&self) -> &HashMap<String, String> {
        &self.names
    }

    /// How a speaker is written in the notes' text.
    pub fn speaker_name<'a>(&'a self, speaker: &'a str) -> &'a str {
        self.names.get(speaker).map_or(speaker, String::as_str)
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn get(&self, id: u64) -> Option<&Segment> {
        self.by_id.get(&id).map(|&i| &self.segments[i])
    }

    /// Position of a segment in [`Transcript::segments`].
    pub fn index_of(&self, id: u64) -> Option<usize> {
        self.by_id.get(&id).copied()
    }

    /// True when segments carry real times (not all zero).
    pub fn has_times(&self) -> bool {
        self.segments.iter().any(|s| s.t1_ms > s.t0_ms)
    }

    /// The language most of the text is in (`vi`/`en`), by characters; `None`
    /// when no segment has a language.
    pub fn dominant_lang(&self) -> Option<&str> {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for s in &self.segments {
            if let Some(l) = s.lang.as_deref() {
                *counts.entry(l).or_default() += s.text.chars().count();
            }
        }
        counts
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
            .map(|(l, _)| l)
    }

    /// Speaker labels in order of first appearance.
    pub fn speakers(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for s in &self.segments {
            if let Some(sp) = s.speaker.as_deref()
                && !out.contains(&sp)
            {
                out.push(sp);
            }
        }
        out
    }
}

static ALIAS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bSPK([0-9]{1,3})\b").expect("valid regex"));
/// `SPK2: ` at the start of an item: a transcript line copied with its label.
static LEADING_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^SPK[0-9]{1,3}:\s*").expect("valid regex"));

/// Speakers are shown to the model as `SPK1`, `SPK2`, ... (first appearance
/// order), never by name: owner accuracy doesn't depend on spelling, and names
/// never reach a cloud model. Output aliases are mapped back with
/// [`Aliases::original`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aliases {
    originals: Vec<String>,
    aliases: Vec<String>,
    /// Display name per alias.
    names: Vec<String>,
}

impl Aliases {
    pub fn new(t: &Transcript) -> Aliases {
        let originals: Vec<String> = t.speakers().into_iter().map(String::from).collect();
        let aliases = (1..=originals.len()).map(|i| format!("SPK{i}")).collect();
        let names = originals
            .iter()
            .map(|o| t.speaker_name(o).to_string())
            .collect();
        Aliases {
            originals,
            aliases,
            names,
        }
    }

    /// Replaces aliases the model wrote in text (`SPK2 will send...`) with the
    /// speaker's name, so text never shows the prompt's private numbering,
    /// and drops a leading `SPK2: ` label.
    pub fn expand(&self, text: &str) -> String {
        let text = LEADING_LABEL.replace(text, "");
        ALIAS
            .replace_all(&text, |c: &regex::Captures| {
                c[1].parse::<usize>()
                    .ok()
                    .and_then(|n| self.names.get(n.wrapping_sub(1)))
                    .cloned()
                    .unwrap_or_else(|| c[0].to_string())
            })
            .into_owned()
    }

    /// `SPK1`, `SPK2`, ... in order.
    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }

    pub fn alias(&self, original: &str) -> Option<&str> {
        let i = self.originals.iter().position(|o| o == original)?;
        Some(&self.aliases[i])
    }

    pub fn original(&self, alias: &str) -> Option<&str> {
        let i = self.aliases.iter().position(|a| a == alias)?;
        Some(&self.originals[i])
    }
}

/// `[s12] (03:21) SPK1: text`, the line format of every prompt.
pub fn render_line(s: &Segment, aliases: &Aliases, with_time: bool) -> String {
    let speaker = s
        .speaker
        .as_deref()
        .and_then(|sp| aliases.alias(sp))
        .unwrap_or("?");
    if with_time {
        let secs = s.t0_ms.max(0) / 1000;
        format!(
            "[s{}] ({:02}:{:02}) {}: {}",
            s.id,
            secs / 60,
            secs % 60,
            speaker,
            s.text.trim()
        )
    } else {
        format!("[s{}] {}: {}", s.id, speaker, s.text.trim())
    }
}

/// Renders segments one per line (times shown only if the transcript has them).
pub fn render(t: &Transcript, aliases: &Aliases, segments: &[&Segment]) -> String {
    let with_time = t.has_times();
    segments
        .iter()
        .map(|s| render_line(s, aliases, with_time))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn seg(id: u64, t0: f64, t1: f64, speaker: &str, text: &str, lang: &str) -> Segment {
        Segment {
            id,
            t0_ms: (t0 * 1000.0) as i64,
            t1_ms: (t1 * 1000.0) as i64,
            speaker: (!speaker.is_empty()).then(|| speaker.to_string()),
            text: text.to_string(),
            lang: (!lang.is_empty()).then(|| lang.to_string()),
        }
    }

    #[test]
    fn rejects_duplicate_ids_and_drops_empty_text() {
        let t = Transcript::new(vec![
            seg(0, 0.0, 1.0, "S1", "hello", "en"),
            seg(1, 1.0, 2.0, "S2", "  ", "en"),
        ])
        .unwrap();
        assert_eq!(t.segments().len(), 1);
        assert!(t.get(1).is_none());
        let dup = Transcript::new(vec![
            seg(3, 0.0, 1.0, "S1", "a", ""),
            seg(3, 1.0, 2.0, "S1", "b", ""),
        ]);
        assert!(matches!(dup, Err(LlmError::Invalid(_))));
    }

    #[test]
    fn renders_ids_times_and_speakers() {
        let t = Transcript::new(vec![seg(12, 201.5, 203.0, "Linh", " Chốt nhé ", "vi")]).unwrap();
        let a = Aliases::new(&t);
        let s = &t.segments()[0];
        assert_eq!(render(&t, &a, &[s]), "[s12] (03:21) SPK1: Chốt nhé");
        assert_eq!(a.original("SPK1"), Some("Linh"));
        assert_eq!(
            a.expand("SPK1 chốt, SPK9, S3 và OSPK1 thì không"),
            "Linh chốt, SPK9, S3 và OSPK1 thì không"
        );
        assert_eq!(a.expand("SPK1: Chốt nhé"), "Chốt nhé");
        let gids = Transcript::new(vec![seg(0, 0.0, 1.0, "gid-7", "x", "")])
            .unwrap()
            .with_speaker_names(HashMap::from([("gid-7".to_string(), "Nam".to_string())]));
        assert_eq!(Aliases::new(&gids).expand("SPK1 gửi"), "Nam gửi");
        assert_eq!(a.original("SPK2"), None);
        let gold = Transcript::new(vec![seg(0, 0.0, 0.0, "", "hi", "")]).unwrap();
        assert!(!gold.has_times());
        let ga = Aliases::new(&gold);
        assert_eq!(render(&gold, &ga, &[&gold.segments()[0]]), "[s0] ?: hi");
    }

    #[test]
    fn dominant_lang_counts_characters() {
        let t = Transcript::new(vec![
            seg(0, 0.0, 1.0, "S1", "ok", "en"),
            seg(1, 1.0, 2.0, "S2", "Mình chốt scope cho beta nhé", "vi"),
        ])
        .unwrap();
        assert_eq!(t.dominant_lang(), Some("vi"));
        assert_eq!(t.speakers(), vec!["S1", "S2"]);
    }
}
