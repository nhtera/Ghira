// SPDX-License-Identifier: Apache-2.0
//! Continuous speech without a pause: the live engine cuts the line (the
//! stream is flushed and reopened), so every word arrives once, in order, in
//! lines no longer than the cut; and after a Vietnamese line the stream starts
//! over (the live bench: fresh streams read Vietnamese better).

use std::sync::Arc;
use std::time::{Duration, Instant};

use ghi_audio::Track;
use ghi_core::capture::{ReplayTrack, replay};
use ghi_core::engines::{SpeechEngines, Talk, TalkEngines};
use ghi_core::events::{Event, LineInfo, bus};
use ghi_core::live::{FORCE_FINAL_S, Mode, Prefix, cut_due, turn_change};
use ghi_core::session::{Session, SessionConfig};
use ghi_speech::{AsrResult, SpeakerSegment, Word};
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::Store;

/// Runs `talk` through a room session; the lines and how many ASR streams opened.
fn session(talk: Talk, seconds: usize) -> (Vec<(u8, LineInfo)>, u32) {
    let (events, opened) = events(talk, seconds);
    let lines = events
        .into_iter()
        .filter_map(|e| match e {
            Event::TranscriptFinal { line, track, .. } => Some((track, line)),
            _ => None,
        })
        .collect();
    (lines, opened)
}

/// Runs `talk` through a room session; its events and the ASR streams opened.
fn events(talk: Talk, seconds: usize) -> (Vec<Event>, u32) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    let engines = TalkEngines::new(talk);
    let (tx, rx) = bus();
    let capture = replay(
        vec![ReplayTrack {
            track: Track::Mic,
            samples: (0..16_000 * seconds)
                .map(|i| (i as f32 * 0.03).sin() * 0.2)
                .collect(),
            sample_rate: 16_000,
        }],
        None,
    )
    .unwrap();
    let s = Session::start(
        store,
        Some(engines.clone() as Arc<dyn SpeechEngines>),
        capture,
        SessionConfig {
            sensitive: false,
            mode: Mode::Room,
            language: None,
            title: "cut".into(),
            queue_jobs: false,
            lossless: true,
            echo_cancellation: false,
        },
        tx,
        None,
    )
    .unwrap();
    let t = Instant::now();
    while !s.source_ended() {
        assert!(t.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(20));
    }
    s.stop().unwrap();
    (
        rx.try_iter().map(|env| env.event).collect(),
        engines.opened(),
    )
}

fn words(n: usize, word: &str) -> Vec<String> {
    (1..=n).map(|i| format!("{word}{i}")).collect()
}

#[test]
fn a_long_utterance_is_cut_without_losing_or_repeating_words() {
    // 60 words over 30 s, no pause: no endpoint would ever fire on its own.
    let talk = Talk {
        words: words(60, "w"),
        start: 0.5,
        step: 0.5,
        pauses: Vec::new(),
        turns: Vec::new(),
        show_lag: 0.0,
        diar_lag: 0.0,
    };
    let (lines, opened) = session(talk.clone(), 32);
    assert!(lines.len() >= 2, "the utterance was cut: {lines:?}");
    assert!(opened >= 2, "the cut reopened the stream");
    let heard: Vec<&str> = lines
        .iter()
        .flat_map(|(_, l)| l.text.split_whitespace())
        .collect();
    assert_eq!(heard, talk.words, "every word once, in order");
    for (track, l) in &lines {
        assert_eq!(*track, 0);
        let len = (l.t1_ms - l.t0_ms) as f64 / 1000.0;
        assert!(len <= FORCE_FINAL_S + 1.0, "line of {len} s: {l:?}");
    }
    assert!(lines.windows(2).all(|w| w[0].1.t1_ms <= w[1].1.t0_ms));
    // Times stay on the meeting clock across the reopen.
    let last = &lines.last().unwrap().1;
    assert_eq!(last.t1_ms, 30_500, "{last:?}");
}

#[test]
fn a_vietnamese_line_starts_a_fresh_stream_an_english_one_does_not() {
    // Three short utterances (pauses after words 4 and 8).
    let talk = |w: &str| Talk {
        words: words(12, w),
        start: 0.5,
        step: 0.5,
        pauses: vec![4, 8, 12],
        turns: Vec::new(),
        show_lag: 0.0,
        diar_lag: 0.0,
    };
    let (en, en_opened) = session(talk("w"), 10);
    let (vi, vi_opened) = session(talk("đ"), 10);
    assert_eq!(en.len(), 3, "{en:?}");
    assert_eq!(vi.len(), 3, "{vi:?}");
    assert_eq!(en_opened, 1, "English keeps its stream");
    assert!(
        vi_opened >= 3,
        "a fresh stream after each Vietnamese line: {vi_opened}"
    );
    let heard: Vec<&str> = vi
        .iter()
        .flat_map(|(_, l)| l.text.split_whitespace())
        .collect();
    assert_eq!(heard, talk("đ").words, "every word once, in order");
}

