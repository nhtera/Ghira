// SPDX-License-Identifier: Apache-2.0
//! Multi-track import (phase 14d, D9): a Zoom "separate audio file for each
//! participant" recording becomes one meeting whose speakers come from the
//! tracks. The final pass attributes words by each participant's speech spans
//! and never runs the diarizer; overlap survives a reload. Scripted engines.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ghi_core::engines::{BoxAsr, BoxDiar, FakeEngines, Script, SpeechEngines};
use ghi_core::events::bus;
use ghi_core::final_pass::FinalPassJob;
use ghi_core::import::{CANCELLED, ImportOptions, import_file, import_tracks};
use ghi_core::jobs::{JobRunner, always_ready};
use ghi_core::presets::local_ms;
use ghi_speech::SpeakerSegment;
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::{Store, TrackKind};

fn open_store() -> (tempfile::TempDir, Arc<Store>) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    (tmp, store)
}

/// A 16 kHz mono WAV of `secs` with a tone of `hz` over each of `talk` (s).
fn participant(path: &Path, secs: f32, hz: f32, talk: &[(f32, f32)]) {
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
    for i in 0..(16_000.0 * secs) as usize {
        let t = i as f32 / 16_000.0;
        let on = talk.iter().any(|&(a, b)| t >= a && t < b);
        let x = if on {
            (i as f32 * hz * std::f32::consts::TAU / 16_000.0).sin() * 0.2
        } else {
            0.0
        };
        w.write_sample((x * 32_767.0) as i16).unwrap();
    }
    w.finalize().unwrap();
}

/// Engines whose diarizer is counted (and, if ever run, would say the wrong
/// thing: one speaker for everything).
struct Counting {
    inner: Arc<FakeEngines>,
    diars: AtomicUsize,
}

impl SpeechEngines for Counting {
    fn asr(&self, language: Option<&str>) -> ghi_speech::Result<BoxAsr> {
        self.inner.asr(language)
    }
    fn diar(&self) -> ghi_speech::Result<BoxDiar> {
        self.diars.fetch_add(1, Ordering::SeqCst);
        self.inner.diar()
    }
    fn chunk_ms(&self) -> u32 {
        self.inner.chunk_ms()
    }
}

fn counting(script: Script) -> Arc<Counting> {
    Arc::new(Counting {
        inner: FakeEngines::new(script),
        diars: AtomicUsize::new(0),
    })
}

fn runner(store: &Arc<Store>, engines: Arc<Counting>) -> Arc<JobRunner> {
    let (tx, _rx) = bus();
    JobRunner::new(
        store.clone(),
        tx,
        vec![Arc::new(FinalPassJob {
            engines: Arc::new(move || Ok(engines.clone() as Arc<dyn SpeechEngines>)),
            chunk_s: 600.0,
            ready: always_ready(),
            voice: None,
        })],
    )
}

struct Zoom {
    _dir: tempfile::TempDir,
    files: Vec<(PathBuf, Option<String>)>,
}

/// Linh talks 1-6 s and 14-18 s, Minh 8-13 s and 15-19 s (they overlap at
/// 15-18 s), Sarah is silent. 20 s each.
fn zoom() -> Zoom {
    let dir = tempfile::tempdir().unwrap();
    let rec = dir
        .path()
        .join("2026-07-03 14.05.02 Sprint 81234567890/Audio Record");
    std::fs::create_dir_all(&rec).unwrap();
    let make = |name: &str, hz: f32, talk: &[(f32, f32)]| {
        let p = rec.join(name);
        participant(&p, 20.0, hz, talk);
        p
    };
    let files = vec![
        (
            make("audioLinh1111.wav", 300.0, &[(1.0, 6.0), (14.0, 18.0)]),
            Some("Linh".to_string()),
        ),
        (
            make("audioMinh2222.wav", 700.0, &[(8.0, 13.0), (15.0, 19.0)]),
            Some("Minh".to_string()),
        ),
        (
            make("audioSarah3333.wav", 1100.0, &[]),
            Some("Sarah".to_string()),
        ),
    ];
    Zoom { _dir: dir, files }
}

