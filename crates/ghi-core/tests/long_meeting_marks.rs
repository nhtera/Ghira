// SPDX-License-Identifier: Apache-2.0
//! Marks on a long real meeting through the real notes path (`NotesJob`, the
//! local model, the store): `--ignored`, minutes. `GHI_LONG_TRANSCRIPT` is a
//! `ghi.transcript/1` file (e.g. `ghi transcribe` of a 45+ minute recording
//! from tools/eval/data; recordings and transcripts never enter git).
//!
//! Notes are written twice, without marks and with 10, and each run reports:
//! time, blocks per kind, action items, and two coverage numbers:
//! - "cited": the marked line is cited by some AI sentence or action (what
//!   `ghi-llm`'s `long_meeting` test counts);
//! - "UI": `ghi_core::marks::coverage` over the AI sentence blocks and actions,
//!   as `meeting_notes` computes it (time overlap of anchors with the line).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use ghi_core::events::bus;
use ghi_core::jobs::{JobRunner, Outcome, always_ready};
use ghi_core::marks::{coverage, mark_lines};
use ghi_core::notes_job::{NOTES_FINAL_JOB, NotesJob};
use ghi_core::session::JOB_PAYLOAD_VERSION;
use ghi_llm::Llm;
use ghi_llm::local::LocalLlm;
use ghi_store::anchors::Anchor;
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::{MarkTag, NewMeeting, NewSegment, Provenance, Store};

const MODEL: &str = "qwen3-4b";
/// The app's context sizing (`ghi-app` `core::llm_factory`).
const LLM_MAX_CTX: u32 = 32_768;

/// Kinds of block that cover a mark in the apps (`ghi-app` `detail::covers_marks`:
/// what the Notes tab draws as a sentence; topics and the user's own do not).
fn covers_marks(kind: &str) -> bool {
    matches!(
        kind,
        "tldr" | "decision" | "proposal" | "question" | "quote" | "answer"
    ) || kind.starts_with("section:")
        || kind.starts_with("enhanced:")
}

/// The models directory and the worker, or `None` (skipped) without them.
fn setup() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::var_os("GHI_MODELS_DIR").map_or(root.join("models"), PathBuf::from);
    // A missing model file shows up as a failed job in the report.
    if !dir.is_dir() {
        eprintln!("skipped: no models directory (tools/scripts/fetch-models.sh {MODEL})");
        return None;
    }
    if std::env::var_os("GHI_LLM_WORKER").is_none() {
        let worker = root.join("target/debug/ghi-llm-worker");
        if !worker.is_file() {
            eprintln!("skipped: build the worker first (cargo build -p ghi-llm-worker)");
            return None;
        }
        // SAFETY: set before any worker starts, in the only test of this file.
        unsafe { std::env::set_var("GHI_LLM_WORKER", worker) };
    }
    Some(dir)
}

struct Run {
    label: &'static str,
    seconds: f64,
    summary: String,
}

