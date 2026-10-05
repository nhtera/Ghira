// SPDX-License-Identifier: Apache-2.0
//! The phone's app layer: builds the shared [`ghi_app::core::Core`] with the
//! mobile hooks (no local notes or embeddings; the final pass and voice
//! learning from [`crate::engine::job_handlers`]; crash recovery that queues
//! only the final pass, and nothing below the live tier), wires the recorder
//! and the lifecycle to it, and starts the background chores: the share
//! inbox, retention, the app lock and the launch recovery.
//!
//! Data lives in `Library/Application Support/Ghira` (excluded from device
//! backups); the store key is the Keychain item `com.nhtera.ghira` (a key file
//! beside the data directory in debug builds, see `ghi_app::core`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use ghi_app::core::{Core, CoreHooks};
use ghi_app::sync_service::{SyncConfig, SyncService};
use ghi_app::system::MeetingLanguage;
use tauri::{AppHandle, Manager};

use crate::cmd::lifecycle::TierClass;
use crate::cmd::types::ProcessingTarget;
use crate::inbox::Inbox;
use crate::lifecycle::{Lifecycle, Transition};
use crate::session::{Recorder, RecorderDeps};

/// Where the app keeps its data (the store, models, backlog, metrics).
pub fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    // iOS: Library/Application Support/Ghira. Elsewhere (host builds) a
    // subfolder of the app data dir, which is the desktop app's (the two
    // share an identifier): a stray run never touches the desktop's data.
    if cfg!(target_os = "ios") {
        Ok(app
            .path()
            .data_dir()
            .map_err(|e| e.to_string())?
            .join("Ghira"))
    } else {
        Ok(app
            .path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("mobile-dev"))
    }
}

/// The job kinds crash recovery queues on a device of this tier: the final
/// pass on a live device; nothing below the tier (those meetings stay as
/// recorded, with the `Phone` target disabled).
pub fn recover_kinds(tier: TierClass) -> Vec<&'static str> {
    match tier {
        TierClass::Live => vec![ghi_core::session::FINAL_PASS_JOB],
        TierClass::RecordOnly => vec![],
    }
}

/// Registers the `ghi-audio://` scheme (meeting audio for the webview, by
/// token only). A simulator build asked for the audio spike serves its own.
pub fn register_audio_scheme(b: tauri::Builder<tauri::Wry>) -> tauri::Builder<tauri::Wry> {
    #[cfg(feature = "test-hooks")]
    if std::env::var("GHI_SPIKE").as_deref() == Ok("audio") {
        return b.register_uri_scheme_protocol("ghi-audio", crate::spikes::audio_scheme);
    }
    b.register_asynchronous_uri_scheme_protocol("ghi-audio", move |ctx, request, responder| {
        let app = ctx.app_handle();
        let core = app.try_state::<Arc<Core>>().map(|c| c.inner().clone());
        let tokens = app
            .try_state::<Arc<ghi_app::audio_protocol::AudioTokens>>()
            .map(|t| t.inner().clone());
        std::thread::spawn(move || {
            // A bug while decoding must still answer (or the request hangs).
            let r =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match (core, tokens) {
                    (Some(core), Some(tokens)) => {
                        ghi_app::audio_protocol::respond(core.store().ok(), &tokens, &request)
                    }
                    _ => ghi_app::audio_protocol::server_error(),
                }));
            responder.respond(r.unwrap_or_else(|_| ghi_app::audio_protocol::server_error()));
        });
    })
}

/// The microphone for a voice enrollment (Me): the Swift audio tap into a
/// ring, released when the guard drops.
fn enrollment_mic() -> ghi_app::core::MicSource {
    Arc::new(crate::cmd::voice::open_mic)
}