fn script() -> Script {
    let turn = |speaker, start, end| SpeakerSegment {
        start,
        end,
        speaker,
    };
    Script {
        utterances: vec![
            (2.0, 4.0, "xin chào mọi người".into()),
            (9.0, 11.0, "tôi báo cáo tiến độ".into()),
            (15.5, 17.5, "hai người nói cùng lúc".into()),
        ],
        // If the diarizer ran it would call this all one voice.
        turns: vec![turn(1, 0.0, 20.0)],
    }
}

fn opts() -> ImportOptions {
    ImportOptions::default()
}

fn near(a: i64, b: i64) -> bool {
    (a - b).abs() <= 40
}

#[test]
fn one_meeting_with_the_participants_as_speakers() {
    let (_t, store) = open_store();
    let z = zoom();
    let (tx, _rx) = bus();
    let r = import_tracks(&store, &z.files, &opts(), &tx).unwrap();
    assert_eq!((r.tracks, r.duplicate), (3, false));
    assert!(
        near(r.duration_ms, 20_000),
        "mixed length is the longest: {}",
        r.duration_ms
    );
    let m = store.get_meeting(&r.meeting).unwrap();
    assert_eq!(m.source, "file");
    assert_eq!(m.source_app.as_deref(), Some("zoom"));
    assert_eq!(m.title, "Sprint");
    assert_eq!(m.started_at, local_ms(2026, 7, 3, 14, 5, 2).unwrap());
    let tracks = store.tracks(&r.meeting).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].0, TrackKind::File);
    // Three named speakers, with their spans (the end carries a 300 ms hangover).
    let speakers = store.speakers(&r.meeting).unwrap();
    let names: Vec<_> = speakers
        .iter()
        .map(|s| s.display_name.as_deref().unwrap())
        .collect();
    assert_eq!(names, ["Linh", "Minh", "Sarah"]);
    let ts = store.track_speakers(&r.meeting).unwrap();
    assert_eq!(ts.len(), 3);
    assert_eq!(ts[0].label, "Linh");
    assert_eq!(ts[0].speaker_gid, speakers[0].gid);
    assert_eq!(ts[0].spans.len(), 2);
    assert!(
        near(ts[0].spans[0][0], 1_000) && near(ts[0].spans[0][1], 6_300),
        "{:?}",
        ts[0].spans
    );
    assert!(
        near(ts[0].spans[1][0], 14_000) && near(ts[0].spans[1][1], 18_300),
        "{:?}",
        ts[0].spans
    );
    assert!(near(ts[1].spans[0][0], 8_000), "{:?}", ts[1].spans);
    assert!(
        ts[2].spans.is_empty(),
        "the silent participant has no spans"
    );
    assert_eq!(store.get_meeting(&r.meeting).unwrap().status, "processing");
}

