// SPDX-License-Identifier: Apache-2.0
//! After stop: notes from the live transcript, the final pass (v2, name
//! carry-over, edited lines kept, custom vocabulary), notes again, ready.
//! Scripted speech engines and a scripted notes model.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ghi_audio::Track;
use ghi_core::capture::{ReplayTrack, replay};
use ghi_core::engines::{FakeEngines, Script, SpeechEngines};
use ghi_core::events::{Event, bus};
use ghi_core::final_pass::{FinalPassJob, VOCABULARY_SETTING};
use ghi_core::jobs::{JobRunner, always_ready};
use ghi_core::live::Mode;
use ghi_core::notes_job::{NOTES_FINAL_JOB, NotesJob};
use ghi_core::session::{NOTES_LIVE_JOB, Session, SessionConfig};
use ghi_llm::{Completion, EngineInfo, Llm, Request};
use ghi_speech::SpeakerSegment;
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::Store;

/// Notes with one TL;DR citing the first line, whatever is asked.
struct OneLiner;

impl Llm for OneLiner {
    fn engine(&self) -> EngineInfo {
        EngineInfo {
            name: "scripted".into(),
            version: "1".into(),
        }
    }
    fn context_tokens(&self) -> u32 {
        16_384
    }
    fn complete(&mut self, _req: &Request) -> ghi_llm::Result<Completion> {
        Ok(Completion {
            text: r#"{"tldr":[{"text":"Chốt lịch beta","cite":[0]}],"decisions":[],"action_items":[],"open_questions":[],"key_quotes":[],"topics":[]}"#.into(),
            tokens_in: 10,
            tokens_out: 5,
            truncated: false,
        })
    }
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
            (6.0, 7.0, "anh le minh anh deploy lên kubernetis".into()),
        ],
        turns: vec![turn(1, 0.5, 2.5), turn(2, 2.8, 4.5), turn(1, 5.5, 7.5)],
    }
}

#[test]
fn notes_then_final_pass_then_final_notes() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    store
        .set_setting(
            VOCABULARY_SETTING,
            &serde_json::json!(["Lê Minh Anh", "Kubernetes"]),
        )
        .unwrap();
    let (tx, rx) = bus();
    let engines: Arc<dyn SpeechEngines> = FakeEngines::new(script());
    let llm: ghi_core::notes_job::LlmFactory =
        Arc::new(|_| Ok(Box::new(OneLiner) as Box<dyn Llm + Send>));
    let template = ghi_llm::template::builtin("general").unwrap();
    let final_engines = engines.clone();
    let runner = JobRunner::new(
        store.clone(),
        tx.clone(),
        vec![
            Arc::new(NotesJob {
                kind: NOTES_LIVE_JOB,
                version: 1,
                template: template.clone(),
                llm: llm.clone(),
                ready: always_ready(),
            }),
            Arc::new(FinalPassJob {
                engines: Arc::new(move || Ok(final_engines.clone())),
                chunk_s: 600.0,
                ready: always_ready(),
                voice: None,
            }),
            Arc::new(NotesJob {
                kind: NOTES_FINAL_JOB,
                version: 2,
                template,
                llm,
                ready: always_ready(),
            }),
        ],
    );

    let capture = replay(
        vec![ReplayTrack {
            track: Track::Mic,
            samples: (0..48_000 * 9)
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
            title: "beta".into(),
            queue_jobs: true,
            lossless: true,
            echo_cancellation: true,
        },
        tx,
        Some(runner.clone()),
    )
    .unwrap();
    let meeting = s.meeting().to_string();
    // Wait for the three lines, name speaker 1 live.
    let t = Instant::now();
    let mut finals = 0;
    let mut first_speaker = None;
    while finals < 3 {
        assert!(t.elapsed() < Duration::from_secs(20));
        if let Ok(env) = rx.recv_timeout(Duration::from_millis(50))
            && let Event::TranscriptFinal { line, .. } = env.event
        {
            finals += 1;
            first_speaker.get_or_insert(line.speaker.unwrap());
        }
    }
    s.rename(first_speaker.unwrap(), "Lan");
    // While recording, no job runs.
    assert!(runner.run_one().is_none());
    s.stop().unwrap();

    // The user fixes line 2 before the final pass runs.
    let v1 = store.segments(&meeting).unwrap();
    assert_eq!(v1.len(), 3);
    store
        .update_segment_text(&v1[1].gid, "Hôm nay chốt lịch beta.")
        .unwrap();

    assert_eq!(
        runner.run_pending(),
        3,
        "notes_live, final_pass, notes_final"
    );
    let m = store.get_meeting(&meeting).unwrap();
    assert_eq!((m.status.as_str(), m.transcript_version), ("ready", 2));
    let v2 = store.segments(&meeting).unwrap();
    let texts: Vec<&str> = v2.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "xin chào mọi người",
            "Hôm nay chốt lịch beta.",
            "anh Lê Minh Anh deploy lên Kubernetes"
        ]
    );
    assert!(
        v2[1].edited,
        "the user's text, kept and still marked edited"
    );
    // Lan (live speaker 1) carried over to both of her lines.
    let speakers = store.speakers(&meeting).unwrap();
    let lan = speakers
        .iter()
        .find(|s| s.display_name.as_deref() == Some("Lan"))
        .unwrap();
    assert_eq!(v2[0].speaker_gid.as_deref(), Some(lan.gid.as_str()));
    assert_eq!(v2[2].speaker_gid.as_deref(), Some(lan.gid.as_str()));
    assert_eq!(speakers.len(), 2, "no new speaker: both clusters matched");
    let notes = store.note_blocks(&meeting).unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].body, "Chốt lịch beta");
    // The detail's footer says which model wrote them.
    assert_eq!(
        store.notes_model(&meeting).unwrap().as_deref(),
        Some("scripted")
    );
    let versions: Vec<u32> = rx
        .try_iter()
        .filter_map(|e| match e.event {
            Event::NotesReady { version, .. } => Some(version),
            _ => None,
        })
        .collect();
    assert_eq!(versions, [1, 2]);
}

