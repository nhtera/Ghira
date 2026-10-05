// SPDX-License-Identifier: Apache-2.0
//! A final pass that yields (a recording, the app going to the background) or
//! is killed resumes from its checkpoints: finished ASR chunks and the
//! diarization are skipped, the result equals an uninterrupted run, and the
//! checkpoints are dropped when the audio, the engine or the pass changes.
//! Scripted engines that count what they are asked to do.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ghi_core::engines::{BoxAsr, BoxDiar, SpeechEngines};
use ghi_core::events::{Event, EventRx, bus};
use ghi_core::final_pass::FinalPassJob;
use ghi_core::import::{ImportOptions, import_file};
use ghi_core::jobs::{JobRunner, always_ready};
use ghi_core::session::{FINAL_PASS_JOB, RecordingHooks};
use ghi_speech::{AsrResult, AsrStream, DiarStream, SpeakerSegment, Word};
use ghi_store::jobs::JobState;
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::Store;

/// One final word per second of audio a stream was fed: "w<k>" for the k-th
/// second *of that stream* (a chunk), so the text shows where the pass cut.
struct Probe {
    id: Mutex<String>,
    asr_opened: AtomicUsize,
    diar_opened: AtomicUsize,
    /// Calls `hook` when the n-th ASR stream (1-based, since it was set) opens.
    preempt_at: AtomicUsize,
    hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

struct ProbeAsr {
    pushed: f64,
    out: Vec<AsrResult>,
}

impl AsrStream for ProbeAsr {
    fn push(&mut self, pcm: &[f32], sample_rate: u32) -> ghi_speech::Result<()> {
        let k = self.pushed;
        self.pushed += pcm.len() as f64 / f64::from(sample_rate);
        self.out.push(AsrResult {
            is_final: true,
            text: format!("w{}", k as u64),
            words: vec![Word {
                text: format!("w{}", k as u64),
                start: k,
                end: self.pushed,
                confidence: 0.9,
                speaker: None,
            }],
            languages: Vec::new(),
            audio_processed: self.pushed,
        });
        Ok(())
    }
    fn finish(&mut self) -> ghi_speech::Result<()> {
        Ok(())
    }
    fn next_result(&mut self) -> ghi_speech::Result<Option<AsrResult>> {
        Ok((!self.out.is_empty()).then(|| self.out.remove(0)))
    }
}

struct ProbeDiar(f64);

impl DiarStream for ProbeDiar {
    fn push(&mut self, pcm: &[f32], sample_rate: u32) -> ghi_speech::Result<()> {
        self.0 += pcm.len() as f64 / f64::from(sample_rate);
        Ok(())
    }
    fn finish(&mut self) -> ghi_speech::Result<()> {
        Ok(())
    }
    fn segments(&self) -> ghi_speech::Result<Vec<SpeakerSegment>> {
        Ok(vec![SpeakerSegment {
            start: 0.0,
            end: self.0,
            speaker: 1,
        }])
    }
}

impl SpeechEngines for Probe {
    fn asr(&self, _language: Option<&str>) -> ghi_speech::Result<BoxAsr> {
        let n = self.asr_opened.fetch_add(1, Ordering::SeqCst) + 1;
        let at = self.preempt_at.load(Ordering::SeqCst);
        if at != 0 && n == at {
            self.preempt_at.store(0, Ordering::SeqCst);
            if let Some(h) = self.hook.lock().unwrap().clone() {
                h();
            }
        }
        Ok(Box::new(ProbeAsr {
            pushed: 0.0,
            out: Vec::new(),
        }))
    }
    fn diar(&self) -> ghi_speech::Result<BoxDiar> {
        self.diar_opened.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(ProbeDiar(0.0)))
    }
    fn chunk_ms(&self) -> u32 {
        560
    }
    fn checkpoint_id(&self) -> String {
        self.id.lock().unwrap().clone()
    }
}

/// A 16 kHz mono WAV of `secs` of a steady tone.
fn tone(path: &Path, secs: usize, hz: f32) {
    let mut w = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for i in 0..16_000 * secs {
        let x = (i as f32 * hz * std::f32::consts::TAU / 16_000.0).sin() * 0.2;
        w.write_sample((x * 32_767.0) as i16).unwrap();
    }
    w.finalize().unwrap();
}

/// Time, text and who (label, name, Me).
type Line = (i64, i64, String, Option<(i64, Option<String>, bool)>);

struct Rig {
    _tmp: tempfile::TempDir,
    store: Arc<Store>,
    probe: Arc<Probe>,
    runner: Arc<JobRunner>,
    rx: EventRx,
    meeting: String,
}

