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
use ghi_core::live::{FORCE_FINAL_S, Mode, cut_due};
use ghi_core::session::{Session, SessionConfig};
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::Store;

/// Runs `talk` through a room session; the lines and how many ASR streams opened.
fn session(talk: Talk, seconds: usize) -> (Vec<(u8, LineInfo)>, u32) {
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
    let lines = rx
        .try_iter()
        .filter_map(|env| match env.event {
            Event::TranscriptFinal { line, track, .. } => Some((track, line)),
            _ => None,
        })
        .collect();
    (lines, engines.opened())
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
