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
use ghi_llm::notes::{self, MarkHint, MarkKind, Notes, Options};
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

/// Digit runs ("135", "9:30" gives "9" and "30") in `s`.
fn numbers(s: &str) -> Vec<String> {
    // A speaker label ("S1") is a name, not a number.
    let no_labels: String = s
        .split_inclusive(char::is_whitespace)
        .filter(|w| {
            let w = w.trim_matches(|c: char| !c.is_alphanumeric());
            !(w.len() > 1 && w.starts_with('S') && w[1..].chars().all(|c| c.is_ascii_digit()))
        })
        .collect();
    no_labels
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect()
}

/// The consultation template on a synthetic call (no real patient data):
/// its sections exist, every cited line exists, and no number or date in the
/// notes is absent from the transcript (nothing invented, nothing converted).
#[test]
fn golden_consultation() {
    let Some(mut llm) = model() else { return };
    let t = golden("consultation");
    let run = notes::generate(
        &mut llm,
        &t,
        &Options::new(template::builtin("consultation").unwrap(), OutLang::En),
    )
    .unwrap();
    eprintln!(
        "consultation: {:?}, diagnostics {:?}\n{}",
        run.strategy,
        run.diagnostics,
        serde_json::to_string_pretty(&run.notes).unwrap()
    );
    let n = &run.notes;
    for cites in n.all_citations() {
        assert!(!cites.is_empty(), "an AI item without a citation");
        assert!(cites.iter().all(|id| t.get(*id).is_some()));
    }
    let ids: Vec<&str> = n.sections.iter().map(|s| s.id.as_str()).collect();
    for want in ["findings", "advice", "next_steps"] {
        assert!(ids.contains(&want), "no `{want}` section in {ids:?}");
    }
    assert!(
        n.sections
            .iter()
            .any(|s| s.id == "findings" && !s.items.is_empty()),
        "nothing under what was said"
    );
    let said: std::collections::HashSet<String> =
        t.segments().iter().flat_map(|s| numbers(&s.text)).collect();
    for text in texts(n) {
        for num in numbers(text) {
            assert!(said.contains(&num), "invented number {num} in `{text}`");
        }
        let low = text.to_lowercase();
        assert!(!low.contains("diagnos"), "diagnosis wording in `{text}`");
    }
    assert_eq!(ghi_net::connections_opened(), 0);
}

/// Phase 2 measurement (`--ignored`): a ~60-minute meeting (the EN golden
/// repeated 32 times, so the same story recurs; real recordings never enter
/// git) with 10 marks, notes with and without them: time, how many marked
/// lines are cited, and how many items the notes hold.
#[test]
#[ignore = "measurement: minutes on the local model"]
fn marks_on_a_long_meeting() {
    let Some(mut llm) = model() else { return };
    let base = golden("en");
    let n = base.segments().len() as u64;
    let span = base.segments().last().unwrap().t1_ms + 2000;
    let mut segs = Vec::new();
    for rep in 0..32u64 {
        for s in base.segments() {
            segs.push(Segment {
                id: s.id + rep * n,
                t0_ms: s.t0_ms + rep as i64 * span,
                t1_ms: s.t1_ms + rep as i64 * span,
                ..s.clone()
            });
        }
    }
    let t = Transcript::new(segs).unwrap();
    // (repeat, line in the repeat, tag): decisions, actions and questions across the hour.
    let picks = [
        (1, 4, MarkKind::Decision),
        (4, 6, MarkKind::Action),
        (8, 11, MarkKind::Question),
        (12, 17, MarkKind::Action),
        (15, 19, MarkKind::Decision),
        (19, 12, MarkKind::Action),
        (23, 4, MarkKind::Star),
        (26, 14, MarkKind::Action),
        (29, 9, MarkKind::Decision),
        (31, 20, MarkKind::Star),
    ];
    let marks: Vec<MarkHint> = picks
        .iter()
        .map(|&(rep, line, kind)| {
            let id = rep * n + line;
            MarkHint {
                id,
                kind,
                t_ms: t.get(id).unwrap().t0_ms,
            }
        })
        .collect();
    let count = |n: &Notes| {
        n.tldr.len()
            + n.decisions.len()
            + n.proposals.len()
            + n.action_items.len()
            + n.open_questions.len()
            + n.key_quotes.len()
            + n.topics.len()
    };
    let mut report = Vec::new();
    for (label, with) in [("no marks", false), ("marks", true)] {
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
        let cited: std::collections::HashSet<u64> =
            run.notes.all_citations().flatten().copied().collect();
        let hit = marks.iter().filter(|m| cited.contains(&m.id)).count();
        report.push(format!(
            "{label}: {wall:.1} s, {:?}, {} items, {} of {} marked lines cited, diagnostics {:?}",
            run.strategy,
            count(&run.notes),
            hit,
            marks.len(),
            run.diagnostics
        ));
    }
    eprintln!("{}", report.join("\n"));
}

