// SPDX-License-Identifier: Apache-2.0
//! Notes on a long real meeting with the local model: time, items and marks.
//! Separate from `golden_notes.rs` so it also builds on older commits (time
//! comparisons). Skipped without the model, the worker or a transcript.

use std::path::{Path, PathBuf};
use std::time::Instant;

use ghi_llm::local::LocalLlm;
use ghi_llm::notes::{self, MarkHint, MarkKind, Notes, Options};
use ghi_llm::template::{self, OutLang};
use ghi_llm::{Segment, Transcript};

const MODEL: &str = "qwen3-4b";

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

/// Notes on a long real meeting, with and without 10 marks (`--ignored`;
/// minutes on the local model). `GHI_LONG_TRANSCRIPT` is a `ghi.transcript/1`
/// file (e.g. `ghi transcribe` of a 45+ minute recording from tools/eval/data;
/// recordings and transcripts never enter git). Prints time, items per list,
/// and how many marked lines are cited ("covered" = the marked line is cited
/// by some item).
#[test]
#[ignore = "measurement: minutes on the local model"]
fn marks_on_a_long_meeting() {
    let Some(path) = std::env::var_os("GHI_LONG_TRANSCRIPT") else {
        eprintln!("skipped: set GHI_LONG_TRANSCRIPT to a ghi.transcript/1 file");
        return;
    };
    let Some(mut llm) = model() else { return };
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let t = Transcript::new(
        doc["segments"]
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
            .collect(),
    )
    .unwrap();
    // Ten marks spread over the meeting, on lines with some words in them.
    let segs = t.segments();
    let kinds = [
        MarkKind::Decision,
        MarkKind::Action,
        MarkKind::Question,
        MarkKind::Star,
        MarkKind::Action,
    ];
    let marks: Vec<MarkHint> = (0..10)
        .map(|k| {
            let mut i = (k * segs.len() / 10 + segs.len() / 20).min(segs.len() - 1);
            while i + 1 < segs.len() && segs[i].text.split_whitespace().count() < 6 {
                i += 1;
            }
            MarkHint {
                id: segs[i].id,
                kind: kinds[k % kinds.len()],
                t_ms: segs[i].t0_ms,
            }
        })
        .collect();
    // Counts through JSON so the same test runs on older commits.
    let counts = |n: &Notes| {
        let v = serde_json::to_value(n).unwrap();
        let len = |k: &str| v[k].as_array().map_or(0, Vec::len);
        let sections: usize = v["sections"].as_array().map_or(0, |a| {
            a.iter()
                .map(|s| s["items"].as_array().map_or(0, Vec::len))
                .sum()
        });
        format!(
            "tldr {} decisions {}+{} actions {} questions {} quotes {} topics {} sections {}",
            len("tldr"),
            len("decisions"),
            len("proposals"),
            len("action_items"),
            len("open_questions"),
            len("key_quotes"),
            len("topics"),
            sections
        )
    };
    let mut report = Vec::new();
    let only_plain = std::env::var_os("GHI_LONG_NO_MARKS_ONLY").is_some();
    for (label, with) in [("no marks", false), ("marks", true)] {
        if with && only_plain {
            continue;
        }
        let mut o = Options::new(template::builtin("general").unwrap(), OutLang::En);
        if with {
            o.marks = marks.clone();
        }
        let started = Instant::now();
        let run = match notes::generate(&mut llm, &t, &o) {
            Ok(run) => run,
            Err(e) => {
                report.push(format!(
                    "{label}: failed after {:.1} s: {e}",
                    started.elapsed().as_secs_f64()
                ));
                continue;
            }
        };
        let wall = started.elapsed().as_secs_f64();
        // Each listed decision with its lines and the verdict of the status pass.
        for (verdict, items) in [
            ("decided", &run.notes.decisions),
            ("proposed", &run.notes.proposals),
        ] {
            for i in items {
                eprintln!("DECISION [{verdict}] {} cites {:?}", i.text, i.citations);
                for c in &i.citations {
                    if let Some(x) = t.get(*c) {
                        eprintln!("    s{c}: {}", x.text);
                    }
                }
                if let Some(last) = i.citations.iter().filter_map(|c| t.index_of(*c)).max()
                    && let Some(next) = t.segments().get(last + 1)
                {
                    eprintln!("    next s{}: {}", next.id, next.text);
                }
            }
        }
        let cited: std::collections::HashSet<u64> =
            run.notes.all_citations().flatten().copied().collect();
        let hit = marks.iter().filter(|m| cited.contains(&m.id)).count();
        report.push(format!(
            "{label}: {wall:.1} s, {:?}, {}; marked lines cited {hit} of {}; diagnostics {:?}",
            run.strategy,
            counts(&run.notes),
            marks.len(),
            run.diagnostics
        ));
    }
    eprintln!("{} segments\n{}", segs.len(), report.join("\n"));
}
