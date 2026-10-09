// SPDX-License-Identifier: Apache-2.0
//! Cloud AI (doc 02 §K) and "Ask this meeting" (D9-lite).
//!
//! - API keys go from a masked field straight into the Keychain; nothing
//!   ever reads one back to the webview (only "stored: yes/no").
//! - "Improve with cloud…" / a cloud Ask is two steps: `cloud_preview` builds
//!   the exact request (redacted, text only) and keeps it here under an id;
//!   the sheet shows it; `cloud_send` sends exactly those bytes
//!   (`ghi_core::cloud`). A failed notes request falls back to the local
//!   model (a regenerate is queued). Strict offline refuses every send.
//! - Ask on this device uses the local model; refused while recording or
//!   while notes are being written (one model in memory at a time).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ghi_core::cloud::{self, Outcome, Planned, Sent, Task};
use ghi_llm::ask::Answer;
use ghi_llm::cloud::CloudProvider;
use ghi_llm::preview::Prices;
use ghi_llm::template::OutLang;
use ghi_net::{NetPolicy, Secret};
use ghi_store::anchors::Anchor;
use ghi_store::store::{NewNoteBlock, Provenance, Segment, Store};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::core::Core;
use crate::detail::{Citation, NotesLanguage};
use crate::{CoreState, blocking};

/// How long a reviewed request may wait for its confirmation.
const PLAN_TTL: Duration = Duration::from_secs(10 * 60);
pub const PROVIDERS: [&str; 3] = ["openai", "anthropic", "gemini"];

fn err(e: ghi_store::StoreError) -> String {
    e.to_string()
}

#[derive(Default)]
pub struct CloudPlans(Mutex<HashMap<String, (Box<cloud::Plan>, Instant)>>);

impl CloudPlans {
    /// Keeps `plan` as the meeting's only sendable preview.
    fn put(&self, plan: Box<cloud::Plan>) -> String {
        let mut b = [0u8; 16];
        OsRng.fill_bytes(&mut b);
        let id: String = b.iter().map(|x| format!("{x:02x}")).collect();
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        m.retain(|_, (p, at)| at.elapsed() < PLAN_TTL && p.meeting != plan.meeting);
        m.insert(id.clone(), (plan, Instant::now()));
        id
    }

    fn provider_of(&self, id: &str) -> Option<String> {
        let m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        m.get(id).map(|(p, _)| p.preview.provider.clone())
    }

    fn meeting_of(&self, id: &str) -> Option<String> {
        let m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        m.get(id).map(|(p, _)| p.meeting.clone())
    }

    fn take(&self, id: &str) -> Option<Box<cloud::Plan>> {
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        m.remove(id)
            .filter(|(_, at)| at.elapsed() < PLAN_TTL)
            .map(|(p, _)| p)
    }
}

// ------------------------------------------------------------------ keys