/// Builds the core and everything around it and puts it into Tauri state.
pub fn init(app: &AppHandle) -> Result<(), String> {
    let data = data_dir(app)?;
    std::fs::create_dir_all(&data).map_err(|e| e.to_string())?;
    // Recordings, the store and the models never go into device backups.
    if let Err(e) = ghi_store::backup::exclude_from_backup(&data, true) {
        log::warn!("backup exclusion: {e}");
    }
    let tier = crate::tier::detect();
    let class = tier.tier;

    // The lifecycle needs the job runner, the core needs the lifecycle (it
    // pauses a runner created in the background): meet in a cell.
    let cell: Arc<OnceLock<Weak<Core>>> = Arc::new(OnceLock::new());
    let lifecycle: &'static Lifecycle = {
        let cell = cell.clone();
        crate::lifecycle::install(Arc::new(move || {
            cell.get().and_then(Weak::upgrade).and_then(|c| c.runner())
        }))
    };
    lifecycle.launch(launched_in_background());

    let hooks = CoreHooks {
        data_dir: Some(data.clone()),
        handlers: Some(Arc::new(move |store, models| {
            crate::engine::job_handlers(models, store, class)
        })),
        recover_kinds: Some(recover_kinds(class)),
        before_spawn: Some(Arc::new(move |runner| lifecycle.sync_runner(runner))),
        mic: Some(enrollment_mic()),
        gate_launch: true,
        ..CoreHooks::default()
    };
    let core = Arc::new(Core::with_hooks(app, |_| {}, hooks)?);
    let _ = cell.set(Arc::downgrade(&core));

    let backlog_dir = data.join("backlog");
    let metrics_dir = data.join("metrics");
    // The app makes the models folder itself: one created from outside (a
    // `devicectl` copy into a fresh container) is not writable by the app, and
    // its parent would block the folders above at the next launch.
    for d in [&backlog_dir, &metrics_dir, &core.models()] {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let recorder = Recorder::new(RecorderDeps {
        store: {
            let core = core.clone();
            Arc::new(move || core.store_even_locked())
        },
        events: core.events(),
        locked: {
            let core = core.clone();
            Arc::new(move || core.locked())
        },
        runner: {
            let core = core.clone();
            Arc::new(move || core.runner())
        },
        models: core.models(),
        backlog_dir,
        metrics_dir,
        tier,
        provider: None,
        fake_mic: crate::session::fake_mic_from_env(),
    });
    let inbox = Inbox::new(crate::share::inbox_root(&data), data.join("import-tmp"));
    // Shared audio waiting in the App Group stays out of device backups too.
    if std::fs::create_dir_all(inbox.root()).is_ok()
        && let Err(e) = ghi_store::backup::exclude_from_backup(inbox.root(), true)
    {
        log::warn!("inbox backup exclusion: {e}");
    }
    let lock = Arc::new(ghi_app::lock_cmd::Lock::default());

    app.manage(core.clone());
    app.manage(recorder.clone());
    app.manage(inbox.clone());
    app.manage(lock);
    app.manage(Arc::new(ghi_app::audio_protocol::AudioTokens::default()));
    app.manage(Arc::new(ghi_app::cloud_cmd::CloudPlans::default()));
    app.manage(Arc::new(crate::models_cmd::Downloads::default()));

    let sync = start_sync(app, &core, &recorder, lifecycle);
    app.manage(sync.clone());

    // Opens the store and recovers from a crash; the library and the
    // "recovered" notice are there on first paint.
    core.init_in_background();
    crate::privacy_cmd::spawn_maintenance(core.clone());
    spawn_inbox_watch(core.clone(), recorder.clone(), inbox, lifecycle);
    // Calendar events don't stay in memory behind the lock screen.
    {
        use tauri_specta::Event;
        ghi_app::lock_cmd::LockChanged::listen(app, |e| {
            if e.payload.locked {
                crate::cmd::calendar::forget();
            }
        });
    }
    wire_app_lock(app.clone(), core, recorder, lifecycle, sync);
    Ok(())
}

