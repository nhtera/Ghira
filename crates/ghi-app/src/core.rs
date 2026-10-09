// SPDX-License-Identifier: Apache-2.0
//! The thin command layer over `ghi-core` (phase 8): recording controls,
//! speaker edits, import, and the core's events forwarded to the webview as
//! the typed `coreEvent`. The UI arrives in phases 10–11.
//!
//! The store lives in the app data directory. Its master key: the Keychain
//! (`com.nhtera.ghira`) in release builds; in debug builds a key file next to
//! the data directory, so dev runs never touch the Keychain
//! (`GHI_KEYSTORE=keychain` switches; release binaries are checked for the dev
//! store by `tools/scripts/check-no-dev-key.sh`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use crate::store_problem::StoreProblem;
use ghi_core::events::{Envelope, ErrorKind, EventTx, bus};
use ghi_core::jobs::JobRunner;
use ghi_core::live::Mode;
use ghi_core::session::{Session, SessionConfig};
use ghi_store::keys::{KeyStore, Protection};
use ghi_store::store::Store;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, Runtime};
use tauri_specta::Event;

/// Every core event, in order (`seq` is gap-free; on a gap, re-read state).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
pub struct CoreEvent(pub Envelope);

pub struct Core {
    data: PathBuf,
    store: Mutex<Option<Arc<Store>>>,
    /// Shared so a long command (a discard) never holds the lock.
    session: Mutex<Option<Arc<Session>>>,
    /// Held for a whole start or stop, so they never overlap; the session
    /// slot itself is only locked briefly.
    lifecycle: Mutex<()>,
    /// Why the last attempt to open the store failed (none while it is open).
    open_problem: Mutex<Option<crate::store_problem::StoreProblem>>,
    /// "Delete all data" is running: the store must not be reopened.
    deleting: std::sync::atomic::AtomicBool,
    /// The app lock is on: commands get no store, and transcript events
    /// don't reach the webview (lock_cmd.rs).
    locked: Arc<std::sync::atomic::AtomicBool>,
    /// A warm of the search vectors runs, and another was asked for meanwhile.
    warming: std::sync::atomic::AtomicBool,
    warm_again: std::sync::atomic::AtomicBool,
    runner: Mutex<Option<Arc<JobRunner>>>,
    /// Meetings closed by crash recovery at this launch (D12 "recovered"),
    /// until the user dismisses the notice.
    recovered: Mutex<Vec<String>>,
    /// Sensitive mode is armed for the next recording (any window or the tray
    /// starts it); cleared once one started.
    sensitive_next: std::sync::atomic::AtomicBool,
    /// App settings as last read or written (system.rs).
    settings: Mutex<Option<crate::system::AppSettings>>,
    /// What the app does after a settings change (the desktop re-registers
    /// its global shortcuts).
    settings_hook: Mutex<Option<SettingsHook>>,
    /// "Record your voice" in progress (voice_cmd.rs).
    enrollment: Mutex<Option<crate::voice_cmd::Enrollment>>,
    /// The runner's thread, joined (bounded) at shutdown.
    runner_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Opens the local notes model (the jobs' factory; "Ask this meeting").
    llm: Mutex<Option<ghi_core::notes_job::LlmFactory>>,
    /// The embedding model for search queries, kept loaded while the user
    /// searches (dropped after [`QUERY_EMBEDDER_IDLE`]).
    query_embedder: QueryEmbedder,
    /// Ask answers waiting to be saved to the notes (by id).
    answers: crate::cloud_cmd::AnswerCache,
    events: EventTx,
    hooks: CoreHooks,
    /// The launch lock check ran (see [`CoreHooks::gate_launch`]).
    launch_checked: std::sync::atomic::AtomicBool,
    /// The app's sync service (phase 15): told when the lock changes and
    /// before everything is deleted. Weak: the service holds the core.
    sync: Mutex<std::sync::Weak<crate::sync_service::SyncService>>,
}

/// See [`Core::set_settings_hook`].
pub type SettingsHook = Arc<dyn Fn(&AppHandle) + Send + Sync>;

/// The job handlers an app registers in place of the desktop's.
pub type HandlerFactory =
    Arc<dyn Fn(&Arc<Store>, &Path) -> Vec<Arc<dyn ghi_core::jobs::JobHandler>> + Send + Sync>;

/// Opens the microphone for a voice enrollment where `ghi_core::capture::live`
/// does not exist (iOS): the ring to read and a guard that releases the mic
/// when dropped. Errors are the `voice_cmd` codes (`micPermission`, `noMic`).
pub type MicSource = Arc<
    dyn Fn() -> Result<(ghi_audio::ring::RingConsumer, Box<dyn std::any::Any + Send>), String>
        + Send
        + Sync,
>;

/// See [`CoreHooks::before_spawn`].
pub type RunnerHook = Arc<dyn Fn(&Arc<JobRunner>) + Send + Sync>;

/// See [`CoreHooks::secrets`].
pub type SecretsFactory =
    Arc<dyn Fn() -> Result<Box<dyn ghi_store::keys::secrets::SecretStore>, String> + Send + Sync>;

/// What an app plugs into the core instead of the desktop's defaults. The
/// desktop passes none ([`Core::new`]); the iOS app sets its own data
/// directory, handlers (no local notes), recovery kinds and key stores.
#[derive(Default, Clone)]
pub struct CoreHooks {
    /// The data directory (default: Tauri's app data dir).
    pub data_dir: Option<PathBuf>,
    /// The job handlers (default: notes, final pass, voice, semantic index).
    pub handlers: Option<HandlerFactory>,
    /// The job kinds crash recovery queues (default: live notes and final pass).
    pub recover_kinds: Option<Vec<&'static str>>,
    /// Recovery also gives a final pass to recorded meetings whose stop never
    /// finished (`recover::requeue_unfinished_stops`; the live-tier phone).
    pub requeue_unfinished_stops: bool,
    /// This device writes no notes itself (the phone): recovery calls off its
    /// own notes jobs (`recover::drop_local_notes_jobs`).
    pub no_local_notes: bool,
    /// This device has no "save an Ask answer to the notes" (the phone): Ask
    /// answers are not kept in memory.
    pub no_saved_answers: bool,
    /// Runs on the new job runner right before it is spawned (the phone
    /// pauses it when launched in the background).
    pub before_spawn: Option<RunnerHook>,
    /// The master key store (default: the platform's, see `keystore`).
    pub keystore: Option<Arc<dyn KeyStore>>,
    /// Cloud key store (default: the platform's, see [`secrets`]).
    pub secrets: Option<SecretsFactory>,
    /// The microphone for a voice enrollment (default: `ghi_core::capture`).
    pub mic: Option<MicSource>,
    /// `store()` refuses ("the app is starting") until the launch lock check
    /// has run ([`Core::mark_launch_checked`]), so no content command can
    /// slip in before the app lock engages (the phone).
    pub gate_launch: bool,
}

type QueryEmbedder = Arc<Mutex<Option<(Box<dyn ghi_llm::embed::Embedder + Send>, Instant)>>>;

/// The query embedder is unloaded after this long unused.
#[cfg(feature = "embeddings")]
const QUERY_EMBEDDER_IDLE: Duration = Duration::from_secs(120);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The processes of the meeting app in a call, for a per-app system-audio
/// tap (the setting "only the meeting app's audio"). Empty (the whole
/// system is captured instead) when none is in a call or the platform
/// cannot list audio processes.
#[cfg(target_os = "macos")]
fn meeting_app_pids(apps: &[String]) -> Vec<i32> {
    ghi_audio::macos::audio_processes()
        .map(|p| ghi_audio::detect::meeting_app_pids(&p, Some(apps), std::process::id() as i32))
        .unwrap_or_default()
}

#[cfg(not(target_os = "macos"))]
fn meeting_app_pids(_apps: &[String]) -> Vec<i32> {
    Vec::new()
}

fn keystore(dir: &Path) -> Result<Arc<dyn KeyStore>, String> {
    #[cfg(debug_assertions)]
    if std::env::var("GHI_KEYSTORE").as_deref() != Ok("keychain") {
        let mut name = dir
            .file_name()
            .ok_or("the data directory has no name")?
            .to_owned();
        name.push(".devkey");
        return Ok(Arc::new(ghi_store::keys::dev::FileKeyStore::new(
            dir.with_file_name(name),
        )));
    }
    platform_keystore(dir)
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn platform_keystore(_dir: &Path) -> Result<Arc<dyn KeyStore>, String> {
    Ok(Arc::new(ghi_store::keys::apple::KeychainStore::new(
        "com.nhtera.ghira",
        "store",
    )))
}

#[cfg(windows)]
fn platform_keystore(dir: &Path) -> Result<Arc<dyn KeyStore>, String> {
    Ok(Arc::new(ghi_store::keys::windows::DpapiStore::new(
        dir.join("keys").join("master.dpapi"),
    )))
}

#[cfg(not(any(target_os = "macos", target_os = "ios", windows)))]
fn platform_keystore(_dir: &Path) -> Result<Arc<dyn KeyStore>, String> {
    Err("no OS key store on this platform yet".into())
}

/// Where cloud API keys live: the Keychain (Windows: DPAPI files); debug
/// builds use files next to the data directory unless `GHI_KEYSTORE=keychain`.
pub fn secrets(data: &Path) -> Result<Box<dyn ghi_store::keys::secrets::SecretStore>, String> {
    #[cfg(debug_assertions)]
    if std::env::var("GHI_KEYSTORE").as_deref() != Ok("keychain") {
        return Ok(Box::new(ghi_store::keys::secrets::FileSecrets::new(
            data.join("store.devsecrets"),
        )));
    }
    platform_secrets(data)
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn platform_secrets(
    _data: &Path,
) -> Result<Box<dyn ghi_store::keys::secrets::SecretStore>, String> {
    Ok(Box::new(ghi_store::keys::secrets::KeychainSecrets::new(
        "com.nhtera.ghira",
    )))
}

#[cfg(windows)]
fn platform_secrets(data: &Path) -> Result<Box<dyn ghi_store::keys::secrets::SecretStore>, String> {
    Ok(Box::new(ghi_store::keys::secrets::DpapiSecrets::new(
        data.join("secrets"),
    )))
}

#[cfg(not(any(target_os = "macos", target_os = "ios", windows)))]
fn platform_secrets(
    _data: &Path,
) -> Result<Box<dyn ghi_store::keys::secrets::SecretStore>, String> {
    Err("no OS key store on this platform yet".into())
}

/// Models whose file failed its SHA-256 at load (D12 "model damaged"): the UI
/// offers a re-download; a successful download clears the mark.
static DAMAGED: Mutex<std::collections::BTreeSet<String>> =
    Mutex::new(std::collections::BTreeSet::new());

pub fn damaged_models() -> std::collections::BTreeSet<String> {
    lock(&DAMAGED).clone()
}

pub fn clear_damaged(id: &str) {
    lock(&DAMAGED).remove(id);
}

/// Verifies a model before native code parses it, remembering damage.
pub fn checked_model(models: &Path, id: &str) -> Result<PathBuf, String> {
    let m = ghi_models::find(id).ok_or_else(|| format!("{id} is not in the registry"))?;
    let p = ghi_models::path_in(models, &m);
    ghi_models::verify_for_load(&p, &m).map_err(|e| {
        if matches!(
            e,
            ghi_models::VerifyError::Hash | ghi_models::VerifyError::Size { .. }
        ) {
            lock(&DAMAGED).insert(id.to_string());
        }
        format!("model {id}: {e}")
    })?;
    Ok(p)
}

/// Store setting holding the onboarding test recording's meeting id.
const TEST_MEETING_KEY: &str = "test_capture_meeting";

/// This machine's tier preset (the hardware is probed once).
pub fn preset() -> &'static ghi_models::Preset {
    static PRESET: OnceLock<ghi_models::Preset> = OnceLock::new();
    PRESET.get_or_init(|| ghi_models::preset(ghi_models::tier_for(&ghi_models::detect())))
}

/// This build has speech engines and their models are installed. Without
/// them a recording keeps the audio only and its jobs wait (doc 02 §L).
pub fn speech_ready(models: &Path) -> bool {
    let ids: Vec<&str> = preset().speech_models.iter().map(String::as_str).collect();
    cfg!(feature = "nemo") && ghi_models::installed(models, &ids)
}

/// The embedding model (semantic search) is installed, on a tier that has one.
pub fn embed_ready(models: &Path) -> bool {
    cfg!(feature = "embeddings")
        && preset()
            .embed_id
            .is_some_and(|id| ghi_models::installed(models, &[id]))
}

/// The speaker-voice model (voice profiles) is installed.
pub fn voice_ready(models: &Path) -> bool {
    ghi_models::installed(models, &[preset().voice_id])
}

/// [`voice_ready`] remembered for 30 s (settings are read often).
pub fn voice_ready_cached(models: &Path) -> bool {
    static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let mut c = lock(&CACHE);
    if let Some((at, v)) = *c
        && at.elapsed() < Duration::from_secs(30)
    {
        return v;
    }
    let v = voice_ready(models);
    *c = Some((Instant::now(), v));
    v
}

/// Opens the speaker model (checked against its SHA-256 first).
pub fn voice_factory(models: &Path) -> ghi_core::profiles::VoiceFactory {
    let models = models.to_path_buf();
    Arc::new(move || {
        let path = checked_model(&models, preset().voice_id)?;
        ghi_core::profiles::open_tract(&path)
    })
}

fn voice_ready_fn(models: &Path) -> ghi_core::jobs::Ready {
    let models = models.to_path_buf();
    Arc::new(move || voice_ready(&models))
}

/// Hands out the third-party proof only when `enforce()`'s rule says the
/// flag is on (never, while `THIRD_PARTY_APPROVED` is false).
fn third_party_gate(store: &Arc<Store>) -> ghi_core::voice_step::ThirdPartyGate {
    let store = store.clone();
    Arc::new(move || crate::system::third_party_token(&store))
}

/// The final pass's voice step: Me and, behind the flag, other people.
fn voice_step(models: &Path, store: &Arc<Store>) -> ghi_core::voice_step::VoiceStep {
    ghi_core::voice_step::VoiceStep {
        embedder: voice_factory(models),
        ready: voice_ready_fn(models),
        third_party: third_party_gate(store),
    }
}

/// Opens the notes model (from `models`, the app data dir) sized for a
/// transcript of `bytes`: the worker process on a computer, the engine in
/// process on a phone (`ghi-llm` feature `inproc`).
pub fn llm_factory(models: &Path) -> ghi_core::notes_job::LlmFactory {
    let llm_dir = models.to_path_buf();
    Arc::new(move |bytes| {
        let n_ctx = ((bytes / 3) as u32 * 6 / 5 + 6_144).clamp(8_192, LLM_MAX_CTX);
        #[cfg(feature = "local-llm")]
        {
            // Marks a damaged file for the UI (the worker checks it too).
            checked_model(&llm_dir, preset().llm_id)?;
            ghi_llm::local::LocalLlm::open_registry_in(&llm_dir, preset().llm_id, n_ctx)
                .map(|l| Box::new(l) as Box<dyn ghi_llm::Llm + Send>)
                .map_err(|e| e.to_string())
        }
        #[cfg(not(feature = "local-llm"))]
        {
            let _ = (&llm_dir, n_ctx);
            Err("this build has no local notes model".into())
        }
    })
}

/// The longest notes context; longer meetings are written in parts.
pub const LLM_MAX_CTX: u32 = 32_768;

/// The notes model for this machine's tier is installed (and this build runs
/// it: without feature `local-llm` the notes jobs wait, as they do for any
/// missing model).
pub fn llm_ready(models: &Path) -> bool {
    cfg!(feature = "local-llm") && ghi_models::installed(models, &[preset().llm_id])
}

/// Speech engines for a recording (live chunk) or the final pass (1120 ms).
#[cfg(feature = "nemo")]
fn engines(
    models: &Path,
    chunk_ms: u32,
) -> Result<Arc<dyn ghi_core::engines::SpeechEngines>, String> {
    // Checked against their pinned SHA-256 before native code parses them.
    let path = |id: &str| checked_model(models, id);
    log::info!(
        "speech models load asr={} diar={}",
        preset().speech_models[0],
        preset().speech_models[1]
    );
    let e = ghi_core::engines::NemoEngines::load(
        &path(&preset().speech_models[0])?,
        &path(&preset().speech_models[1])?,
        chunk_ms,
        ghi_speech::nemo::Device::Gpu,
    )
    .map_err(|e| e.to_string())?;
    Ok(Arc::new(e))
}

#[cfg(not(feature = "nemo"))]
fn engines(
    _models: &Path,
    _chunk_ms: u32,
) -> Result<Arc<dyn ghi_core::engines::SpeechEngines>, String> {
    Err("this build has no speech engines".into())
}

/// Store setting choosing the final pass's recognizer: `"nemo"` (default) or
/// `"whisper"` (Settings → Models on the computer, `settings_cmd`).
/// `GHI_ASR_FINAL` overrides it in debug builds.
pub const ASR_FINAL_KEY: &str = "asr_final";

/// The optional models the Whisper final pass reads with.
pub const WHISPER_MODELS: [&str; 2] = ["whisper-large-v3-turbo", "silero-vad"];

/// This build can read the final pass with Whisper (desktop with `whisper`).
pub const WHISPER_BUILT: bool = cfg!(all(feature = "nemo", feature = "whisper"));

/// The stored choice is Whisper (the debug override aside).
pub fn whisper_chosen(store: &Store) -> bool {
    store
        .get_setting(ASR_FINAL_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(|c| c.eq_ignore_ascii_case("whisper")))
        .unwrap_or(false)
}

/// Optional models the user's choices need: Whisper's when chosen in a build
/// that has it (Settings → Models lists and downloads them).
pub fn wanted_optional_models(store: &Store) -> Vec<&'static str> {
    if WHISPER_BUILT && whisper_chosen(store) {
        WHISPER_MODELS.to_vec()
    } else {
        Vec::new()
    }
}

/// Whether the final pass should read with Whisper: this build has it, the
/// setting asks for it and both of its models are installed.
#[cfg(all(feature = "nemo", feature = "whisper"))]
fn whisper_final_wanted(models: &Path, store: &Store) -> bool {
    // The dev override works in debug builds only (a release app follows the setting).
    let env = if cfg!(debug_assertions) {
        std::env::var("GHI_ASR_FINAL").ok()
    } else {
        None
    };
    let chosen = match env {
        Some(c) => c.eq_ignore_ascii_case("whisper"),
        None => whisper_chosen(store),
    };
    chosen && ghi_models::installed(models, &WHISPER_MODELS)
}

/// An optional model's path, verified. Unlike [`checked_model`] a bad file is
/// not remembered as "damaged": the UI's re-download offer is for the models
/// the app needs, and this one is only used when chosen.
#[cfg(all(feature = "nemo", feature = "whisper"))]
fn optional_model(models: &Path, id: &str) -> Result<PathBuf, String> {
    let m = ghi_models::find(id).ok_or_else(|| format!("{id} is not in the registry"))?;
    let p = ghi_models::path_in(models, &m);
    ghi_models::verify_for_load(&p, &m).map_err(|e| format!("model {id}: {e}"))?;
    Ok(p)
}

#[cfg(all(feature = "nemo", feature = "whisper"))]
fn whisper_engines(models: &Path) -> Result<Arc<dyn ghi_core::engines::SpeechEngines>, String> {
    let e = ghi_core::engines::WhisperFinalEngines::load(
        &optional_model(models, "whisper-large-v3-turbo")?,
        &optional_model(models, "silero-vad")?,
        &checked_model(models, &preset().speech_models[1])?,
        &checked_model(models, &preset().speech_models[0])?,
        1120,
        ghi_speech::nemo::Device::Gpu,
    )
    .map_err(|e| e.to_string())?;
    Ok(Arc::new(e))
}

/// Speech engines for the final pass: Whisper for the words (NeMo still
/// diarizes) when chosen, else [`engines`] at 1120 ms. If Whisper cannot be
/// used (a bad file, a load failure) this pass falls back to NeMo.
fn final_engines(
    models: &Path,
    store: &Store,
) -> Result<Arc<dyn ghi_core::engines::SpeechEngines>, String> {
    #[cfg(all(feature = "nemo", feature = "whisper"))]
    if whisper_final_wanted(models, store) {
        log::info!("speech models load asr=whisper-large-v3-turbo diar=nemotron-3-diarization");
        match whisper_engines(models) {
            Ok(e) => return Ok(e),
            Err(e) => log::warn!("whisper final pass unavailable, using NeMo: {e}"),
        }
    }
    let _ = store;
    engines(models, 1120)
}

impl Core {
    pub(crate) fn answers(&self) -> &crate::cloud_cmd::AnswerCache {
        &self.answers
    }

