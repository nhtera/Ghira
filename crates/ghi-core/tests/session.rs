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
        Some(FakeEngines::new(script()) as Arc<dyn ghi_core::engines::SpeechEngines>),
        capture,
        SessionConfig {
            mode: Mode::Room,
            language: None,
            title: "standup".into(),
            queue_jobs: true,
            lossless: true,
            echo_cancellation: true,
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
    let started = events
        .iter()
        .position(|e| matches!(e, Event::SessionStarted { mode, title, .. } if mode == "room" && title == "standup"))
        .expect("SessionStarted");
    let recording = events
        .iter()
        .position(|e| {
            matches!(
                e,
                Event::StateChanged {
                    state: SessionState::Recording,
                    ..
                }
            )
        })
        .unwrap();
    assert_eq!(started + 1, recording, "right before Recording");
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
        Some(Arc::new(Broken)),
        capture,
        SessionConfig {
            mode: Mode::Room,
            language: None,
            title: "x".into(),
            queue_jobs: true,
            lossless: true,
            echo_cancellation: true,
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

/// Counts the hook calls and records what the count was when asked.
#[derive(Default)]
struct CountingHooks {
    started: std::sync::atomic::AtomicUsize,
    stopped: std::sync::atomic::AtomicUsize,
}

impl ghi_core::session::RecordingHooks for CountingHooks {
    fn recording_started(&self) {
        self.started
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    fn recording_stopped(&self) {
        self.stopped
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn config(mode: Mode) -> SessionConfig {
    SessionConfig {
        mode,
        language: None,
        title: "t".into(),
        queue_jobs: false,
        lossless: false,
        echo_cancellation: true,
    }
}

fn mic_replay(samples: Vec<f32>, speed: Option<f64>) -> ghi_core::capture::Capture {
    replay(
        vec![ReplayTrack {
            track: Track::Mic,
            samples,
            sample_rate: 16_000,
        }],
        speed,
    )
    .unwrap()
}

#[test]
fn engines_load_after_recording_started_and_inside_the_undo_guard() {
    use std::sync::atomic::Ordering::SeqCst;
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    let (tx, _rx) = bus();
    let hooks = Arc::new(CountingHooks::default());

    // A load that fails: jobs resume once, no meeting is left.
    let h = hooks.clone();
    let r = Session::start_with_loader(
        store.clone(),
        || {
            assert_eq!(h.started.load(SeqCst), 1, "recording started before load");
            assert_eq!(h.stopped.load(SeqCst), 0);
            Err(ghi_core::session::SessionError("no memory".into()))
        },
        mic_replay(vec![0.1; 16_000], None),
        config(Mode::Room),
        tx.clone(),
        Some(hooks.clone() as Arc<dyn ghi_core::session::RecordingHooks>),
    );
    assert_eq!(r.err().unwrap().0, "no memory");
    assert_eq!(
        (hooks.started.load(SeqCst), hooks.stopped.load(SeqCst)),
        (1, 1)
    );
    assert!(store.list_meetings(10, 0).unwrap().is_empty());

    // A load that works: started once, stopped at stop.
    let h = hooks.clone();
    let s = Session::start_with_loader(
        store.clone(),
        || {
            assert_eq!(h.started.load(SeqCst), 2);
            Ok(Some(
                FakeEngines::new(script()) as Arc<dyn ghi_core::engines::SpeechEngines>
            ))
        },
        mic_replay(tone(2), Some(1.0)),
        config(Mode::Room),
        tx.clone(),
        Some(hooks.clone() as Arc<dyn ghi_core::session::RecordingHooks>),
    )
    .unwrap();
    assert!(s.transcribing());
    s.stop().unwrap();
    assert_eq!(
        (hooks.started.load(SeqCst), hooks.stopped.load(SeqCst)),
        (2, 2)
    );

    // Nothing to load: records without a transcript.
    let s = Session::start_with_loader(
        store.clone(),
        || Ok(None),
        mic_replay(tone(1), Some(1.0)),
        config(Mode::Room),
        tx,
        None,
    )
    .unwrap();
    assert!(!s.transcribing());
    s.stop().unwrap();
}

#[test]
fn concurrent_discards_and_splits_do_not_interleave() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    let (tx, rx) = bus();
    let s = start(&store, tx);
    wait_finals(&rx, 3);
    std::thread::scope(|scope| {
        let a = scope.spawn(|| s.discard(1.0));
        let b = scope.spawn(|| s.discard(2.0));
        let c = scope.spawn(|| s.split(1, vec![]));
        let (a, b) = (a.join().unwrap(), b.join().unwrap());
        c.join().unwrap().unwrap();
        assert!(a.is_ok() && b.is_ok(), "{a:?} {b:?}");
    });
    let meeting = s.meeting().to_string();
    s.stop().unwrap();
    // The store is consistent: audio and lines both end before the cuts.
    let m = store.get_meeting(&meeting).unwrap();
    assert!(m.duration_ms > 0);
    assert!(
        store
            .open_bundle(&meeting, TrackKind::Mic)
            .unwrap()
            .complete()
    );
}

#[test]
fn snapshot_has_speakers_lines_and_marks_for_a_reloaded_webview() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    let (tx, rx) = bus();
    let s = start(&store, tx);
    wait_finals(&rx, 3);
    let t = s.mark();
    let snap = s.snapshot();
    assert_eq!(snap.meeting, s.meeting());
    assert!(snap.transcribing);
    assert_eq!(
        (
            snap.mode.as_str(),
            snap.language.as_deref(),
            snap.title.as_str()
        ),
        ("room", None, "standup")
    );
    assert_eq!(snap.state, SessionState::Recording);
    assert!(snap.seq > 0);
    assert_eq!(snap.marks, [t]);
    assert_eq!(snap.speakers.len(), 2);
    assert_eq!(
        snap.speakers
            .iter()
            .map(|s| s.color_slot)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(snap.speakers.iter().all(|s| !s.provisional));
    let texts: Vec<&str> = snap.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "xin chào mọi người",
            "hôm nay chốt lịch beta",
            "okay ship it"
        ]
    );
    // Lines point at the session speaker ids the events used.
    assert_eq!(snap.lines[0].speaker, snap.lines[2].speaker);
    assert_ne!(snap.lines[0].speaker, snap.lines[1].speaker);
    assert!(
        snap.speakers
            .iter()
            .any(|s| Some(s.id) == snap.lines[0].speaker)
    );
    assert_eq!(snap.lines[1].words.len(), 5);
    assert!(snap.lines.iter().all(|l| !l.gid.is_empty()));
    // The shape the UI gets.
    let v = serde_json::to_value(&snap).unwrap();
    assert_eq!(v["mode"], "room");
    assert!(v["nowMs"].is_number() && v["lines"][0]["t0Ms"].is_number());
    s.stop().unwrap();
}

#[test]
fn snapshot_without_engines_is_just_state_and_marks() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    let (tx, _rx) = bus();
    let s = Session::start_with_loader(
        store.clone(),
        || Ok(None),
        mic_replay(tone(3), Some(1.0)),
        config(Mode::Room),
        tx,
        None,
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let t = s.mark();
    let snap = s.snapshot();
    assert!(!snap.transcribing);
    assert!(snap.speakers.is_empty() && snap.lines.is_empty());
    assert_eq!(snap.marks, [t]);
    s.stop().unwrap();
}

/// Collects `LevelMeter` events until the replay is over.
fn levels_of(samples: Vec<f32>) -> Vec<(Option<f32>, Option<f32>, u64)> {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    let (tx, rx) = bus();
    let s = Session::start_with_loader(
        store,
        || Ok(None),
        mic_replay(samples, Some(1.0)),
        config(Mode::Room),
        tx,
        None,
    )
    .unwrap();
    while !s.source_ended() {
        std::thread::sleep(Duration::from_millis(20));
    }
    s.stop().unwrap();
    rx.try_iter()
        .filter_map(|e| match e.event {
            Event::LevelMeter {
                mic_dbfs,
                system_dbfs,
                ..
            } => Some((mic_dbfs, system_dbfs, e.at_ms as u64)),
            _ => None,
        })
        .collect()
}

#[test]
fn level_meter_reports_rms_at_most_ten_times_a_second() {
    // A 0.2-amplitude sine: RMS 0.1414 = -17 dBFS.
    let samples: Vec<f32> = (0..16_000 * 2)
        .map(|i| (i as f32 * 0.05).sin() * 0.2)
        .collect();
    let lv = levels_of(samples);
    assert!(lv.len() >= 10, "{} level events in 2 s", lv.len());
    for (mic, sys, _) in &lv {
        let m = mic.expect("the mic flows");
        assert!((-19.0..=-15.0).contains(&m), "{m}");
        assert_eq!(*sys, None, "a room has no system track");
    }
    let span = lv.last().unwrap().2 - lv.first().unwrap().2;
    assert!(
        (lv.len() as u64 - 1) * 100 <= span + 20,
        "{} events over {span} ms",
        lv.len()
    );

    // Digital silence reads the floor, not -inf or None.
    let lv = levels_of(vec![0.0; 16_000]);
    assert!(!lv.is_empty());
    assert!(lv.iter().all(|(m, _, _)| *m == Some(-100.0)));
}

#[test]
fn capture_lifecycle_events_reach_the_bus_and_sleep_stops_the_timeline() {
    use ghi_audio::{CaptureEvent as C, Route};
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    let (tx, rx) = bus();
    let mut capture = mic_replay(tone(1).repeat(8), Some(1.0));
    let inject = capture.event_sender();
    let s = Session::start_with_loader(store, || Ok(None), capture, config(Mode::Room), tx, None)
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));

    inject.send(C::Sleep).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let at_sleep = s.now_ms();
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        s.now_ms(),
        at_sleep,
        "the sleeping Mac's time is not recorded"
    );
    inject.send(C::Wake).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    assert!(s.now_ms() > at_sleep + 200, "the timeline resumes on wake");

    for ev in [
        C::SystemRestarted,
        C::SilentSystemTrack { silent_s: 31.0 },
        C::DiskLow { free_bytes: 123 },
        C::DiskFull,
        C::TrackLost {
            track: Track::System,
        },
        C::RouteChanged {
            route: Route::Bluetooth,
            input_bluetooth_hfp: true,
        },
    ] {
        inject.send(ev).unwrap();
    }
    std::thread::sleep(Duration::from_millis(300));
    // A retry answers either way (a replay has no devices to rebuild).
    s.retry_capture();
    std::thread::sleep(Duration::from_millis(300));
    s.stop().unwrap();
    let seen: Vec<Event> = rx.try_iter().map(|e| e.event).collect();
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::CaptureRecovered { .. })),
        "{seen:?}"
    );
    let m = |f: fn(&Event) -> bool| seen.iter().filter(|e| f(e)).count();
    assert_eq!(m(|e| matches!(e, Event::Slept { .. })), 1);
    assert_eq!(m(|e| matches!(e, Event::Woke { .. })), 1);
    assert_eq!(m(|e| matches!(e, Event::SystemAudioRestarted { .. })), 1);
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::SilentSystemTrack { silent_s, .. } if *silent_s == 31.0))
    );
    assert!(seen.iter().any(|e| matches!(
        e,
        Event::DiskLow {
            free_bytes: 123,
            ..
        }
    )));
    assert_eq!(m(|e| matches!(e, Event::DiskFull { .. })), 1);
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::TrackLost { track: 1, .. }))
    );
    assert!(seen.iter().any(|e| matches!(
        e,
        Event::RouteChanged {
            bluetooth_hfp: true,
            ..
        }
    )));
}