#[test]
fn the_final_pass_attributes_words_by_the_tracks_and_never_diarizes() {
    let (_t, store) = open_store();
    let z = zoom();
    let (tx, _rx) = bus();
    let r = import_tracks(&store, &z.files, &opts(), &tx).unwrap();
    let engines = counting(script());
    let run = runner(&store, engines.clone());
    assert_eq!(run.run_pending(), 1);
    assert_eq!(
        engines.diars.load(Ordering::SeqCst),
        0,
        "the diarizer was not run"
    );
    let speakers = store.speakers(&r.meeting).unwrap();
    assert_eq!(speakers.len(), 3, "no cluster was added");
    let by_name = |n: &str| {
        speakers
            .iter()
            .find(|s| s.display_name.as_deref() == Some(n))
            .unwrap()
    };
    let segs = store.segments(&r.meeting).unwrap();
    assert_eq!(segs.len(), 3, "{segs:?}");
    assert_eq!(
        segs[0].speaker_gid.as_deref(),
        Some(by_name("Linh").gid.as_str())
    );
    assert_eq!(
        segs[1].speaker_gid.as_deref(),
        Some(by_name("Minh").gid.as_str())
    );
    // Both talk over each other in the third line: it is marked, the others are not.
    assert!(!segs[0].overlap && !segs[1].overlap);
    assert!(segs[2].overlap, "overlap is stored with the line");
    let m = store.get_meeting(&r.meeting).unwrap();
    assert_eq!(m.transcript_version, 2);

    // A retry keeps the attribution (the spans are stored, not in the job).
    store
        .enqueue_job(
            Some(&r.meeting),
            ghi_core::session::FINAL_PASS_JOB,
            ghi_core::session::JOB_PAYLOAD_VERSION,
            &serde_json::json!({}),
        )
        .unwrap();
    assert_eq!(run.run_pending(), 1);
    assert_eq!(engines.diars.load(Ordering::SeqCst), 0);
    let again = store.segments(&r.meeting).unwrap();
    assert_eq!(again.len(), 3);
    assert_eq!(
        again[0].speaker_gid.as_deref(),
        Some(by_name("Linh").gid.as_str())
    );
    assert_eq!(
        again[1].speaker_gid.as_deref(),
        Some(by_name("Minh").gid.as_str())
    );
    assert!(again[2].overlap, "and the mark survives the second pass");
}

#[test]
fn cancel_removes_the_meeting_and_frees_the_files_for_another_try() {
    let (_t, store) = open_store();
    let z = zoom();
    let (tx, _rx) = bus();
    let cancel = Arc::new(AtomicBool::new(true));
    let e = import_tracks(
        &store,
        &z.files,
        &ImportOptions {
            cancel: Some(cancel),
            ..opts()
        },
        &tx,
    )
    .unwrap_err();
    assert_eq!(e, CANCELLED);
    assert!(
        store.list_meetings(10, 0).unwrap().is_empty(),
        "no half-made meeting"
    );
    // The same files import fine afterwards.
    let r = import_tracks(&store, &z.files, &opts(), &tx).unwrap();
    assert!(!r.duplicate);
}

#[test]
fn the_same_files_in_any_order_are_a_duplicate() {
    let (_t, store) = open_store();
    let z = zoom();
    let (tx, _rx) = bus();
    let first = import_tracks(&store, &z.files, &opts(), &tx).unwrap();
    let mut reversed = z.files.clone();
    reversed.reverse();
    let second = import_tracks(&store, &reversed, &opts(), &tx).unwrap();
    assert!(second.duplicate);
    assert_eq!(second.meeting, first.meeting);
    assert_eq!(store.list_meetings(10, 0).unwrap().len(), 1);
}

#[test]
fn an_empty_or_huge_set_is_refused_with_a_code() {
    let (_t, store) = open_store();
    let (tx, _rx) = bus();
    assert_eq!(
        import_tracks(&store, &[], &opts(), &tx).unwrap_err(),
        "noTracks"
    );
    let many: Vec<_> = (0..50)
        .map(|i| (PathBuf::from(format!("/x/{i}.wav")), None))
        .collect();
    assert_eq!(
        import_tracks(&store, &many, &opts(), &tx).unwrap_err(),
        "tooManyTracks"
    );
}

#[test]
fn unnamed_participants_are_unnamed_speakers() {
    let (_t, store) = open_store();
    let z = zoom();
    let files: Vec<_> = z.files.iter().map(|(p, _)| (p.clone(), None)).collect();
    let (tx, _rx) = bus();
    let r = import_tracks(&store, &files, &opts(), &tx).unwrap();
    let speakers = store.speakers(&r.meeting).unwrap();
    assert_eq!(speakers.len(), 3);
    assert!(speakers.iter().all(|s| s.display_name.is_none()));
    assert_eq!(
        speakers.iter().map(|s| s.label_idx).collect::<Vec<_>>(),
        [0, 1, 2]
    );
}

