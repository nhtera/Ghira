// SPDX-License-Identifier: Apache-2.0
//! The notes job gives the local model the spellings of terms the meeting
//! actually says (the user's vocabulary, attendees, enabled glossary packs);
//! a cloud request never carries them.

use std::sync::{Arc, Mutex};

use ghi_core::cloud::{Planned, Task, plan};
use ghi_core::events::bus;
use ghi_core::jobs::{JobRunner, Outcome, always_ready};
use ghi_core::notes_job::{NOTES_FINAL_JOB, NotesJob};
use ghi_core::session::JOB_PAYLOAD_VERSION;
use ghi_llm::cloud::CloudProvider;
use ghi_llm::preview::Prices;
use ghi_llm::template::OutLang;
use ghi_llm::{Completion, EngineInfo, Llm, Request};
use ghi_store::keys::{MemoryKeyStore, Protection};
use ghi_store::store::{NewMeeting, NewSegment, Store};

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
            text: r#"{"tldr":[{"text":"ok","cite":[1]}],"decisions":[],"action_items":[],"open_questions":[],"key_quotes":[],"topics":[]}"#.into(),
            tokens_in: 10,
            tokens_out: 5,
            truncated: false,
        })
    }
}

fn fixture(lang: &str, lines: &[&str]) -> (tempfile::TempDir, Arc<Store>, String) {
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
            lines
                .iter()
                .enumerate()
                .map(|(i, t)| NewSegment {
                    t0_ms: i as i64 * 5000,
                    t1_ms: i as i64 * 5000 + 4000,
                    text: (*t).into(),
                    lang: Some(lang.into()),
                    ..Default::default()
                })
                .collect(),
        )
        .unwrap();
    (tmp, store, m)
}

fn local_prompt(store: &Arc<Store>, m: &str, lang: &str) -> String {
    store
        .enqueue_job(
            Some(m),
            NOTES_FINAL_JOB,
            JOB_PAYLOAD_VERSION,
            &serde_json::json!({ "lang": lang }),
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
    requests[0].messages[1].content.clone()
}

#[test]
fn the_local_prompt_spells_the_terms_the_meeting_said_and_only_those() {
    let (_t, store, m) = fixture(
        "en",
        &[
            "we run nemotron on the metformin study",
            "the hypertension arm is next",
        ],
    );
    store
        .set_setting("vocabulary", &serde_json::json!(["Nemotron", "Plaud"]))
        .unwrap();
    store
        .set_setting("vocabulary.packs", &serde_json::json!(["medical-en"]))
        .unwrap();
    let prompt = local_prompt(&store, &m, "en");
    assert!(
        prompt.contains("These terms are said in the meeting. Write them exactly like this: Nemotron; metformin; hypertension\n"),
        "{prompt}"
    );
    assert!(!prompt.contains("Plaud"), "a term nobody said stays out");
    assert!(
        !prompt.contains("insulin"),
        "and so does a pack term nobody said"
    );
}

#[test]
fn the_vietnamese_prompt_has_the_vietnamese_block() {
    let (_t, store, m) = fixture(
        "vi",
        &[
            "bác sĩ cho tôi uống paracetamol sau bữa ăn",
            "huyết áp của tôi hơi cao",
        ],
    );
    store
        .set_setting("vocabulary.packs", &serde_json::json!(["medical-vi"]))
        .unwrap();
    let prompt = local_prompt(&store, &m, "vi");
    assert!(
        prompt.contains("Hãy viết đúng như sau: paracetamol; huyết áp\n"),
        "{prompt}"
    );
}

#[test]
fn without_terms_there_is_no_block() {
    let (_t, store, m) = fixture("en", &["hello there everyone"]);
    let prompt = local_prompt(&store, &m, "en");
    assert!(!prompt.contains("exactly like this"), "{prompt}");
}

#[test]
fn a_cloud_request_never_has_the_spellings() {
    let (_t, store, m) = fixture("en", &["we run nemotron on the metformin study today"]);
    store
        .set_setting("vocabulary", &serde_json::json!(["Nemotron"]))
        .unwrap();
    store
        .set_setting("vocabulary.packs", &serde_json::json!(["medical-en"]))
        .unwrap();
    // The same meeting, locally: the block is there.
    assert!(
        local_prompt(&store, &m, "en")
            .contains("Write them exactly like this: Nemotron; metformin")
    );
    // To the cloud: the transcript text only. Neither the block nor the user's capitalized spelling.
    let Planned::Send(p) = plan(
        &store,
        &m,
        CloudProvider::preset("openai", "gpt-4.1-mini").unwrap(),
        Task::Notes {
            template: ghi_llm::template::builtin("general").unwrap(),
            lang: OutLang::En,
        },
        true,
        &[],
        &Prices::builtin(),
    )
    .unwrap() else {
        panic!("a request")
    };
    let payload = &p.preview.payload;
    assert!(
        payload.contains("nemotron"),
        "the transcript itself is sent"
    );
    assert!(!payload.contains("Nemotron"), "{payload}");
    assert!(
        !payload.contains("exactly like this") && !payload.contains("terms are said"),
        "{payload}"
    );
}
