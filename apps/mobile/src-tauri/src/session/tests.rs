// SPDX-License-Identifier: Apache-2.0
//! Host tests of the recording core: real `ghi-core` store, job runner and
//! pipeline, the scripted `FakeEngines`, no microphone (PCM is pushed into
//! the ring like the audio tap does). The tap's ring is process-wide, so the
//! tests run one at a time.

use std::sync::atomic::AtomicUsize;

use ghi_core::engines::{FakeEngines, Script, SpeechEngines};
use ghi_core::events::{EventRx, bus};
use ghi_core::jobs::{JobCtx, JobHandler, JobRunner, Outcome};
use ghi_speech::SpeakerSegment;
use ghi_store::jobs::JobState;
use ghi_store::keys::{MemoryKeyStore, Protection};

use super::*;
use crate::lifecycle::Lifecycle;

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let t = Instant::now();
    while !f() {
        assert!(
            t.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn live_tier() -> DeviceTier {
    crate::tier::describe("iPhone16,1".into(), 8.0, false, None)
}

fn low_tier() -> DeviceTier {
    crate::tier::describe("iPhone12,1".into(), 4.0, false, None)
}

/// Two voices over eight seconds.
fn script() -> Script {
    let turn = |speaker, start, end| SpeakerSegment {
        start,
        end,
        speaker,
    };
    Script {
        utterances: vec![
            (0.5, 1.5, "xin chào mọi người".into()),
            (2.5, 3.5, "hôm nay chốt lịch".into()),
            (4.5, 5.5, "đồng ý nhé".into()),
        ],
        turns: vec![turn(1, 0.0, 2.0), turn(2, 2.2, 4.0), turn(1, 4.2, 6.0)],
    }
}

/// The scripted engines, counting how many are alive (to see them dropped).
struct Counted {
    inner: Arc<FakeEngines>,
    alive: Arc<AtomicUsize>,
}

impl SpeechEngines for Counted {
    fn asr(&self, l: Option<&str>) -> ghi_core::engines::Result<ghi_core::engines::BoxAsr> {
        self.inner.asr(l)
    }
    fn diar(&self) -> ghi_core::engines::Result<ghi_core::engines::BoxDiar> {
        self.inner.diar()
    }
    fn chunk_ms(&self) -> u32 {
        self.inner.chunk_ms()
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.alive.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Loads {
    alive: Arc<AtomicUsize>,
    loaded: Arc<AtomicUsize>,
}

fn counting_provider() -> (EnginesProvider, Loads) {
    let alive = Arc::new(AtomicUsize::new(0));
    let loaded = Arc::new(AtomicUsize::new(0));
    let (a, l) = (alive.clone(), loaded.clone());
    let provider: EnginesProvider = Arc::new(move || {
        a.fetch_add(1, Ordering::SeqCst);
        l.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(Counted {
            inner: FakeEngines::new(script()),
            alive: a.clone(),
        }) as Arc<dyn SpeechEngines>)
    });
    (provider, Loads { alive, loaded })
}

/// What the probe job saw when the runner finally claimed it.
#[derive(Default)]
struct Seen {
    ran: AtomicUsize,
    alive_engines: AtomicUsize,
    session_present: AtomicBool,
}

struct Probe {
    seen: Arc<Seen>,
    alive: Arc<AtomicUsize>,
    recorder: Mutex<Option<Arc<Recorder>>>,
}

impl JobHandler for Probe {
    fn kind(&self) -> &'static str {
        FINAL_PASS_JOB
    }
    fn run(&self, _: &JobCtx) -> Result<Outcome, String> {
        self.seen.ran.fetch_add(1, Ordering::SeqCst);
        self.seen
            .alive_engines
            .store(self.alive.load(Ordering::SeqCst), Ordering::SeqCst);
        let present = self
            .recorder
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|r| r.latest().is_some());
        self.seen.session_present.store(present, Ordering::SeqCst);
        Ok(Outcome::Done)
    }
}

struct Rig {
    _tmp: tempfile::TempDir,
    store: Arc<Store>,
    recorder: Arc<Recorder>,
    runner: Arc<JobRunner>,
    rx: EventRx,
    loads: Loads,
    seen: Arc<Seen>,
    _guard: MutexGuard<'static, ()>,
}

fn rig(tier: DeviceTier) -> Rig {
    rig_with(tier, true)
}

/// `fake_engines: false` leaves the provider to the build (no models here).
fn rig_with(tier: DeviceTier, fake_engines: bool) -> Rig {
    #[cfg(target_os = "ios")]
    std::hint::black_box(crate::test_swift::keep());
    let guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            &tmp.path().join("store"),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    let (tx, rx) = bus();
    let (provider, loads) = counting_provider();
    let seen = Arc::new(Seen::default());
    let probe = Arc::new(Probe {
        seen: seen.clone(),
        alive: loads.alive.clone(),
        recorder: Mutex::new(None),
    });
    let runner = JobRunner::new(store.clone(), tx.clone(), vec![probe.clone()]);
    let slot = runner.clone();
    let st = store.clone();
    let recorder = Recorder::new(RecorderDeps {
        store: Arc::new(move || Ok(st.clone())),
        events: tx,
        runner: Arc::new(move || Some(slot.clone())),
        models: tmp.path().join("models"),
        backlog_dir: tmp.path().join("backlog"),
        metrics_dir: tmp.path().join("metrics"),
        tier,
        provider: fake_engines.then_some(provider),
        fake_mic: false,
    });
    *probe.recorder.lock().unwrap() = Some(recorder.clone());
    Rig {
        _tmp: tmp,
        store,
        recorder,
        runner,
        rx,
        loads,
        seen,
        _guard: guard,
    }
}

fn request() -> RecordStart {
    RecordStart {
        mode: RecordMode::Room,
        language: ghi_app::system::MeetingLanguage::Auto,
        title: Some("Standup".into()),
        target: ProcessingTarget::Phone,
        consent_acknowledged: true,
        call_acknowledged: false,
    }
}

/// Pushes `seconds` of a tone the way the tap does (100 ms blocks of 48 kHz).
fn feed(seconds: usize) {
    static HOST: AtomicU64 = AtomicU64::new(1_000_000_000);
    for i in 0..seconds * 10 {
        let block: Vec<f32> = (0..4800)
            .map(|n| ((i * 4800 + n) as f32 * 0.03).sin() * 0.1)
            .collect();
        push_pcm(
            &block,
            48_000.0,
            HOST.fetch_add(100_000_000, Ordering::SeqCst),
        );
        thread::sleep(Duration::from_millis(3));
    }
}

fn drained(r: &Rig) -> bool {
    r.recorder.latest().is_none()
}

#[test]
fn records_without_an_engine_below_the_live_tier() {
    let r = rig(low_tier());
    let mut bad = request();
    bad.target = ProcessingTarget::Phone;
    assert!(
        r.recorder.start(&bad).is_err(),
        "the Phone target is disabled"
    );
    bad.target = ProcessingTarget::Cloud;
    let id = r.recorder.start(&bad).unwrap();
    feed(3);
    r.recorder.mark().unwrap();
    wait_for("recorded audio", || r.recorder.snapshot().elapsed_s > 1.5);
    let st = r.recorder.snapshot();
    assert_eq!(st.phase, RecordPhase::RecordOnly);
    assert_eq!(st.record_only_reason, Some(RecordOnlyReason::DeviceTier));
    assert!(!st.live);
    r.recorder.stop().unwrap();
    wait_for("the session to drain", || drained(&r));
    let m = r.store.get_meeting(&id).unwrap();
    assert_eq!((m.status.as_str(), m.source.as_str()), ("done", "mobile"));
    assert!(m.consent_confirmed);
    assert!(m.duration_ms >= 1500, "{m:?}");
    assert!(r.store.jobs_for_meeting(&id).unwrap().is_empty(), "no jobs");
    assert_eq!(r.store.marks(&id).unwrap().len(), 1);
    let bundle = r.store.open_bundle(&id, TrackKind::Mic).unwrap();
    assert!(bundle.page_count() > 0);
    assert!(
        std::fs::read_dir(r._tmp.path().join("backlog"))
            .map(|d| d.count())
            .unwrap_or(0)
            == 0,
        "record-only keeps no backlog"
    );
    assert_eq!(r.loads.loaded.load(Ordering::SeqCst), 0, "no model loaded");
    let events: Vec<_> = r.rx.try_iter().map(|e| e.event).collect();
    assert!(
        !events.iter().any(|e| matches!(
            e,
            Event::Error {
                kind: ghi_core::events::ErrorKind::ModelsMissing,
                ..
            }
        )),
        "not told the models are missing"
    );
    assert!(events.iter().any(|e| matches!(
        e,
        Event::StateChanged {
            state: SessionState::Idle,
            ..
        }
    )));
    assert!(r.seen.ran.load(Ordering::SeqCst) == 0);
}

#[test]
fn live_lines_reach_the_store_and_the_final_pass_is_queued() {
    let r = rig(live_tier());
    let id = r.recorder.start(&request()).unwrap();
    assert!(r.recorder.start(&request()).is_err(), "one at a time");
    feed(8);
    wait_for("three lines", || {
        r.recorder
            .snapshot()
            .session
            .is_some_and(|s| s.lines.len() >= 3)
    });
    let st = r.recorder.snapshot();
    assert!(st.live && st.record_only_reason.is_none());
    let snap = st.session.unwrap();
    assert_eq!(snap.title, "Standup");
    assert!(snap.speakers.len() >= 2, "{:?}", snap.speakers);
    r.recorder.stop().unwrap();
    wait_for("the session to drain", || drained(&r));
    assert_eq!(r.store.segments(&id).unwrap().len(), 3);
    assert_eq!(r.store.speakers(&id).unwrap().len(), 2);
    let jobs = r.store.jobs_for_meeting(&id).unwrap();
    let kinds: Vec<_> = jobs.iter().map(|j| j.kind.as_str()).collect();
    assert_eq!(kinds, [FINAL_PASS_JOB], "no live notes on the phone");
    assert_eq!(r.store.get_meeting(&id).unwrap().status, "processing");
    let events: Vec<_> = r.rx.try_iter().map(|e| e.event).collect();
    assert!(events.iter().any(|e| matches!(
        e,
        Event::StateChanged {
            state: SessionState::Processing,
            ..
        }
    )));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::TranscriptFinal { .. }))
    );
    // The sealed backlog is gone.
    assert_eq!(
        std::fs::read_dir(r._tmp.path().join("backlog"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn no_job_is_claimed_while_a_session_exists_and_the_live_engine_is_gone_first() {
    let r = rig(live_tier());
    let id = r.recorder.start(&request()).unwrap();
    feed(2);
    // A queued job exists from an earlier meeting: it must wait.
    let other = r.store.create_meeting(NewMeeting::default()).unwrap().gid;
    r.store
        .enqueue_job(Some(&other), FINAL_PASS_JOB, 1, &serde_json::json!({}))
        .unwrap();
    assert!(r.runner.run_one().is_none(), "recording");
    assert_eq!(
        r.loads.alive.load(Ordering::SeqCst),
        1,
        "live engines loaded"
    );
    r.recorder.stop().unwrap();
    // Draining: still nothing is claimed.
    while !drained(&r) {
        assert!(r.runner.run_one().is_none() || drained(&r));
        thread::sleep(Duration::from_millis(5));
    }
    // Released: the runner may claim, and no live engine is resident.
    assert!(r.runner.run_pending() >= 1);
    assert_eq!(r.seen.alive_engines.load(Ordering::SeqCst), 0);
    assert!(!r.seen.session_present.load(Ordering::SeqCst));
    assert_eq!(
        r.store
            .jobs_for_meeting(&id)
            .unwrap()
            .iter()
            .filter(|j| j.state == JobState::Done)
            .count(),
        1
    );
}

#[test]
fn locked_holds_the_engine_and_unlocking_catches_up() {
    let r = rig(live_tier());
    let life = Lifecycle::new({
        let runner = r.runner.clone();
        Arc::new(move || Some(runner.clone()))
    });
    life.launch(false);
    let id = r.recorder.start(&request()).unwrap();
    wait_for("models loaded", || {
        r.recorder.snapshot().phase == RecordPhase::Live
    });
    life.resign_active();
    assert!(life.entered_background());
    assert_eq!(r.recorder.snapshot().phase, RecordPhase::Locked);
    feed(8);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(
        r.store.segments(&id).unwrap().len(),
        0,
        "nothing transcribed while locked"
    );
    assert!(r.recorder.snapshot().backlog_s > 4.0);
    // Back in the foreground: the backlog drains into lines.
    life.become_active();
    wait_for("catch-up lines", || {
        r.recorder
            .snapshot()
            .session
            .is_some_and(|s| s.lines.len() >= 3)
    });
    r.recorder.stop().unwrap();
    wait_for("the session to drain", || drained(&r));
    assert_eq!(r.store.segments(&id).unwrap().len(), 3);
}

#[test]
fn thirty_seconds_inactive_unloads_the_models_and_return_reloads_them() {
    let r = rig(live_tier());
    let life = Lifecycle::new({
        let runner = r.runner.clone();
        Arc::new(move || Some(runner.clone()))
    });
    let id = r.recorder.start(&request()).unwrap();
    wait_for("models loaded", || {
        r.recorder.snapshot().phase == RecordPhase::Live
    });
    assert_eq!(r.loads.alive.load(Ordering::SeqCst), 1);
    life.resign_active();
    life.entered_background();
    r.recorder
        .latest()
        .unwrap()
        .shared
        .gate
        .pretend_suspended_for(engine::UNLOAD_AFTER + Duration::from_secs(1));
    wait_for("models unloaded", || {
        r.loads.alive.load(Ordering::SeqCst) == 0
    });
    feed(8);
    life.become_active();
    wait_for("models reloaded", || {
        r.loads.loaded.load(Ordering::SeqCst) == 2
    });
    wait_for("lines after the reload", || {
        r.recorder
            .snapshot()
            .session
            .is_some_and(|s| s.lines.len() >= 3)
    });
    r.recorder.stop().unwrap();
    wait_for("the session to drain", || drained(&r));
    assert_eq!(r.store.segments(&id).unwrap().len(), 3);
}

#[test]
fn a_stop_while_locked_finishes_after_the_app_returns() {
    let r = rig(live_tier());
    let life = Lifecycle::new({
        let runner = r.runner.clone();
        Arc::new(move || Some(runner.clone()))
    });
    let id = r.recorder.start(&request()).unwrap();
    wait_for("models loaded", || {
        r.recorder.snapshot().phase == RecordPhase::Live
    });
    life.resign_active();
    life.entered_background();
    feed(8);
    life.stop_requested();
    thread::sleep(Duration::from_millis(300));
    assert!(!drained(&r), "draining waits for the foreground");
    assert_eq!(r.recorder.snapshot().phase, RecordPhase::Finishing);
    assert!(
        r.runner.run_one().is_none(),
        "no job while inactive or draining"
    );
    life.become_active();
    wait_for("the session to drain", || drained(&r));
    assert_eq!(r.store.segments(&id).unwrap().len(), 3);
    assert_eq!(r.store.get_meeting(&id).unwrap().status, "processing");
}

#[test]
fn a_second_start_waits_for_the_draining_session() {
    let r = rig(live_tier());
    let life = Lifecycle::new({
        let runner = r.runner.clone();
        Arc::new(move || Some(runner.clone()))
    });
    let first = r.recorder.start(&request()).unwrap();
    wait_for("models loaded", || {
        r.recorder.snapshot().phase == RecordPhase::Live
    });
    life.resign_active();
    life.entered_background();
    feed(3);
    r.recorder.stop().unwrap();
    let second = {
        let rec = r.recorder.clone();
        thread::spawn(move || rec.start(&request()))
    };
    thread::sleep(Duration::from_millis(300));
    assert!(!second.is_finished(), "waits while the first drains");
    life.become_active();
    let id = second.join().unwrap().unwrap();
    assert_ne!(id, first);
    assert_eq!(
        r.store.get_meeting(&first).unwrap().status,
        "processing",
        "the first finished before the second began"
    );
    r.recorder.stop().unwrap();
    wait_for("the session to drain", || drained(&r));
}

#[test]
fn an_interruption_pauses_and_only_the_user_resumes() {
    let r = rig(live_tier());
    let id = r.recorder.start(&request()).unwrap();
    feed(2);
    wait_for("2 s recorded", || r.recorder.snapshot().elapsed_s > 1.9);
    let at = r.recorder.snapshot().elapsed_s;
    let life = Lifecycle::new(Arc::new(|| None));
    life.interruption(true);
    let st = r.recorder.snapshot();
    assert_eq!(st.phase, RecordPhase::Interrupted);
    assert!(
        !r.recorder.resume_prompt().pending,
        "not asked until it ends"
    );
    thread::sleep(Duration::from_millis(100)); // the pump takes the command
    feed(2); // the tap delivers nothing real, but a stray block must not count
    thread::sleep(Duration::from_millis(100));
    assert!(
        r.recorder.snapshot().elapsed_s - at < 0.5,
        "the timeline stopped"
    );
    life.interruption(false);
    let prompt = r.recorder.resume_prompt();
    assert!(prompt.pending && prompt.meeting == id && !prompt.call);
    assert_eq!(
        r.recorder.snapshot().phase,
        RecordPhase::Interrupted,
        "never resumes by itself"
    );
    r.recorder.resume().unwrap();
    assert!(!r.recorder.resume_prompt().pending);
    feed(2);
    wait_for("recording again", || {
        r.recorder.snapshot().elapsed_s > at + 1.0
    });
    let states: Vec<_> =
        r.rx.try_iter()
            .filter_map(|e| match e.event {
                Event::StateChanged { state, .. } => Some(state),
                _ => None,
            })
            .collect();
    assert!(states.contains(&SessionState::Paused));
    r.recorder.stop().unwrap();
    wait_for("the session to drain", || drained(&r));
}

#[test]
fn a_user_pause_stops_the_timeline_and_hot_phones_hold_the_engine() {
    let r = rig(live_tier());
    r.recorder.start(&request()).unwrap();
    feed(2);
    r.recorder.pause().unwrap();
    assert_eq!(r.recorder.snapshot().phase, RecordPhase::Paused);
    r.recorder.resume().unwrap();
    let life = Lifecycle::new(Arc::new(|| None));
    life.thermal_changed(2);
    wait_for("hot", || r.recorder.snapshot().phase == RecordPhase::Hot);
    life.thermal_changed(0);
    wait_for("live again", || {
        r.recorder.snapshot().phase != RecordPhase::Hot
    });
    r.recorder.stop().unwrap();
    wait_for("the session to drain", || drained(&r));
}

#[test]
fn a_call_blocks_the_start_until_the_call_notice_is_acknowledged() {
    let r = rig(live_tier());
    FORCE_CALL.store(true, Ordering::SeqCst);
    let mut req = request();
    // The ordinary consent flag does not lift the block.
    assert_eq!(r.recorder.start(&req).unwrap_err(), "call_active");
    req.call_acknowledged = true;
    let started = r.recorder.start(&req);
    FORCE_CALL.store(false, Ordering::SeqCst);
    started.unwrap();
    r.recorder.stop().unwrap();
    wait_for("the session to drain", || drained(&r));
}

#[test]
fn refused_targets_and_idle_state() {
    let r = rig(live_tier());
    let mut req = request();
    req.target = ProcessingTarget::Desktop;
    assert!(r.recorder.start(&req).is_err());
    assert!(r.recorder.stop().is_err(), "nothing to stop");
    assert!(!r.recorder.resume_prompt().pending);
    assert_eq!(r.recorder.snapshot().phase, RecordPhase::Idle);
}

#[test]
fn the_end_of_a_call_asks_even_without_an_interruption_end() {
    let r = rig(live_tier());
    let id = r.recorder.start(&request()).unwrap();
    FORCE_CALL.store(true, Ordering::SeqCst);
    let life = Lifecycle::new(Arc::new(|| None));
    life.call_active_changed(true);
    assert_eq!(r.recorder.snapshot().phase, RecordPhase::Interrupted);
    assert!(!r.recorder.resume_prompt().pending);
    FORCE_CALL.store(false, Ordering::SeqCst);
    life.call_active_changed(false);
    let prompt = r.recorder.resume_prompt();
    assert!(prompt.pending && prompt.call && prompt.meeting == id);
    r.recorder.stop().unwrap();
    wait_for("the session to drain", || drained(&r));
}

#[test]
fn the_live_models_wait_for_a_resident_final_pass() {
    let r = rig(live_tier());
    // A job is running (its models are loaded): the runner is not idle.
    let busy = Arc::new(AtomicBool::new(true));
    let b = busy.clone();
    struct Hold(Arc<AtomicBool>);
    impl JobHandler for Hold {
        fn kind(&self) -> &'static str {
            "hold"
        }
        fn run(&self, _: &JobCtx) -> Result<Outcome, String> {
            while self.0.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(5));
            }
            Ok(Outcome::Done)
        }
    }
    let runner = JobRunner::new(
        r.store.clone(),
        r.recorder.deps.events.clone(),
        vec![Arc::new(Hold(b))],
    );
    let other = r.store.create_meeting(NewMeeting::default()).unwrap().gid;
    r.store
        .enqueue_job(Some(&other), "hold", 1, &serde_json::json!({}))
        .unwrap();
    let r2 = runner.clone();
    let job = thread::spawn(move || r2.run_one());
    wait_for("the job to start", || {
        r.store
            .jobs_for_meeting(&other)
            .unwrap()
            .iter()
            .any(|j| j.state == JobState::Running)
    });
    let slot = runner.clone();
    let st = r.store.clone();
    let recorder = Recorder::new(RecorderDeps {
        store: Arc::new(move || Ok(st.clone())),
        events: r.recorder.deps.events.clone(),
        runner: Arc::new(move || Some(slot.clone())),
        models: r._tmp.path().join("models"),
        backlog_dir: r._tmp.path().join("backlog2"),
        metrics_dir: r._tmp.path().join("metrics2"),
        tier: live_tier(),
        provider: r.recorder.deps.provider.clone(),
        fake_mic: false,
    });
    recorder.start(&request()).unwrap();
    feed(2);
    // Capture runs at once; the models are not loaded while the job runs.
    wait_for("2 s recorded", || recorder.snapshot().elapsed_s > 1.9);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(r.loads.loaded.load(Ordering::SeqCst), 0, "models waited");
    assert_eq!(recorder.snapshot().phase, RecordPhase::Loading);
    busy.store(false, Ordering::SeqCst);
    job.join().unwrap();
    wait_for("the models to load", || {
        r.loads.loaded.load(Ordering::SeqCst) == 1
    });
    recorder.stop().unwrap();
    wait_for("the session to drain", || recorder.latest().is_none());
}