#[test]
fn a_single_file_import_keeps_its_source_title_and_date() {
    let (tmp, store) = open_store();
    let (tx, _rx) = bus();
    // A Voice Memos export name: the date is in it, the title is not.
    let p = tmp.path().join("20260703 140502-1A2B3C4D.wav");
    participant(&p, 3.0, 300.0, &[(0.0, 3.0)]);
    let r = import_file(&store, &p, &opts(), &tx).unwrap();
    let m = store.get_meeting(&r.meeting).unwrap();
    assert_eq!(m.source_app.as_deref(), Some("voice_memos"));
    assert_eq!(m.started_at, local_ms(2026, 7, 3, 14, 5, 2).unwrap());
    assert_eq!(
        m.title, "20260703 140502-1A2B3C4D",
        "no title in the name: the file name"
    );
    // A Meet download carries both.
    let p = tmp
        .path()
        .join("Weekly sync (2026-07-03 14:05 GMT+7) - Recording.wav");
    participant(&p, 3.0, 400.0, &[(0.0, 3.0)]);
    let r = import_file(&store, &p, &opts(), &tx).unwrap();
    let m = store.get_meeting(&r.meeting).unwrap();
    assert_eq!(m.source_app.as_deref(), Some("meet"));
    assert_eq!(m.title, "Weekly sync");
    assert_eq!(m.started_at, 1_783_062_300_000);
    // The caller's own title and time win.
    let p = tmp.path().join("other.wav");
    participant(&p, 3.0, 500.0, &[(0.0, 3.0)]);
    let r = import_file(
        &store,
        &p,
        &ImportOptions {
            title: Some("Mine".into()),
            started_at: Some(5),
            ..opts()
        },
        &tx,
    )
    .unwrap();
    let m = store.get_meeting(&r.meeting).unwrap();
    assert_eq!(
        (m.title.as_str(), m.started_at, m.source_app),
        ("Mine", 5, None)
    );
}

/// A plain room recording: the diarizer says two voices overlap in the
/// middle line, and the mark is stored with it.
#[test]
fn diarized_overlap_is_stored_with_the_line() {
    let (tmp, store) = open_store();
    let (tx, _rx) = bus();
    let p = tmp.path().join("room.wav");
    participant(&p, 12.0, 330.0, &[(0.5, 11.5)]);
    let r = import_file(&store, &p, &opts(), &tx).unwrap();
    let turn = |speaker, start, end| SpeakerSegment {
        start,
        end,
        speaker,
    };
    let engines = counting(Script {
        utterances: vec![
            (1.0, 2.5, "dòng đầu tiên".into()),
            (5.0, 7.0, "hai người cùng nói".into()),
            (9.0, 10.5, "dòng cuối cùng".into()),
        ],
        turns: vec![turn(1, 0.5, 11.5), turn(2, 4.0, 8.0)],
    });
    let run = runner(&store, engines.clone());
    assert_eq!(run.run_pending(), 1);
    assert_eq!(
        engines.diars.load(Ordering::SeqCst),
        1,
        "the diarizer ran for a plain file"
    );
    let segs = store.segments(&r.meeting).unwrap();
    assert_eq!(segs.len(), 3);
    assert_eq!(
        segs.iter().map(|s| s.overlap).collect::<Vec<_>>(),
        [false, true, false]
    );
}

// ------------------------------------------------------------ review round

use ghi_core::session::RecordingHooks;
use std::sync::OnceLock;
use std::time::Duration;

/// Participants of the given talk spans, 12 s each, in a Zoom-like folder.
type Talk<'a> = (&'a str, f32, &'a [(f32, f32)]);