#[test]
fn lines_end_at_a_sentence_once_long_and_anywhere_once_too_long() {
    assert!(
        !cut_due("we ship on friday.", 3.0),
        "short: wait for the pause"
    );
    assert!(
        cut_due("we ship on friday.", 8.5),
        "long: at the sentence end"
    );
    assert!(
        !cut_due("we ship on friday and", 8.5),
        "long: not mid-sentence"
    );
    assert!(cut_due("we ship on friday and", FORCE_FINAL_S));
    assert!(cut_due("chốt lịch beta nhé?  ", 9.0));
}

fn seg(speaker: u32, start: f64, end: f64) -> SpeakerSegment {
    SpeakerSegment {
        start,
        end,
        speaker,
    }
}

#[test]
fn a_new_speakers_turn_is_found_but_a_short_word_or_a_tail_is_not() {
    // The utterance began showing at 10 s.
    assert_eq!(turn_change(&[seg(1, 8.0, 14.0)], 10.0), None, "one speaker");
    assert_eq!(
        turn_change(&[seg(1, 8.0, 12.0), seg(2, 12.2, 13.5)], 10.0),
        Some(12.2),
        "speaker 2 took over for 1.3 s"
    );
    assert_eq!(
        turn_change(&[seg(1, 8.0, 14.0), seg(2, 11.0, 11.6)], 10.0),
        None,
        "a short word of agreement"
    );
    assert_eq!(
        turn_change(&[seg(1, 5.0, 10.4), seg(2, 10.5, 13.0)], 10.0),
        None,
        "the previous speaker's tail before the new speaker's own utterance"
    );
}

fn word(text: &str, start: f64, end: f64) -> Word {
    Word {
        text: text.into(),
        start,
        end,
        confidence: 0.9,
        speaker: None,
    }
}

fn fin(text: &str, words: &[&str]) -> AsrResult {
    AsrResult {
        is_final: true,
        text: text.into(),
        words: words
            .iter()
            .enumerate()
            .map(|(i, w)| word(w, i as f64 * 0.4, i as f64 * 0.4 + 0.3))
            .collect(),
        languages: Vec::new(),
        audio_processed: 4.0,
    }
}

#[test]
fn a_prefix_shows_once_and_the_final_drops_it() {
    let mut p = Prefix::default();
    p.partial("so the", 1.0);
    p.partial("so the budget is", 2.0);
    p.partial("so the budget is fine yes", 3.0);
    // Words that showed by 2 s, never the partial's last word; their times
    // spread from the utterance's start (0.2 s) to the turn (1.8 s).
    let shown = p.take(2.0, 0.2, 1.8).unwrap();
    assert_eq!(shown.text, "so the budget is");
    assert_eq!(shown.words[0].start, 0.2);
    assert_eq!(shown.words[1].end, 1.0);
    assert_eq!(shown.words[3].end, 1.8, "ends at the turn");
    assert!(shown.words.windows(2).all(|w| w[0].end <= w[1].start));
    assert!(p.take(2.0, 0.2, 1.8).is_none(), "shown once");
    assert_eq!(
        p.rest("so the budget is fine yes I agree"),
        "fine yes I agree"
    );
    let words = ["so", "the", "budget", "is", "fine", "yes", "I", "agree"];
    let f = p.strip(fin("so the budget is fine yes I agree", &words));
    assert_eq!(f.text, "fine yes I agree");
    assert_eq!(f.words.len(), 4);
    assert_eq!(f.words[0].text, "fine");
}

#[test]
fn the_never_finished_last_word_is_not_shown() {
    let mut p = Prefix::default();
    p.partial("we ship on fri", 1.0);
    assert_eq!(p.take(5.0, 0.0, 5.0).unwrap().text, "we ship on");
    p.partial("we ship on friday", 1.5);
    assert!(
        p.take(5.0, 0.0, 5.0).is_none(),
        "friday is still the last word"
    );
}

#[test]
fn a_final_with_cleaned_punctuation_still_drops_the_shown_words() {
    // Partials are raw ("is , fine"), finals cleaned ("is, fine").
    let mut p = Prefix::default();
    p.partial("budget is , fine and", 1.0);
    assert_eq!(p.take(1.0, 0.0, 1.0).unwrap().text, "budget is , fine");
    let f = p.strip(fin(
        "Budget is, fine and then we",
        &["Budget", "is,", "fine", "and", "then", "we"],
    ));
    assert_eq!(f.text, "and then we");
    assert_eq!(f.words[0].text, "and", "dropped by count, not by time");
    // A word list that disagrees with the text: as many words as text tokens.
    let mut p = Prefix::default();
    p.partial("alpha beta gamma", 1.0);
    p.take(1.0, 0.0, 1.0).unwrap();
    let f = p.strip(fin(
        "alpha beta gamma delta",
        &["alph", "abeta", "gamma", "delta"],
    ));
    assert_eq!(f.text, "gamma delta");
    assert_eq!(f.words.len(), 2);
    // A final that does not start with them is kept whole.
    let mut p = Prefix::default();
    p.partial("alpha beta gamma", 1.0);
    p.take(1.0, 0.0, 1.0).unwrap();
    assert_eq!(p.strip(fin("alfa beta gamma", &[])).text, "alfa beta gamma");
}

