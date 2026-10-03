// SPDX-License-Identifier: Apache-2.0
//! Overlap is stored for live lines too (phase 14d, D11): the live aligner
//! flags a line another speaker talked over and the persist thread keeps the
//! flag, so it is still there after a reload.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ghi_audio::Track;
use ghi_core::capture::{ReplayTrack, replay};
use ghi_core::engines::{FakeEngines, Script, SpeechEngines};
use ghi_core::events::{Event, bus};
use ghi_core::live::Mode;
use ghi_core::session::{Session, SessionConfig};
use ghi_speech::SpeakerSegment;
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::Store;

#[test]
fn a_talked_over_live_line_keeps_its_overlap_mark() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    let turn = |speaker, start, end| SpeakerSegment {
        start,
        end,
        speaker,
    };
    let engines: Arc<dyn SpeechEngines> = FakeEngines::new(Script {
        utterances: vec![
            (1.0, 2.5, "dòng đầu tiên".into()),
            (4.0, 5.5, "hai người cùng nói".into()),
            (7.0, 8.0, "dòng cuối".into()),
        ],
        turns: vec![turn(1, 0.5, 8.5), turn(2, 3.0, 6.0)],
    });
    let (tx, rx) = bus();
    let capture = replay(
        vec![ReplayTrack {
            track: Track::Mic,
            samples: (0..48_000 * 10)
                .map(|i| (i as f32 * 0.03).sin() * 0.2)
                .collect(),
            sample_rate: 48_000,
        }],
        None,
    )
    .unwrap();
    let s = Session::start(
        store.clone(),
        Some(engines),
        capture,
        SessionConfig {
            mode: Mode::Room,
            language: None,
            title: "overlap".into(),
            queue_jobs: false,
            lossless: true,
        },
        tx,
        None,
    )
    .unwrap();
    let meeting = s.meeting().to_string();
    let t = Instant::now();
    let mut finals = 0;
    while finals < 3 {
        assert!(t.elapsed() < Duration::from_secs(20));
        if let Ok(env) = rx.recv_timeout(Duration::from_millis(50))
            && let Event::TranscriptFinal { .. } = env.event
        {
            finals += 1;
        }
    }
    s.stop().unwrap();
    let segs = store.segments(&meeting).unwrap();
    assert_eq!(segs.len(), 3);
    assert!(segs[1].overlap, "the middle line was talked over: {segs:?}");
    assert!(!segs[0].overlap);
    // And a reloaded snapshot of the lines says the same.
    let flags: Vec<bool> = store
        .segments(&meeting)
        .unwrap()
        .iter()
        .map(|g| g.overlap)
        .collect();
    assert_eq!(flags.iter().filter(|f| **f).count(), 1);
}