fn crowd(who: &[Talk]) -> Zoom {
    let dir = tempfile::tempdir().unwrap();
    let rec = dir
        .path()
        .join("2026-07-03 14.05.02 Sprint 81234567890/Audio Record");
    std::fs::create_dir_all(&rec).unwrap();
    let files = who
        .iter()
        .enumerate()
        .map(|(i, (name, hz, talk))| {
            let p = rec.join(format!("audio{name}{:04}.wav", 1111 * (i + 1)));
            participant(&p, 12.0, *hz, talk);
            (p, Some(name.to_string()))
        })
        .collect();
    Zoom { _dir: dir, files }
}

fn lines_of(store: &Store, meeting: &str) -> Vec<(i64, i64, String, bool)> {
    let names: std::collections::HashMap<String, String> = store
        .speakers(meeting)
        .unwrap()
        .into_iter()
        .map(|s| (s.gid, s.display_name.unwrap_or_default()))
        .collect();
    store
        .segments(meeting)
        .unwrap()
        .into_iter()
        .map(|s| {
            (
                s.t0_ms,
                s.t1_ms,
                s.speaker_gid
                    .and_then(|g| names.get(&g).cloned())
                    .unwrap_or_default(),
                s.overlap,
            )
        })
        .collect()
}

/// Taking turns is not talking over each other: only a real, lasting overlap
/// is marked; and a line no track covers goes to the nearest participant.
#[test]
fn only_real_overlap_is_marked_and_stray_lines_go_to_the_nearest_speaker() {
    let (_t, store) = open_store();
    // Linh 1-6 s, Minh takes over at 6.1 s (a quick turn-over) until 9 s, and
    // both talk at 10-11.5 s.
    let z = crowd(&[
        ("Linh", 300.0, &[(1.0, 6.0), (10.0, 11.5)]),
        ("Minh", 700.0, &[(6.1, 9.0), (10.0, 11.5)]),
    ]);
    let (tx, _rx) = bus();
    let r = import_tracks(&store, &z.files, &opts(), &tx).unwrap();
    let engines = counting(Script {
        // 4.5-5.9 Linh; 6.2-7.5 Minh right after (turn-over); 10.2-11.2 both;
        // 0.2-0.6 s is before anyone spoke.
        utterances: vec![
            (0.2, 0.6, "mở đầu".into()),
            (4.5, 5.9, "linh nói xong".into()),
            (6.2, 7.5, "minh nói tiếp".into()),
            (10.2, 11.2, "cả hai cùng nói".into()),
        ],
        turns: vec![],
    });
    let run = runner(&store, engines);
    assert_eq!(run.run_pending(), 1);
    let lines = lines_of(&store, &r.meeting);
    assert_eq!(lines.len(), 4, "{lines:?}");
    let flags: Vec<bool> = lines.iter().map(|l| l.3).collect();
    assert_eq!(flags, [false, false, false, true], "{lines:?}");
    assert_eq!(lines[0].2, "Linh", "before anyone spoke: the nearest voice");
    assert_eq!(lines[1].2, "Linh");
    assert_eq!(lines[2].2, "Minh");
}

/// A speaker merged into another, down a chain, answers for the last one.
#[test]
fn merged_track_speakers_follow_the_chain() {
    let (_t, store) = open_store();
    let z = crowd(&[
        ("Linh", 300.0, &[(1.0, 4.0)]),
        ("Minh", 700.0, &[(5.0, 8.0)]),
        ("Sarah", 1100.0, &[(9.0, 11.5)]),
    ]);
    let (tx, _rx) = bus();
    let r = import_tracks(&store, &z.files, &opts(), &tx).unwrap();
    let sp = store.speakers(&r.meeting).unwrap();
    // Linh into Minh, then Minh into Sarah.
    store.merge_speakers(&sp[0].gid, &sp[1].gid).unwrap();
    store.merge_speakers(&sp[1].gid, &sp[2].gid).unwrap();
    let engines = counting(Script {
        utterances: vec![
            (1.5, 3.5, "một".into()),
            (5.5, 7.5, "hai".into()),
            (9.5, 11.0, "ba".into()),
        ],
        turns: vec![],
    });
    let run = runner(&store, engines);
    assert_eq!(run.run_pending(), 1);
    let segs = store.segments(&r.meeting).unwrap();
    assert_eq!(segs.len(), 3);
    for s in &segs {
        assert_eq!(
            s.speaker_gid.as_deref(),
            Some(sp[2].gid.as_str()),
            "all three are Sarah now"
        );
    }
}