/// Models missing: the session records audio only, its jobs wait in the
/// queue (no attempt spent), and transcript + notes come once they arrive.
#[test]
fn record_now_process_when_models_arrive() {
    use std::sync::atomic::{AtomicBool, Ordering};

    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    let (tx, rx) = bus();
    let installed = Arc::new(AtomicBool::new(false));
    let ready: ghi_core::jobs::Ready = {
        let i = installed.clone();
        Arc::new(move || i.load(Ordering::SeqCst))
    };
    let engines: Arc<dyn SpeechEngines> = FakeEngines::new(script());
    let llm: ghi_core::notes_job::LlmFactory =
        Arc::new(|_| Ok(Box::new(OneLiner) as Box<dyn Llm + Send>));
    let template = ghi_llm::template::builtin("general").unwrap();
    let runner = JobRunner::new(
        store.clone(),
        tx.clone(),
        vec![
            Arc::new(NotesJob {
                kind: NOTES_LIVE_JOB,
                version: 1,
                template: template.clone(),
                llm: llm.clone(),
                ready: ready.clone(),
            }),
            Arc::new(FinalPassJob {
                engines: Arc::new(move || Ok(engines.clone())),
                chunk_s: 600.0,
                ready: ready.clone(),
                voice: None,
            }),
            Arc::new(NotesJob {
                kind: NOTES_FINAL_JOB,
                version: 2,
                template,
                llm,
                ready,
            }),
        ],
    );
    let capture = replay(
        vec![ReplayTrack {
            track: Track::Mic,
            samples: (0..48_000 * 9)
                .map(|i| (i as f32 * 0.03).sin() * 0.2)
                .collect(),
            sample_rate: 48_000,
        }],
        None,
    )
    .unwrap();
    let s = Session::start(
        store.clone(),
        None,
        capture,
        SessionConfig {
            mode: Mode::Room,
            language: None,
            title: "no models".into(),
            queue_jobs: true,
            // Ignored without engines (nothing to wait for).
            lossless: true,
            echo_cancellation: true,
        },
        tx,
        Some(runner.clone()),
    )
    .unwrap();
    assert!(!s.transcribing());
    let meeting = s.meeting().to_string();
    let t = Instant::now();
    // The source ends before the pump has drained it: wait for the clock.
    while s.now_ms() < 8_900 {
        assert!(
            t.elapsed() < Duration::from_secs(20),
            "the replay plays out"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    s.mark();
    s.discard(1.0).unwrap();
    let report = s.stop().unwrap();
    assert_eq!(report.jobs.len(), 2);
    assert!(report.duration_ms >= 8_900, "{}", report.duration_ms);
    let spans = store.discarded_spans(&meeting).unwrap();
    assert!(spans.len() == 1 && spans[0].0 >= 7_900, "{spans:?}");
    assert!(store.segments(&meeting).unwrap().is_empty());
    assert_eq!(
        store.tracks(&meeting).unwrap().len(),
        1,
        "the audio is kept"
    );

    // No models: nothing runs, nothing fails.
    assert_eq!(runner.run_pending(), 0);
    for id in &report.jobs {
        let j = store.job(*id).unwrap();
        assert_eq!(
            (j.state, j.attempts),
            (ghi_store::jobs::JobState::Queued, 0)
        );
    }
    assert_eq!(store.get_meeting(&meeting).unwrap().status, "processing");

    // The models arrive.
    installed.store(true, Ordering::SeqCst);
    assert_eq!(
        runner.run_pending(),
        3,
        "notes_live, final_pass, notes_final"
    );
    let m = store.get_meeting(&meeting).unwrap();
    assert_eq!((m.status.as_str(), m.transcript_version), ("ready", 2));
    let v2 = store.segments(&meeting).unwrap();
    assert_eq!(v2.len(), 3, "transcribed from the stored audio");
    assert!(v2.iter().all(|s| s.speaker_gid.is_some()));
    let notes = store.note_blocks(&meeting).unwrap();
    assert_eq!(notes.len(), 1);
    let events: Vec<Event> = rx.try_iter().map(|e| e.event).collect();
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Error {
            kind: ghi_core::events::ErrorKind::ModelsMissing,
            ..
        }
    )));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Event::TranscriptFinal { .. })),
        "no live transcript"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::DiscardApplied { .. }))
    );
}