    /// Ask answers can be saved to the notes here (not on the phone).
    pub(crate) fn keeps_answers(&self) -> bool {
        !self.hooks.no_saved_answers
    }

    /// A meeting was deleted: its Ask answers go from memory too.
    pub fn forget_meeting_answers(&self, meeting: &str) {
        self.answers.forget_meeting(meeting);
    }

    /// Starts forwarding core events to the webview.
    /// `on_event` also sees every event (the tray follows the session).
    pub fn new<R: Runtime>(
        app: &AppHandle<R>,
        on_event: impl Fn(&ghi_core::events::Event) + Send + 'static,
    ) -> Result<Core, String> {
        let mut hooks = CoreHooks::default();
        // Debug builds only: `GHI_DATA_DIR` runs the app on another data folder
        // (a second instance, a test run next to the real data).
        #[cfg(debug_assertions)]
        if let Some(dir) = std::env::var_os("GHI_DATA_DIR") {
            hooks.data_dir = Some(PathBuf::from(dir));
        }
        Core::with_hooks(app, on_event, hooks)
    }

    /// [`Core::new`] with an app's own data directory, handlers and key stores.
    pub fn with_hooks<R: Runtime>(
        app: &AppHandle<R>,
        on_event: impl Fn(&ghi_core::events::Event) + Send + 'static,
        hooks: CoreHooks,
    ) -> Result<Core, String> {
        let data = match &hooks.data_dir {
            Some(d) => d.clone(),
            None => app.path().app_data_dir().map_err(|e| e.to_string())?,
        };
        let (events, rx) = bus();
        let handle = app.clone();
        let locked = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let gate = locked.clone();
        std::thread::Builder::new()
            .name("ghi-events".into())
            .spawn(move || {
                for env in rx {
                    on_event(&env.event);
                    // While locked, what was said and who said it stays out
                    // of the webview; it re-reads the snapshot on unlock.
                    use ghi_core::events::Event as E;
                    let content = matches!(
                        env.event,
                        E::TranscriptPartial { .. }
                            | E::TranscriptFinal { .. }
                            | E::SpeakerArrived { .. }
                            | E::SpeakerConfirmed { .. }
                            | E::SpeakerRenamed { .. }
                    );
                    if content && gate.load(std::sync::atomic::Ordering::Acquire) {
                        continue;
                    }
                    let _ = CoreEvent(env).emit(&handle);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Core::assemble(data, events, locked, hooks))
    }

    fn assemble(
        data: PathBuf,
        events: EventTx,
        locked: Arc<std::sync::atomic::AtomicBool>,
        hooks: CoreHooks,
    ) -> Core {
        Core {
            data,
            store: Mutex::new(None),
            open_problem: Mutex::new(None),
            session: Mutex::new(None),
            lifecycle: Mutex::new(()),
            deleting: std::sync::atomic::AtomicBool::new(false),
            locked,
            warming: std::sync::atomic::AtomicBool::new(false),
            warm_again: std::sync::atomic::AtomicBool::new(false),
            runner: Mutex::new(None),
            runner_thread: Mutex::new(None),
            enrollment: Mutex::new(None),
            llm: Mutex::new(None),
            query_embedder: Arc::new(Mutex::new(None)),
            settings: Mutex::new(None),
            settings_hook: Mutex::new(None),
            recovered: Mutex::new(Vec::new()),
            sensitive_next: std::sync::atomic::AtomicBool::new(false),
            answers: Default::default(),
            events,
            hooks,
            launch_checked: std::sync::atomic::AtomicBool::new(false),
            sync: Mutex::new(std::sync::Weak::new()),
        }
    }

    /// A core over a data directory with no window: the events stay in the
    /// returned receiver. For the real-model harness test only.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(data: PathBuf) -> (Arc<Core>, ghi_core::events::EventRx) {
        Core::for_test_with(data, CoreHooks::default())
    }

    /// [`Core::for_test`] with an app's hooks (the phone's harness tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test_with(
        data: PathBuf,
        hooks: CoreHooks,
    ) -> (Arc<Core>, ghi_core::events::EventRx) {
        let (events, rx) = bus();
        let locked = Arc::new(std::sync::atomic::AtomicBool::new(false));
        (Arc::new(Core::assemble(data, events, locked, hooks)), rx)
    }