/// A recording that starts during the final pass stops it; it resumes later
/// with the same result and no extra speakers.
#[test]
fn a_preempted_multi_track_final_pass_resumes() {
    let (_t, store) = open_store();
    let z = zoom();
    let (tx, _rx) = bus();
    let r = import_tracks(&store, &z.files, &opts(), &tx).unwrap();
    let engines = counting(script());
    let slot: Arc<OnceLock<Arc<JobRunner>>> = Arc::default();
    let first = Arc::new(AtomicBool::new(true));
    let run = {
        let (engines, slot, first) = (engines.clone(), slot.clone(), first.clone());
        let (tx, _rx) = bus();
        JobRunner::new(
            store.clone(),
            tx,
            vec![Arc::new(FinalPassJob {
                engines: Arc::new(move || {
                    if first.swap(false, Ordering::SeqCst) {
                        slot.get().unwrap().recording_started();
                    }
                    Ok(engines.clone() as Arc<dyn SpeechEngines>)
                }),
                chunk_s: 600.0,
                ready: always_ready(),
                voice: None,
            })],
        )
    };
    slot.set(run.clone()).ok();
    assert_eq!(run.run_pending(), 1, "it yields");
    let m = store.get_meeting(&r.meeting).unwrap();
    assert_eq!(m.transcript_version, 1, "nothing was stored");
    assert_eq!(store.speakers(&r.meeting).unwrap().len(), 3);
    assert_eq!(
        store.track_speakers(&r.meeting).unwrap().len(),
        3,
        "the spans are still there"
    );
    // The recording ends: it finishes.
    run.recording_stopped();
    assert_eq!(run.run_pending(), 1);
    assert_eq!(store.get_meeting(&r.meeting).unwrap().transcript_version, 2);
    assert_eq!(store.segments(&r.meeting).unwrap().len(), 3);
    assert_eq!(store.speakers(&r.meeting).unwrap().len(), 3);
    assert_eq!(engines.diars.load(Ordering::SeqCst), 0);
}

