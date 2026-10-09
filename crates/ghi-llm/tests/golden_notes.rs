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
    v.extend(n.proposals.iter().map(|i| i.text.as_str()));
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
    weak_rate(name, &run);
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

/// Digit runs ("135", "9:30" gives "9" and "30") in `s`, leaving out the
/// transcript's own speaker labels (`labels`, e.g. "S1"): a name, not a number.
fn numbers(s: &str, labels: &[&str]) -> Vec<String> {
    let no_labels: String = s
        .split_inclusive(char::is_whitespace)
        .filter(|w| !labels.contains(&w.trim_matches(|c: char| !c.is_alphanumeric())))
        .collect();
    no_labels
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect()
}

/// Weak-source items among all cited items of a run, printed for the record.
fn weak_rate(label: &str, run: &notes::Run) -> f64 {
    let items = run.notes.all_citations().count();
    let weak = run.diagnostics.weak_anchors as usize;
    eprintln!("{label}: {weak} of {items} items flagged to check");
    weak as f64 / items.max(1) as f64
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
    weak_rate("consultation", &run);
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
    let labels = t.speakers();
    let said: std::collections::HashSet<String> = t
        .segments()
        .iter()
        .flat_map(|s| numbers(&s.text, &labels))
        .collect();
    for text in texts(n) {
        for num in numbers(text, &labels) {
            assert!(said.contains(&num), "invented number {num} in `{text}`");
        }
        let low = text.to_lowercase();
        assert!(!low.contains("diagnos"), "diagnosis wording in `{text}`");
    }
    assert_eq!(ghi_net::connections_opened(), 0);
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
            weak_rate(&format!("{:?} {label}", set.lang), &run);
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

/// A longer, messier meeting: lines of small talk, suggestions that stay open,
/// agreements (some hedged: "let's maybe go with X then" is agreed), and a
/// suggestion that is agreed a line later.
struct Messy {
    lang: OutLang,
    lines: &'static [&'static str],
    /// Lines that make up one decision (or proposal), and whether it was agreed.
    groups: &'static [(&'static [u64], bool)],
}

const MESSY: &[Messy] = &[
    Messy {
        lang: OutLang::En,
        lines: &[
            "Okay, let's start with the Q3 roadmap.",
            "I think we should drop the legacy importer.",
            "Hmm, maybe, but some customers still use it.",
            "Fair. Okay, let's maybe go with deprecating it in Q4 then.",
            "We could also try a public beta of the plugin API.",
            "Not sure, let's see how the review goes first.",
            "Agreed, the security review happens on the 12th.",
            "Should we switch the CI to the new provider?",
            "Let's revisit that next month.",
            "Okay, so the launch date is October 30, final.",
            "Maybe we could add a dark theme before launch.",
            "Yes, go ahead and hire the two contractors.",
            "Perhaps we should also run a customer survey.",
            "Sounds good, the survey goes out after launch, agreed.",
            "Quick note, lunch is at noon.",
            "Thanks everyone, that is all.",
        ],
        groups: &[
            (&[1, 2, 3], true),
            (&[4, 5], false),
            (&[6], true),
            (&[7, 8], false),
            (&[9], true),
            (&[10], false),
            (&[11], true),
            (&[12, 13], true),
        ],
    },
    Messy {
        lang: OutLang::Vi,
        lines: &[
            "Được rồi, mình bắt đầu với lộ trình quý ba.",
            "Em nghĩ mình nên bỏ công cụ nhập dữ liệu cũ.",
            "Ừm, có thể, nhưng vẫn còn khách hàng dùng nó.",
            "Cũng đúng. Vậy thôi, mình cứ ngừng hỗ trợ nó vào quý bốn nhé.",
            "Mình cũng có thể thử mở bản beta công khai cho API plugin.",
            "Chưa chắc, xem buổi đánh giá thế nào đã.",
            "Đồng ý, buổi đánh giá bảo mật diễn ra vào ngày mười hai.",
            "Mình có nên chuyển CI sang nhà cung cấp mới không?",
            "Để tháng sau mình xem lại.",
            "Rồi, vậy ngày ra mắt là ba mươi tháng mười, chốt luôn.",
            "Có thể mình thêm giao diện tối trước khi ra mắt.",
            "Vâng, cứ thuê hai bạn làm hợp đồng đi.",
            "Hay là mình làm thêm một cuộc khảo sát khách hàng.",
            "Được đấy, khảo sát gửi sau khi ra mắt, đồng ý.",
            "Nhắc nhỏ, mười hai giờ ăn trưa.",
            "Cảm ơn mọi người, hết rồi.",
        ],
        groups: &[
            (&[1, 2, 3], true),
            (&[4, 5], false),
            (&[6], true),
            (&[7, 8], false),
            (&[9], true),
            (&[10], false),
            (&[11], true),
            (&[12, 13], true),
        ],
    },
];