    /// Opens the store, runs crash recovery and starts the jobs off the main
    /// thread at launch, so the library and "recovered" states are there on
    /// first paint (commands that need the store wait for it).
    pub fn init_in_background(self: &Arc<Self>) {
        let me = self.clone();
        let _ = std::thread::Builder::new()
            .name("ghi-store-open".into())
            .spawn(move || {
                if let Err(e) = me.store() {
                    me.events.emit(ghi_core::events::Event::Error {
                        meeting: None,
                        kind: ErrorKind::Storage,
                        message: format!("opening the store: {e}"),
                    });
                } else {
                    me.warm_now();
                }
            });
    }

    pub fn enrollment_mutex(&self) -> &Mutex<Option<crate::voice_cmd::Enrollment>> {
        &self.enrollment
    }

    /// Held while a recording starts or stops (an enrollment checks
    /// `recording()` under it).
    pub fn lifecycle_guard(&self) -> MutexGuard<'_, ()> {
        lock(&self.lifecycle)
    }

    pub fn enrollment_slot(&self) -> MutexGuard<'_, Option<crate::voice_cmd::Enrollment>> {
        lock(&self.enrollment)
    }

    /// A recording is running (quitting must ask first).
    pub fn recording(&self) -> bool {
        lock(&self.session).is_some()
    }

    /// A recording is running or starting/stopping.
    pub fn busy(&self) -> bool {
        self.recording() || self.lifecycle.try_lock().is_err()
    }

    /// Takes the meetings recovered at launch (the notice is shown once).
    pub fn take_recovered(&self) -> Vec<String> {
        std::mem::take(&mut *lock(&self.recovered))
    }

    /// Sets what runs after every settings change.
    pub fn set_settings_hook(&self, hook: SettingsHook) {
        *lock(&self.settings_hook) = Some(hook);
    }

    /// Runs the settings hook (see [`Core::set_settings_hook`]).
    pub fn settings_changed(&self, app: &AppHandle) {
        let hook = lock(&self.settings_hook).clone();
        if let Some(h) = hook {
            h(app);
        }
    }

    pub fn settings_cache(&self) -> &Mutex<Option<crate::system::AppSettings>> {
        &self.settings
    }

    /// Stops the job runner for a quit (not a crash): the running job yields
    /// at its next checkpoint, or — if it can't within `wait` (notes mid-LLM
    /// call) — its claim goes back to the queue without spending an attempt.
    /// Then the LLM worker processes are killed so none outlives the app.
    pub fn shutdown(&self, wait: std::time::Duration) {
        crate::voice_cmd::drop_enrollment(&self.enrollment);
        if let Some(r) = lock(&self.runner).as_ref() {
            r.shutdown_and_release(wait);
        }
        // Kill first: a query may hold the embedder's lock while its worker
        // loads or embeds (minutes at worst); a killed worker returns at once.
        ghi_llm::local::kill_workers();
        lock(&self.query_embedder).take();
        ghi_llm::local::kill_workers();
        if let Some(t) = lock(&self.runner_thread).take()
            && t.is_finished()
        {
            let _ = t.join();
        }
    }

    /// Runs `f` with the embedding model for a search query, or with `None`
    /// when meaning search is off (8 GB Macs), the model isn't installed yet
    /// or can't be opened, and while recording (the GPU belongs to live
    /// speech). The model stays loaded between queries and is dropped after
    /// [`QUERY_EMBEDDER_IDLE`]; queries take turns.
    pub fn with_query_embedder<T>(
        &self,
        store: &Store,
        f: impl FnOnce(Option<&mut dyn ghi_llm::embed::Embedder>) -> T,
    ) -> T {
        let models = self.models();
        if self.recording() || !ghi_core::index_job::enabled(store) || !embed_ready(&models) {
            lock(&self.query_embedder).take();
            return f(None);
        }
        let mut slot = lock(&self.query_embedder);
        if slot.is_none() {
            #[cfg(feature = "embeddings")]
            match ghi_llm::embed::LocalEmbedder::open_registry_in(
                &models,
                ghi_core::index_job::MODEL_ID,
            ) {
                Ok(e) => {
                    *slot = Some((Box::new(e), Instant::now()));
                    self.unload_when_idle();
                }
                Err(e) => {
                    log::warn!("search embedder unavailable: {e}");
                    drop(slot);
                    return f(None);
                }
            }
            #[cfg(not(feature = "embeddings"))]
            {
                drop(slot);
                return f(None);
            }
        }
        let Some((e, used)) = slot.as_mut() else {
            return f(None);
        };
        let out = f(Some(e.as_mut()));
        *used = Instant::now();
        out
    }

    /// Drops the query embedder once it has been idle long enough.
    #[cfg(feature = "embeddings")]
    fn unload_when_idle(&self) {
        let slot = self.query_embedder.clone();
        let _ = std::thread::Builder::new()
            .name("ghi-embed-idle".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_secs(15));
                    let mut s = lock(&slot);
                    match s.as_ref() {
                        None => return,
                        Some((_, used)) if used.elapsed() >= QUERY_EMBEDDER_IDLE => {
                            s.take();
                            return;
                        }
                        Some(_) => {}
                    }
                }
            });
    }

    /// Reads the search vectors into memory on a background thread, so the
    /// first search after launch or unlock doesn't decrypt them. Never while
    /// locked (or before the launch lock check), on machines without meaning
    /// search, or during a recording; a recording that starts meanwhile stops
    /// it. Meetings indexed later are warmed by the index job itself. One
    /// warm runs at a time: a request that arrives meanwhile makes it go
    /// round once more.
    pub fn warm_vectors(self: &Arc<Self>) {
        if !self.claim_warm() {
            return;
        }
        let me = self.clone();
        let spawned = std::thread::Builder::new()
            .name("ghi-warm".into())
            .spawn(move || me.warm_claimed());
        if spawned.is_err() {
            self.warming
                .store(false, std::sync::atomic::Ordering::Release);
        }
    }

    /// Takes the one warming slot; `false` (and a note to go round again)
    /// when someone holds it.
    fn claim_warm(&self) -> bool {
        use std::sync::atomic::Ordering::{AcqRel, Release};
        if self.warming.swap(true, AcqRel) {
            self.warm_again.store(true, Release);
            return false;
        }
        true
    }

    /// [`Core::warm_vectors`] on this thread (the launch thread, after the
    /// store opened).
    fn warm_now(&self) {
        if self.claim_warm() {
            self.warm_claimed();
        }
    }

    /// Warms while holding the slot, once more for each request that came in
    /// meanwhile, then gives the slot back.
    fn warm_claimed(&self) {
        use std::sync::atomic::Ordering::{AcqRel, Release};
        loop {
            self.warm_again.store(false, Release);
            self.warm_vectors_now();
            self.warming.store(false, Release);
            // A request that came in during the run: take the slot back.
            if !self.warm_again.load(std::sync::atomic::Ordering::Acquire)
                || self.warming.swap(true, AcqRel)
            {
                return;
            }
        }
    }

    fn warm_vectors_now(&self) {
        // Looks at the session: only ever called without the store's connection.
        let keep_going = |c: &Core| {
            !c.locked()
                && !c.recording()
                && (!c.hooks.gate_launch
                    || c.launch_checked.load(std::sync::atomic::Ordering::Acquire))
        };
        if !keep_going(self) {
            return;
        }
        let Ok(store) = self.store_even_locked() else {
            return;
        };
        if !ghi_core::index_job::enabled(&store) || !embed_ready(&self.models()) {
            return;
        }
        // With the connection held only the lock flag is read (never the
        // session), and it is set before `lock()` takes the connection.
        let locked = self.locked.clone();
        ghi_core::index_job::warm_vectors(
            &store,
            ghi_core::index_job::MODEL_ID,
            &|| keep_going(self),
            &|| !locked.load(std::sync::atomic::Ordering::Acquire),
        );
    }

    /// The app lock is on (lock_cmd.rs).
    pub fn locked(&self) -> bool {
        self.locked.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn set_locked(&self, locked: bool) {
        // The mic never stays open behind the lock screen.
        if locked {
            crate::voice_cmd::drop_enrollment(&self.enrollment);
            // Answers waiting to be saved are meeting text: not behind the lock screen.
            self.answers.clear();
        }
        self.locked
            .store(locked, std::sync::atomic::Ordering::Release);
        // Sync's listener exists only while the app is unlocked (doc 07
        // §2.4): the service follows on its own thread (the phone calls this
        // from its main thread).
        if let Some(s) = self.sync_service() {
            s.poke();
        }
    }

    /// Registers the app's sync service (`SyncService::new` does).
    pub fn attach_sync(&self, service: &Arc<crate::sync_service::SyncService>) {
        *lock(&self.sync) = Arc::downgrade(service);
    }

    pub fn sync_service(&self) -> Option<Arc<crate::sync_service::SyncService>> {
        lock(&self.sync).upgrade()
    }

    /// The app's data directory (store, models, updates, diagnostics).
    pub fn data_dir(&self) -> &Path {
        &self.data
    }

    pub fn models(&self) -> PathBuf {
        self.data.join("models")
    }

    /// "Delete all data" (Settings → Privacy): stops the jobs, crypto-shreds
    /// every meeting and removes the database, audio, snapshots and cloud
    /// keys; a fresh key ring is saved. The app restarts into onboarding.
    pub fn delete_everything(&self) -> Result<(), String> {
        self.delete_everything_with(&|| Ok(()))
    }

    /// [`Core::delete_everything`] after `prepare`, which runs once nothing
    /// can start a recording any more and the store is still usable: the
    /// sync service queues the wipe of paired devices and waits for them
    /// there (`SyncService::delete_everywhere_prepare`). If `prepare` fails
    /// nothing is deleted.
    pub fn delete_everything_with(
        &self,
        prepare: &dyn Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        use std::sync::atomic::Ordering;
        if self.locked() {
            return Err("the app is locked".into());
        }
        // No recording can start meanwhile.
        let _lifecycle = lock(&self.lifecycle);
        if lock(&self.session).is_some() {
            return Err("stop the recording first".into());
        }
        if let Err(e) = prepare() {
            // `prepare` may have queued wipes before it failed.
            self.undo_prepare();
            return Err(e);
        }
        if self.deleting.swap(true, Ordering::AcqRel) {
            self.undo_prepare();
            return Err("all data is already being deleted".into());
        }
        // From here `store()` refuses: nothing reopens the database.
        self.answers.clear();
        let r = self.delete_locked();
        if r.is_err() {
            // Nothing was deleted: the next `store()` opens it again, with
            // its job runner, once the last user has let go.
            *lock(&self.settings) = None;
        }
        self.deleting.store(false, Ordering::Release);
        if r.is_err() {
            self.undo_prepare();
        }
        r
    }

    /// The delete did not happen: the devices `prepare` queued for a wipe go
    /// back to paired (nothing was removed, so nothing is to be wiped) and sync
    /// carries on. Runs once `store()` works again.
    fn undo_prepare(&self) {
        if let Some(s) = self.sync_service() {
            s.rollback_wipes();
            s.resume();
        }
    }

    fn delete_locked(&self) -> Result<(), String> {
        // Closes the listener and ends the sessions: they hold the store.
        if let Some(s) = self.sync_service() {
            s.stop_for_delete();
        }
        self.shutdown(std::time::Duration::from_secs(5));
        *lock(&self.runner) = None;
        *lock(&self.llm) = None;
        let Some(mut store) = lock(&self.store).take() else {
            return Err("the store is not open".into());
        };
        // Short-lived users (an audio response, a command) let go quickly.
        let since = std::time::Instant::now();
        let store = loop {
            match Arc::try_unwrap(store) {
                Ok(s) => break s,
                Err(again) if since.elapsed() < std::time::Duration::from_secs(10) => {
                    store = again;
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(_) => return Err("the data is still in use: try again in a moment".into()),
            }
        };
        let dir = self.data.join("store");
        let ks = self.keystore(&dir)?;
        store
            .delete_all(ks.as_ref(), Protection::default())
            .map_err(|e| e.to_string())?;
        if let Ok(s) = self.secrets() {
            for p in crate::cloud_cmd::PROVIDERS {
                // The data is already gone; a key left behind is removable
                // in Settings → AI.
                let _ = s.delete(&format!("provider-{p}"));
            }
            // The sync identity goes with the data: paired devices refuse
            // this device from now on (doc 07 §3.5).
            if let Err(e) = ghi_sync::identity::Identity::delete(s.as_ref()) {
                log::warn!("removing the sync identity failed: {e}");
            }
        }
        *lock(&self.settings) = None;
        Ok(())
    }

    /// The desktop's job handlers: live and final notes, the final pass, voice
    /// learning and (with `embeddings`) the semantic index.
    fn desktop_handlers(
        &self,
        store: &Arc<Store>,
        models: &Path,
    ) -> Result<Vec<Arc<dyn ghi_core::jobs::JobHandler>>, String> {
        let models = models.to_path_buf();
        let store = store.clone();
        let speech_models = models.clone();
        #[cfg(feature = "embeddings")]
        let embed_dir = models.clone();
        let template = ghi_llm::template::builtin("general").map_err(|e| e.to_string())?;
        let notes_ready: ghi_core::jobs::Ready = {
            let dir = models.clone();
            Arc::new(move || llm_ready(&dir))
        };
        let llm = llm_factory(&models);
        *lock(&self.llm) = Some(llm.clone());
        #[allow(unused_mut)]
        let mut handlers: Vec<Arc<dyn ghi_core::jobs::JobHandler>> = vec![
            Arc::new(ghi_core::notes_job::NotesJob {
                kind: ghi_core::session::NOTES_LIVE_JOB,
                version: 1,
                template: template.clone(),
                llm: llm.clone(),
                ready: notes_ready.clone(),
            }),
            Arc::new(ghi_core::final_pass::FinalPassJob {
                engines: {
                    let models = models.clone();
                    let store = store.clone();
                    Arc::new(move || final_engines(&models, &store))
                },
                chunk_s: 600.0,
                ready: Arc::new(move || speech_ready(&speech_models)),
                voice: Some(voice_step(&models, &store)),
            }),
            // After a "This is me" or an accepted suggestion: add that
            // voice to the profile (checks consent again when it runs).
            Arc::new(ghi_core::voice_job::VoiceLearnJob {
                embedder: voice_factory(&models),
                ready: voice_ready_fn(&models),
                third_party: third_party_gate(&store),
            }),
            Arc::new(ghi_core::notes_job::NotesJob {
                kind: ghi_core::notes_job::NOTES_FINAL_JOB,
                version: 2,
                template,
                llm,
                ready: notes_ready,
            }),
        ];
        // Semantic search: lowest priority, after the notes.
        #[cfg(feature = "embeddings")]
        handlers.push(Arc::new(ghi_core::index_job::IndexJob {
            embedder: {
                let dir = embed_dir.clone();
                Arc::new(move || {
                    ghi_llm::embed::LocalEmbedder::open_registry_in(
                        &dir,
                        ghi_core::index_job::MODEL_ID,
                    )
                    .map(|e| Box::new(e) as Box<dyn ghi_llm::embed::Embedder + Send>)
                    .map_err(|e| e.to_string())
                })
            },
            ready: {
                let dir = embed_dir;
                Arc::new(move || embed_ready(&dir))
            },
            warm_gate: {
                let locked = self.locked.clone();
                Some(Arc::new(move || {
                    !locked.load(std::sync::atomic::Ordering::Acquire)
                }))
            },
        }));
        Ok(handlers)
    }

    /// Opens the store on first use, runs crash recovery and starts the job runner.
    pub fn store(&self) -> Result<Arc<Store>, String> {
        if self.deleting.load(std::sync::atomic::Ordering::Acquire) {
            return Err("all data is being deleted".into());
        }
        if self.locked() {
            return Err("the app is locked".into());
        }
        if self.hooks.gate_launch
            && !self
                .launch_checked
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("the app is starting".into());
        }
        self.store_even_locked()
    }

    /// The launch lock check ran: `store()` serves content from now on.
    pub fn mark_launch_checked(&self) {
        self.launch_checked
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// The store for what keeps working while the app is locked: starting a
    /// recording (⌘⇧R, detection) and imports. Never for a command that
    /// returns content to the webview.
    pub fn store_even_locked(&self) -> Result<Arc<Store>, String> {
        if self.deleting.load(std::sync::atomic::Ordering::Acquire) {
            return Err("all data is being deleted".into());
        }
        let mut slot = lock(&self.store);
        if let Some(s) = slot.as_ref() {
            return Ok(s.clone());
        }
        *lock(&self.open_problem) = None;
        match self.open_new() {
            Ok(store) => {
                *slot = Some(store.clone());
                Ok(store)
            }
            Err(e) => {
                // The store opened but starting on top of it failed.
                let problem = *lock(&self.open_problem).get_or_insert(StoreProblem::Startup);
                log::warn!("opening the store failed: {}", problem.code());
                Err(e)
            }
        }
    }

    /// Why the store can't be opened (`None`: it is open, or nothing failed
    /// yet). Every `store()` call tries again, so this is also "try again".
    pub fn open_problem(&self) -> Option<StoreProblem> {
        *lock(&self.open_problem)
    }

    /// Removes the store that can't be opened and its key, then opens a new
    /// empty one (the app starts over at onboarding).
    ///
    /// Only for data that is gone for good: under the store lock the open is
    /// tried once more, and the wipe goes ahead only if it still fails with
    /// [`StoreProblem::KeyMissing`] or [`StoreProblem::Damaged`]. A store that
    /// opens, a locked key, a keystore or disk error, a failed upgrade all
    /// refuse with the problem's code (`storeReadable` when it opened), so a
    /// stray call can't erase meetings that a retry would open. An open store
    /// is deleted with [`Core::delete_everything`] instead.
    pub fn start_fresh(&self) -> Result<(), String> {
        use std::sync::atomic::Ordering;
        if self.deleting.swap(true, Ordering::AcqRel) {
            return Err("all data is already being deleted".into());
        }
        let r = self.start_fresh_locked();
        self.deleting.store(false, Ordering::Release);
        r?;
        self.store_even_locked().map(|_| ())
    }

    fn start_fresh_locked(&self) -> Result<(), String> {
        let slot = lock(&self.store);
        if slot.is_some() {
            return Err("storeReadable".into());
        }
        let dir = self.data.join("store");
        let keys = self
            .keystore(&dir)
            .map_err(|_| StoreProblem::Keystore.code().to_owned())?;
        match Store::open(&dir, keys.clone(), Protection::default()) {
            // It opens after all: nothing to start over from. Dropped again;
            // the normal open runs recovery and the job runner.
            Ok(_) => return Err("storeReadable".into()),
            Err(e) => {
                let problem = StoreProblem::of(&e);
                if !problem.allows_start_fresh() {
                    return Err(problem.code().to_owned());
                }
            }
        }
        // The key goes first: from here the old data can't be read, whatever
        // happens to the files (crypto-shred).
        keys.delete().map_err(|e| {
            log::warn!("start fresh: removing the key failed: {e}");
            StoreProblem::Keystore.code().to_owned()
        })?;
        if let Err(e) = std::fs::remove_dir_all(&dir)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            // Sealed without its key; a new store can't be created over them,
            // so this is reported and "Start fresh" can be run again.
            log::warn!("start fresh: removing the store files failed: {e}");
            return Err(StoreProblem::Disk.code().to_owned());
        }
        if let Ok(s) = self.secrets() {
            for p in crate::cloud_cmd::PROVIDERS {
                let _ = s.delete(&format!("provider-{p}"));
            }
            let _ = ghi_sync::identity::Identity::delete(s.as_ref());
        }
        *lock(&self.settings) = None;
        *lock(&self.open_problem) = None;
        Ok(())
    }

    fn open_new(&self) -> Result<Arc<Store>, String> {
        let dir = self.data.join("store");
        std::fs::create_dir_all(&dir).map_err(|e| {
            *lock(&self.open_problem) = Some(StoreProblem::Disk);
            e.to_string()
        })?;
        let keys = self.keystore(&dir).inspect_err(|_| {
            *lock(&self.open_problem) = Some(StoreProblem::Keystore);
        })?;
        let store = Arc::new(Store::open(&dir, keys, Protection::default()).map_err(|e| {
            let problem = StoreProblem::of(&e);
            // The code only: the error text can carry paths.
            log::warn!("opening the store failed: {}", problem.code());
            *lock(&self.open_problem) = Some(problem);
            e.to_string()
        })?);
        // A test recording interrupted by a quit or crash goes first, so
        // recovery doesn't turn it into a meeting with jobs.
        if let Some(test) = Self::pending_test(&store) {
            let _ = store.delete_meeting(&test);
            let _ = store.set_setting(TEST_MEETING_KEY, &serde_json::Value::Null);
        }
        let recovered = match &self.hooks.recover_kinds {
            Some(kinds) => ghi_core::recover::recover_with_kinds(&store, kinds)?,
            None => ghi_core::recover::recover(&store)?,
        };
        if self.hooks.no_local_notes {
            match ghi_core::recover::drop_local_notes_jobs(&store) {
                Ok(0) => {}
                Ok(n) => log::info!("recovery: {n} notes job(s) called off (no local notes here)"),
                Err(e) => log::warn!("recovery of notes jobs: {e}"),
            }
        }
        if self.hooks.requeue_unfinished_stops {
            match ghi_core::recover::requeue_unfinished_stops(&store) {
                Ok(0) => {}
                Ok(n) => log::info!("recovery: {n} stopped recording(s) queued for the final pass"),
                Err(e) => log::warn!("recovery of unfinished stops: {e}"),
            }
        }
        *lock(&self.recovered) = recovered.meetings;
        // Names given before 14c get their person rows (idempotent).
        if let Err(e) = store.link_named_speakers() {
            log::warn!("linking named speakers: {e}");
        }
        let models = self.models();
        let handlers = match &self.hooks.handlers {
            Some(f) => f(&store, &models),
            None => self.desktop_handlers(&store, &models)?,
        };
        let runner = JobRunner::new(store.clone(), self.events.clone(), handlers);
        // Jobs a paired phone leased to this device run under the lease's
        // fence (doc 07 §8); jobs without a lease never ask it.
        runner.set_fence(crate::sync_service::lease_fence(store.clone()));
        if self.hooks.handlers.is_none() {
            // Light machines search by keywords only (no embedding model).
            let _ = ghi_core::index_job::set_enabled(
                &store,
                cfg!(feature = "embeddings") && preset().embed_id.is_some(),
            );
            let _ = ghi_core::index_job::queue_missing(&store);
        }
        if let Some(f) = &self.hooks.before_spawn {
            f(&runner);
        }
        *lock(&self.runner_thread) = Some(runner.spawn().map_err(|e| e.to_string())?);
        *lock(&self.runner) = Some(runner);
        Ok(store)
    }

    pub fn start(
        &self,
        mode: Mode,
        language: Option<String>,
        title: String,
    ) -> Result<String, String> {
        // Taken now: an arm from another window while the engines load is
        // for the next recording. Put back if this one does not start.
        let sensitive = self
            .sensitive_next
            .swap(false, std::sync::atomic::Ordering::AcqRel);
        self.start_with(mode, language, title, true, sensitive)
            .inspect_err(|_| {
                if sensitive {
                    self.set_sensitive_next(true);
                }
            })
    }

    /// Whether the next recording will be sensitive.
    pub fn sensitive_next(&self) -> bool {
        self.sensitive_next
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn set_sensitive_next(&self, on: bool) {
        self.sensitive_next
            .store(on, std::sync::atomic::Ordering::Release);
    }

    /// The onboarding's test recording: a short session whose meeting is
    /// deleted afterwards (levels and a line of transcript reach the UI as
    /// usual core events for the returned meeting id).
    pub fn start_test(self: &Arc<Self>, seconds: u32) -> Result<String, String> {
        let id = self.start_with(Mode::Call, None, String::new(), false, false)?;
        // Remembered in the store too: a quit or crash during the test must
        // not leave it behind as a meeting (deleted at the next launch,
        // before crash recovery would process it).
        let store = self.store_even_locked()?;
        store
            .set_setting(TEST_MEETING_KEY, &serde_json::json!(id))
            .map_err(|e| e.to_string())?;
        let me = self.clone();
        let meeting = id.clone();
        std::thread::Builder::new()
            .name("ghi-test-capture".into())
            .spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(u64::from(
                    seconds.clamp(3, 30),
                )));
                me.end_test(&meeting);
            })
            .map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// Stops the test recording if it is still the one running (checked and
    /// stopped under the lifecycle lock, so a real recording is never hit)
    /// and deletes its meeting.
    fn end_test(&self, meeting: &str) {
        {
            let _lifecycle = lock(&self.lifecycle);
            let current = lock(&self.session).as_ref().map(|s| s.meeting() == meeting);
            if current == Some(true) {
                let _ = self.stop_locked();
            }
        }
        if let Ok(store) = self.store() {
            let _ = store.delete_meeting(meeting);
            let _ = store.set_setting(TEST_MEETING_KEY, &serde_json::Value::Null);
        }
    }

    /// The test recording's meeting, if one is running or was left behind.
    fn pending_test(store: &Store) -> Option<String> {
        store
            .get_setting(TEST_MEETING_KEY)
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(String::from))
    }

    fn start_with(
        &self,
        mode: Mode,
        language: Option<String>,
        title: String,
        queue_jobs: bool,
        sensitive: bool,
    ) -> Result<String, String> {
        let _lifecycle = lock(&self.lifecycle);
        if lock(&self.session).is_some() {
            return Err("a recording is already running".into());
        }
        // The mic is the recording's: a voice enrollment in progress ends.
        crate::voice_cmd::drop_enrollment(&self.enrollment);
        let store = self.store_even_locked()?;
        // Models missing: record now, transcribe when they arrive. Engines
        // that fail to load must not cost the recording either. They load
        // after the jobs paused (the session calls this once recording is
        // announced), so the notes LLM and NeMo are never resident together.
        let models = self.models();
        let events = self.events.clone();
        // Fast / Accurate (doc 02 §B), read when the recording starts.
        let chunk_ms = crate::system::load_settings(self)
            .map(|s| s.live_mode)
            .unwrap_or_default()
            .chunk_ms(preset().tier);
        let load = move || {
            if !speech_ready(&models) {
                return Ok(None);
            }
            Ok(engines(&models, chunk_ms)
                .inspect_err(|e| {
                    events.emit(ghi_core::events::Event::Error {
                        meeting: None,
                        kind: ErrorKind::Engine,
                        message: format!("speech engines: {e}"),
                    })
                })
                .ok())
        };
        let settings = crate::system::load_settings(self).unwrap_or_default();
        let want_app_only = mode == Mode::Call && settings.app_audio_only;
        let tap_pids = if want_app_only {
            meeting_app_pids(&settings.detect_apps)
        } else {
            Vec::new()
        };
        // No meeting app in a call: all system audio is recorded, and the
        // recording says so. The pids are resolved once, here.
        let app_audio_fallback = want_app_only && tap_pids.is_empty();
        let capture =
            ghi_core::capture::live(mode == Mode::Call, &tap_pids).map_err(|e| e.to_string())?;
        let hooks = lock(&self.runner)
            .clone()
            .map(|r| r as Arc<dyn ghi_core::session::RecordingHooks>);
        let s = Session::start_with_loader(
            store,
            load,
            capture,
            SessionConfig {
                sensitive,
                mode,
                language,
                title,
                queue_jobs,
                lossless: false,
                echo_cancellation: settings.echo_cancellation,
            },
            self.events.clone(),
            hooks,
        )
        .map_err(|e| e.to_string())?;
        let id = s.meeting().to_string();
        *lock(&self.session) = Some(Arc::new(s));
        if app_audio_fallback {
            self.events.emit(ghi_core::events::Event::AppAudioFallback {
                meeting: id.clone(),
            });
        }
        Ok(id)
    }

    pub fn stop(&self) -> Result<ghi_core::session::StopReport, String> {
        let _lifecycle = lock(&self.lifecycle);
        let report = self.stop_locked();
        // Stopping the onboarding test (quit, menu): it is not a meeting.
        if let (Ok(r), Ok(store)) = (&report, self.store())
            && Self::pending_test(&store).as_deref() == Some(r.meeting.as_str())
        {
            let _ = store.delete_meeting(&r.meeting);
            let _ = store.set_setting(TEST_MEETING_KEY, &serde_json::Value::Null);
        }
        report
    }

    /// `stop` with the lifecycle lock already held.
    fn stop_locked(&self) -> Result<ghi_core::session::StopReport, String> {
        let mut s = lock(&self.session).take().ok_or("nothing is recording")?;
        // Wait for commands still using the session (a discard, a split).
        let session = loop {
            match Arc::try_unwrap(s) {
                Ok(session) => break session,
                Err(again) => {
                    s = again;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
        };
        session.stop().map_err(|e| e.to_string())
    }

    /// Runs `f` on the recording in progress, without holding the lock.
    /// The recording in progress, for a command from the window: refused
    /// while the app is locked (its transcript and speakers are content).
    pub fn with_session_unlocked<T>(&self, f: impl FnOnce(&Session) -> T) -> Result<T, String> {
        if self.locked() {
            return Err("the app is locked".into());
        }
        self.with_session(f)
    }

    pub fn with_session<T>(&self, f: impl FnOnce(&Session) -> T) -> Result<T, String> {
        let s = lock(&self.session).clone().ok_or("nothing is recording")?;
        Ok(f(&s))
    }

    /// Cloud API keys (never shown, never sent anywhere but their provider).
    pub fn secrets(&self) -> Result<Box<dyn ghi_store::keys::secrets::SecretStore>, String> {
        match &self.hooks.secrets {
            Some(f) => f(),
            None => secrets(&self.data),
        }
    }

    fn keystore(&self, dir: &Path) -> Result<Arc<dyn KeyStore>, String> {
        match &self.hooks.keystore {
            Some(k) => Ok(k.clone()),
            None => keystore(dir),
        }
    }

    /// The event bus (the phone's recorder and lifecycle emit on it).
    pub fn events(&self) -> EventTx {
        self.events.clone()
    }

    /// The job runner, once the store is open.
    pub fn runner(&self) -> Option<Arc<JobRunner>> {
        lock(&self.runner).clone()
    }

    /// The microphone hook for voice enrollment (see [`MicSource`]).
    pub fn mic_source(&self) -> Option<MicSource> {
        self.hooks.mic.clone()
    }

    /// The local notes model's factory (once the store is open).
    pub fn llm(&self) -> Result<ghi_core::notes_job::LlmFactory, String> {
        self.store_even_locked()?;
        lock(&self.llm)
            .clone()
            .ok_or_else(|| "the notes model is not set up".into())
    }

    /// Wakes the job runner (models arrived, a job was queued).
    pub fn notify_jobs(&self) {
        if let Some(r) = lock(&self.runner).as_ref() {
            r.notify();
        }
    }

    /// Imports one recording from several participants' tracks (a staged
    /// Zoom group); see `ghi_core::import::import_tracks`.
    pub fn import_tracks(
        &self,
        files: &[(PathBuf, Option<String>)],
        opts: ghi_core::import::ImportOptions,
    ) -> Result<ghi_core::import::ImportReport, String> {
        if files.iter().any(|(p, _)| !p.is_absolute() || !p.is_file()) {
            return Err("not a file".into());
        }
        let store = self.store_even_locked()?;
        let r = ghi_core::import::import_tracks(&store, files, &opts, &self.events)?;
        if let Some(runner) = lock(&self.runner).as_ref() {
            runner.notify();
        }
        Ok(r)
    }

    /// Imports a file the user chose (staged by import_cmd.rs).
    pub fn import(
        &self,
        path: &Path,
        opts: ghi_core::import::ImportOptions,
    ) -> Result<ghi_core::import::ImportReport, String> {
        if !path.is_absolute() || !path.is_file() {
            return Err("not a file".into());
        }
        let store = self.store_even_locked()?;
        let r = ghi_core::import::import_file(&store, path, &opts, &self.events)?;
        if let Some(runner) = lock(&self.runner).as_ref() {
            runner.notify();
        }
        Ok(r)
    }
}

#[cfg(test)]
mod warm_tests {
    use super::*;

    #[test]
    fn warming_never_touches_the_store_while_locked_or_before_the_launch_check() {
        let tmp = tempfile::tempdir().unwrap();
        let (core, _rx) = Core::for_test(tmp.path().to_path_buf());
        core.set_locked(true);
        core.warm_vectors_now();
        assert!(lock(&core.store).is_none(), "locked: store not even opened");

        let (core, _rx) = Core::for_test_with(
            tmp.path().to_path_buf(),
            CoreHooks {
                gate_launch: true,
                ..CoreHooks::default()
            },
        );
        core.warm_vectors_now();
        assert!(lock(&core.store).is_none(), "before the launch lock check");
    }

    #[test]
    fn one_warm_runs_at_a_time_and_a_request_meanwhile_goes_round_once_more() {
        use std::sync::atomic::Ordering::SeqCst;
        let tmp = tempfile::tempdir().unwrap();
        let (core, _rx) = Core::for_test(tmp.path().to_path_buf());
        assert!(core.claim_warm());
        assert!(!core.claim_warm(), "second lock/unlock doesn't stack");
        assert!(core.warm_again.load(SeqCst));
        // The runner finishes its pass, sees the request and keeps the slot.
        core.set_locked(true);
        core.warm_claimed();
        assert!(!core.warming.load(SeqCst), "slot given back");
        assert!(!core.warm_again.load(SeqCst));
        assert!(core.claim_warm());
    }
}