/// Decoding waits while `hold` says a recording runs, for a file and for a set
/// of tracks, and goes on when it ends.
#[test]
fn imports_wait_for_a_recording() {
    let (tmp, store) = open_store();
    let held = Arc::new(AtomicBool::new(true));
    let hold = |held: &Arc<AtomicBool>| {
        let held = held.clone();
        Some(ghi_core::import::Hold(Arc::new(move || {
            held.load(Ordering::SeqCst)
        })))
    };
    let z = zoom();
    let (tx, _rx) = bus();
    let single = tmp.path().join("single.wav");
    participant(&single, 3.0, 330.0, &[(0.0, 3.0)]);
    type Job =
        Box<dyn FnOnce(ImportOptions) -> Result<ghi_core::import::ImportReport, String> + Send>;
    let jobs: Vec<Job> = vec![
        {
            let (store, files, tx) = (store.clone(), z.files.clone(), tx.clone());
            Box::new(move |o| import_tracks(&store, &files, &o, &tx))
        },
        {
            let (store, tx) = (store.clone(), tx.clone());
            Box::new(move |o| import_file(&store, &single, &o, &tx))
        },
    ];
    for job in jobs {
        let before = store.list_meetings(10, 0).unwrap().len();
        let o = ImportOptions {
            hold: hold(&held),
            ..opts()
        };
        let h = std::thread::spawn(move || job(o));
        std::thread::sleep(Duration::from_millis(500));
        assert!(!h.is_finished(), "waiting for the recording");
        let ms = store.list_meetings(10, 0).unwrap();
        assert!(
            ms.len() == before + 1 && ms[0].status == "importing",
            "started, not decoded: {ms:?}"
        );
        assert_eq!(ms[0].duration_ms, 0, "nothing decoded yet");
        held.store(false, Ordering::SeqCst);
        let r = h.join().unwrap().unwrap();
        assert!(store.get_meeting(&r.meeting).unwrap().duration_ms > 0);
        held.store(true, Ordering::SeqCst);
    }
    // Cancelling while held ends the wait.
    let cancel = Arc::new(AtomicBool::new(false));
    let o = ImportOptions {
        hold: hold(&held),
        cancel: Some(cancel.clone()),
        ..opts()
    };
    let (s2, files2, tx2) = (
        store.clone(),
        crowd(&[("A", 300.0, &[(1.0, 2.0)]), ("B", 400.0, &[(3.0, 4.0)])]),
        tx.clone(),
    );
    let h = std::thread::spawn(move || import_tracks(&s2, &files2.files, &o, &tx2));
    std::thread::sleep(Duration::from_millis(300));
    cancel.store(true, Ordering::SeqCst);
    assert_eq!(h.join().unwrap().unwrap_err(), CANCELLED);
}

/// Everything refusable is refused up front, with a code and no path, and
/// leaves nothing behind.
#[test]
fn bad_track_sets_fail_early_with_codes_and_no_meeting() {
    let (tmp, store) = open_store();
    let (tx, _rx) = bus();
    let z = zoom();
    let mut missing = z.files.clone();
    missing.push((tmp.path().join("gone.wav"), None));
    assert_eq!(
        import_tracks(&store, &missing, &opts(), &tx).unwrap_err(),
        "trackMissing"
    );
    let junk = tmp.path().join("audioJunk12345678.wav");
    std::fs::write(&junk, "not audio at all").unwrap();
    let mut unreadable = z.files.clone();
    unreadable.push((junk, None));
    assert_eq!(
        import_tracks(&store, &unreadable, &opts(), &tx).unwrap_err(),
        "trackUnreadable"
    );
    assert!(store.list_meetings(10, 0).unwrap().is_empty());
}

/// 49 tracks import; 50 are refused before anything is read.
#[test]
fn forty_nine_tracks_import_and_fifty_do_not() {
    let (tmp, store) = open_store();
    let (tx, _rx) = bus();
    let files: Vec<_> = (0..50)
        .map(|i| {
            let p = tmp.path().join(format!("t{i}.wav"));
            participant(&p, 1.0, 200.0 + 7.0 * i as f32, &[(0.2, 0.8)]);
            (p, Some(format!("P{i}")))
        })
        .collect();
    assert_eq!(
        import_tracks(&store, &files, &opts(), &tx).unwrap_err(),
        "tooManyTracks"
    );
    assert!(store.list_meetings(10, 0).unwrap().is_empty());
    let r = import_tracks(&store, &files[..49], &opts(), &tx).unwrap();
    assert_eq!(r.tracks, 49);
    assert_eq!(store.speakers(&r.meeting).unwrap().len(), 49);
    // Beyond the eight palette colors the slot is 0 (never color alone).
    let slots: Vec<i64> = store
        .speakers(&r.meeting)
        .unwrap()
        .iter()
        .map(|s| s.color_slot)
        .collect();
    assert!(slots[8..].iter().all(|s| *s == 0) && slots[..8].iter().all(|s| *s != 0));
}