/// Recall and precision of decided / proposed per language and
/// strategy (`--ignored`; a few minutes). An item is right when it cites a
/// line of a group and its status is that group's.
#[test]
#[ignore = "measurement: minutes on the local model"]
fn decision_status_recall_and_precision() {
    let Some(mut llm) = model() else { return };
    let mut total = [(0usize, 0usize, 0usize, 0usize); 2]; // per strategy: found, groups, right, listed
    for set in MESSY {
        for (si, (label, gap_s)) in [("single pass", 10.0), ("map-reduce", 120.0)]
            .into_iter()
            .enumerate()
        {
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
            // Runs are repeatable (the same prompt gives the same notes), so once is enough.
            for run_no in 1..=1 {
                let mut o = Options::new(template::builtin("general").unwrap(), set.lang);
                o.chunk_minutes = 5;
                let run = match notes::generate_steps(&mut llm, &t, &o, None, &mut |_| {}) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("{:?} {label} #{run_no}: failed: {e}", set.lang);
                        continue;
                    }
                };
                weak_rate(&format!("messy {:?} {label}", set.lang), &run);
                let n = &run.notes;
                let listed: Vec<(&notes::Item, bool)> = n
                    .decisions
                    .iter()
                    .map(|i| (i, true))
                    .chain(n.proposals.iter().map(|i| (i, false)))
                    .collect();
                let group_of = |i: &notes::Item| {
                    set.groups
                        .iter()
                        .find(|(lines, _)| i.citations.iter().any(|c| lines.contains(c)))
                };
                let right = listed
                    .iter()
                    .filter(|(i, decided)| group_of(i).is_some_and(|(_, agreed)| agreed == decided))
                    .count();
                let found = set
                    .groups
                    .iter()
                    .filter(|(lines, agreed)| {
                        listed.iter().any(|(i, decided)| {
                            decided == agreed && i.citations.iter().any(|c| lines.contains(c))
                        })
                    })
                    .count();
                let cites = |items: &[notes::Item], lines: &[u64]| {
                    items
                        .iter()
                        .any(|i| i.citations.iter().any(|c| lines.contains(c)))
                };
                let agreed_total = set.groups.iter().filter(|(_, a)| *a).count();
                let agreed_decided = set
                    .groups
                    .iter()
                    .filter(|(l, a)| *a && cites(&n.decisions, l))
                    .count();
                let open_total = set.groups.len() - agreed_total;
                let open_proposed = set
                    .groups
                    .iter()
                    .filter(|(l, a)| !*a && cites(&n.proposals, l))
                    .count();
                // How long the status pass takes: run it again over everything listed.
                let mut again = run.clone();
                again
                    .notes
                    .decisions
                    .extend(std::mem::take(&mut again.notes.proposals));
                let pass = Instant::now();
                notes::classify_decisions(&mut llm, &t, &o, &mut again);
                let pass_s = pass.elapsed().as_secs_f64();
                eprintln!(
                    "{:?} {label} #{run_no}: agreed as decided {agreed_decided}/{agreed_total}, open as proposed {open_proposed}/{open_total}; recall {found}/{}, precision {right}/{} ({} decided, {} proposed); status pass {pass_s:.1} s",
                    set.lang,
                    set.groups.len(),
                    listed.len(),
                    n.decisions.len(),
                    n.proposals.len()
                );
                for (name, items) in [("decided", &n.decisions), ("proposed", &n.proposals)] {
                    for i in items {
                        eprintln!("    {name}: {} {:?}", i.text, i.citations);
                    }
                }
                let t = &mut total[si];
                *t = (
                    t.0 + found,
                    t.1 + set.groups.len(),
                    t.2 + right,
                    t.3 + listed.len(),
                );
            }
        }
    }
    for (label, t) in ["single pass", "map-reduce"].iter().zip(total) {
        eprintln!(
            "TOTAL {label}: recall {}/{} = {:.0}%, precision {}/{} = {:.0}%",
            t.0,
            t.1,
            100.0 * t.0 as f64 / t.1.max(1) as f64,
            t.2,
            t.3,
            100.0 * t.2 as f64 / t.3.max(1) as f64
        );
    }
}

