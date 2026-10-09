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
    /// The notes use the built-in General template because the one asked for
    /// is one of yours: its instructions never leave this device.
    pub template_fallback: bool,
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
/// Answers kept in all, whatever the meetings.
pub const ANSWERS_KEPT_TOTAL: usize = 50;
/// How long an answer waits to be saved.
const ANSWER_TTL: Duration = Duration::from_secs(30 * 60);
/// Answers saved into one meeting's notes at most (prompts stay bounded).
pub const ANSWERS_SAVED_MAX: usize = 10;
/// A saved answer's question and answer are cut to this many characters.
pub const SAVED_QUESTION_CHARS: usize = 300;
pub const SAVED_ANSWER_CHARS: usize = 1200;
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

/// An answer taken out of the cache to be saved: put it back if the save
/// fails ([`AnswerCache::give_back`]).
pub struct TakenAnswer {
    id: String,
    draft: AnswerDraft,
    at: Instant,
}

/// The recent answers of each meeting, in memory only: gone after
/// [`ANSWER_TTL`], when the app locks, and when their meeting is deleted.
#[derive(Default)]
pub struct AnswerCache(Mutex<VecDeque<(String, AnswerDraft, Instant)>>);

impl AnswerCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<(String, AnswerDraft, Instant)>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn put(&self, d: AnswerDraft) -> String {
        let mut b = [0u8; 16];
        OsRng.fill_bytes(&mut b);
        let id: String = b.iter().map(|x| format!("{x:02x}")).collect();
        let mut q = self.lock();
        q.retain(|(_, _, at)| at.elapsed() < ANSWER_TTL);
        let meeting = d.meeting.clone();
        q.push_back((id.clone(), d, Instant::now()));
        while q.iter().filter(|(_, x, _)| x.meeting == meeting).count() > ANSWERS_KEPT {
            if let Some(i) = q.iter().position(|(_, x, _)| x.meeting == meeting) {
                q.remove(i);
            }
        }
        while q.len() > ANSWERS_KEPT_TOTAL {
            q.pop_front();
        }
        id
    }

    /// Takes the answer made for `meeting` under `id` out of the cache (a
    /// forged, used, expired or other meeting's id finds nothing). Two saves
    /// at once cannot both get it.
    fn take(&self, meeting: &str, id: &str) -> Option<TakenAnswer> {
        let mut q = self.lock();
        let i = q
            .iter()
            .position(|(i, d, at)| i == id && d.meeting == meeting && at.elapsed() < ANSWER_TTL)?;
        let (id, draft, at) = q.remove(i)?;
        Some(TakenAnswer { id, draft, at })
    }

    /// Puts a taken answer back (the save did not happen).
    fn give_back(&self, t: TakenAnswer) {
        self.lock().push_front((t.id, t.draft, t.at));
    }

    /// Forgets everything (the app locked, or all data was deleted).
    pub fn clear(&self) {
        self.lock().clear();
    }

    /// Forgets a meeting's answers (it was deleted).
    pub fn forget_meeting(&self, meeting: &str) {
        self.lock().retain(|(_, d, _)| d.meeting != meeting);
    }

    /// Forgets the answers of every meeting `exists` says is gone (deleted on
    /// another device and synced here).
    pub fn prune_missing(&self, exists: &dyn Fn(&str) -> bool) {
        self.lock().retain(|(_, d, _)| exists(&d.meeting));
    }

    /// Whether any answer of `meeting` is waiting.
    #[cfg(test)]
    pub(crate) fn has_meeting(&self, meeting: &str) -> bool {
        self.lock().iter().any(|(_, d, _)| d.meeting == meeting)
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
    // The lines the answer's numbers point into, and the transcript version
    // they were read at (one snapshot: a final pass in between cannot shift them).
    segs: &[Segment],
    version: i64,
    engine: &str,
) -> Result<AskAnswer, String> {
    Ok(match a {
        Answer::Answered { text, citations } => {
            let cited: Vec<&Segment> = citations
                .iter()
                .filter_map(|&i| segs.get(i as usize))
                .collect();
            // The phone has no "save to notes": nothing is kept for it.
            let id = c.keeps_answers().then(|| {
                c.answers().put(AnswerDraft {
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
                })
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
                id,
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

/// The template a cloud notes request uses: the one asked for, else the
/// meeting's own, else General. One of the user's own (`user:<gid>`) is never
/// sent, instructions and all: General goes instead, and the flag says so for
/// the preview.
pub(crate) fn cloud_notes_template(
    store: &Store,
    meeting: &str,
    asked: Option<String>,
) -> Result<(ghi_llm::template::Template, bool), String> {
    let id = asked
        .or(store.get_meeting(meeting).map_err(err)?.template)
        .unwrap_or_else(|| "general".into());
    if ghi_core::user_templates::gid_of(&id).is_some() {
        let general = ghi_llm::template::builtin("general").map_err(|e| e.to_string())?;
        return Ok((general, true));
    }
    Ok((
        ghi_llm::template::builtin(&id).map_err(|e| e.to_string())?,
        false,
    ))
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
        let mut template_fallback = false;
        let task = match ask.task {
            CloudTask::Notes { template, language } => {
                let (template, fallback) = cloud_notes_template(&store, &meeting, template)?;
                template_fallback = fallback;
                Task::Notes {
                    template,
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
                let version = store.get_meeting(&meeting).map_err(err)?.transcript_version;
                let (_, segs) = ghi_core::notes_job::stored_transcript(&store, &meeting)?;
                Ok(CloudPreviewResult::Answer(answer_view(
                    c, &meeting, "", a, &segs, version, "local",
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
                // Also when the core itself swapped a template that is not a built-in one.
                template_fallback |= p.template_fallback;
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
                    template_fallback,
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
        // The lines and version the request was built from, not whatever the
        // transcript is when the answer comes back.
        let (version, segs) = (plan.version(), plan.segments().to_vec());
        match cloud::send(&store, *plan, &key, policy)? {
            Outcome::Done(Sent::Notes(_)) => Ok(CloudSendResult::Notes),
            Outcome::Done(Sent::Answer(a)) => Ok(CloudSendResult::Answer(answer_view(
                c, &meeting, &question, a, &segs, version, &provider,
            )?)),
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
        let version = store.get_meeting(&meeting).map_err(err)?.transcript_version;
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
        answer_view(c, &meeting, &question, run.answer, &segs, version, "local")
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
    // Taken out of the cache first: two saves at once cannot both write it.
    let taken = c
        .answers()
        .take(meeting, id)
        .ok_or_else(|| ANSWER_EXPIRED.to_string())?;
    let saved = match store.note_blocks(meeting) {
        Ok(b) => b.iter().filter(|b| b.kind == ANSWER_KIND).count(),
        Err(e) => {
            c.answers().give_back(taken);
            return Err(err(e));
        }
    };
    if saved >= ANSWERS_SAVED_MAX {
        c.answers().give_back(taken);
        return Err(ANSWER_LIMIT.into());
    }
    // Kept in the notes prompt (pinned text), so bounded.
    let body = format!(
        "Q: {}\nA: {}",
        crate::detail::shorten(taken.draft.question.trim(), SAVED_QUESTION_CHARS),
        crate::detail::shorten(taken.draft.text.trim(), SAVED_ANSWER_CHARS)
    );
    match store.add_note_block(
        meeting,
        NewNoteBlock {
            kind: ANSWER_KIND.into(),
            provenance: Provenance::Ai,
            body,
            anchors: taken.draft.anchors.clone(),
            pinned: true,
        },
    ) {
        Ok(block) => Ok(block.gid),
        Err(e) => {
            c.answers().give_back(taken);
            Err(err(e))
        }
    }
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
        assert_eq!(
            save_answer_now(&f.core, &f.meeting, &id),
            Err(ANSWER_EXPIRED.into())
        );
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
        let ids: Vec<String> = (0..11)
            .map(|n| f.core.answers().put(draft(&f, n)))
            .collect();
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
    fn a_locked_app_refuses_and_forgets_the_waiting_answers() {
        let f = fix();
        let id = f.core.answers().put(draft(&f, 1));
        f.core.set_locked(true);
        assert!(save_answer_now(&f.core, &f.meeting, &id).is_err());
        f.core.set_locked(false);
        assert!(answers(&f).is_empty());
        // Locking wiped the answers waiting in memory: ask again to save.
        assert_eq!(
            save_answer_now(&f.core, &f.meeting, &id),
            Err(ANSWER_EXPIRED.into())
        );
        assert!(answers(&f).is_empty());
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

    #[test]
    fn deleting_a_meeting_forgets_its_answers() {
        let f = fix();
        let id = f.core.answers().put(draft(&f, 1));
        let other = f.core.answers().put(AnswerDraft {
            meeting: "other".into(),
            ..draft(&f, 2)
        });
        f.core.forget_meeting_answers(&f.meeting);
        assert_eq!(
            save_answer_now(&f.core, &f.meeting, &id),
            Err(ANSWER_EXPIRED.into())
        );
        assert!(f.core.answers().take("other", &other).is_some());
    }

    #[test]
    fn the_cache_is_capped_overall_and_expires() {
        let f = fix();
        let first = f.core.answers().put(AnswerDraft {
            meeting: "m0".into(),
            ..draft(&f, 0)
        });
        for n in 1..=ANSWERS_KEPT_TOTAL {
            f.core.answers().put(AnswerDraft {
                meeting: format!("m{n}"),
                ..draft(&f, n)
            });
        }
        assert!(
            f.core.answers().take("m0", &first).is_none(),
            "oldest dropped"
        );
        assert_eq!(f.core.answers().lock().len(), ANSWERS_KEPT_TOTAL);
        // An answer past its time is not found.
        let old = f.core.answers().put(draft(&f, 7));
        {
            let mut q = f.core.answers().lock();
            let e = q.iter_mut().find(|(i, _, _)| *i == old).unwrap();
            e.2 = Instant::now() - ANSWER_TTL - Duration::from_secs(1);
        }
        assert_eq!(
            save_answer_now(&f.core, &f.meeting, &old),
            Err(ANSWER_EXPIRED.into())
        );
    }

    #[test]
    fn two_saves_at_once_write_one_block() {
        let f = fix();
        let id = f.core.answers().put(draft(&f, 1));
        let results: Vec<Result<String, String>> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..4)
                .map(|_| s.spawn(|| save_answer_now(&f.core, &f.meeting, &id)))
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(
            results.iter().filter(|r| r.is_ok()).count(),
            1,
            "{results:?}"
        );
        assert_eq!(answers(&f).len(), 1);
    }

    #[test]
    fn a_refused_save_leaves_the_answer_to_try_again() {
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
        // Still there: once a block is deleted the same id saves.
        let gone = answers(&f)[0].gid.clone();
        f.core.store().unwrap().delete_note_blocks(&[gone]).unwrap();
        save_answer_now(&f.core, &f.meeting, &id).unwrap();
    }

    #[test]
    fn a_saved_answer_is_cut_to_a_bounded_size() {
        let f = fix();
        let id = f.core.answers().put(AnswerDraft {
            question: "q ".repeat(500),
            text: "a ".repeat(2000),
            ..draft(&f, 1)
        });
        save_answer_now(&f.core, &f.meeting, &id).unwrap();
        let body = answers(&f).remove(0).body;
        let (q, a) = body.split_once("\nA: ").unwrap();
        assert!(
            q.chars().count() <= SAVED_QUESTION_CHARS + 3 + 1,
            "{}",
            q.len()
        );
        assert!(a.chars().count() <= SAVED_ANSWER_CHARS + 1);
        assert!(q.ends_with('…') && a.ends_with('…'));
    }

    #[test]
    fn answer_view_turns_citation_numbers_into_anchors_and_keeps_the_answer() {
        let f = fix();
        let store = f.core.store().unwrap();
        let segs = store.segments(&f.meeting).unwrap();
        let v = answer_view(
            &f.core,
            &f.meeting,
            "When?",
            Answer::Answered {
                text: "On the 12th.".into(),
                citations: vec![0, 5],
            },
            &segs,
            7,
            "local",
        )
        .unwrap();
        // The out-of-range number is dropped; the line is the citation and the anchor.
        assert_eq!(v.citations.len(), 1);
        assert_eq!(v.citations[0].t0_ms, 1000.0);
        let id = v.id.clone().expect("a real answer has an id");
        let kept = f.core.answers().take(&f.meeting, &id).unwrap().draft;
        assert_eq!(kept.question, "When?");
        assert_eq!(kept.anchors.len(), 1);
        assert_eq!(
            (kept.anchors[0].t0_ms, kept.anchors[0].transcript_version),
            (1000, 7)
        );
        // Not discussed: nothing to save.
        let nd = answer_view(
            &f.core,
            &f.meeting,
            "x?",
            Answer::NotDiscussed {
                searched: vec!["x".into()],
            },
            &segs,
            7,
            "local",
        )
        .unwrap();
        assert!(!nd.answered && nd.id.is_none());
    }

    #[test]
    fn a_core_that_cannot_save_keeps_no_answers() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, _rx) = Core::for_test_with(
            tmp.path().join("data"),
            crate::core::CoreHooks {
                no_saved_answers: true,
                ..Default::default()
            },
        );
        let store = core.store().unwrap();
        let m = store.create_meeting(NewMeeting::default()).unwrap().gid;
        let v = answer_view(
            &core,
            &m,
            "q?",
            Answer::Answered {
                text: "t".into(),
                citations: vec![],
            },
            &[],
            1,
            "local",
        )
        .unwrap();
        assert!(v.id.is_none());
        assert!(core.answers().lock().is_empty());
    }

    #[test]
    fn a_cloud_request_never_carries_what_is_in_one_of_your_templates() {
        use ghi_core::cloud::{Planned, plan};
        use ghi_core::user_templates::{Records, UserTemplate};
        use ghi_llm::cloud::CloudProvider;
        use ghi_llm::preview::Prices;
        use ghi_llm::template::{Editor, EditorSection};
        let f = fix();
        let store = f.core.store().unwrap();
        // A template with words nobody else would write, in every field that could be sent.
        let t = ghi_llm::template::Template::from_editor(
            "t77",
            &Editor {
                name: "Zebra quarterly".into(),
                lang: OutLang::En,
                guidance: "XYLOPHONE-GUIDANCE meetings about zebras".into(),
                sections: vec![EditorSection {
                    id: None,
                    title: "Quokka findings".into(),
                    instruction: "QUOKKA-INSTRUCTION list every marsupial".into(),
                }],
            },
            &[],
            &[],
        )
        .unwrap();
        let section_id = t.sections[0].id.clone();
        let mut r = Records::load(&store).unwrap();
        r.push(UserTemplate {
            gid: "t77".into(),
            lang: "en".into(),
            toml: t.to_toml(),
            retired: vec![],
        });
        r.save(&store).unwrap();

        let payload_for = |asked: Option<&str>| {
            let (template, fallback) =
                cloud_notes_template(&store, &f.meeting, asked.map(String::from)).unwrap();
            let Planned::Send(p) = plan(
                &store,
                &f.meeting,
                CloudProvider::preset("openai", "gpt-4.1-mini").unwrap(),
                ghi_core::cloud::Task::Notes {
                    template,
                    lang: OutLang::En,
                },
                true,
                &[],
                &Prices::builtin(),
            )
            .unwrap() else {
                panic!("a request")
            };
            (p.preview.payload.clone(), fallback)
        };

        // Asked for explicitly, or the meeting's own template: General goes, flagged.
        store
            .set_meeting_template(&f.meeting, Some("user:t77"))
            .unwrap();
        for asked in [Some("user:t77"), None] {
            let (payload, fallback) = payload_for(asked);
            assert!(fallback, "{asked:?}");
            for secret in [
                "XYLOPHONE",
                "QUOKKA",
                "Zebra",
                "Quokka",
                section_id.as_str(),
                "user:t77",
            ] {
                assert!(
                    !payload.contains(secret),
                    "{secret} left the device: {payload}"
                );
            }
            assert!(
                payload.contains("we ship on the 12th"),
                "still the transcript"
            );
        }
        // A built-in template is sent as it is, and says nothing about falling back.
        store.set_meeting_template(&f.meeting, None).unwrap();
        let (payload, fallback) = payload_for(Some("standup"));
        assert!(!fallback);
        assert!(
            payload.contains("blockers"),
            "the standup's own section: {payload}"
        );
        let (_, fallback) = payload_for(None);
        assert!(!fallback, "no template at all is General, not a fallback");
        // An id that is neither is refused, not silently General.
        assert!(cloud_notes_template(&store, &f.meeting, Some("nope".into())).is_err());
    }
}
