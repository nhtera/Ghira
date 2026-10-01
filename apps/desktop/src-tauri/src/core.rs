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
    runner: Mutex<Option<Arc<JobRunner>>>,
    /// App settings as last read or written (system.rs).
    settings: Mutex<Option<crate::system::AppSettings>>,
    /// The runner's thread, joined (bounded) at shutdown.
    runner_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
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
    let path = |id: &str| {
        let m = ghi_models::find(id).ok_or_else(|| format!("{id} is not in the registry"))?;
        let p = ghi_models::path_in(models, &m);
        // Checked against its pinned SHA-256 before native code parses it.
        ghi_models::verify_for_load(&p, &m).map_err(|e| format!("model {id}: {e}"))?;
        Ok::<_, String>(p)
    };
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
    pub fn new<R: Runtime>(app: &AppHandle<R>) -> Result<Core, String> {
        let data = app.path().app_data_dir().map_err(|e| e.to_string())?;
        let (events, rx) = bus();
        let handle = app.clone();
        std::thread::Builder::new()
            .name("ghi-events".into())
            .spawn(move || {
                for env in rx {
                    let _ = CoreEvent(env).emit(&handle);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Core {
            data,
            store: Mutex::new(None),
            session: Mutex::new(None),
            lifecycle: Mutex::new(()),
            runner: Mutex::new(None),
            runner_thread: Mutex::new(None),
            settings: Mutex::new(None),
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

    /// Opens the store on first use, runs crash recovery and starts the job runner.
    pub fn store(&self) -> Result<Arc<Store>, String> {
        let mut slot = lock(&self.store);
        if let Some(s) = slot.as_ref() {
            return Ok(s.clone());
        }
        let dir = self.data.join("store");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let store = Arc::new(
            Store::open(&dir, keystore(&dir)?, Protection::default()).map_err(|e| e.to_string())?,
        );
        ghi_core::recover::recover(&store)?;
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
            ghi_llm::local::LocalLlm::open_registry_in(&llm_dir, preset().llm_id, n_ctx)
                .map(|l| Box::new(l) as Box<dyn ghi_llm::Llm + Send>)
                .map_err(|e| e.to_string())
        });
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
                queue_jobs: true,
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

    /// Wakes the job runner (models arrived, a job was queued).
    pub fn notify_jobs(&self) {
        if let Some(r) = lock(&self.runner).as_ref() {
            r.notify();
        }
    }

    pub fn import(
        &self,
        path: &Path,
        split_channels: bool,
    ) -> Result<ghi_core::import::ImportReport, String> {
        // Only a real file (the webview passes a path).
        if !path.is_absolute() || !path.is_file() {
            return Err("not a file".into());
        }
        let store = self.store()?;
        let r = ghi_core::import::import_file(
            &store,
            path,
            &ghi_core::import::ImportOptions {
                split_channels,
                ..Default::default()
            },
            &self.events,
        )?;
        if let Some(runner) = lock(&self.runner).as_ref() {
            runner.notify();
        }
        Ok(r)
    }
}
