// SPDX-License-Identifier: Apache-2.0
//! The notes job asks for a one-sentence headline and up to three tags. The
//! headline becomes the library row's line, the tags are kept as suggestions
//! (`tag_hint` blocks, never applied). The local prompt lists the user's tags
//! (most used first) so the model reuses them; a cloud request never does.

use std::sync::{Arc, Mutex};

use ghi_core::cloud::{Planned, Task, plan};
use ghi_core::events::bus;
use ghi_core::jobs::{JobRunner, Outcome, always_ready};
use ghi_core::notes_job::{HEADLINE_KIND, NOTES_FINAL_JOB, NotesJob, TAG_HINT_KIND};
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
            text: r##"{"tldr":[{"text":"Pricing moves to twelve dollars","cite":[0]}],"decisions":[],"action_items":[],"open_questions":[],"key_quotes":[],"topics":[],
                "headline":"The team agreed to raise the Pro price to twelve dollars.",
                "tags":["Pricing","#acme"]}"##
                .into(),
            tokens_in: 10,
            tokens_out: 5,
            truncated: false,
        })
    }
}

fn fixture() -> (tempfile::TempDir, Arc<Store>, String) {
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
            [
                "we raise the pro price to twelve dollars for acme",
                "agreed",
            ]
            .iter()
            .enumerate()
            .map(|(i, t)| NewSegment {
                t0_ms: i as i64 * 5000,
                t1_ms: i as i64 * 5000 + 4000,
                text: (*t).into(),
                lang: Some("en".into()),
                ..Default::default()
            })
            .collect(),
        )
        .unwrap();
    (tmp, store, m)
}

/// Runs the notes job on `m`; returns the local prompt it sent.
fn run_notes(store: &Arc<Store>, m: &str) -> String {
    store
        .enqueue_job(
            Some(m),
            NOTES_FINAL_JOB,
            JOB_PAYLOAD_VERSION,
            &serde_json::json!({ "lang": "en" }),
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
    seen.lock().unwrap()[0].messages[1].content.clone()
}

#[test]
fn the_headline_is_the_row_line_and_the_tags_are_kept_as_suggestions() {
    let (_t, store, m) = fixture();
    // Clients is on two meetings, Recruiting on none: Clients comes first.
    let other = store.create_meeting(NewMeeting::default()).unwrap().gid;
    let clients = store.create_tag("Clients").unwrap();
    store
        .tag_meetings(&[m.clone(), other], &clients.gid)
        .unwrap();
    store.create_tag("Recruiting").unwrap();

    let prompt = run_notes(&store, &m);
    assert!(
        prompt.contains("Tags already in use (reuse one when it fits): Clients, Recruiting\n"),
        "{prompt}"
    );
    assert!(prompt.contains("- headline: one sentence"), "{prompt}");

    let blocks = store.note_blocks(&m).unwrap();
    let of = |kind: &str| -> Vec<String> {
        blocks
            .iter()
            .filter(|b| b.kind == kind)
            .map(|b| b.body.clone())
            .collect()
    };
    assert_eq!(
        of(HEADLINE_KIND),
        ["The team agreed to raise the Pro price to twelve dollars."]
    );
    assert_eq!(of(TAG_HINT_KIND), ["Pricing", "Acme"]);
    // Suggestions only: the meeting still has just the tag the user gave it.
    let on = &store.meeting_tags(std::slice::from_ref(&m)).unwrap()[&m];
    assert_eq!(on.len(), 1);
    assert_eq!(
        store.row_summaries(std::slice::from_ref(&m)).unwrap()[&m],
        "The team agreed to raise the Pro price to twelve dollars."
    );
}

#[test]
fn a_cloud_request_asks_for_tags_but_never_carries_the_users_tags() {
    let (_t, store, m) = fixture();
    store.create_tag("Recruiting").unwrap();
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
    assert!(payload.contains("- tags: 1 to 3 short tags"), "{payload}");
    assert!(
        !payload.contains("Recruiting") && !payload.contains("already in use"),
        "{payload}"
    );
}
