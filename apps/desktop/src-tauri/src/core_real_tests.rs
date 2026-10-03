// SPDX-License-Identifier: Apache-2.0
//! The real-model harness: a `Core` over a temp data directory with the real
//! speech, notes, embedding and speaker models, driven the way the app drives
//! it (import a WAV, let the job runner work, ask across meetings, list
//! people). Every UI test runs on the mock; this is the one that exercises
//! the engines together, so it is `#[ignore]`d and skips cleanly without the
//! models or the eval audio.
//!
//! ```sh
//! tools/scripts/build-nemo.sh && tools/scripts/fetch-models.sh   # once
//! cargo build -p ghi-llm-worker
//! cargo test -p ghi-desktop --features nemo -- --ignored real_models --nocapture
//! ```
//! The models come from `$GHI_MODELS_DIR` or the repo's `models/`; the audio
//! from `tools/eval/data` (never in git). The key store is the debug file
//! store, so a debug build is required. Unix only (the models are symlinked in).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ghi_core::events::{Event, EventRx};

use ghi_app::core::{
    Core, embed_ready, llm_ready, preset, speech_ready, voice_factory, voice_ready,
};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// Peak resident memory of this process and its finished children (the notes
/// worker), in MB.
fn peak_rss_mb() -> f64 {
    let max = |who| {
        let mut u = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        // SAFETY: getrusage fills the struct we pass.
        if unsafe { libc::getrusage(who, u.as_mut_ptr()) } != 0 {
            return 0.0;
        }
        // SAFETY: initialized by the successful call above.
        let raw = unsafe { u.assume_init() }.ru_maxrss as f64;
        // Bytes on macOS, KiB on Linux.
        raw / if cfg!(target_os = "macos") { 1e6 } else { 1e3 }
    };
    max(libc::RUSAGE_SELF).max(max(libc::RUSAGE_CHILDREN))
}

/// Generous ceilings (MB) for this tier: the measured numbers are printed, and
/// a regression of a few GB trips these before the OS does.
fn rss_budget_mb(tier: ghi_models::Tier) -> f64 {
    match tier {
        ghi_models::Tier::Light => 7_000.0,
        ghi_models::Tier::Balanced => 12_000.0,
        ghi_models::Tier::Max => 10_000.0,
    }
}

/// Every event until `done` says stop, with the time each arrived.
fn drain_until(
    rx: &EventRx,
    seen: &mut Vec<(Duration, Event)>,
    began: Instant,
    limit: Duration,
    done: impl Fn(&Event) -> bool,
) {
    while began.elapsed() < limit {
        if let Ok(env) = rx.recv_timeout(Duration::from_millis(250)) {
            let stop = done(&env.event);
            seen.push((began.elapsed(), env.event));
            if stop {
                return;
            }
        }
    }
    panic!("timed out after {limit:?}; events so far: {}", seen.len());
}

fn first(seen: &[(Duration, Event)], kind: &str) -> Option<usize> {
    seen.iter()
        .position(|(_, e)| matches!(e, Event::JobProgress { kind: k, .. } if k == kind))
}