/// A hook whose `wait_idle` takes long (a job that will not yield).
struct SlowIdle(Duration);

impl ghi_core::session::RecordingHooks for SlowIdle {
    fn recording_started(&self) {}
    fn recording_stopped(&self) {}
    fn wait_idle(&self, _: Duration) -> bool {
        std::thread::sleep(self.0);
        false
    }
}

#[test]
fn waiting_for_a_job_to_yield_does_not_lose_live_audio() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    let (tx, _rx) = bus();
    // Longer than the capture ring (4 s): the pump must already be draining.
    let s = Session::start_with_loader(
        store,
        || Ok(None),
        mic_replay(tone(7).into_iter().step_by(3).collect(), Some(1.0)),
        config(Mode::Room),
        tx,
        Some(Arc::new(SlowIdle(Duration::from_millis(4500)))),
    )
    .unwrap();
    assert!(
        s.now_ms() >= 4_000,
        "the timeline ran during the wait: {} ms",
        s.now_ms()
    );
    while !s.source_ended() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let report = s.stop().unwrap();
    // 7 s of audio at real time: nothing was dropped from the ring.
    assert!(report.duration_ms >= 6_500, "{}", report.duration_ms);
}

#[test]
fn a_panicking_loader_stops_the_pump_and_leaves_no_meeting() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    let (tx, _rx) = bus();
    let hooks = Arc::new(CountingHooks::default());
    let r = Session::start_with_loader(
        store.clone(),
        || panic!("model exploded"),
        mic_replay(vec![0.1; 16_000 * 2], Some(1.0)),
        config(Mode::Room),
        tx,
        Some(hooks.clone() as Arc<dyn ghi_core::session::RecordingHooks>),
    );
    assert!(r.err().unwrap().0.contains("panicked"));
    assert_eq!(
        hooks.stopped.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "jobs resume"
    );
    assert!(store.list_meetings(10, 0).unwrap().is_empty());
}