/// What this phone calls itself to its computer (iOS hides the user's device
/// name from apps).
const DEVICE_NAME: &str = "iPhone";

/// Creates the LAN sync service as the spoke and starts its session loop:
/// the events go to the webview as the typed `syncEvent`, a synced setting
/// reloads the settings, and the hours the computer may stay away come from
/// the phone settings.
fn start_sync(
    app: &AppHandle,
    core: &Arc<Core>,
    recorder: &Arc<Recorder>,
    lifecycle: &'static Lifecycle,
) -> Arc<SyncService> {
    use tauri_specta::Event;
    let (events, settings) = (app.clone(), app.clone());
    let recording = {
        let recorder = recorder.clone();
        Arc::new(move || recorder.latest().is_some())
    };
    let mut cfg = SyncConfig::spoke(
        DEVICE_NAME,
        "ios",
        crate::sync_link::MobileLink::new(recording),
        move |e: ghi_app::sync_cmd::SyncEvent| {
            let _ = e.emit(&events);
        },
        {
            let core = core.clone();
            move || core.settings_changed(&settings)
        },
    );
    cfg.offline_hours = {
        let core = core.clone();
        Arc::new(move || {
            core.store_even_locked()
                .and_then(|s| crate::cmd::settings::load(&s))
                .map_or(crate::cmd::settings::DEFAULT_OFFLINE_HOURS, |m| {
                    m.desktop_offline_hours
                })
        })
    };
    let sync = SyncService::new(core.clone(), cfg);
    sync.start();
    sync.set_app_active(lifecycle.is_active());
    sync
}

/// The app was launched in the background (a Live Activity intent, a
/// relaunch), so the job runner must start paused (Swift:
/// `ghi_swift_launched_in_background`; false off iOS).
fn launched_in_background() -> bool {
    crate::platform::launched_in_background()
}

/// Imports one inbox item with the user's (or the extension's) choices.
pub fn import_item(
    core: &Arc<Core>,
    recorder: &Arc<Recorder>,
    inbox: &Arc<Inbox>,
    id: &str,
    language: MeetingLanguage,
    target: ProcessingTarget,
) -> Result<String, String> {
    // "Delete everything" cannot begin while an import runs.
    let _guard = crate::privacy_cmd::DATA_GUARD
        .read()
        .unwrap_or_else(|e| e.into_inner());
    // Decoding waits while a recording runs.
    let hold = ghi_core::import::Hold({
        let recorder = recorder.clone();
        Arc::new(move || recorder.active().is_some())
    });
    let core = core.clone();
    inbox.import(id, language, target, Some(hold), &move |path, opts| {
        core.import(path, opts)
    })
}

const INBOX_EVERY: Duration = Duration::from_secs(3);

/// Imports the items the share extension's user confirmed. The extension tells
/// the app with a Darwin notification (`InboxChanged`, which the webview
/// answers with `inbox_list`); the app also looks every few seconds while
/// active, so an item copied while the app was in the background is picked
/// up on return.
fn spawn_inbox_watch(
    core: Arc<Core>,
    recorder: Arc<Recorder>,
    inbox: Arc<Inbox>,
    lifecycle: &'static Lifecycle,
) {
    let _ = std::thread::Builder::new()
        .name("ghi-inbox".into())
        .spawn(move || {
            inbox.sweep();
            loop {
                if lifecycle.is_active() {
                    for e in inbox.confirmed() {
                        if let Err(code) =
                            import_item(&core, &recorder, &inbox, &e.id, e.language, e.target)
                        {
                            log::warn!("inbox item not imported: {code}");
                        }
                    }
                }
                std::thread::sleep(INBOX_EVERY);
            }
        });
}