/// Remembers the prompts it was given.
struct Recorder(Arc<std::sync::Mutex<Vec<String>>>);

impl Llm for Recorder {
    fn engine(&self) -> EngineInfo {
        OneLiner.engine()
    }
    fn context_tokens(&self) -> u32 {
        16_384
    }
    fn complete(&mut self, req: &Request) -> ghi_llm::Result<Completion> {
        let text: Vec<&str> = req.messages.iter().map(|m| m.content.as_str()).collect();
        self.0.lock().unwrap().push(text.join("\n"));
        OneLiner.complete(req)
    }
}

/// Regenerate (D6): the job's payload picks the template and the language.
#[test]
fn regenerate_uses_the_chosen_template_and_language() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    let m = store
        .create_meeting(ghi_store::store::NewMeeting::default())
        .unwrap()
        .gid;
    store
        .add_segments(
            &m,
            vec![ghi_store::store::NewSegment {
                t0_ms: 0,
                t1_ms: 2000,
                text: "we ship on friday".into(),
                lang: Some("en".into()),
                ..Default::default()
            }],
        )
        .unwrap();
    let prompts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = prompts.clone();
    let llm: ghi_core::notes_job::LlmFactory =
        Arc::new(move |_| Ok(Box::new(Recorder(seen.clone())) as Box<dyn Llm + Send>));
    let (tx, _rx) = bus();
    let runner = JobRunner::new(
        store.clone(),
        tx,
        vec![Arc::new(NotesJob {
            kind: NOTES_FINAL_JOB,
            version: 2,
            template: ghi_llm::template::builtin("general").unwrap(),
            llm,
            ready: always_ready(),
        })],
    );
    store
        .enqueue_job(
            Some(&m),
            NOTES_FINAL_JOB,
            1,
            &serde_json::json!({"template": "standup", "lang": "vi"}),
        )
        .unwrap();
    assert_eq!(runner.run_pending(), 1);
    let p = prompts.lock().unwrap().join("\n");
    let standup = ghi_llm::template::builtin("standup").unwrap();
    assert!(p.contains(&standup.guidance_vi), "standup, in Vietnamese");
    assert!(!p.contains(&ghi_llm::template::builtin("general").unwrap().guidance_en));
}

