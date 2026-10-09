// SPDX-License-Identifier: Apache-2.0
//! The notes job gives the local model the moments the user marked: three
//! marks in, three line ids in the prompt; a mark in silence is left out.

use std::sync::{Arc, Mutex};

use ghi_core::events::bus;
use ghi_core::jobs::{JobRunner, Outcome, always_ready};
use ghi_core::notes_job::{NOTES_FINAL_JOB, NotesJob};
use ghi_core::session::JOB_PAYLOAD_VERSION;
use ghi_llm::{Completion, EngineInfo, Llm, Request};
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::{MarkTag, NewMeeting, NewSegment, Store};

/// Records the requests and answers with notes citing line 1.
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
        Ok(Completion {
            text: r#"{"tldr":[{"text":"Chốt lịch beta","cite":[1]}],"decisions":[],"action_items":[],"open_questions":[],"key_quotes":[],"topics":[]}"#.into(),
            tokens_in: 10,
            tokens_out: 5,
            truncated: false,
        })
    }
}

#[test]
fn three_marks_in_three_line_ids_in_the_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    let m = store.create_meeting(NewMeeting::default()).unwrap().gid;
    let line = |t0: i64, text: &str| NewSegment {
        t0_ms: t0,
        t1_ms: t0 + 4000,
        text: text.into(),
        lang: Some("vi".into()),
        ..Default::default()
    };
    store
        .add_segments(
            &m,
            vec![
                line(0, "mở đầu cuộc họp hôm nay"),
                line(5000, "chốt lịch beta vào thứ sáu"),
                line(10_000, "Nam sẽ gửi tài liệu"),
                line(15_000, "còn câu hỏi về ngân sách"),
            ],
        )
        .unwrap();
    store.add_mark(&m, 6000, MarkTag::Decision).unwrap();
    store.add_mark(&m, 11_000, MarkTag::Action).unwrap();
    store.add_mark(&m, 16_000, MarkTag::Star).unwrap();
    // 60 s of silence after the last line: no line to point at.
    store.add_mark(&m, 80_000, MarkTag::Star).unwrap();
    store
        .enqueue_job(
            Some(&m),
            NOTES_FINAL_JOB,
            JOB_PAYLOAD_VERSION,
            &serde_json::json!({ "lang": "vi" }),
        )
        .unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    let (tx, _rx) = bus();
    let runner = JobRunner::new(
        store.clone(),
        tx,
        vec![Arc::new(NotesJob {
            kind: NOTES_FINAL_JOB,
            version: 2,
            template: ghi_llm::template::builtin("general").unwrap(),
            llm: Arc::new(move |_| Ok(Box::new(Recorder(s.clone())) as Box<dyn Llm + Send>)),
            ready: always_ready(),
        })],
    );
    let (_, outcome) = runner.run_one().expect("a job ran");
    assert!(matches!(outcome, Ok(Outcome::Done)), "{outcome:?}");
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let prompt = &requests[0].messages[1].content;
    assert!(
        prompt.contains("[s1 decision] [s2 action] [s3 star]"),
        "{prompt}"
    );
    assert_eq!(
        prompt.matches(" star]").count(),
        1,
        "the silent mark is out"
    );
}

/// A short meeting with ten saved answers (pinned, up to 1,500 characters each)
/// puts all of that text in the prompt: the model is opened for it.
#[test]
fn saved_answers_count_when_the_context_is_sized() {
    use ghi_store::store::Provenance;
    let tmp = tempfile::tempdir().unwrap();
    let store = Arc::new(
        Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap(),
    );
    let m = store.create_meeting(NewMeeting::default()).unwrap().gid;
    store
        .add_segments(
            &m,
            vec![NewSegment {
                t0_ms: 0,
                t1_ms: 3000,
                text: "chốt lịch beta".into(),
                lang: Some("vi".into()),
                ..Default::default()
            }],
        )
        .unwrap();
    for i in 0..10 {
        store
            .add_note_block(
                &m,
                ghi_store::store::NewNoteBlock {
                    kind: "answer".into(),
                    provenance: Provenance::Ai,
                    body: format!("Q: {}\nA: {}", "q".repeat(300), "a".repeat(1200 + i)),
                    anchors: vec![],
                    pinned: true,
                },
            )
            .unwrap();
    }
    store
        .enqueue_job(
            Some(&m),
            NOTES_FINAL_JOB,
            JOB_PAYLOAD_VERSION,
            &serde_json::json!({ "lang": "vi" }),
        )
        .unwrap();
    let sized = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (sz, sn) = (sized.clone(), seen.clone());
    let (tx, _rx) = bus();
    let runner = JobRunner::new(
        store.clone(),
        tx,
        vec![Arc::new(NotesJob {
            kind: NOTES_FINAL_JOB,
            version: 2,
            template: ghi_llm::template::builtin("general").unwrap(),
            llm: Arc::new(move |bytes| {
                sz.lock().unwrap().push(bytes);
                Ok(Box::new(Recorder(sn.clone())) as Box<dyn Llm + Send>)
            }),
            ready: always_ready(),
        })],
    );
    let (_, outcome) = runner.run_one().expect("a job ran");
    assert!(matches!(outcome, Ok(Outcome::Done)), "{outcome:?}");
    let bytes = sized.lock().unwrap()[0];
    assert!(bytes >= 10 * 1500, "{bytes} bytes sized the context");
    // And they are in the prompt the model got.
    let prompt = seen.lock().unwrap()[0].messages[1].content.clone();
    assert!(
        prompt.matches("Q: qqqq").count() == 10,
        "ten saved answers kept"
    );
}