/// Keystore account of a provider's key (same as the CLI's).
fn account(provider: &str) -> Result<String, String> {
    if PROVIDERS.contains(&provider) {
        Ok(format!("provider-{provider}"))
    } else {
        Err(format!("unknown provider `{provider}`"))
    }
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderKey {
    pub provider: String,
    pub stored: bool,
}

/// Any provider has a stored key (the `cloudOffered` migration; a keychain
/// error counts as none).
pub(crate) fn any_key_stored(c: &Core) -> bool {
    c.secrets().is_ok_and(|s| {
        PROVIDERS
            .iter()
            .any(|p| s.get(&format!("provider-{p}")).ok().flatten().is_some())
    })
}

/// Which providers have a key (never the key).
#[tauri::command]
#[specta::specta]
pub async fn cloud_keys(core: CoreState<'_>) -> Result<Vec<ProviderKey>, String> {
    blocking(&core, move |c| {
        let s = c.secrets()?;
        PROVIDERS
            .iter()
            .map(|p| {
                Ok(ProviderKey {
                    provider: p.to_string(),
                    stored: s.get(&account(p)?).map_err(err)?.is_some(),
                })
            })
            .collect()
    })
    .await
}

/// Stores a provider's API key in the Keychain (from a masked field).
#[tauri::command]
#[specta::specta]
pub async fn set_cloud_key(
    core: CoreState<'_>,
    provider: String,
    key: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        let key = Secret::new(key.trim());
        if key.expose().is_empty() || key.expose().len() > 512 {
            return Err("that doesn't look like an API key".into());
        }
        c.secrets()?
            .set(&account(&provider)?, key.expose().as_bytes())
            .map_err(err)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_cloud_key(core: CoreState<'_>, provider: String) -> Result<(), String> {
    blocking(&core, move |c| {
        c.secrets()?.delete(&account(&provider)?).map_err(err)
    })
    .await
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CloudModel {
    pub provider: String,
    pub model: String,
    /// USD per 1M input / output tokens today (an introductory price while it
    /// lasts), for the menus.
    pub input_usd_per_m: Option<f64>,
    pub output_usd_per_m: Option<f64>,
}

/// The models with a known price, per provider (the sheet's menus).
#[tauri::command]
#[specta::specta]
pub fn cloud_models() -> Vec<CloudModel> {
    let prices = Prices::builtin();
    prices
        .models()
        .into_iter()
        .map(|(provider, model)| {
            let price = prices.lookup(&provider, &model);
            CloudModel {
                input_usd_per_m: price.map(|p| p.0),
                output_usd_per_m: price.map(|p| p.1),
                provider,
                model,
            }
        })
        .collect()
}

// ------------------------------------------------------- preview / send

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CloudTask {
    /// Rewrite the notes ("Improve with cloud…").
    Notes {
        template: Option<String>,
        language: NotesLanguage,
    },
    /// Ask this meeting.
    Ask {
        question: String,
        language: NotesLanguage,
    },
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CloudAsk {
    pub provider: String,
    pub model: String,
    pub task: CloudTask,
    /// Replace names and personal data with placeholders (restored locally).
    pub redact: bool,
    /// More names to hide besides the speakers'.
    pub extra_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Redaction {
    pub kind: String,
    pub count: u32,
}

/// The send preview the sheet shows (nothing has left the device).
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CloudPreview {
    /// Confirm with `cloud_send(id)`.
    pub id: String,
    pub provider: String,
    pub model: String,
    pub host: String,
    /// The request body, byte for byte.
    pub payload: String,
    pub sha256: String,
    pub tokens_est: u32,
    pub cost_est_usd: Option<f64>,
    /// The most it can cost (all of the answer allowance used).
    pub cost_max_usd: Option<f64>,
    pub retention_note: String,
    /// Things in the text that still look like personal data.
    pub warnings: Vec<String>,
    pub redactions: Vec<Redaction>,
    /// The request's text as it is on this Mac (names and personal data
    /// restored), shortened; `None` when it has no user text.
    pub excerpt_before: Option<String>,
    /// The same text as sent (placeholders for what is hidden).
    pub excerpt_after: Option<String>,
}

/// An answer to "Ask this meeting".
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AskAnswer {
    /// False: "Not discussed in this meeting".
    pub answered: bool,
    pub text: String,
    pub citations: Vec<Citation>,
    /// The terms looked for (not discussed).
    pub searched: Vec<String>,
    /// `local` or the cloud provider.
    pub engine: String,
    /// What to pass to `save_answer`: the core keeps the answer under it for
    /// a while (none for "not discussed"). The webview never sends the text.
    pub id: Option<String>,
}

/// Answers kept per meeting for "Save to notes".
pub const ANSWERS_KEPT: usize = 10;
/// Answers saved into one meeting's notes at most (prompts stay bounded).
pub const ANSWERS_SAVED_MAX: usize = 10;
/// Block kind of a saved answer.
pub const ANSWER_KIND: &str = "answer";

/// An answer the core made, waiting to be saved if the user asks.
#[derive(Debug, Clone)]
pub struct AnswerDraft {
    pub meeting: String,
    pub question: String,
    pub text: String,
    /// The cited lines as time anchors, from the core's own transcript.
    pub anchors: Vec<Anchor>,
}

/// The last [`ANSWERS_KEPT`] answers of each meeting, in memory only.
#[derive(Default)]
pub struct AnswerCache(Mutex<VecDeque<(String, AnswerDraft)>>);

impl AnswerCache {
    fn put(&self, d: AnswerDraft) -> String {
        let mut b = [0u8; 16];
        OsRng.fill_bytes(&mut b);
        let id: String = b.iter().map(|x| format!("{x:02x}")).collect();
        let mut q = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let meeting = d.meeting.clone();
        q.push_back((id.clone(), d));
        while q.iter().filter(|(_, x)| x.meeting == meeting).count() > ANSWERS_KEPT {
            if let Some(i) = q.iter().position(|(_, x)| x.meeting == meeting) {
                q.remove(i);
            }
        }
        id
    }

    /// The answer made for `meeting` under `id` (a forged or expired id, or
    /// another meeting's, finds nothing).
    fn get(&self, meeting: &str, id: &str) -> Option<AnswerDraft> {
        let q = self.0.lock().unwrap_or_else(|e| e.into_inner());
        q.iter()
            .find(|(i, d)| i == id && d.meeting == meeting)
            .map(|(_, d)| d.clone())
    }

    fn forget(&self, id: &str) {
        let mut q = self.0.lock().unwrap_or_else(|e| e.into_inner());
        q.retain(|(i, _)| i != id);
    }
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum CloudPreviewResult {
    Preview(CloudPreview),
    /// An Ask that nothing matched: answered without any request.
    Answer(AskAnswer),
}

pub fn out_lang(l: NotesLanguage, store: &Store, meeting: &str) -> Result<OutLang, String> {
    Ok(match l {
        NotesLanguage::En => OutLang::En,
        NotesLanguage::Vi => OutLang::Vi,
        NotesLanguage::Meeting => {
            let (t, _) = ghi_core::notes_job::stored_transcript(store, meeting)?;
            OutLang::resolve("meeting", &t).unwrap_or(OutLang::En)
        }
    })
}

/// Citations of an answer (segment positions) as time spans with quotes. A
/// real answer is also kept in the core's cache (with its cited lines as
/// anchors) so "Save to notes" can name it by id.
fn answer_view(
    c: &Core,
    meeting: &str,
    question: &str,
    a: Answer,
    segs: &[Segment],
    engine: &str,
) -> Result<AskAnswer, String> {
    let version = c
        .store()?
        .get_meeting(meeting)
        .map_err(err)?
        .transcript_version;
    Ok(match a {
        Answer::Answered { text, citations } => {
            let cited: Vec<&Segment> = citations
                .iter()
                .filter_map(|&i| segs.get(i as usize))
                .collect();
            let id = c.answers().put(AnswerDraft {
                meeting: meeting.to_string(),
                question: question.to_string(),
                text: text.clone(),
                anchors: cited
                    .iter()
                    .map(|s| Anchor {
                        meeting_gid: meeting.to_string(),
                        t0_ms: s.t0_ms,
                        t1_ms: s.t1_ms,
                        transcript_version: version,
                    })
                    .collect(),
            });
            AskAnswer {
                answered: true,
                text,
                citations: cited
                    .iter()
                    .map(|s| crate::detail::segment_citation(s, version))
                    .collect(),
                searched: Vec::new(),
                engine: engine.into(),
                id: Some(id),
            }
        }
        Answer::NotDiscussed { searched } => AskAnswer {
            answered: false,
            text: String::new(),
            citations: Vec::new(),
            searched,
            engine: engine.into(),
            id: None,
        },
    })
}

/// Builds the exact request for the sheet (nothing is sent).
#[tauri::command]
#[specta::specta]
pub async fn cloud_preview(
    core: CoreState<'_>,
    plans: tauri::State<'_, Arc<CloudPlans>>,
    meeting: String,
    ask: CloudAsk,
) -> Result<CloudPreviewResult, String> {
    let plans = plans.inner().clone();
    blocking(&core, move |c| {
        let settings = crate::system::load_settings(c)?;
        crate::system::cloud_allowed(&settings)?;
        if settings.strict_offline {
            return Err("strict offline is on: nothing can be sent".into());
        }
        let store = c.store()?;
        let provider =
            CloudProvider::preset(&ask.provider, &ask.model).map_err(|e| e.to_string())?;
        let task = match ask.task {
            CloudTask::Notes { template, language } => {
                let id = template
                    .or(store.get_meeting(&meeting).map_err(err)?.template)
                    .unwrap_or_else(|| "general".into());
                Task::Notes {
                    template: ghi_llm::template::builtin(&id).map_err(|e| e.to_string())?,
                    lang: out_lang(language, &store, &meeting)?,
                }
            }
            CloudTask::Ask { question, language } => Task::Ask {
                question: question.chars().take(1000).collect(),
                lang: out_lang(language, &store, &meeting)?,
            },
        };
        match cloud::plan(
            &store,
            &meeting,
            provider,
            task,
            ask.redact,
            &ask.extra_names,
            &Prices::builtin(),
        )? {
            Planned::NotDiscussed(a) => {
                let (_, segs) = ghi_core::notes_job::stored_transcript(&store, &meeting)?;
                Ok(CloudPreviewResult::Answer(answer_view(
                    c, &meeting, "", a, &segs, "local",
                )?))
            }
            Planned::Send(p) => {
                let pv = p.preview.clone();
                let redactions = p
                    .redactions
                    .iter()
                    .map(|(kind, n)| Redaction {
                        kind: kind.to_string(),
                        count: *n as u32,
                    })
                    .collect();
                let (excerpt_before, excerpt_after) = match p.excerpts() {
                    Some((b, a)) => (Some(b), Some(a)),
                    None => (None, None),
                };
                let id = plans.put(p);
                Ok(CloudPreviewResult::Preview(CloudPreview {
                    id,
                    provider: pv.provider,
                    model: pv.model,
                    host: pv.host,
                    payload: pv.payload,
                    sha256: pv.sha256,
                    tokens_est: pv.tokens_est,
                    cost_est_usd: pv.cost_est_usd,
                    cost_max_usd: pv.cost_max_usd,
                    retention_note: pv.retention_note,
                    warnings: pv.warnings,
                    redactions,
                    excerpt_before,
                    excerpt_after,
                }))
            }
        }
    })
    .await
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum CloudSendResult {
    /// The notes were rewritten (what the user wrote stays).
    Notes,
    Answer(AskAnswer),
    /// The provider failed: notes are being written on this device instead
    /// (an Ask is answered on this device by the caller).
    Failed {
        reason: String,
        /// The request may have left the device (it is in the request log).
        left_device: bool,
    },
}

/// Sends the previewed request, exactly.
#[tauri::command]
#[specta::specta]
pub async fn cloud_send(
    core: CoreState<'_>,
    plans: tauri::State<'_, Arc<CloudPlans>>,
    id: String,
) -> Result<CloudSendResult, String> {
    let plans = plans.inner().clone();
    blocking(&core, move |c| {
        let settings = crate::system::load_settings(c)?;
        crate::system::cloud_allowed(&settings)?;
        let policy = if settings.strict_offline {
            NetPolicy::StrictOffline
        } else {
            NetPolicy::Default
        };
        let store = c.store()?;
        let provider = plans
            .provider_of(&id)
            .ok_or("this preview expired: review it again")?;
        // Checked before the preview is used up: adding the key keeps it.
        let key = c
            .secrets()?
            .get(&account(&provider)?)
            .map_err(err)?
            .ok_or_else(|| format!("no API key for {provider}: add one in Settings → AI"))?;
        let key = Secret::new(std::str::from_utf8(&key).map_err(|_| "the stored key is not text")?);
        let meeting = plans
            .meeting_of(&id)
            .ok_or("this preview expired: review it again")?;
        if notes_running(&store, &meeting)? {
            return Err(
                "notes are being written for this meeting: try again when they're done".into(),
            );
        }
        let plan = plans
            .take(&id)
            .ok_or("this preview expired: review it again")?;
        let is_notes = plan.is_notes();
        let question = plan.ask_question().unwrap_or_default().to_string();
        match cloud::send(&store, *plan, &key, policy)? {
            Outcome::Done(Sent::Notes(_)) => Ok(CloudSendResult::Notes),
            Outcome::Done(Sent::Answer(a)) => {
                let (_, segs) = ghi_core::notes_job::stored_transcript(&store, &meeting)?;
                Ok(CloudSendResult::Answer(answer_view(
                    c, &meeting, &question, a, &segs, &provider,
                )?))
            }
            Outcome::Failed {
                reason,
                left_device,
            } => {
                if is_notes {
                    queue_local_notes(c, &store, &meeting)?;
                }
                Ok(CloudSendResult::Failed {
                    reason,
                    left_device,
                })
            }
        }
    })
    .await
}

/// A notes job is running or queued for the meeting.
fn notes_running(store: &Store, meeting: &str) -> Result<bool, String> {
    for kind in [
        ghi_core::session::NOTES_LIVE_JOB,
        ghi_core::notes_job::NOTES_FINAL_JOB,
    ] {
        if store.active_job(meeting, kind).map_err(err)?.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The local model writes the notes instead (a failed cloud request).
fn queue_local_notes(c: &Core, store: &Store, meeting: &str) -> Result<(), String> {
    let job = ghi_core::notes_job::NOTES_FINAL_JOB;
    if store.active_job(meeting, job).map_err(err)?.is_none() {
        store
            .enqueue_job(
                Some(meeting),
                job,
                ghi_core::session::JOB_PAYLOAD_VERSION,
                &serde_json::json!({}),
            )
            .map_err(err)?;
        store
            .set_meeting_status(meeting, "processing")
            .map_err(err)?;
        c.notify_jobs();
    }
    Ok(())
}

/// "Never send to cloud" for a meeting.
#[tauri::command]
#[specta::specta]
pub async fn set_meeting_cloud_locked(
    core: CoreState<'_>,
    meeting: String,
    locked: bool,
) -> Result<(), String> {
    blocking(&core, move |c| {
        c.store()?.set_cloud_locked(&meeting, locked).map_err(err)
    })
    .await
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CloudLogEntry {
    pub meeting: String,
    pub meeting_title: String,
    pub provider: String,
    pub model: String,
    pub tokens_in: f64,
    pub tokens_out: f64,
    /// Unix ms.
    pub at: f64,
}

/// Every cloud request made, newest first (no content is kept).
#[tauri::command]
#[specta::specta]
pub async fn cloud_request_log(
    core: CoreState<'_>,
    limit: u32,
) -> Result<Vec<CloudLogEntry>, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let mut titles: HashMap<String, String> = HashMap::new();
        store
            .cloud_requests(limit.clamp(1, 1000) as usize)
            .map_err(err)?
            .into_iter()
            .map(|r| {
                let title = match titles.get(&r.meeting_gid) {
                    Some(t) => t.clone(),
                    None => {
                        let t = store
                            .get_meeting(&r.meeting_gid)
                            .map(|m| m.title)
                            .unwrap_or_default();
                        titles.insert(r.meeting_gid.clone(), t.clone());
                        t
                    }
                };
                Ok(CloudLogEntry {
                    meeting: r.meeting_gid,
                    meeting_title: title,
                    provider: r.provider,
                    model: r.model,
                    tokens_in: r.tokens_in as f64,
                    tokens_out: r.tokens_out as f64,
                    at: r.at as f64,
                })
            })
            .collect()
    })
    .await
}

// ------------------------------------------------------------------ ask

/// Error codes of [`local_model_free`] the UI turns into words
/// (`ask.busy.*`).
pub const BUSY_RECORDING: &str = "busyRecording";
pub const BUSY_NOTES: &str = "busyNotes";
pub const NO_MODEL: &str = "noModel";

/// The local model can run now: installed, not recording, no notes job
/// using it (one model in memory at a time). Errors are the codes above.
pub fn local_model_free(c: &Core, store: &Store) -> Result<(), String> {
    if c.recording() {
        return Err(BUSY_RECORDING.into());
    }
    let running = store.active_jobs().map_err(err)?;
    if running.iter().any(|j| {
        j.state == ghi_store::jobs::JobState::Running
            && (j.kind == ghi_core::session::NOTES_LIVE_JOB
                || j.kind == ghi_core::notes_job::NOTES_FINAL_JOB)
    }) {
        return Err(BUSY_NOTES.into());
    }
    if !crate::core::llm_ready(&c.models()) {
        return Err(NO_MODEL.into());
    }
    Ok(())
}

/// Ask this meeting, answered on this device by the local model.
#[tauri::command]
#[specta::specta]
pub async fn ask_meeting(
    core: CoreState<'_>,
    meeting: String,
    question: String,
    language: NotesLanguage,
) -> Result<AskAnswer, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        local_model_free(c, &store)?;
        let question: String = question.chars().take(1000).collect();
        if question.trim().is_empty() {
            return Err("ask a question".into());
        }
        let (t, segs) = ghi_core::notes_job::stored_transcript(&store, &meeting)?;
        if t.is_empty() {
            return Err("this meeting has no transcript yet".into());
        }
        let lang = out_lang(language, &store, &meeting)?;
        let bytes: usize = segs.iter().map(|s| s.text.len()).sum();
        let mut llm = (c.llm()?)(bytes)?;
        let run =
            ghi_llm::ask::ask(llm.as_mut(), &t, &question, lang).map_err(|e| e.to_string())?;
        drop(llm);
        answer_view(c, &meeting, &question, run.answer, &segs, "local")
    })
    .await
}

// ------------------------------------------------------- follow-up email

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum EmailTone {
    Friendly,
    Neutral,
    Formal,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EmailDraft {
    pub subject: String,
    pub body: String,
}

/// A follow-up email from the meeting's notes, written on this device (to
/// edit and copy; nothing is sent).
#[tauri::command]
#[specta::specta]
pub async fn draft_followup_email(
    core: CoreState<'_>,
    meeting: String,
    language: NotesLanguage,
    tone: EmailTone,
) -> Result<EmailDraft, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        local_model_free(c, &store)?;
        let lang = out_lang(language, &store, &meeting)?;
        let tone = match tone {
            EmailTone::Friendly => ghi_core::email::Tone::Friendly,
            EmailTone::Neutral => ghi_core::email::Tone::Neutral,
            EmailTone::Formal => ghi_core::email::Tone::Formal,
        };
        // The notes are short: a small context is enough.
        let mut llm = (c.llm()?)(16_000)?;
        let e = ghi_core::email::draft(&store, &meeting, llm.as_mut(), lang, tone)?;
        Ok(EmailDraft {
            subject: e.subject,
            body: e.body,
        })
    })
    .await
}

// ------------------------------------------------------- save an answer

/// Why a save was refused, as stable words the webview words itself.
pub const ANSWER_EXPIRED: &str = "answerExpired";
pub const ANSWER_LIMIT: &str = "answerLimit";

/// Writes the answer kept under `id` into the meeting's notes as a pinned AI
/// block of kind `answer` ("Q: … A: …", anchored to the cited lines). Pinned,
/// so Regenerate keeps it. Returns the new block's gid.
pub(crate) fn save_answer_now(c: &Core, meeting: &str, id: &str) -> Result<String, String> {
    // Refused while the app is locked, like every content command.
    let store = c.store()?;
    let draft = c
        .answers()
        .get(meeting, id)
        .ok_or_else(|| ANSWER_EXPIRED.to_string())?;
    let saved = store
        .note_blocks(meeting)
        .map_err(err)?
        .iter()
        .filter(|b| b.kind == ANSWER_KIND)
        .count();
    if saved >= ANSWERS_SAVED_MAX {
        return Err(ANSWER_LIMIT.into());
    }
    let block = store
        .add_note_block(
            meeting,
            NewNoteBlock {
                kind: ANSWER_KIND.into(),
                provenance: Provenance::Ai,
                body: format!("Q: {}\nA: {}", draft.question.trim(), draft.text.trim()),
                anchors: draft.anchors,
                pinned: true,
            },
        )
        .map_err(err)?;
    c.answers().forget(id);
    Ok(block.gid)
}

/// "Save to notes" on an Ask answer, by the id the answer came with.
#[tauri::command]
#[specta::specta]
pub async fn save_answer(
    core: CoreState<'_>,
    meeting: String,
    answer_id: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        save_answer_now(c, &meeting, &answer_id).map(|_| ())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_store::store::{NewMeeting, NewSegment};

    struct Fix {
        _tmp: tempfile::TempDir,
        core: Arc<Core>,
        meeting: String,
    }

    fn fix() -> Fix {
        let tmp = tempfile::tempdir().unwrap();
        let (core, _rx) = Core::for_test(tmp.path().join("data"));
        let store = core.store().unwrap();
        let meeting = store
            .create_meeting(NewMeeting {
                title: "x".into(),
                ..Default::default()
            })
            .unwrap()
            .gid;
        store
            .add_segments(
                &meeting,
                vec![NewSegment {
                    t0_ms: 1000,
                    t1_ms: 4000,
                    text: "we ship on the 12th".into(),
                    ..Default::default()
                }],
            )
            .unwrap();
        drop(store);
        Fix {
            _tmp: tmp,
            core,
            meeting,
        }
    }

    fn draft(f: &Fix, n: usize) -> AnswerDraft {
        AnswerDraft {
            meeting: f.meeting.clone(),
            question: format!("When do we ship {n}?"),
            text: "On the 12th.".into(),
            anchors: vec![Anchor {
                meeting_gid: f.meeting.clone(),
                t0_ms: 1000,
                t1_ms: 4000,
                transcript_version: 1,
            }],
        }
    }

    fn answers(f: &Fix) -> Vec<ghi_store::store::NoteBlock> {
        f.core
            .store()
            .unwrap()
            .note_blocks(&f.meeting)
            .unwrap()
            .into_iter()
            .filter(|b| b.kind == ANSWER_KIND)
            .collect()
    }

    #[test]
    fn saves_a_pinned_ai_block_with_the_cores_anchors() {
        let f = fix();
        let id = f.core.answers().put(draft(&f, 1));
        save_answer_now(&f.core, &f.meeting, &id).unwrap();
        let b = answers(&f);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].body, "Q: When do we ship 1?\nA: On the 12th.");
        assert!(b[0].pinned);
        assert_eq!(b[0].provenance, Provenance::Ai);
        assert_eq!((b[0].anchors[0].t0_ms, b[0].anchors[0].t1_ms), (1000, 4000));
        // Saved once: the id is used up.
        assert_eq!(save_answer_now(&f.core, &f.meeting, &id), Err(ANSWER_EXPIRED.into()));
        assert_eq!(answers(&f).len(), 1);
    }

    #[test]
    fn an_unknown_forged_or_other_meetings_id_is_refused() {
        let f = fix();
        assert_eq!(
            save_answer_now(&f.core, &f.meeting, "nope"),
            Err(ANSWER_EXPIRED.into())
        );
        let id = f.core.answers().put(draft(&f, 1));
        assert_eq!(
            save_answer_now(&f.core, "another-meeting", &id),
            Err(ANSWER_EXPIRED.into())
        );
        assert!(answers(&f).is_empty());
    }

    #[test]
    fn only_the_last_ten_answers_of_a_meeting_are_kept() {
        let f = fix();
        let ids: Vec<String> = (0..11).map(|n| f.core.answers().put(draft(&f, n))).collect();
        assert_eq!(
            save_answer_now(&f.core, &f.meeting, &ids[0]),
            Err(ANSWER_EXPIRED.into())
        );
        save_answer_now(&f.core, &f.meeting, &ids[10]).unwrap();
    }

    #[test]
    fn at_most_ten_saved_answers_per_meeting() {
        let f = fix();
        for n in 0..ANSWERS_SAVED_MAX {
            let id = f.core.answers().put(draft(&f, n));
            save_answer_now(&f.core, &f.meeting, &id).unwrap();
        }
        let id = f.core.answers().put(draft(&f, 99));
        assert_eq!(
            save_answer_now(&f.core, &f.meeting, &id),
            Err(ANSWER_LIMIT.into())
        );
        assert_eq!(answers(&f).len(), ANSWERS_SAVED_MAX);
    }

    #[test]
    fn a_locked_app_refuses_and_writes_nothing() {
        let f = fix();
        let id = f.core.answers().put(draft(&f, 1));
        f.core.set_locked(true);
        assert!(save_answer_now(&f.core, &f.meeting, &id).is_err());
        f.core.set_locked(false);
        assert!(answers(&f).is_empty());
        // The answer is still there to save once unlocked.
        save_answer_now(&f.core, &f.meeting, &id).unwrap();
    }

    #[test]
    fn a_saved_answer_survives_regenerate() {
        let f = fix();
        let id = f.core.answers().put(draft(&f, 1));
        save_answer_now(&f.core, &f.meeting, &id).unwrap();
        let store = f.core.store().unwrap();
        store
            .replace_ai_notes(
                &f.meeting,
                vec![NewNoteBlock {
                    kind: "tldr".into(),
                    provenance: Provenance::Ai,
                    body: "fresh".into(),
                    anchors: vec![],
                    pinned: false,
                }],
                vec![],
            )
            .unwrap();
        let kinds: Vec<String> = store
            .note_blocks(&f.meeting)
            .unwrap()
            .into_iter()
            .map(|b| b.kind)
            .collect();
        assert!(kinds.contains(&ANSWER_KIND.to_string()), "{kinds:?}");
        assert!(kinds.contains(&"tldr".to_string()));
    }
}