#[test]
fn the_launch_in_the_background_syncs_the_runner_before_it_spawns() {
    let r = rig(live_tier());
    let life = Lifecycle::new({
        let runner = r.runner.clone();
        Arc::new(move || Some(runner.clone()))
    });
    life.launch(true);
    let m = r.store.create_meeting(NewMeeting::default()).unwrap().gid;
    r.store
        .enqueue_job(Some(&m), FINAL_PASS_JOB, 1, &serde_json::json!({}))
        .unwrap();
    life.sync_runner(&r.runner);
    assert!(r.runner.run_one().is_none(), "inactive: nothing claimed");
    life.become_active();
    assert_eq!(r.runner.run_pending(), 1);
}

/// While live, only audio older than the unsettled margin is final; a flush
/// makes everything final.
#[test]
fn live_commits_keep_a_margin_for_undecoded_audio() {
    let dir = std::env::temp_dir().join(format!("ghi-commit-{}", std::process::id()));
    let shared = test_shared(&dir);
    let sec = engine::SAMPLE_RATE as u64;
    let fin = |end: f64| Update::Final {
        start: 0.0,
        end,
        text: "x".into(),
        words: Vec::new(),
        segs: Vec::new(),
    };
    shared.commit(&[], 2 * sec, false);
    assert_eq!(shared.committed_position(), 0, "within the margin");
    shared.commit(&[], 10 * sec, false);
    assert_eq!(shared.committed_position(), 7 * sec);
    shared.commit(&[Update::Partial("đang nói".into())], 12 * sec, false);
    assert_eq!(
        shared.committed_position(),
        7 * sec,
        "an utterance is in progress"
    );
    shared.commit(&[fin(11.5)], 13 * sec, false);
    assert_eq!(
        shared.committed_position(),
        sec * 23 / 2,
        "the final's end (later than 13 s minus the margin)"
    );
    shared.commit(&[], 14 * sec, true);
    assert_eq!(shared.committed_position(), 14 * sec, "flushed");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_synthetic_microphone_records() {
    let guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let mic = FakeMic::start();
    thread::sleep(Duration::from_millis(50));
    mic.stop();
    drop(guard);
}

#[test]
fn missing_models_record_only_and_the_final_pass_waits_for_them() {
    let r = rig_with(live_tier(), false);
    let id = r.recorder.start(&request()).unwrap();
    feed(2);
    wait_for("2 s recorded", || r.recorder.snapshot().elapsed_s > 1.9);
    let st = r.recorder.snapshot();
    assert_eq!(st.phase, RecordPhase::RecordOnly);
    assert_eq!(st.record_only_reason, Some(RecordOnlyReason::ModelsMissing));
    r.recorder.stop().unwrap();
    wait_for("the session to drain", || drained(&r));
    let jobs = r.store.jobs_for_meeting(&id).unwrap();
    assert_eq!(jobs.len(), 1, "the final pass is queued for later");
    assert_eq!(jobs[0].state, JobState::Queued);
    assert!(
        r.rx.try_iter().any(|e| matches!(
            e.event,
            Event::Error {
                kind: ghi_core::events::ErrorKind::ModelsMissing,
                ..
            }
        )),
        "the UI is told"
    );
}

#[test]
fn free_space_is_readable() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(free_bytes(tmp.path()).is_some_and(|f| f > 0));
    assert!(free_bytes(&tmp.path().join("missing")).is_none());
}

