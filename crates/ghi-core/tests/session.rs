// SPDX-License-Identifier: Apache-2.0
//! End to end over the scripted engines: replayed audio → pump → bundles,
//! engine → persist → store, events, discard, stop → jobs.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ghi_audio::Track;
use ghi_core::capture::{ReplayTrack, replay};
use ghi_core::engines::{FakeEngines, Script};
use ghi_core::events::{Event, SessionState, bus};
use ghi_core::live::Mode;
use ghi_core::session::{Session, SessionConfig};
use ghi_speech::SpeakerSegment;
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::{Store, TrackKind};

fn store(dir: &std::path::Path) -> Arc<Store> {
    Arc::new(
        Store::open(
            dir,
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    )
}

fn script() -> Script {
    let turn = |speaker, start, end| SpeakerSegment {
        start,
        end,
        speaker,
    };
    Script {
        utterances: vec![
            (1.0, 2.0, "xin chào mọi người".into()),
            (3.0, 4.0, "hôm nay chốt lịch beta".into()),
            (6.0, 7.0, "okay ship it".into()),
        ],
        turns: vec![turn(1, 0.5, 2.5), turn(2, 2.8, 4.5), turn(1, 5.5, 7.5)],
    }
}

fn tone(seconds: usize) -> Vec<f32> {
    (0..48_000 * seconds)
        .map(|i| (i as f32 * 0.03).sin() * 0.2)
        .collect()
}

fn start(store: &Arc<Store>, events: ghi_core::events::EventTx) -> Session {
    let capture = replay(
        vec![ReplayTrack {
            track: Track::Mic,
            samples: tone(9),
            sample_rate: 48_000,
        }],
        None,
    )
    .unwrap();
    Session::start(
        store.clone(),
        FakeEngines::new(script()),
        capture,
        SessionConfig {
            mode: Mode::Room,
            language: None,
            title: "standup".into(),
            queue_jobs: true,
            lossless: true,
        },
        events,
        None,
    )
    .unwrap()
}

/// Waits for `n` final lines on the bus, returning every event seen.
fn wait_finals(
    rx: &crossbeam_channel::Receiver<ghi_core::events::Envelope>,
    n: usize,
) -> Vec<Event> {
    let mut seen = Vec::new();
    let t = Instant::now();
    while seen
        .iter()
        .filter(|e| matches!(e, Event::TranscriptFinal { .. }))
        .count()
        < n
    {
        assert!(
            t.elapsed() < Duration::from_secs(20),
            "timed out: {seen:#?}"
        );
        if let Ok(env) = rx.recv_timeout(Duration::from_millis(50)) {
            seen.push(env.event);
        }
    }
    seen
}

#[test]
fn a_room_meeting_ends_up_in_the_store_with_speakers_and_jobs() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    let (tx, rx) = bus();
    let s = start(&store, tx);
    let meeting = s.meeting().to_string();
    let mut events = wait_finals(&rx, 3);
    let report = s.stop().unwrap();
    events.extend(rx.try_iter().map(|e| e.event));

    let segs = store.segments(&meeting).unwrap();
    let texts: Vec<&str> = segs.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "xin chào mọi người",
            "hôm nay chốt lịch beta",
            "okay ship it"
        ]
    );
    assert_eq!(
        segs[0].speaker_gid, segs[2].speaker_gid,
        "speaker 1 came back"
    );
    assert_ne!(segs[0].speaker_gid, segs[1].speaker_gid);
    assert_eq!(segs[0].lang.as_deref(), Some("vi"));
    assert_eq!(segs[2].lang.as_deref(), Some("en"));
    assert!((900..=1100).contains(&segs[0].t0_ms), "{}", segs[0].t0_ms);
    assert_eq!(store.segment_words(&segs[1].gid).unwrap().len(), 5);
    let speakers = store.speakers(&meeting).unwrap();
    assert_eq!(speakers.len(), 2);
    assert_eq!(
        speakers.iter().map(|s| s.color_slot).collect::<Vec<_>>(),
        [1, 2]
    );

    let audio = store.open_bundle(&meeting, TrackKind::Mic).unwrap();
    assert!(
        audio.complete() && audio.page_count() >= 9,
        "{} pages",
        audio.page_count()
    );
    let m = store.get_meeting(&meeting).unwrap();
    assert_eq!(m.status, "processing");
    assert!(report.duration_ms >= 8_900, "{}", report.duration_ms);
    let kinds: Vec<String> = store
        .jobs_for_meeting(&meeting)
        .unwrap()
        .into_iter()
        .map(|j| j.kind)
        .collect();
    assert_eq!(kinds, ["notes_live", "final_pass"]);

    let count = |f: fn(&Event) -> bool| events.iter().filter(|e| f(e)).count();
    assert_eq!(count(|e| matches!(e, Event::SpeakerArrived { .. })), 2);
    assert_eq!(count(|e| matches!(e, Event::SpeakerConfirmed { .. })), 2);
    assert!(count(|e| matches!(e, Event::TranscriptPartial { .. })) >= 3);
    let states: Vec<SessionState> = events
        .iter()
        .filter_map(|e| match e {
            Event::StateChanged { state, .. } => Some(*state),
            _ => None,
        })
        .collect();
    assert_eq!(
        states,
        [
            SessionState::Starting,
            SessionState::Recording,
            SessionState::Stopping,
            SessionState::Processing
        ]
    );
}