/// Meetings with lines that were agreed and lines only suggested: `agreed`
/// and `suggested` are the lines each kind of decision comes from.
struct Statuses {
    lang: OutLang,
    lines: &'static [&'static str],
    agreed: &'static [u64],
    suggested: &'static [u64],
}

const STATUS_SETS: &[Statuses] = &[
    Statuses {
        lang: OutLang::En,
        lines: &[
            "Agreed, we ship the beta on Friday.",
            "Yes, confirmed, Friday it is.",
            "We could maybe try a dark mode theme next quarter.",
            "Maybe we should consider moving support to a new vendor.",
            "Okay, final: the budget cap is fifty thousand dollars.",
            "Agreed on the cap, fifty thousand.",
            "Perhaps we could hire a contractor for the design work, not sure yet.",
            "Good, that is all for today.",
        ],
        agreed: &[0, 1, 4, 5],
        suggested: &[2, 3, 6],
    },
    Statuses {
        lang: OutLang::Vi,
        lines: &[
            "Đồng ý, mình phát hành bản beta vào thứ Sáu.",
            "Vâng, chốt rồi, thứ Sáu.",
            "Hay là quý sau mình thử làm giao diện tối nhỉ.",
            "Có thể mình nên cân nhắc chuyển bộ phận hỗ trợ sang nhà cung cấp mới.",
            "Được, chốt: mức trần ngân sách là năm mươi nghìn đô.",
            "Đồng ý với mức trần năm mươi nghìn.",
            "Có lẽ mình thuê thêm một bạn làm thiết kế, chưa chắc lắm.",
            "Tốt, hôm nay vậy thôi.",
        ],
        agreed: &[0, 1, 4, 5],
        suggested: &[2, 3, 6],
    },
];

/// Decided vs proposed on the local model, single pass and map-reduce (the
/// same lines spread over 20 minutes so they are cut into parts).
#[test]
fn golden_decided_and_proposed() {
    let Some(mut llm) = model() else { return };
    for set in STATUS_SETS {
        for (label, gap_s) in [("single pass", 10.0), ("map-reduce", 180.0)] {
            let segs: Vec<Segment> = set
                .lines
                .iter()
                .enumerate()
                .map(|(i, text)| Segment {
                    id: i as u64,
                    t0_ms: (i as f64 * gap_s * 1000.0) as i64,
                    t1_ms: (i as f64 * gap_s * 1000.0) as i64 + 4000,
                    speaker: Some(format!("S{}", i % 3 + 1)),
                    text: text.to_string(),
                    lang: Some(set.lang.code().to_string()),
                })
                .collect();
            let t = Transcript::new(segs).unwrap();
            let mut o = Options::new(template::builtin("general").unwrap(), set.lang);
            o.chunk_minutes = 5;
            let started = Instant::now();
            let run = notes::generate_steps(&mut llm, &t, &o, None, &mut |_| {}).unwrap();
            let n = &run.notes;
            // An item is right when it comes from the lines of its own status.
            let right = |items: &[notes::Item], from: &[u64]| {
                items
                    .iter()
                    .filter(|i| i.citations.iter().all(|c| from.contains(c)))
                    .count()
            };
            let total = n.decisions.len() + n.proposals.len();
            let ok = right(&n.decisions, set.agreed) + right(&n.proposals, set.suggested);
            eprintln!(
                "{:?} {label}: {:.1} s, {:?}: {ok} of {total} decisions have the right status \
                 ({} decided, {} proposed)\n  decided: {:?}\n  proposed: {:?}",
                set.lang,
                started.elapsed().as_secs_f64(),
                run.strategy,
                n.decisions.len(),
                n.proposals.len(),
                n.decisions.iter().map(|i| &i.text).collect::<Vec<_>>(),
                n.proposals.iter().map(|i| &i.text).collect::<Vec<_>>(),
            );
            assert!(total >= 2, "{label}: too few decisions");
            assert!(ok * 10 >= total * 8, "{label}: {ok} of {total} right");
            assert!(!n.proposals.is_empty(), "{label}: nothing proposed");
            assert!(!n.decisions.is_empty(), "{label}: nothing decided");
            check(&t, n);
        }
    }
}
