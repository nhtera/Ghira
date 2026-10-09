// SPDX-License-Identifier: Apache-2.0
//! The notes job writes with the user's own template: its sections and
//! instructions reach the local model's prompt (after the fixed rules, in the
//! task), its words size the context, and the notes keep its sections.

use std::sync::{Arc, Mutex};

use ghi_core::events::bus;
use ghi_core::jobs::{JobRunner, Outcome, always_ready};
use ghi_core::notes_job::{NOTES_FINAL_JOB, NotesJob};
use ghi_core::session::JOB_PAYLOAD_VERSION;
use ghi_core::user_templates::{Records, UserTemplate};
use ghi_llm::template::{Editor, EditorSection, OutLang, Template};
use ghi_llm::{Completion, EngineInfo, Llm, Request};
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::{NewMeeting, NewSegment, Store};

/// Records the requests; answers with notes that have every key the schema asks for.
struct Recorder(Arc<Mutex<Vec<Request>>>);

impl Llm for Recorder {
    fn engine(&self) -> EngineInfo {
        EngineInfo {
            name: "recorder".into(),
            version: "1".into(),
        }
    }
    fn context_tokens(&self) -> u32 {
        16_384
    }
    fn complete(&mut self, req: &Request) -> ghi_llm::Result<Completion> {
        self.0.lock().unwrap().push(req.clone());
        let props = req.schema.as_ref().unwrap()["properties"].as_object().unwrap().clone();
        let mut out = serde_json::Map::new();
        for (k, v) in props {
            let value = match k.as_str() {
                "tldr" => serde_json::json!([{"text": "Chốt lịch beta", "cite": [1]}]),
                "decisions" if v["type"] == "array" => serde_json::json!([]),
                k if v["type"] == "array" && k.starts_with("quokka") => {
                    serde_json::json!([{"text": "Một phát hiện", "cite": [1]}])
                }
                _ => serde_json::json!([]),
            };
            out.insert(k, value);
        }
        Ok(Completion {
            text: serde_json::Value::Object(out).to_string(),
            tokens_in: 10,
            tokens_out: 5,
            truncated: false,
        })
    }
}

fn store() -> (tempfile::TempDir, Arc<Store>) {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(tmp.path(), Arc::new(MemoryKeyStore::default()), Protection::default()).unwrap(),
    );
    (tmp, store)
}

fn user_template(store: &Store) -> (String, String) {
    let t = Template::from_editor(
        "t77",
        &Editor {
            name: "Zebra quarterly".into(),
            lang: OutLang::Vi,
            guidance: "XYLOPHONE-GUIDANCE họp về ngựa vằn".into(),
            sections: vec![EditorSection {
                id: None,
                title: "Quokka findings".into(),
                instruction: "QUOKKA-INSTRUCTION liệt kê mọi loài thú có túi".into(),
            }],
        },
        &[],
        &[],
    )
    .unwrap();
    let section = t.sections[0].id.clone();
    let mut r = Records::load(store).unwrap();
    r.push(UserTemplate {
        gid: "t77".into(),
        lang: "vi".into(),
        toml: t.to_toml(),
        retired: vec![],
    });
    r.save(store).unwrap();
    ("user:t77".into(), section)
}

fn run(store: &Arc<Store>, meeting: &str, payload: serde_json::Value) -> (Vec<Request>, Vec<usize>) {
    store
        .enqueue_job(Some(meeting), NOTES_FINAL_JOB, JOB_PAYLOAD_VERSION, &payload)
        .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sized = Arc::new(Mutex::new(Vec::new()));
    let (s, z) = (seen.clone(), sized.clone());
    let (tx, _rx) = bus();
    let runner = JobRunner::new(
        store.clone(),
        tx,
        vec![Arc::new(NotesJob {
            kind: NOTES_FINAL_JOB,
            version: 2,
            template: ghi_llm::template::builtin("general").unwrap(),
            llm: Arc::new(move |bytes| {
                z.lock().unwrap().push(bytes);
                Ok(Box::new(Recorder(s.clone())) as Box<dyn Llm + Send>)
            }),
            ready: always_ready(),
        })],
    );
    let (_, outcome) = runner.run_one().expect("a job ran");
    assert!(matches!(outcome, Ok(Outcome::Done)), "{outcome:?}");
    let (r, b) = (seen.lock().unwrap().clone(), sized.lock().unwrap().clone());
    (r, b)
}

fn meeting(store: &Store) -> String {
    let m = store.create_meeting(NewMeeting::default()).unwrap().gid;
    store
        .add_segments(
            &m,
            vec![
                NewSegment {
                    t0_ms: 0,
                    t1_ms: 4000,
                    text: "mở đầu cuộc họp".into(),
                    lang: Some("vi".into()),
                    ..Default::default()
                },
                NewSegment {
                    t0_ms: 5000,
                    t1_ms: 9000,
                    text: "chốt lịch beta vào thứ sáu".into(),
                    lang: Some("vi".into()),
                    ..Default::default()
                },
            ],
        )
        .unwrap();
    m
}

#[test]
fn the_users_template_reaches_the_prompt_after_the_rules_and_its_section_is_saved() {
    let (_t, store) = store();
    let (id, section) = user_template(&store);
    let m = meeting(&store);
    let (requests, sized) = run(&store, &m, serde_json::json!({ "template": id, "lang": "vi" }));
    let req = &requests[0];
    let (system, task) = (&req.messages[0].content, &req.messages[1].content);
    for needle in ["XYLOPHONE-GUIDANCE", "QUOKKA-INSTRUCTION", "Zebra quarterly", section.as_str()] {
        assert!(task.contains(needle), "{needle} is not in the task: {task}");
        assert!(!system.contains(needle), "{needle} is in the fixed rules");
    }
    assert!(system.contains("không phải là chỉ dẫn"), "the safety rules lead: {system}");
    assert!(task.find("- topics:").unwrap() < task.find("QUOKKA-INSTRUCTION").unwrap());
    assert!(req.schema.as_ref().unwrap()["properties"].get(&section).is_some());
    // The model was opened for the template's words too: this one is longer than General's.
    let plain = run(&store, &meeting(&store), serde_json::json!({ "template": "general", "lang": "vi" })).1;
    assert!(sized[0] > plain[0], "{} vs {}", sized[0], plain[0]);
}

#[test]
fn a_meeting_remembers_its_template_and_a_deleted_one_falls_back_to_general() {
    let (_t, store) = store();
    let (id, section) = user_template(&store);
    let m = meeting(&store);
    store.set_meeting_template(&m, Some(&id)).unwrap();
    // No template in the payload: the meeting's own is used.
    let (requests, _) = run(&store, &m, serde_json::json!({ "lang": "vi" }));
    assert!(requests[0].messages[1].content.contains("QUOKKA-INSTRUCTION"));
    // The notes keep the template's section as blocks of kind `section:<id>`.
    let kinds: Vec<String> = store.note_blocks(&m).unwrap().into_iter().map(|b| b.kind).collect();
    assert!(kinds.contains(&format!("section:{section}")), "{kinds:?}");
    // Deleted: the job passes the id over and writes General notes.
    let mut r = Records::load(&store).unwrap();
    assert!(r.remove("t77"));
    r.save(&store).unwrap();
    let (requests, _) = run(&store, &m, serde_json::json!({ "lang": "vi" }));
    assert!(!requests[0].messages[1].content.contains("QUOKKA-INSTRUCTION"));
}