#[test]
#[ignore = "measurement: minutes on the local model"]
fn marks_on_a_long_meeting_through_the_notes_job() {
    let Some(path) = std::env::var_os("GHI_LONG_TRANSCRIPT") else {
        eprintln!("skipped: set GHI_LONG_TRANSCRIPT to a ghi.transcript/1 file");
        return;
    };
    let Some(models) = setup() else { return };
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let lines: Vec<NewSegment> = doc["segments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| NewSegment {
            t0_ms: (s["start"].as_f64().unwrap() * 1000.0) as i64,
            t1_ms: (s["end"].as_f64().unwrap() * 1000.0) as i64,
            text: s["text"].as_str().unwrap().to_string(),
            lang: Some(doc["lang"].as_str().unwrap_or("en").to_string()),
            ..Default::default()
        })
        .collect();
    assert!(lines.len() > 20, "a long meeting, please");

    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    // Ten marks spread over the meeting, on lines with some words in them.
    let tags = [
        MarkTag::Decision,
        MarkTag::Action,
        MarkTag::Question,
        MarkTag::Star,
        MarkTag::Action,
    ];
    let marked: Vec<(i64, MarkTag)> = (0..10)
        .map(|k| {
            let mut i = (k * lines.len() / 10 + lines.len() / 20).min(lines.len() - 1);
            while i + 1 < lines.len() && lines[i].text.split_whitespace().count() < 6 {
                i += 1;
            }
            (lines[i].t0_ms, tags[k % tags.len()])
        })
        .collect();

    let mut runs = Vec::new();
    for (label, with_marks) in [("no marks", false), ("10 marks", true)] {
        let m = store.create_meeting(NewMeeting::default()).unwrap().gid;
        store.add_segments(&m, lines.clone()).unwrap();
        if with_marks {
            for (t, tag) in &marked {
                store.add_mark(&m, *t, *tag).unwrap();
            }
        }
        store
            .enqueue_job(
                Some(&m),
                NOTES_FINAL_JOB,
                JOB_PAYLOAD_VERSION,
                &serde_json::json!({ "lang": "en" }),
            )
            .unwrap();
        let models = models.clone();
        let (tx, _rx) = bus();
        let runner = JobRunner::new(
            store.clone(),
            tx,
            vec![Arc::new(NotesJob {
                kind: NOTES_FINAL_JOB,
                version: 2,
                template: ghi_llm::template::builtin("general").unwrap(),
                llm: Arc::new(move |bytes| {
                    let n_ctx = ((bytes / 3) as u32 * 6 / 5 + 6_144).clamp(8_192, LLM_MAX_CTX);
                    LocalLlm::open_registry_in(&models, MODEL, n_ctx)
                        .map(|l| Box::new(l) as Box<dyn Llm + Send>)
                        .map_err(|e| e.to_string())
                }),
                ready: always_ready(),
            })],
        );
        let started = Instant::now();
        let (_, outcome) = runner.run_one().expect("a job ran");
        let seconds = started.elapsed().as_secs_f64();
        if !matches!(outcome, Ok(Outcome::Done)) {
            runs.push(Run {
                label,
                seconds,
                summary: format!("failed: {outcome:?}"),
            });
            continue;
        }

        let blocks = store.note_blocks(&m).unwrap();
        let actions = store.action_items(&m).unwrap();
        let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
        for b in blocks.iter().filter(|b| b.provenance != Provenance::User) {
            *kinds.entry(b.kind.clone()).or_default() += 1;
        }
        let segs = store.segments(&m).unwrap();
        let mark_rows = store.marks(&m).unwrap();
        let on_lines = mark_lines(&mark_rows, &segs);
        let sentence_anchors: Vec<&[Anchor]> = blocks
            .iter()
            .filter(|b| b.provenance != Provenance::User && covers_marks(&b.kind))
            .map(|b| b.anchors.as_slice())
            .collect();
        let action_anchors: Vec<&[Anchor]> = actions.iter().map(|a| a.anchors.as_slice()).collect();
        let ui = coverage(&on_lines, &sentence_anchors, &action_anchors);
        let ui_covered = ui.iter().filter(|c| c.is_covered()).count();
        // "Cited": some sentence or action has an anchor that is exactly the line.
        let cited = on_lines
            .iter()
            .filter(|l| {
                l.segment.is_some()
                    && sentence_anchors
                        .iter()
                        .chain(&action_anchors)
                        .any(|a| a.iter().any(|x| (x.t0_ms, x.t1_ms) == l.range))
            })
            .count();
        runs.push(Run {
            label,
            seconds,
            summary: format!(
                "blocks {kinds:?}, action items {}; marks on a line {} of {}; \
                 marked line cited {cited}; UI coverage (time overlap) {ui_covered}",
                actions.len(),
                on_lines.iter().filter(|l| l.segment.is_some()).count(),
                on_lines.len(),
            ),
        });
    }
    let report: Vec<String> = runs
        .iter()
        .map(|r| format!("{}: {:.1} s, {}", r.label, r.seconds, r.summary))
        .collect();
    eprintln!("{} segments\n{}", lines.len(), report.join("\n"));
}