#[test]
#[ignore = "real models (several minutes); see the module docs"]
fn real_models_import_notes_embed_voice_ask_people() {
    if !cfg!(debug_assertions) {
        eprintln!("skipped: needs the debug file key store (a debug build)");
        return;
    }
    let models_src = std::env::var_os("GHI_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("models"));
    let wav = repo().join("tools/eval/data/voxconverse/audio/msbyq.wav");
    if !wav.is_file() || !models_src.is_dir() {
        eprintln!("skipped: no eval audio or models directory");
        return;
    }

    // The app looks for models in <data>/models: link the real ones in.
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::os::unix::fs::symlink(&models_src, data.join("models")).unwrap();
    let models = data.join("models");
    let p = preset();
    let have = speech_ready(&models) && llm_ready(&models) && voice_ready(&models);
    if !have {
        eprintln!(
            "skipped: models missing for the {} tier (speech/notes/voice); run fetch-models.sh",
            p.tier.as_str()
        );
        return;
    }
    let embedding = p.embed_id.is_some();
    if embedding && !embed_ready(&models) {
        eprintln!("skipped: the embedding model is missing");
        return;
    }
    eprintln!("tier={} embedding={embedding}", p.tier.as_str());

    let (core, rx) = Core::for_test(data);
    let began = Instant::now();
    let store = core.store().expect("the store opens");

    // The app lock: a locked core hands out no store (and nothing else that
    // returns content), then works again once unlocked.
    core.set_locked(true);
    assert!(core.store().is_err(), "locked: the store is refused");
    assert!(
        core.store_even_locked().is_ok(),
        "recording/imports still work"
    );
    core.set_locked(false);
    assert!(core.store().is_ok());

    // Me's voice, from the first 25 s of the same file (consent given).
    let pcm: Vec<f32> = {
        let mut r = hound::WavReader::open(&wav).unwrap();
        assert_eq!((r.spec().sample_rate, r.spec().channels), (16_000, 1));
        r.samples::<i16>()
            .take(16_000 * 25)
            .map(|s| f32::from(s.unwrap()) / 32_768.0)
            .collect()
    };
    let mut embedder = (voice_factory(&models))().expect("the speaker model opens");
    let t = Instant::now();
    ghi_core::voice_job::enroll_from_pcm(
        &store,
        embedder.as_mut(),
        &pcm,
        &ghi_core::voice_job::self_consent("onboarding.voice.consent_mac"),
    )
    .expect("enrollment");
    drop(embedder);
    eprintln!("enroll Me (25 s): {:?}", t.elapsed());

    // Import: the job runner takes it from here, in the background.
    let report = core
        .import(&wav, ghi_core::import::ImportOptions::default())
        .expect("import");
    let meeting = report.meeting.clone();
    let mut seen: Vec<(Duration, Event)> = Vec::new();

    // Locked mid-flow: still refused, and the jobs keep going.
    core.set_locked(true);
    assert!(core.store().is_err(), "locked mid-flow: refused");
    core.set_locked(false);

    let limit = Duration::from_secs(20 * 60);
    drain_until(&rx, &mut seen, began, limit, |e| {
        matches!(e, Event::NotesReady { version: 2, .. })
    });
    let notes_at = seen.last().unwrap().0;
    // Semantic indexing runs last; wait until the queue is empty.
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(10 * 60) {
        let busy = store.jobs_for_meeting(&meeting).unwrap().iter().any(|j| {
            matches!(
                j.state,
                ghi_store::jobs::JobState::Queued | ghi_store::jobs::JobState::Running
            )
        });
        if !busy {
            break;
        }
        while let Ok(env) = rx.try_recv() {
            seen.push((began.elapsed(), env.event));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    while let Ok(env) = rx.try_recv() {
        seen.push((began.elapsed(), env.event));
    }
    let total = began.elapsed();

    // The event sequence: final pass, then notes, then ready.
    let fp = first(&seen, ghi_core::session::FINAL_PASS_JOB).expect("final pass progress");
    let nf = first(&seen, ghi_core::notes_job::NOTES_FINAL_JOB).expect("notes progress");
    let ready = seen
        .iter()
        .position(|(_, e)| matches!(e, Event::NotesReady { version: 2, .. }))
        .expect("notes ready");
    assert!(
        fp < nf && nf < ready,
        "final pass {fp} < notes {nf} < ready {ready}"
    );
    let errors: Vec<&Event> = seen
        .iter()
        .map(|(_, e)| e)
        .filter(|e| matches!(e, Event::Error { .. }))
        .collect();
    assert!(errors.is_empty(), "no error events: {errors:?}");

    // Every job finished well.
    let jobs = store.jobs_for_meeting(&meeting).unwrap();
    for j in &jobs {
        assert_eq!(
            j.state,
            ghi_store::jobs::JobState::Done,
            "{} {:?}",
            j.kind,
            j.state
        );
    }
    let kinds: Vec<&str> = jobs.iter().map(|j| j.kind.as_str()).collect();
    assert!(
        kinds.contains(&"final_pass") && kinds.contains(&"notes_final"),
        "{kinds:?}"
    );

    // The meeting: transcript v2, notes, speakers, embeddings.
    let m = store.get_meeting(&meeting).unwrap();
    assert_eq!((m.status.as_str(), m.transcript_version), ("ready", 2));
    let segs = store.segments(&meeting).unwrap();
    assert!(segs.len() > 3, "a transcript: {} lines", segs.len());
    assert!(
        !store.note_blocks(&meeting).unwrap().is_empty(),
        "notes exist"
    );
    let speakers = store.speakers(&meeting).unwrap();
    assert!(!speakers.is_empty());
    if embedding {
        assert!(kinds.contains(&"embed_index"), "{kinds:?}");
        let chunks = store
            .embeddings(&meeting, ghi_core::index_job::MODEL_ID)
            .unwrap();
        assert!(!chunks.is_empty(), "the meeting is indexed");
    }

    // People: consistent with the meeting, and Me has her voice.
    let people = store.people_overview().unwrap();
    let me = people.iter().find(|p| p.is_me).expect("Me");
    assert!(me.voice.is_some(), "Me's profile is listed");
    for person in &people {
        let listed = store.person_meetings(&person.gid, 50).unwrap();
        assert_eq!(
            listed.len() as i64,
            person.meetings.min(50),
            "{}",
            person.gid
        );
        assert!(person.open_actions >= 0);
        if !person.is_me {
            assert!(person.voice.is_none(), "the flag is off: no other voices");
        }
    }
    assert_eq!(store.raw_voice_counts().unwrap().other_profiles, 0);
    assert_eq!(store.raw_voice_counts().unwrap().speaker_voices, 0);
    let suggested = speakers.iter().filter(|s| s.suggestion.is_some()).count();
    eprintln!(
        "speakers={} (suggestions {suggested}), me exemplars={}",
        speakers.len(),
        store
            .me_voice_profile(ghi_core::profiles::VOICE_MODEL)
            .unwrap()
            .map_or(0, |p| p
                .sets
                .iter()
                .map(|s| s.exemplars.len())
                .sum::<usize>())
    );

    // Ask across meetings, once, as the command does.
    let t = Instant::now();
    let question = "What was discussed in the meeting?";
    let (passages, semantic) = core
        .with_query_embedder(&store, |e| {
            let semantic = e.is_some();
            ghi_core::ask_all::retrieve(
                &store,
                e,
                question,
                &ghi_core::ask_all::Scope::default(),
                ghi_core::ask_all::ASK_PASSAGES,
            )
            .map(|p| (p, semantic))
        })
        .expect("retrieval");
    assert_eq!(semantic, embedding, "meaning search follows the tier");
    assert!(!passages.is_empty(), "passages from the imported meeting");
    assert!(passages.iter().all(|p| p.meeting_gid == meeting));
    let bytes = 2 * ghi_core::ask_all::ASK_PASSAGES * ghi_core::index_job::MAX_CHARS;
    let mut llm = (core.llm().unwrap())(bytes).expect("notes model opens");
    let answer = ghi_core::ask_all::ask_all(
        &store,
        llm.as_mut(),
        &passages,
        question,
        ghi_llm::template::OutLang::En,
    )
    .expect("ask");
    drop(llm);
    match &answer.answer {
        ghi_llm::ask::Answer::Answered { text, .. } => {
            assert!(!text.trim().is_empty());
            assert!(!answer.citations.is_empty(), "an answer cites its lines");
        }
        ghi_llm::ask::Answer::NotDiscussed { .. } => {}
    }
    eprintln!("ask: {:?}", t.elapsed());

    // Quit: the worker goes, then the memory peak is read.
    core.shutdown(Duration::from_secs(5));
    let rss = peak_rss_mb();
    let budget = rss_budget_mb(p.tier);
    eprintln!(
        "final pass starts at {:?}, notes ready at {notes_at:?}, all jobs done at {total:?}",
        seen[fp].0
    );
    eprintln!(
        "peak RSS {rss:.0} MB (budget {budget:.0} MB, tier {})",
        p.tier.as_str()
    );
    assert!(
        rss < budget,
        "peak RSS {rss:.0} MB over the {budget:.0} MB budget"
    );
}