#[test]
fn a_restarted_stream_keeps_only_the_shown_words() {
    let mut p = Prefix::default();
    p.partial("one two three four", 1.0);
    p.take(1.0, 0.0, 1.0).unwrap();
    p.restart();
    // The new stream hears it again (new times) and goes on.
    p.partial("one two three four five", 9.0);
    assert_eq!(p.rest("one two three four five"), "four five");
    let f = p.strip(fin(
        "one two three four five",
        &["one", "two", "three", "four", "five"],
    ));
    assert_eq!(f.text, "four five");
}

/// Speakers of the lines, runs of the same speaker merged.
fn turns_of(lines: &[(u8, LineInfo)]) -> Vec<Option<u32>> {
    let mut out: Vec<Option<u32>> = Vec::new();
    for (_, l) in lines {
        let s = l.speaker;
        if out.last() != Some(&s) {
            out.push(s);
        }
    }
    out
}

#[test]
fn another_speakers_turn_shows_the_first_speakers_words_before_the_utterance_ends() {
    // 20 words without a pause, each showing 0.5 s after it ends; speaker 2
    // takes over at 5 s (after w9).
    let talk = Talk {
        words: words(20, "w"),
        start: 0.5,
        step: 0.5,
        pauses: Vec::new(),
        turns: vec![seg(1, 0.0, 5.0), seg(2, 5.0, 11.0)],
        show_lag: 0.5,
        diar_lag: 1.0,
    };
    let (events, opened) = events(talk.clone(), 12);
    assert_eq!(opened, 1, "the stream is never reopened for a turn");
    let lines: Vec<(u8, LineInfo)> = events
        .iter()
        .filter_map(|e| match e {
            Event::TranscriptFinal { line, track, .. } => Some((*track, line.clone())),
            _ => None,
        })
        .collect();
    // Only words sure to be speaker 1's show early: at least w1..w7 (w9 ends
    // at the turn, w8 is on the edge of the 100 ms blocks).
    let first = &lines[0].1;
    let n = first.text.split_whitespace().count();
    assert!(
        (7..=8).contains(&n) && first.text.starts_with(&words(7, "w").join(" ")),
        "{lines:?}"
    );
    assert!(first.t1_ms <= 5_000);
    // Every word with its own speaker: w1..w9 the first's, w10.. the second's.
    let s1 = first.speaker;
    for (_, l) in &lines {
        for w in l.text.split_whitespace() {
            let n: usize = w[1..].parse().unwrap();
            assert_eq!(l.speaker == s1, n <= 9, "{w} in {lines:?}");
        }
    }
    assert_eq!(turns_of(&lines).len(), 2, "speaker 1 then 2: {lines:?}");
    let heard: Vec<&str> = lines
        .iter()
        .flat_map(|(_, l)| l.text.split_whitespace())
        .collect();
    assert_eq!(heard, talk.words, "every word once, in order");
    assert!(
        lines.windows(2).all(|w| w[0].1.t0_ms <= w[1].1.t0_ms),
        "sorted by start, the lines read in order: {lines:?}"
    );
    // Shown while the utterance was still going: words in progress came after
    // it, without the words it showed.
    let at = events
        .iter()
        .position(|e| matches!(e, Event::TranscriptFinal { .. }))
        .unwrap();
    let later: Vec<&str> = events[at..]
        .iter()
        .filter_map(|e| match e {
            Event::TranscriptPartial { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !later.is_empty(),
        "the first line came before the utterance ended"
    );
    assert!(later.iter().all(|t| !t.starts_with("w1 ")), "{later:?}");
}

#[test]
fn two_turns_in_one_utterance_show_in_order() {
    // Speaker 1, then 2 from 4 s, then 1 again from 8 s; no pause.
    let talk = Talk {
        words: words(22, "w"),
        start: 0.5,
        step: 0.5,
        pauses: Vec::new(),
        turns: vec![seg(1, 0.0, 4.0), seg(2, 4.0, 8.0), seg(1, 8.0, 12.0)],
        show_lag: 0.5,
        diar_lag: 1.0,
    };
    let (lines, opened) = session(talk.clone(), 13);
    assert_eq!(opened, 1);
    let order = turns_of(&lines);
    assert_eq!(order.len(), 3, "{lines:?}");
    assert_eq!(order[0], order[2]);
    assert_ne!(order[0], order[1]);
    let heard: Vec<&str> = lines
        .iter()
        .flat_map(|(_, l)| l.text.split_whitespace())
        .collect();
    assert_eq!(heard, talk.words);
    assert!(
        lines.windows(2).all(|w| w[0].1.t0_ms <= w[1].1.t0_ms),
        "{lines:?}"
    );
}