/// When the app lock engages on the phone, driven by scene transitions (never
/// polling): at launch (once the settings can be read); at
/// `didEnterBackground` at once when the delay is 0; and on `didBecomeActive`
/// when the app has been away for at least the delay, before the UI is shown.
/// The privacy cover goes up at `willResignActive` whenever the lock is on
/// (the app switcher snapshot is taken after it) and always comes down at
/// `didBecomeActive`: it is only for that snapshot, the web lock gate covers
/// the content while locked. The lock gates the store commands and the transcript events; a
/// recording carries on behind it (`store_even_locked`), and while one runs the
/// app only covers the UI in the background: it locks for real on return.
pub struct BgLock {
    /// Seconds away before locking; [`BgLock::OFF`] when the app lock is off.
    after_s: AtomicU64,
    /// When the app left, on the continuous clock (ns), while away.
    left: Mutex<Option<u64>>,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
}

impl BgLock {
    const OFF: u64 = u64::MAX;

    pub fn new(clock: Box<dyn Fn() -> u64 + Send + Sync>) -> BgLock {
        BgLock {
            after_s: AtomicU64::new(Self::OFF),
            left: Mutex::new(None),
            clock,
        }
    }

    /// The app lock setting changed (or was read for the first time).
    pub fn configure(&self, delay: Option<Duration>) {
        self.after_s
            .store(delay.map_or(Self::OFF, |d| d.as_secs()), Ordering::Release);
    }

    pub fn enabled(&self) -> bool {
        self.after_s.load(Ordering::Acquire) != Self::OFF
    }

    fn left(&self) -> std::sync::MutexGuard<'_, Option<u64>> {
        self.left.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `didEnterBackground`: true when the app must lock now.
    pub fn background(&self) -> bool {
        if !self.enabled() {
            *self.left() = None;
            return false;
        }
        *self.left() = Some((self.clock)());
        self.after_s.load(Ordering::Acquire) == 0
    }

    /// `didBecomeActive`: true when the app was away long enough to lock.
    pub fn foreground(&self) -> bool {
        let Some(at) = self.left().take() else {
            return false;
        };
        let after = self.after_s.load(Ordering::Acquire);
        after != Self::OFF
            && (self.clock)().saturating_sub(at) >= after.saturating_mul(1_000_000_000)
    }
}