impl Rig {
    fn new() -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::open(
                tmp.path(),
                Arc::new(MemoryKeyStore::default()),
                Protection::default(),
            )
            .unwrap(),
        );
        let wav = tmp.path().join("long.wav");
        tone(&wav, 200, 330.0);
        let (tx, rx) = bus();
        let probe = Arc::new(Probe {
            id: Mutex::new("probe-1".into()),
            asr_opened: AtomicUsize::new(0),
            diar_opened: AtomicUsize::new(0),
            preempt_at: AtomicUsize::new(0),
            hook: Mutex::new(None),
        });
        let engines = probe.clone();
        let runner = JobRunner::new(
            store.clone(),
            tx.clone(),
            vec![Arc::new(FinalPassJob {
                engines: Arc::new(move || Ok(engines.clone() as Arc<dyn SpeechEngines>)),
                chunk_s: 600.0,
                ready: always_ready(),
                voice: None,
            })],
        );
        let meeting = import_file(&store, &wav, &ImportOptions::default(), &tx)
            .unwrap()
            .meeting;
        Rig {
            _tmp: tmp,
            store,
            probe,
            runner,
            rx,
            meeting,
        }
    }

    fn counts(&self) -> (usize, usize) {
        (
            self.probe.asr_opened.load(Ordering::SeqCst),
            self.probe.diar_opened.load(Ordering::SeqCst),
        )
    }

    /// Runs the pass to its end; what it opened (ASR streams, diarizers).
    fn run(&self) -> (usize, usize) {
        let before = self.counts();
        self.runner.run_pending();
        let after = self.counts();
        (after.0 - before.0, after.1 - before.1)
    }

    /// Queues another pass over the same meeting.
    fn again(&self) -> i64 {
        self.store
            .enqueue_job(
                Some(&self.meeting),
                FINAL_PASS_JOB,
                1,
                &serde_json::json!({}),
            )
            .unwrap()
    }

    /// The pass yields when its `n`-th next ASR chunk opens (a recording
    /// starts); `attempts` sees the job's attempts at that moment.
    fn preempt_at_chunk(&self, n: usize, attempts: Arc<AtomicUsize>) {
        let (runner, store, m) = (
            self.runner.clone(),
            self.store.clone(),
            self.meeting.clone(),
        );
        *self.probe.hook.lock().unwrap() = Some(Arc::new(move || {
            let job = store.jobs_for_meeting(&m).unwrap().pop().unwrap();
            attempts.store(job.attempts as usize, Ordering::SeqCst);
            runner.recording_started();
        }));
        self.probe.preempt_at.store(
            self.probe.asr_opened.load(Ordering::SeqCst) + n,
            Ordering::SeqCst,
        );
    }

    /// What the user sees: time, text, and who (label, name, Me).
    fn lines(&self) -> Vec<Line> {
        let speakers = self.store.speakers(&self.meeting).unwrap();
        self.store
            .segments(&self.meeting)
            .unwrap()
            .into_iter()
            .map(|s| {
                let who = s.speaker_gid.as_ref().map(|g| {
                    let sp = speakers.iter().find(|p| &p.gid == g).unwrap();
                    (sp.label_idx, sp.display_name.clone(), sp.is_me)
                });
                (s.t0_ms, s.t1_ms, s.text, who)
            })
            .collect()
    }

    fn progress(&self) -> Vec<f32> {
        self.rx
            .try_iter()
            .filter_map(|e| match e.event {
                Event::JobProgress {
                    stage: Some(_),
                    progress,
                    ..
                } => Some(progress),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn a_yielded_pass_resumes_without_redoing_finished_chunks_and_matches_an_uninterrupted_one() {
    let rig = Rig::new();
    // Uninterrupted: K chunks and one diarization.
    let (chunks, diars) = rig.run();
    assert!(chunks >= 3, "the audio spans several chunks: {chunks}");
    assert_eq!(diars, 1);
    let baseline = rig.lines();
    assert!(baseline.len() > 150, "{}", baseline.len());
    assert_eq!(
        rig.store
            .get_meeting(&rig.meeting)
            .unwrap()
            .transcript_version,
        2
    );
    let _ = rig.progress();

    // Again, preempted when the third chunk opens: two chunks and the
    // diarization are saved, nothing is stored, and the yield cost nothing.
    let job = rig.again();
    let attempts = Arc::new(AtomicUsize::new(99));
    rig.preempt_at_chunk(3, attempts.clone());
    let (opened, diars) = rig.run();
    assert_eq!((opened, diars), (3, 1));
    let j = rig.store.job(job).unwrap();
    assert_eq!((j.state, j.attempts), (JobState::Queued, 0));
    assert_eq!(j.payload["done"], 3, "the diarization and two chunks");
    assert_eq!(j.payload["total"], chunks as u64 + 1);
    assert_eq!(
        rig.store
            .get_meeting(&rig.meeting)
            .unwrap()
            .transcript_version,
        2
    );
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        0,
        "a run that saved something gave its attempt back"
    );
    let shown = rig.progress();
    let paused_at = j.payload["p"].as_f64().unwrap() as f32;
    assert!(paused_at > 0.12, "{paused_at}");

    // The recording ends: only the rest is computed, the diarizer is not run,
    // the result is the uninterrupted one, and the percent never went back.
    rig.runner.recording_stopped();
    let (opened, diars) = rig.run();
    assert_eq!((opened, diars), (chunks - 2, 0));
    assert_eq!(rig.store.job(job).unwrap().state, JobState::Done);
    assert_eq!(rig.lines(), baseline);
    assert_eq!(
        rig.store
            .get_meeting(&rig.meeting)
            .unwrap()
            .transcript_version,
        3
    );
    let resumed = rig.progress();
    assert!(
        resumed.iter().all(|p| *p >= paused_at - 1e-6),
        "{paused_at} then {resumed:?}"
    );
    assert!(
        resumed.windows(2).all(|w| w[0] <= w[1] + 1e-6),
        "{resumed:?}"
    );
    assert_eq!(*resumed.last().unwrap(), 1.0);
    assert!(shown.iter().all(|p| *p <= paused_at + 1e-6));

    // A second pause, with the user at work in between: a speaker renamed and
    // a line edited. The resumed pass keeps both, and all else is unchanged.
    rig.again();
    rig.preempt_at_chunk(2, Arc::new(AtomicUsize::new(0)));
    assert_eq!(rig.run(), (2, 1));
    let first = rig.store.speakers(&rig.meeting).unwrap()[0].gid.clone();
    rig.store.rename_speaker(&first, Some("An")).unwrap();
    let seg = rig.store.segments(&rig.meeting).unwrap()[5].clone();
    rig.store
        .update_segment_text(&seg.gid, "đã sửa tay")
        .unwrap();
    rig.runner.recording_stopped();
    assert_eq!(rig.run(), (chunks - 1, 0));
    let mut expected = baseline.clone();
    expected[5].2 = "đã sửa tay".into();
    for l in &mut expected {
        if l.3.as_ref().is_some_and(|w| w.0 == 0) {
            l.3.as_mut().unwrap().1 = Some("An".into());
        }
    }
    assert_eq!(rig.lines(), expected);
    let segs = rig.store.segments(&rig.meeting).unwrap();
    assert!(segs[5].edited && segs.iter().filter(|s| s.edited).count() == 1);
    // The edit and the name are part of the transcript now: baseline again.
    let baseline = rig.lines();

    // Done drops the checkpoints: the next pass starts from scratch.
    rig.again();
    assert_eq!(rig.run(), (chunks, 1));
    assert_eq!(rig.lines(), baseline);
}

#[test]
fn another_engine_or_model_starts_over() {
    let rig = Rig::new();
    let (chunks, _) = rig.run();
    let baseline = rig.lines();
    rig.again();
    rig.preempt_at_chunk(2, Arc::new(AtomicUsize::new(0)));
    assert_eq!(rig.run(), (2, 1));
    // The model changed while it was paused: nothing carries over.
    *rig.probe.id.lock().unwrap() = "probe-2".into();
    rig.runner.recording_stopped();
    assert_eq!(rig.run(), (chunks, 1));
    assert_eq!(rig.lines(), baseline);
}

#[test]
fn yields_without_progress_hold_the_job_and_a_quiet_app_releases_it() {
    use ghi_core::jobs::YieldBackoff;
    use std::time::Duration;
    let rig = Rig::new();
    rig.runner.set_yield_backoff(YieldBackoff {
        after: 2,
        idle: Duration::from_millis(200),
    });
    // The import's own pass first.
    rig.runner.run_pending();
    // A recording preempts as the first chunk opens, every time.
    let runner = rig.runner.clone();
    *rig.probe.hook.lock().unwrap() = Some(Arc::new(move || runner.recording_started()));
    let again = rig.again();
    // Diarization is saved before the first chunk, so that is progress:
    // the first yield counts, then nothing new twice in a row.
    for n in 0..3 {
        rig.probe.preempt_at.store(
            rig.probe.asr_opened.load(Ordering::SeqCst) + 1,
            Ordering::SeqCst,
        );
        rig.runner.run_pending();
        rig.runner.recording_stopped();
        let j = rig.store.job(again).unwrap();
        assert_eq!((j.state, j.attempts), (JobState::Queued, 0), "yield {n}");
    }
    assert_eq!(rig.runner.held_jobs(), [again]);
    assert!(rig.runner.run_one().is_none(), "held, not failed");
    std::thread::sleep(Duration::from_millis(250));
    let (opened, _) = rig.run();
    assert!(opened > 0);
    assert_eq!(rig.store.job(again).unwrap().state, JobState::Done);
}
