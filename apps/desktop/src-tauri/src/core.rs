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
    /// "Delete all data" is running: the store must not be reopened.
    deleting: std::sync::atomic::AtomicBool,
    runner: Mutex<Option<Arc<JobRunner>>>,
    /// Meetings closed by crash recovery at this launch (D12 "recovered"),
    /// until the user dismisses the notice.
    recovered: Mutex<Vec<String>>,
    /// App settings as last read or written (system.rs).
    settings: Mutex<Option<crate::system::AppSettings>>,
    /// The runner's thread, joined (bounded) at shutdown.
    runner_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Opens the local notes model (the jobs' factory; "Ask this meeting").
    llm: Mutex<Option<ghi_core::notes_job::LlmFactory>>,
    events: EventTx,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
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

#[cfg(target_os = "macos")]
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

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_keystore(_dir: &Path) -> Result<Arc<dyn KeyStore>, String> {
    Err("no OS key store on this platform yet".into())
}

/// Where cloud API keys live: the Keychain (Windows: DPAPI files); debug
/// builds use files next to the data directory unless `GHI_KEYSTORE=keychain`.
pub(crate) fn secrets(
    data: &Path,
) -> Result<Box<dyn ghi_store::keys::secrets::SecretStore>, String> {
    #[cfg(debug_assertions)]
    if std::env::var("GHI_KEYSTORE").as_deref() != Ok("keychain") {
        return Ok(Box::new(ghi_store::keys::secrets::FileSecrets::new(
            data.join("store.devsecrets"),
        )));
    }
    platform_secrets(data)
}

#[cfg(target_os = "macos")]
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

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_secrets(
    _data: &Path,
) -> Result<Box<dyn ghi_store::keys::secrets::SecretStore>, String> {
    Err("no OS key store on this platform yet".into())
}

/// Models whose file failed its SHA-256 at load (D12 "model damaged"): the UI
/// offers a re-download; a successful download clears the mark.
static DAMAGED: Mutex<std::collections::BTreeSet<String>> =
    Mutex::new(std::collections::BTreeSet::new());

pub(crate) fn damaged_models() -> std::collections::BTreeSet<String> {
    lock(&DAMAGED).clone()
}

pub(crate) fn clear_damaged(id: &str) {
    lock(&DAMAGED).remove(id);
}

/// Verifies a model before native code parses it, remembering damage.
pub(crate) fn checked_model(models: &Path, id: &str) -> Result<PathBuf, String> {
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
fn preset() -> &'static ghi_models::Preset {
    static PRESET: OnceLock<ghi_models::Preset> = OnceLock::new();
    PRESET.get_or_init(|| ghi_models::preset(ghi_models::tier_for(&ghi_models::detect())))
}

/// This build has speech engines and their models are installed. Without
/// them a recording keeps the audio only and its jobs wait (doc 02 §L).
pub(crate) fn speech_ready(models: &Path) -> bool {
    let ids: Vec<&str> = preset().speech_models.iter().map(String::as_str).collect();
    cfg!(feature = "nemo") && ghi_models::installed(models, &ids)
}

/// The notes model for this machine's tier is installed.
pub(crate) fn llm_ready(models: &Path) -> bool {
    ghi_models::installed(models, &[preset().llm_id])
}

/// Speech engines for a recording (live chunk) or the final pass (1120 ms).
#[cfg(feature = "nemo")]
fn engines(
    models: &Path,
    chunk_ms: u32,
) -> Result<Arc<dyn ghi_core::engines::SpeechEngines>, String> {
    // Checked against their pinned SHA-256 before native code parses them.
    let path = |id: &str| checked_model(models, id);
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

impl Core {
    /// Starts forwarding core events to the webview.
    /// `on_event` also sees every event (the tray follows the session).
    pub fn new<R: Runtime>(
        app: &AppHandle<R>,
        on_event: impl Fn(&ghi_core::events::Event) + Send + 'static,
    ) -> Result<Core, String> {
        let data = app.path().app_data_dir().map_err(|e| e.to_string())?;
        let (events, rx) = bus();
        let handle = app.clone();
        std::thread::Builder::new()
            .name("ghi-events".into())
            .spawn(move || {
                for env in rx {
                    on_event(&env.event);
                    let _ = CoreEvent(env).emit(&handle);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Core {
            data,
            store: Mutex::new(None),
            session: Mutex::new(None),
            lifecycle: Mutex::new(()),
            deleting: std::sync::atomic::AtomicBool::new(false),
            runner: Mutex::new(None),
            runner_thread: Mutex::new(None),
            llm: Mutex::new(None),
            settings: Mutex::new(None),
            recovered: Mutex::new(Vec::new()),
            events,
        })
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
                }
            });
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

    pub fn settings_cache(&self) -> &Mutex<Option<crate::system::AppSettings>> {
        &self.settings
    }

    /// Stops the job runner for a quit (not a crash): the running job yields
    /// at its next checkpoint, or — if it can't within `wait` (notes mid-LLM
    /// call) — its claim goes back to the queue without spending an attempt.
    /// Then the LLM worker processes are killed so none outlives the app.
    pub fn shutdown(&self, wait: std::time::Duration) {
        if let Some(r) = lock(&self.runner).as_ref() {
            r.shutdown_and_release(wait);
        }
        ghi_llm::local::kill_workers();
        if let Some(t) = lock(&self.runner_thread).take()
            && t.is_finished()
        {
            let _ = t.join();
        }
    }

    pub fn models(&self) -> PathBuf {
        self.data.join("models")
    }

    /// "Delete all data" (Settings → Privacy): stops the jobs, crypto-shreds
    /// every meeting and removes the database, audio, snapshots and cloud
    /// keys; a fresh key ring is saved. The app restarts into onboarding.
    pub fn delete_everything(&self) -> Result<(), String> {
        use std::sync::atomic::Ordering;
        // No recording can start meanwhile.
        let _lifecycle = lock(&self.lifecycle);
        if lock(&self.session).is_some() {
            return Err("stop the recording first".into());
        }
        if self.deleting.swap(true, Ordering::AcqRel) {
            return Err("all data is already being deleted".into());
        }
        // From here `store()` refuses: nothing reopens the database.
        let r = self.delete_locked();
        if r.is_err() {
            // Nothing was deleted: the next `store()` opens it again, with
            // its job runner, once the last user has let go.
            *lock(&self.settings) = None;
        }
        self.deleting.store(false, Ordering::Release);
        r
    }

    fn delete_locked(&self) -> Result<(), String> {
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
        let ks = keystore(&dir)?;
        store
            .delete_all(ks.as_ref(), Protection::default())
            .map_err(|e| e.to_string())?;
        if let Ok(s) = self.secrets() {
            for p in crate::cloud_cmd::PROVIDERS {
                // The data is already gone; a key left behind is removable
                // in Settings → AI.
                let _ = s.delete(&format!("provider-{p}"));
            }
        }
        *lock(&self.settings) = None;
        Ok(())
    }

    /// Opens the store on first use, runs crash recovery and starts the job runner.
    pub fn store(&self) -> Result<Arc<Store>, String> {
        if self.deleting.load(std::sync::atomic::Ordering::Acquire) {
            return Err("all data is being deleted".into());
        }
        let mut slot = lock(&self.store);
        if let Some(s) = slot.as_ref() {
            return Ok(s.clone());
        }
        let dir = self.data.join("store");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let store = Arc::new(
            Store::open(&dir, keystore(&dir)?, Protection::default()).map_err(|e| e.to_string())?,
        );
        // A test recording interrupted by a quit or crash goes first, so
        // recovery doesn't turn it into a meeting with jobs.
        if let Some(test) = Self::pending_test(&store) {
            let _ = store.delete_meeting(&test);
            let _ = store.set_setting(TEST_MEETING_KEY, &serde_json::Value::Null);
        }
        let recovered = ghi_core::recover::recover(&store)?;
        *lock(&self.recovered) = recovered.meetings;
        let models = self.models();
        let template = ghi_llm::template::builtin("general").map_err(|e| e.to_string())?;
        // The notes model lives with the speech models in the app data dir.
        let llm_dir = models.clone();
        let notes_ready: ghi_core::jobs::Ready = {
            let dir = models.clone();
            Arc::new(move || llm_ready(&dir))
        };
        let llm: ghi_core::notes_job::LlmFactory = Arc::new(move |bytes| {
            let n_ctx = ((bytes / 3) as u32 * 6 / 5 + 6_144).clamp(8_192, 32_768);
            {
                // Marks a damaged file for the UI (the worker checks it too).
                checked_model(&llm_dir, preset().llm_id)?;
                ghi_llm::local::LocalLlm::open_registry_in(&llm_dir, preset().llm_id, n_ctx)
            }
            .map(|l| Box::new(l) as Box<dyn ghi_llm::Llm + Send>)
            .map_err(|e| e.to_string())
        });
        *lock(&self.llm) = Some(llm.clone());
        let runner = JobRunner::new(
            store.clone(),
            self.events.clone(),
            vec![
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
                        Arc::new(move || engines(&models, 1120))
                    },
                    chunk_s: 600.0,
                    ready: Arc::new(move || speech_ready(&models)),
                }),
                Arc::new(ghi_core::notes_job::NotesJob {
                    kind: ghi_core::notes_job::NOTES_FINAL_JOB,
                    version: 2,
                    template,
                    llm,
                    ready: notes_ready,
                }),
            ],
        );
        *lock(&self.runner_thread) = Some(runner.spawn().map_err(|e| e.to_string())?);
        *lock(&self.runner) = Some(runner);
        *slot = Some(store.clone());
        Ok(store)
    }

    pub fn start(
        &self,
        mode: Mode,
        language: Option<String>,
        title: String,
    ) -> Result<String, String> {
        self.start_with(mode, language, title, true)
    }

    /// The onboarding's test recording: a short session whose meeting is
    /// deleted afterwards (levels and a line of transcript reach the UI as
    /// usual core events for the returned meeting id).
    pub fn start_test(self: &Arc<Self>, seconds: u32) -> Result<String, String> {
        let id = self.start_with(Mode::Call, None, String::new(), false)?;
        // Remembered in the store too: a quit or crash during the test must
        // not leave it behind as a meeting (deleted at the next launch,
        // before crash recovery would process it).
        let store = self.store()?;
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
    ) -> Result<String, String> {
        let _lifecycle = lock(&self.lifecycle);
        if lock(&self.session).is_some() {
            return Err("a recording is already running".into());
        }
        let store = self.store()?;
        // Models missing: record now, transcribe when they arrive. Engines
        // that fail to load must not cost the recording either. They load
        // after the jobs paused (the session calls this once recording is
        // announced), so the notes LLM and NeMo are never resident together.
        let models = self.models();
        let events = self.events.clone();
        let load = move || {
            if !speech_ready(&models) {
                return Ok(None);
            }
            Ok(engines(&models, preset().asr_chunk_ms)
                .inspect_err(|e| {
                    events.emit(ghi_core::events::Event::Error {
                        meeting: None,
                        kind: ErrorKind::Engine,
                        message: format!("speech engines: {e}"),
                    })
                })
                .ok())
        };
        let capture =
            ghi_core::capture::live(mode == Mode::Call, &[]).map_err(|e| e.to_string())?;
        let hooks = lock(&self.runner)
            .clone()
            .map(|r| r as Arc<dyn ghi_core::session::RecordingHooks>);
        let s = Session::start_with_loader(
            store,
            load,
            capture,
            SessionConfig {
                mode,
                language,
                title,
                queue_jobs,
                lossless: false,
            },
            self.events.clone(),
            hooks,
        )
        .map_err(|e| e.to_string())?;
        let id = s.meeting().to_string();
        *lock(&self.session) = Some(Arc::new(s));
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
    pub fn with_session<T>(&self, f: impl FnOnce(&Session) -> T) -> Result<T, String> {
        let s = lock(&self.session).clone().ok_or("nothing is recording")?;
        Ok(f(&s))
    }

    /// Cloud API keys (never shown, never sent anywhere but their provider).
    pub fn secrets(&self) -> Result<Box<dyn ghi_store::keys::secrets::SecretStore>, String> {
        secrets(&self.data)
    }

    /// The local notes model's factory (once the store is open).
    pub fn llm(&self) -> Result<ghi_core::notes_job::LlmFactory, String> {
        self.store()?;
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

    /// Imports a file the user chose (staged by import_cmd.rs).
    pub fn import(
        &self,
        path: &Path,
        opts: ghi_core::import::ImportOptions,
    ) -> Result<ghi_core::import::ImportReport, String> {
        if !path.is_absolute() || !path.is_file() {
            return Err("not a file".into());
        }
        let store = self.store()?;
        let r = ghi_core::import::import_file(&store, path, &opts, &self.events)?;
        if let Some(runner) = lock(&self.runner).as_ref() {
            runner.notify();
        }
        Ok(r)
    }
}