/// Spans are stored compactly: a long, busy 49-track recording fits, comes
/// back exactly, and the size bound refuses what could not.
#[test]
fn many_spans_round_trip_and_the_bound_is_early() {
    use ghi_store::organize::{MAX_TRACK_SPEAKERS_BYTES, TrackSpeaker, track_speakers_bound};
    let (_t, store) = open_store();
    let m = store
        .create_meeting(ghi_store::store::NewMeeting {
            title: "busy".into(),
            ..Default::default()
        })
        .unwrap()
        .gid;
    let speakers: Vec<TrackSpeaker> = (0..49)
        .map(|i| {
            let gid = store
                .add_speaker(
                    &m,
                    ghi_store::store::NewSpeaker {
                        label_idx: i,
                        ..Default::default()
                    },
                )
                .unwrap();
            TrackSpeaker {
                label: format!("Participant {i}"),
                speaker_gid: gid,
                // 30,000 spans each, 0.65 s apart: the densest the detector can make.
                spans: (0..30_000i64)
                    .map(|k| [k * 650 + i, k * 650 + i + 250])
                    .collect(),
            }
        })
        .collect();
    store.set_track_speakers(&m, &speakers).unwrap();
    assert_eq!(store.track_speakers(&m).unwrap(), speakers);
    // Out of order spans are refused; an empty list clears.
    let mut bad = speakers[..1].to_vec();
    bad[0].spans = vec![[10, 20], [15, 30]];
    assert!(store.set_track_speakers(&m, &bad).is_err());
    store.set_track_speakers(&m, &[]).unwrap();
    assert!(store.track_speakers(&m).unwrap().is_empty());
    // The importer's bound: 8 h of 49 tracks fits, a day does not.
    let hours = |h: i64| track_speakers_bound(49, h * 3_600_000, 650);
    assert!(hours(8) < MAX_TRACK_SPEAKERS_BYTES);
    assert!(hours(24) > MAX_TRACK_SPEAKERS_BYTES);
}

/// A crash between marking an import `processing` and queueing its final pass
/// is mended at the next start; a half-decoded import is dropped.
#[test]
fn recovery_requeues_an_import_without_a_job() {
    let (_t, store) = open_store();
    let z = zoom();
    let (tx, _rx) = bus();
    let r = import_tracks(&store, &z.files, &opts(), &tx).unwrap();
    // The crash: the job never got queued.
    for j in store.active_jobs().unwrap() {
        store.cancel_job(j.id).unwrap();
    }
    assert!(
        store
            .active_job(&r.meeting, ghi_core::session::FINAL_PASS_JOB)
            .unwrap()
            .is_none()
    );
    let out = ghi_core::recover::recover(&store).unwrap();
    assert_eq!(out.jobs_requeued, 1);
    assert!(
        store
            .active_job(&r.meeting, ghi_core::session::FINAL_PASS_JOB)
            .unwrap()
            .is_some()
    );
    // Run again: nothing more to queue.
    assert_eq!(ghi_core::recover::recover(&store).unwrap().jobs_requeued, 0);
}

/// Mixing many loud tracks neither wraps nor hard-clips.
#[test]
fn a_loud_mix_stays_in_range() {
    let (_t, store) = open_store();
    let tmp = tempfile::tempdir().unwrap();
    let files: Vec<_> = (0..6)
        .map(|i| {
            let p = tmp.path().join(format!("l{i}.wav"));
            participant(&p, 2.0, 300.0 + 11.0 * i as f32, &[(0.0, 2.0)]);
            (p, None)
        })
        .collect();
    let (tx, _rx) = bus();
    let r = import_tracks(&store, &files, &opts(), &tx).unwrap();
    let ogg = store
        .open_bundle(&r.meeting, TrackKind::File)
        .unwrap()
        .read_all()
        .unwrap();
    let pcm = ghi_audio::encoder::read_ogg_opus(&ogg[..]).unwrap();
    let peak = pcm.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    // (The codec rings a little past full scale; a wrapped sum would be far off.)
    assert!(peak > 0.5 && peak < 1.3, "peak {peak}");
}