#[test]
fn discard_from_cuts_at_the_approved_time_not_relative_to_now() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    let (tx, rx) = bus();
    let s = start(&store, tx);
    wait_finals(&rx, 3);
    while !s.source_ended() || s.now_ms() < 8_900 {
        std::thread::sleep(Duration::from_millis(20));
    }
    // Confirmed later than previewed: the cut is still where it was shown.
    assert_eq!(s.discard_from(5_000).unwrap(), 5_000);
    let meeting = s.meeting().to_string();
    assert!(
        store
            .segments(&meeting)
            .unwrap()
            .iter()
            .all(|g| g.t0_ms < 5_000),
        "the line after the cut is gone"
    );
    let now = s.now_ms();
    assert!(s.discard_from(-1).is_err());
    assert!(s.discard_from(now + 60_000).is_err(), "in the future");
    // Consent shows in the snapshot.
    assert!(!s.snapshot().consent_confirmed);
    store.set_consent_confirmed(&meeting, true).unwrap();
    let snap = s.snapshot();
    assert!(snap.consent_confirmed);
    assert_eq!(
        serde_json::to_value(&snap).unwrap()["consentConfirmed"],
        true
    );
    s.stop().unwrap();
}

#[test]
fn speaker_split_event_carries_the_moved_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let store = store(tmp.path());
    let (tx, rx) = bus();
    let s = start(&store, tx);
    let seen = wait_finals(&rx, 3);
    let line = seen
        .iter()
        .find_map(|e| match e {
            Event::TranscriptFinal { line, .. } => Some(line.clone()),
            _ => None,
        })
        .unwrap();
    let from = line.speaker.unwrap();
    // A repeated gid is moved (and reported) once.
    let new = s
        .split(from, vec![line.gid.clone(), line.gid.clone()])
        .unwrap()
        .expect("split");
    // The event comes from the session thread: wait for it (bounded).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let moved = std::iter::from_fn(|| {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        rx.recv_timeout(left).ok()
    })
    .find_map(|e| match e.event {
        Event::SpeakerSplit {
            from: f,
            speaker,
            lines,
            ..
        } => Some((f, speaker.id, lines)),
        _ => None,
    })
    .expect("SpeakerSplit");
    assert_eq!(moved, (from, new, vec![line.gid.clone()]));
    let snap = s.snapshot();
    let l = snap.lines.iter().find(|l| l.gid == line.gid).unwrap();
    assert_eq!(l.speaker, Some(new), "the store moved it too");
    s.stop().unwrap();
}