/// Notes as [`OneLiner`]; for the enhance requests, the first line is
/// supported by segment 0 and the second isn't.
struct Enhancer;

impl Llm for Enhancer {
    fn engine(&self) -> EngineInfo {
        OneLiner.engine()
    }
    fn context_tokens(&self) -> u32 {
        16_384
    }
    fn complete(&mut self, req: &Request) -> ghi_llm::Result<Completion> {
        let enhance = req
            .schema
            .as_ref()
            .is_some_and(|s| s.to_string().contains("\"lines\""));
        if !enhance {
            return OneLiner.complete(req);
        }
        let prompt: String = req.messages.iter().map(|m| m.content.as_str()).collect();
        let text = if prompt.contains("pricing") {
            r#"{"lines":[{"line":1,"found":false,"points":[]}]}"#
        } else {
            r#"{"lines":[{"line":1,"found":true,"points":[{"text":"The release ships on Friday.","cite":[0]}]}]}"#
        };
        Ok(Completion {
            text: text.into(),
            tokens_in: 10,
            tokens_out: 5,
            truncated: false,
        })
    }
}

/// The final notes expand the user's own lines (cited), or say "not found".
#[test]
fn final_notes_enhance_what_the_user_typed() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    let m = store
        .create_meeting(ghi_store::store::NewMeeting::default())
        .unwrap()
        .gid;
    store
        .add_segments(
            &m,
            vec![
                ghi_store::store::NewSegment {
                    t0_ms: 0,
                    t1_ms: 3000,
                    text: "we ship the release on friday after the review".into(),
                    lang: Some("en".into()),
                    ..Default::default()
                },
                ghi_store::store::NewSegment {
                    t0_ms: 3000,
                    t1_ms: 6000,
                    text: "the design review happens thursday morning".into(),
                    lang: Some("en".into()),
                    ..Default::default()
                },
            ],
        )
        .unwrap();
    let user = |text: &str, t: i64| {
        let a = store.anchor_for_range(&m, t, t).unwrap();
        store
            .add_note_block(
                &m,
                ghi_store::store::NewNoteBlock {
                    kind: "note".into(),
                    provenance: ghi_store::store::Provenance::User,
                    body: text.into(),
                    anchors: vec![a],
                    pinned: false,
                },
            )
            .unwrap()
            .gid
    };
    let ship = user("release ship friday", 1000);
    let pricing = user("ask about pricing tiers", 4000);
    let llm: ghi_core::notes_job::LlmFactory =
        Arc::new(|_| Ok(Box::new(Enhancer) as Box<dyn Llm + Send>));
    let (tx, _rx) = bus();
    let runner = JobRunner::new(
        store.clone(),
        tx,
        vec![Arc::new(NotesJob {
            kind: NOTES_FINAL_JOB,
            version: 2,
            template: ghi_llm::template::builtin("general").unwrap(),
            llm,
            ready: always_ready(),
        })],
    );
    store
        .enqueue_job(Some(&m), NOTES_FINAL_JOB, 1, &serde_json::json!({}))
        .unwrap();
    assert_eq!(runner.run_pending(), 1);
    let blocks = store.note_blocks(&m).unwrap();
    let of = |gid: &str| -> Vec<_> {
        let kind = format!("{}{gid}", ghi_core::notes_job::ENHANCED_PREFIX);
        blocks.iter().filter(|b| b.kind == kind).collect()
    };
    let found = of(&ship);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].body, "The release ships on Friday.");
    assert_eq!(found[0].anchors[0].t0_ms, 0, "cites segment 0");
    let missing = of(&pricing);
    assert_eq!(missing.len(), 1);
    assert!(missing[0].body.is_empty(), "not found");
}
