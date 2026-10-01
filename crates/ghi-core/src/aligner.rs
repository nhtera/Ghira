// SPDX-License-Identifier: Apache-2.0
//! Word → speaker alignment: each recognized word gets the diarization
//! speaker most active over its span; runs shorter than [`SMOOTH_S`] between
//! two words of the same speaker are absorbed (a diarizer flicker, not a turn);
//! the words are then cut into lines at speaker changes.
//!
//! Inputs are on one stream's clock (seconds since the stream started).

use ghi_speech::{SpeakerSegment, Word};

/// A speaker run shorter than this (seconds) inside another speaker's turn is
/// treated as noise.
pub const SMOOTH_S: f64 = 0.3;
/// Words below this ASR confidence are flagged.
pub const LOW_CONFIDENCE: f32 = 0.5;

/// One aligned word.
#[derive(Debug, Clone, PartialEq)]
pub struct AlignedWord {
    pub word: Word,
    /// Diarization speaker (1-based), `None` when nobody was active.
    pub speaker: Option<u32>,
    /// Another speaker was active for a large part of the word too.
    pub overlap: bool,
    pub low_confidence: bool,
}

/// A run of words by one speaker.
#[derive(Debug, Clone, PartialEq)]
pub struct AlignedLine {
    pub speaker: Option<u32>,
    pub start: f64,
    pub end: f64,
    pub words: Vec<AlignedWord>,
}

impl AlignedLine {
    pub fn text(&self) -> String {
        self.words
            .iter()
            .map(|w| w.word.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn overlap(&self) -> bool {
        self.words.iter().any(|w| w.overlap)
    }
}

/// Speaker activity (seconds) of each speaker within `start..end`, largest first.
fn activity(segs: &[SpeakerSegment], start: f64, end: f64) -> Vec<(u32, f64)> {
    let mut totals: Vec<(u32, f64)> = Vec::new();
    for s in segs {
        let o = s.end.min(end) - s.start.max(start);
        if o <= 0.0 {
            continue;
        }
        match totals.iter_mut().find(|(k, _)| *k == s.speaker) {
            Some((_, t)) => *t += o,
            None => totals.push((s.speaker, o)),
        }
    }
    totals.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    totals
}

/// The speaker most active over `start..end` (ties: lowest id).
pub fn majority(segs: &[SpeakerSegment], start: f64, end: f64) -> Option<u32> {
    activity(segs, start, end).first().map(|(k, _)| *k)
}

/// Assigns speakers to `words` and cuts them into lines.
pub fn align(words: &[Word], segs: &[SpeakerSegment]) -> Vec<AlignedLine> {
    let mut aligned: Vec<AlignedWord> = words
        .iter()
        .map(|w| {
            // Zero-length words: look at a 80 ms window (one encoder frame).
            let (s, e) = if w.end > w.start {
                (w.start, w.end)
            } else {
                (w.start, w.start + 0.08)
            };
            let act = activity(segs, s, e);
            let dur = e - s;
            AlignedWord {
                speaker: act.first().map(|(k, _)| *k),
                overlap: act.get(1).is_some_and(|(_, t)| *t >= 0.3 * dur),
                low_confidence: w.confidence < LOW_CONFIDENCE,
                word: w.clone(),
            }
        })
        .collect();
    smooth(&mut aligned);
    let mut lines: Vec<AlignedLine> = Vec::new();
    for w in aligned {
        match lines.last_mut() {
            Some(l) if l.speaker == w.speaker => {
                l.end = l.end.max(w.word.end);
                l.words.push(w);
            }
            _ => lines.push(AlignedLine {
                speaker: w.speaker,
                start: w.word.start,
                end: w.word.end,
                words: vec![w],
            }),
        }
    }
    lines
}

/// Absorbs short runs between two runs of the same speaker, and a short
/// unattributed run at either end into its neighbour.
fn smooth(words: &mut [AlignedWord]) {
    // Runs as (start index, end index exclusive, speaker).
    let runs = |words: &[AlignedWord]| {
        let mut out: Vec<(usize, usize, Option<u32>)> = Vec::new();
        for (i, w) in words.iter().enumerate() {
            match out.last_mut() {
                Some(r) if r.2 == w.speaker => r.1 = i + 1,
                _ => out.push((i, i + 1, w.speaker)),
            }
        }
        out
    };
    let rs = runs(words);
    for k in 0..rs.len() {
        let (a, b, spk) = rs[k];
        let span = words[b - 1].word.end - words[a].word.start;
        if span >= SMOOTH_S {
            continue;
        }
        let prev = k.checked_sub(1).map(|p| rs[p].2);
        let next = rs.get(k + 1).map(|r| r.2);
        let to = match (prev, next) {
            (Some(p), Some(n)) if p == n => Some(p),
            (Some(p), None) if spk.is_none() => Some(p),
            (None, Some(n)) if spk.is_none() => Some(n),
            _ => None,
        };
        if let Some(to) = to {
            for w in &mut words[a..b] {
                w.speaker = to;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, start: f64, end: f64) -> Word {
        Word {
            text: text.into(),
            start,
            end,
            confidence: 0.9,
            speaker: None,
        }
    }

    fn seg(speaker: u32, start: f64, end: f64) -> SpeakerSegment {
        SpeakerSegment {
            start,
            end,
            speaker,
        }
    }

    #[test]
    fn cuts_lines_at_speaker_changes() {
        let words = [
            w("chốt", 0.0, 0.4),
            w("lịch", 0.4, 0.8),
            w("okay", 1.2, 1.6),
            w("done", 1.6, 2.0),
        ];
        let segs = [seg(1, 0.0, 1.0), seg(2, 1.1, 2.1)];
        let lines = align(&words, &segs);
        assert_eq!(lines.len(), 2);
        assert_eq!(
            (lines[0].speaker, lines[0].text()),
            (Some(1), "chốt lịch".into())
        );
        assert_eq!(
            (lines[1].speaker, lines[1].text()),
            (Some(2), "okay done".into())
        );
        assert_eq!((lines[1].start, lines[1].end), (1.2, 2.0));
    }

    #[test]
    fn a_short_flicker_inside_a_turn_is_smoothed() {
        let words = [w("one", 0.0, 0.3), w("two", 0.3, 0.5), w("three", 0.5, 0.9)];
        // Speaker 2 blips for 0.2 s in the middle of speaker 1.
        let segs = [seg(1, 0.0, 0.3), seg(2, 0.3, 0.5), seg(1, 0.5, 1.0)];
        let lines = align(&words, &segs);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].speaker, Some(1));
    }

    #[test]
    fn overlap_low_confidence_and_silence() {
        let mut quiet = w("hm", 0.0, 0.4);
        quiet.confidence = 0.2;
        let words = [quiet, w("yes", 5.0, 5.4)];
        let segs = [seg(1, 0.0, 0.4), seg(2, 0.1, 0.4)];
        let lines = align(&words, &segs);
        assert!(lines[0].words[0].overlap && lines[0].words[0].low_confidence);
        // Nobody active at 5 s: an unattributed run after speaker 1 stays its own
        // line unless it is short (it is 0.4 s, so it stays).
        assert_eq!(lines.last().unwrap().speaker, None);
        assert_eq!(majority(&segs, 0.0, 0.4), Some(1));
        assert_eq!(majority(&[], 0.0, 1.0), None);
    }
}