#[test]
fn an_attached_tap_blocks_a_recording_and_is_released() {
    let r = rig(live_tier());
    let (tap, _consumer) = ghi_audio::ring::ring(4800);
    attach_tap(tap).unwrap();
    let (other, _c2) = ghi_audio::ring::ring(4800);
    assert!(attach_tap(other).is_err(), "one tap at a time");
    assert!(r.recorder.start(&request()).is_err(), "the mic is in use");
    detach_tap();
    let id = r.recorder.start(&request()).unwrap();
    detach_tap(); // a recording's ring is not an external tap
    feed(1);
    wait_for("audio recorded", || r.recorder.snapshot().elapsed_s > 0.5);
    r.recorder.stop().unwrap();
    wait_for("the session to drain", || drained(&r));
    assert!(r.store.get_meeting(&id).is_ok());
}

#[test]
fn two_starts_at_once_make_one_recording() {
    let r = rig(live_tier());
    let (a, b) = (r.recorder.clone(), r.recorder.clone());
    let t1 = thread::spawn(move || a.start(&request()));
    let t2 = thread::spawn(move || b.start(&request()));
    let results = [t1.join().unwrap(), t2.join().unwrap()];
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    assert_eq!(
        r.store.list_meetings(10, 0).unwrap().len(),
        1,
        "no stray meeting"
    );
    r.recorder.stop().unwrap();
    wait_for("the session to drain", || drained(&r));
    assert_eq!(
        std::fs::read_dir(r._tmp.path().join("backlog"))
            .unwrap()
            .count(),
        0
    );
}