/// Connects the app lock to the scene transitions and the settings.
fn wire_app_lock(
    app: AppHandle,
    core: Arc<Core>,
    recorder: Arc<Recorder>,
    lifecycle: &'static Lifecycle,
    sync: Arc<SyncService>,
) {
    let gate = Arc::new(BgLock::new(Box::new(crate::platform::continuous_ns)));
    let refresh = {
        let (core, gate) = (core.clone(), gate.clone());
        move || gate.configure(lock_delay(&core))
    };
    // Changes made in Settings (the lock toggle, the minutes).
    {
        let refresh = refresh.clone();
        core.set_settings_hook(Arc::new(move |_| refresh()));
    }
    {
        let (app, core, gate) = (app.clone(), core.clone(), gate.clone());
        lifecycle.observe(Arc::new(move |t| match t {
            Transition::Resign => {
                if gate.enabled() {
                    crate::platform::set_privacy_cover(true);
                }
            }
            Transition::Background => {
                // The sync session says goodbye inside a background task.
                sync.set_app_active(false);
                // A recording carries on behind the cover; the lock engages
                // when the app is opened again and the delay has passed.
                if gate.background() && recorder.latest().is_none() {
                    ghi_app::lock_cmd::engage(&app, &core);
                }
            }
            Transition::Active => {
                sync.set_app_active(true);
                if gate.foreground() {
                    ghi_app::lock_cmd::engage(&app, &core);
                }
                // The cover is only for the app-switcher snapshot; the web
                // lock gate hides the content while locked.
                crate::platform::set_privacy_cover(false);
            }
        }));
    }
    // The launch lock, once the store is open (the settings are in it).
    let _ = std::thread::Builder::new()
        .name("ghi-lock-launch".into())
        .spawn(move || {
            for _ in 0..200 {
                if core.store_even_locked().is_ok() {
                    refresh();
                    if ghi_app::lock_cmd::lock(&app, &core).is_ok() {
                        return;
                    }
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        });
}

/// How long the app may stay in the background before it locks, if the app
/// lock is on (read from the store, which a locked app can still do).
fn lock_delay(core: &Core) -> Option<Duration> {
    let stored = core
        .store_even_locked()
        .ok()?
        .get_setting(ghi_app::system::SETTINGS_KEY)
        .ok()?;
    let s = ghi_app::system::from_stored(stored);
    s.app_lock
        .then(|| Duration::from_secs(u64::from(s.lock_after_minutes) * 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_core(dir: &std::path::Path) -> Arc<Core> {
        let hooks = CoreHooks {
            handlers: Some(Arc::new(|_, _| vec![])),
            recover_kinds: Some(vec![]),
            keystore: Some(Arc::new(ghi_store::keys::MemoryKeyStore::default())),
            ..CoreHooks::default()
        };
        Core::for_test_with(dir.join("Ghira"), hooks).0
    }

    #[test]
    fn the_lock_delay_follows_the_stored_setting() {
        let t = tempfile::tempdir().unwrap();
        let core = test_core(t.path());
        assert_eq!(lock_delay(&core), None, "off by default");
        let store = core.store_even_locked().unwrap();
        let set = |on: bool, minutes: u32| {
            store
                .set_setting(
                    ghi_app::system::SETTINGS_KEY,
                    &serde_json::json!({"appLock": on, "lockAfterMinutes": minutes}),
                )
                .unwrap()
        };
        set(true, 0);
        assert_eq!(lock_delay(&core), Some(Duration::ZERO), "0: at once");
        set(true, 5);
        assert_eq!(lock_delay(&core), Some(Duration::from_secs(300)));
        set(false, 5);
        assert_eq!(lock_delay(&core), None);
        // A locked app still reads it: the lock decision never needs the UI.
        core.set_locked(true);
        set(true, 1);
        assert_eq!(lock_delay(&core), Some(Duration::from_secs(60)));
    }

    fn clocked() -> (BgLock, Arc<AtomicU64>) {
        let now = Arc::new(AtomicU64::new(1_000));
        let n = now.clone();
        (BgLock::new(Box::new(move || n.load(Ordering::SeqCst))), now)
    }

    const S: u64 = 1_000_000_000;

    #[test]
    fn zero_delay_locks_at_once_on_background() {
        let (g, _) = clocked();
        assert!(!g.background(), "off: never");
        g.configure(Some(Duration::ZERO));
        assert!(g.background());
    }

    #[test]
    fn a_delay_is_checked_on_return_with_the_continuous_clock() {
        let (g, now) = clocked();
        g.configure(Some(Duration::from_secs(60)));
        assert!(!g.background(), "not yet");
        now.fetch_add(59 * S, Ordering::SeqCst);
        assert!(!g.foreground(), "59 s away");
        assert!(!g.foreground(), "the departure was consumed");
        assert!(!g.background());
        now.fetch_add(61 * S, Ordering::SeqCst);
        assert!(g.foreground(), "61 s away: locked before the UI shows");
    }

    #[test]
    fn returning_without_leaving_or_with_the_lock_off_never_locks() {
        let (g, now) = clocked();
        g.configure(Some(Duration::ZERO));
        assert!(!g.foreground(), "Control Center: resign and active only");
        g.configure(Some(Duration::from_secs(1)));
        g.background();
        g.configure(None);
        now.fetch_add(100 * S, Ordering::SeqCst);
        assert!(!g.foreground(), "turned off while away");
        assert!(!g.enabled());
    }

    #[test]
    fn recovery_queues_the_final_pass_only_on_a_live_device() {
        assert_eq!(
            recover_kinds(TierClass::Live),
            [ghi_core::session::FINAL_PASS_JOB]
        );
        assert!(recover_kinds(TierClass::RecordOnly).is_empty());
    }
}
