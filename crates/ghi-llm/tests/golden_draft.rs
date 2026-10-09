// SPDX-License-Identifier: Apache-2.0
//! "Draft from description" on the local model (Qwen3-4B): five descriptions,
//! English and Vietnamese. Wording varies, so these check shape: every draft is
//! something the editor would accept and save, with 1 to 8 plain-text sections,
//! and nothing is invented beyond the form. Skipped (with a note) when the
//! model or the worker binary isn't there.

use std::path::{Path, PathBuf};

use ghi_llm::draft::draft;
use ghi_llm::local::LocalLlm;
use ghi_llm::template::{MAX_SECTIONS, OutLang, Template};

const MODEL: &str = "qwen3-4b";

/// The model, or `None` (test skipped) when it or the worker is missing.
fn model() -> Option<LocalLlm> {
    #[cfg(feature = "inproc")]
    ghi_llm::sidecar::use_in_process(true);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let m = ghi_models::find(MODEL).unwrap();
    let dir = std::env::var_os("GHI_MODELS_DIR").map_or(root.join("models"), PathBuf::from);
    if !ghi_models::path_in(&dir, &m).is_file() {
        eprintln!("skipped: {MODEL} not downloaded (tools/scripts/fetch-models.sh {MODEL})");
        return None;
    }
    if !cfg!(feature = "inproc") && std::env::var_os("GHI_LLM_WORKER").is_none() {
        let worker = root.join("target/debug/ghi-llm-worker");
        if !worker.is_file() {
            eprintln!("skipped: build the worker first (cargo build -p ghi-llm-worker)");
            return None;
        }
        // SAFETY: this file's tests share one process but only this runs first, before any worker starts.
        unsafe { std::env::set_var("GHI_LLM_WORKER", worker) };
    }
    // SAFETY: as above.
    unsafe { std::env::set_var("GHI_MODELS_DIR", dir) };
    Some(LocalLlm::open_registry(MODEL, 8192).expect("model loads"))
}

const CASES: &[(&str, OutLang)] = &[
    ("A weekly retrospective for a software team: what went well, what did not, what to change.", OutLang::En),
    ("Customer discovery interviews for a fintech startup, where we learn about their problems.", OutLang::En),
    ("Monthly board meeting of a small non-profit: finances, programs and fundraising.", OutLang::En),
    ("Họp giao ban hằng tuần của phòng kinh doanh: doanh số, khách hàng tiềm năng và khó khăn.", OutLang::Vi),
    ("Buổi tư vấn sức khoẻ cho bệnh nhân: triệu chứng, chẩn đoán và dặn dò.", OutLang::Vi),
];

#[test]
fn five_descriptions_give_drafts_the_editor_accepts() {
    let Some(mut llm) = model() else { return };
    for (description, lang) in CASES {
        let e = draft(&mut llm, description, *lang).unwrap_or_else(|e| panic!("{description}: {e}"));
        let t = Template::from_editor("draft", &e, &[], &[]).unwrap_or_else(|x| panic!("{description}: {x}\n{e:?}"));
        assert!(!e.name.is_empty(), "{description}: no name");
        assert!((1..=MAX_SECTIONS).contains(&e.sections.len()), "{description}: {} sections", e.sections.len());
        for s in &e.sections {
            assert!(s.id.is_none());
            for text in [&s.title, &s.instruction] {
                assert!(!text.contains(['*', '`', '<', '>', '[', ']']), "{description}: markup in {text:?}");
            }
        }
        for s in &e.sections {
            let f = ghi_text::fold(&s.title);
            assert!(!["summary", "action items", "key quotes", "topics", "decisions", "open questions"].contains(&f.as_str()), "{description}: repeats a standard section: {}", s.title);
        }
        // The ids it would get are valid keys, distinct from the standard sections.
        assert_eq!(t.sections.len(), e.sections.len());
        eprintln!("{description}\n  -> {} | {:?}", e.name, e.sections.iter().map(|s| s.title.as_str()).collect::<Vec<_>>());
    }
}