/// The status pass alone, on decisions whose lines and status are known: one
/// decision per group of the messy sets and per line of the first sets
/// (`--ignored`; seconds).
#[test]
#[ignore = "measurement: local model"]
fn status_pass_accuracy() {
    let Some(mut llm) = model() else { return };
    type Case<'a> = (OutLang, &'a [&'a str], Vec<(Vec<u64>, bool)>);
    let mut cases: Vec<Case> = Vec::new();
    for m in MESSY {
        cases.push((
            m.lang,
            m.lines,
            m.groups.iter().map(|(l, a)| (l.to_vec(), *a)).collect(),
        ));
    }
    for s in STATUS_SETS {
        let mut g: Vec<(Vec<u64>, bool)> = s.agreed.iter().map(|&l| (vec![l], true)).collect();
        g.extend(s.suggested.iter().map(|&l| (vec![l], false)));
        cases.push((s.lang, s.lines, g));
    }
    let (mut right, mut all) = (0, 0);
    for (lang, lines, groups) in cases {
        let segs: Vec<Segment> = lines
            .iter()
            .enumerate()
            .map(|(i, text)| Segment {
                id: i as u64,
                t0_ms: i as i64 * 10_000,
                t1_ms: i as i64 * 10_000 + 4000,
                speaker: Some(format!("S{}", i % 3 + 1)),
                text: text.to_string(),
                lang: Some(lang.code().to_string()),
            })
            .collect();
        let t = Transcript::new(segs).unwrap();
        let o = Options::new(template::builtin("general").unwrap(), lang);
        let mut run = notes::Run {
            notes: Notes {
                template: "general".into(),
                lang: lang.code().into(),
                tldr: vec![],
                decisions: groups
                    .iter()
                    .map(|(l, _)| notes::Item {
                        text: format!("decision about line {}", l[0]),
                        citations: l.clone(),
                    })
                    .collect(),
                proposals: vec![],
                action_items: vec![],
                open_questions: vec![],
                key_quotes: vec![],
                topics: vec![],
                sections: vec![],
            },
            engine: llm_engine(),
            strategy: notes::Strategy::Single,
            diagnostics: Default::default(),
        };
        let started = Instant::now();
        notes::classify_decisions(&mut llm, &t, &o, &mut run);
        let proposed: std::collections::HashSet<u64> = run
            .notes
            .proposals
            .iter()
            .flat_map(|i| i.citations.clone())
            .collect();
        let ok = groups
            .iter()
            .filter(|(l, agreed)| proposed.contains(&l[0]) != *agreed)
            .count();
        eprintln!(
            "{:?}: {ok} of {} right ({:.1} s, failed {}); wrong: {:?}",
            lang,
            groups.len(),
            started.elapsed().as_secs_f64(),
            run.diagnostics.status_failed,
            groups
                .iter()
                .filter(|(l, agreed)| proposed.contains(&l[0]) == *agreed)
                .map(|(l, a)| (l[0], *a))
                .collect::<Vec<_>>()
        );
        right += ok;
        all += groups.len();
    }
    eprintln!("STATUS PASS: {right} of {all} right");
}

fn llm_engine() -> ghi_llm::EngineInfo {
    ghi_llm::EngineInfo {
        name: "x".into(),
        version: "1".into(),
    }
}
