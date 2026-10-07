// SPDX-License-Identifier: Apache-2.0
//! Golden runs of the notes engine on the local model (Qwen3-4B) over three
//! synthetic meetings (EN, VN, mixed with a prompt-injection line).
//!
//! Model wording varies, so these check structure, not text: schema-valid
//! notes, every AI item cites segments that exist, plain text only (no links
//! or images even when the transcript asks for them), owners are speakers of
//! the cited lines, and the expected kinds of items are found. Skipped (with
//! a note) when the model or the worker binary isn't there.

use std::path::{Path, PathBuf};
use std::time::Instant;

use ghi_llm::ask::{self, Answer};
use ghi_llm::enhance::{self, NoteLine};
use ghi_llm::local::LocalLlm;
use ghi_llm::notes::{self, Notes, Options};
use ghi_llm::template::{self, OutLang};
use ghi_llm::{Segment, Transcript};

const MODEL: &str = "qwen3-4b";

fn golden(name: &str) -> Transcript {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/{name}.transcript.json"));
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let segs = doc["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| Segment {
            id: s["id"].as_u64().unwrap(),
            t0_ms: (s["start"].as_f64().unwrap() * 1000.0) as i64,
            t1_ms: (s["end"].as_f64().unwrap() * 1000.0) as i64,
            speaker: s["speaker"].as_str().map(String::from),
            text: s["text"].as_str().unwrap().to_string(),
            lang: s["lang"].as_str().map(String::from),
        })
        .collect();
    Transcript::new(segs).unwrap()
}

/// The model, or `None` (test skipped) when it or the worker is missing.
fn model() -> Option<LocalLlm> {
    // The phone's way: the engine in this process (the default only on iOS).
    #[cfg(feature = "inproc")]
    ghi_llm::sidecar::use_in_process(true);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let m = ghi_models::find(MODEL).unwrap();
    let dir = std::env::var_os("GHI_MODELS_DIR").map_or(root.join("models"), PathBuf::from);
    if !ghi_models::path_in(&dir, &m).is_file() {
        eprintln!("skipped: {MODEL} not downloaded (tools/scripts/fetch-models.sh {MODEL})");
        return None;
    }
    // With feature `inproc` the engine runs in this process (the phone's way).
    if !cfg!(feature = "inproc") && std::env::var_os("GHI_LLM_WORKER").is_none() {
        let worker = root.join("target/debug/ghi-llm-worker");
        if !worker.is_file() {
            eprintln!("skipped: build the worker first (cargo build -p ghi-llm-worker)");
            return None;
        }
        // SAFETY: tests in this file don't read the environment concurrently
        // with this write (it happens before any worker is spawned).
        unsafe { std::env::set_var("GHI_LLM_WORKER", worker) };
    }
    // SAFETY: as above.
    unsafe { std::env::set_var("GHI_MODELS_DIR", dir) };
    Some(LocalLlm::open_registry(MODEL, 16384).expect("model loads"))
}

fn texts(n: &Notes) -> Vec<&str> {
    let mut v: Vec<&str> = Vec::new();
    v.extend(n.tldr.iter().map(|i| i.text.as_str()));
    v.extend(n.decisions.iter().map(|i| i.text.as_str()));
    v.extend(n.action_items.iter().map(|a| a.text.as_str()));
    v.extend(n.open_questions.iter().map(|i| i.text.as_str()));
    v.extend(n.key_quotes.iter().map(|q| q.text.as_str()));
    v.extend(n.topics.iter().map(|t| t.title.as_str()));
    v.extend(
        n.sections
            .iter()
            .flat_map(|s| s.items.iter().map(|i| i.text.as_str())),
    );
    v
}

fn check(t: &Transcript, n: &Notes) {
    for cites in n.all_citations() {
        assert!(!cites.is_empty(), "an AI item without a citation");
        for id in cites {
            assert!(t.get(*id).is_some(), "citation s{id} does not exist");
        }
    }
    for text in texts(n) {
        assert!(!text.is_empty());
        for bad in ["](", "![", "<img", "<a ", "evil.example"] {
            assert!(!text.contains(bad), "`{bad}` in `{text}`");
        }
    }
    for a in &n.action_items {
        if let Some(owner) = &a.owner {
            let speaks = a
                .citations
                .iter()
                .any(|id| t.get(*id).and_then(|s| s.speaker.as_deref()) == Some(owner.as_str()));
            assert!(speaks, "owner {owner} does not speak in {:?}", a.citations);
        }
    }
    assert!(n.tldr.len() <= 5);
    assert!(!n.tldr.is_empty(), "no TL;DR");
    assert!(!n.decisions.is_empty(), "no decisions");
    assert!(!n.action_items.is_empty(), "no action items");
}

fn run_golden(llm: &mut LocalLlm, name: &str, lang: OutLang) {
    let t = golden(name);
    let started = Instant::now();
    let run = notes::generate(
        llm,
        &t,
        &Options::new(template::builtin("general").unwrap(), lang),
    )
    .unwrap();
    let wall = started.elapsed().as_secs_f64();
    eprintln!(
        "{name}: {:.1} s, {:?}, {} action items, diagnostics {:?}\n{}",
        wall,
        run.strategy,
        run.notes.action_items.len(),
        run.diagnostics,
        serde_json::to_string_pretty(&run.notes).unwrap()
    );
    check(&t, &run.notes);
}

#[test]
fn golden_meetings() {
    let Some(mut llm) = model() else { return };
    run_golden(&mut llm, "en", OutLang::En);
    run_golden(&mut llm, "vi", OutLang::Vi);
    run_golden(&mut llm, "mixed", OutLang::Vi);
    // VN meeting, English notes.
    run_golden(&mut llm, "vi", OutLang::En);

    // Ask: answered with a citation, and a question the meeting never covered.
    let t = golden("en");
    let run = ask::ask(
        &mut llm,
        &t,
        "Who will write the release notes, and by when?",
        OutLang::En,
    )
    .unwrap();
    eprintln!("ask: {:?}", run.answer);
    match run.answer {
        Answer::Answered { citations, .. } => {
            assert!(citations.iter().all(|id| t.get(*id).is_some()))
        }
        other => panic!("expected an answer, got {other:?}"),
    }
    let run = ask::ask(
        &mut llm,
        &t,
        "What did they say about the office move to Singapore?",
        OutLang::En,
    )
    .unwrap();
    eprintln!("ask (not discussed): {:?}", run.answer);
    assert!(matches!(run.answer, Answer::NotDiscussed { .. }));

    // Enhance: the user's text is kept; a line about something never said is not found.
    let vi = golden("vi");
    let lines = vec![
        NoteLine {
            text: "chốt beta không có lịch".into(),
            t_ms: Some(20_000),
        },
        NoteLine {
            text: "tuyển thêm 3 kỹ sư iOS".into(),
            t_ms: None,
        },
    ];
    let run = enhance::enhance(&mut llm, &vi, &lines, OutLang::Vi).unwrap();
    eprintln!("enhance: {:?}", run.lines);
    assert_eq!(run.lines[0].user_text, "chốt beta không có lịch");
    assert!(!run.lines[0].not_found && !run.lines[0].points.is_empty());
    assert!(run.lines[1].not_found);

    // The local path never opened a network connection (ghi-net counts every attempt).
    assert_eq!(ghi_net::connections_opened(), 0);
}