#[test]
fn discard_removes_the_last_seconds_everywhere() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    let (tx, rx) = bus();
    let s = start(&store, tx);
    let meeting = s.meeting().to_string();
    wait_finals(&rx, 3);
    let t = Instant::now();
    while !s.source_ended() || s.now_ms() < 8_900 {
        assert!(t.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(20));
    }
    s.mark();
    let pages_before = 9;
    let cut = s.discard(4.0).unwrap();
    assert!((4_900..=5_100).contains(&cut), "cut at {cut}");
    s.stop().unwrap();
    let events: Vec<Event> = rx.try_iter().map(|e| e.event).collect();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::DiscardApplied { from_ms, .. } if *from_ms == cut))
    );

    let texts: Vec<String> = store
        .segments(&meeting)
        .unwrap()
        .into_iter()
        .map(|s| s.text)
        .collect();
    assert_eq!(texts, ["xin chào mọi người", "hôm nay chốt lịch beta"]);
    assert!(
        store.marks(&meeting).unwrap().is_empty(),
        "the mark was after the cut"
    );
    assert!(
        store
            .search(&ghi_store::search::SearchQuery::new("ship"))
            .unwrap()
            .is_empty()
    );
    assert!(store.pending_discards().unwrap().is_empty());
    assert_eq!(store.discarded_spans(&meeting).unwrap().len(), 1);
    let audio = store.open_bundle(&meeting, TrackKind::Mic).unwrap();
    assert!(audio.complete());
    // Headers + ~4 one-second pages before the cut, then what came after the
    // discard (nothing: the replay had ended).
    assert!(
        audio.page_count() < pages_before,
        "{} pages",
        audio.page_count()
    );
    assert!(audio.page_count() >= 4, "{} pages", audio.page_count());
    // The audio itself: tone before the cut, silence after it.
    let pcm = ghi_audio::encoder::read_ogg_opus(&audio.read_all().unwrap()[..]).unwrap();
    let rms = |a: usize, b: usize| {
        let s = &pcm[a.min(pcm.len())..b.min(pcm.len())];
        (s.iter().map(|x| x * x).sum::<f32>() / s.len().max(1) as f32).sqrt()
    };
    let at = |ms: i64| (ms as usize) * 16;
    assert!(rms(at(1_000), at(3_000)) > 0.05, "kept audio is there");
    assert!(
        rms(at(cut + 100), pcm.len()) < 0.005,
        "discarded audio is silent: {}",
        rms(at(cut + 100), pcm.len())
    );
}

/// An engine that cannot open streams (e.g. the GPU is out of memory).
struct Broken;

impl ghi_core::engines::SpeechEngines for Broken {
    fn asr(&self, _: Option<&str>) -> ghi_core::engines::Result<ghi_core::engines::BoxAsr> {
        Err(ghi_speech::SpeechError {
            op: "asr",
            message: "out of memory".into(),
        })
    }
    fn diar(&self) -> ghi_core::engines::Result<ghi_core::engines::BoxDiar> {
        Err(ghi_speech::SpeechError {
            op: "diar",
            message: "out of memory".into(),
        })
    }
    fn chunk_ms(&self) -> u32 {
        560
    }
}

#[test]
fn a_failed_start_resumes_jobs_and_leaves_no_meeting() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    let (tx, _rx) = bus();
    struct Probe;
    impl ghi_core::jobs::JobHandler for Probe {
        fn kind(&self) -> &'static str {
            "probe"
        }
        fn run(&self, _: &ghi_core::jobs::JobCtx) -> Result<ghi_core::jobs::Outcome, String> {
            Ok(ghi_core::jobs::Outcome::Done)
        }
    }
    let runner = ghi_core::jobs::JobRunner::new(store.clone(), tx.clone(), vec![Arc::new(Probe)]);
    let capture = replay(
        vec![ReplayTrack {
            track: Track::Mic,
            samples: tone(1),
            sample_rate: 48_000,
        }],
        None,
    )
    .unwrap();
    let r = Session::start(
        store.clone(),
        Arc::new(Broken),
        capture,
        SessionConfig {
            mode: Mode::Room,
            language: None,
            title: "x".into(),
            queue_jobs: true,
            lossless: true,
        },
        tx,
        Some(runner.clone() as Arc<dyn ghi_core::session::RecordingHooks>),
    );
    assert!(r.is_err());
    assert!(
        store.list_meetings(10, 0).unwrap().is_empty(),
        "no half-made meeting"
    );
    // The runner is not stuck in "recording": a queued job would run.
    let m = store
        .create_meeting(ghi_store::store::NewMeeting::default())
        .unwrap()
        .gid;
    store
        .enqueue_job(Some(&m), "probe", 1, &serde_json::json!({}))
        .unwrap();
    assert!(
        runner.run_one().is_some(),
        "the runner is not blocked by a recording"
    );
}
