// SPDX-License-Identifier: Apache-2.0
//! App updates (phase 12) in the desktop app: checks through `ghi-net` and
//! the signed manifest (`ghi-update`), the download, and "Restart to update".
//!
//! - Off until the build has a feed and a key (`ghi_update::configured`).
//! - Never under strict offline. Automatic checks (at launch, then daily)
//!   only while the setting is on; "Check for updates" works either way.
//! - Never while recording or while jobs run: a check or a download waits.
//! - The archive goes to `<app data>/updates/`, checked against the
//!   manifest's SHA-256. Installing swaps the bundle and relaunches, only
//!   when the user asks.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ghi_net::NetPolicy;
use ghi_net::fetch::{FetchOpts, UPDATE_HOSTS, UreqTransport, fetch, fetch_small};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::core::Core;

/// Store setting: what earlier checks learnt (sequence seen, last check).
const STATE_KEY: &str = "update";
const CHANNEL: &str = "alpha";
const DAY: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// This build can update itself (a feed and a key are built in).
    pub configured: bool,
    pub checking: bool,
    /// Unix ms of the last successful check.
    pub last_check: Option<f64>,
    /// A newer version, if any.
    pub available: Option<String>,
    pub notes_url: Option<String>,
    /// Downloaded and checked: "Restart to update" can run.
    pub ready: bool,
    /// The running version was withdrawn: update now.
    pub running_pulled: bool,
    /// Too old to update in place: download the new version.
    pub reinstall_needed: bool,
    pub error: Option<String>,
}

/// The status changed (the About section and a banner follow it).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChanged {
    pub status: UpdateStatus,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Saved {
    sequence: u64,
    last_check: Option<i64>,
}

#[derive(Default)]
pub struct Updates {
    status: Mutex<UpdateStatus>,
    /// The verified archive and its version.
    archive: Mutex<Option<(PathBuf, String)>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl Updates {
    fn set(&self, app: &AppHandle, f: impl FnOnce(&mut UpdateStatus)) {
        let status = {
            let mut s = lock(&self.status);
            f(&mut s);
            s.configured = ghi_update::configured();
            s.clone()
        };
        let _ = UpdateChanged { status }.emit_to(app, "main");
    }

    /// Recording, starting/stopping, or a job running: not now.
    fn busy(core: &Core) -> bool {
        core.busy()
            || core
                .store()
                .ok()
                .and_then(|s| s.active_jobs().ok())
                .is_some_and(|jobs| {
                    jobs.iter()
                        .any(|j| j.state == ghi_store::jobs::JobState::Running)
                })
    }

    /// Checks the feed and downloads a newer version. Blocking.
    pub fn check(&self, app: &AppHandle, core: &Core) -> Result<(), String> {
        let (Some(feed), true) = (ghi_update::FEED_URL, ghi_update::configured()) else {
            return Err("updates aren't set up in this build".into());
        };
        if crate::system::load_settings(core)?.strict_offline {
            return Err("strict offline is on".into());
        }
        if Self::busy(core) {
            return Err("checks wait until the recording and processing are done".into());
        }
        self.set(app, |s| {
            s.checking = true;
            s.error = None;
        });
        let r = self.check_inner(core, feed);
        self.set(app, |s| {
            s.checking = false;
            match &r {
                Ok(()) => s.last_check = Some(now_ms() as f64),
                Err(e) => s.error = Some(e.clone()),
            }
        });
        r
    }

    fn check_inner(&self, core: &Core, feed: &str) -> Result<(), String> {
        let store = core.store()?;
        let mut saved: Saved = store
            .get_setting(STATE_KEY)
            .map_err(|e| e.to_string())?
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let t = UreqTransport;
        let policy = NetPolicy::Default;
        let body = fetch_small(
            policy,
            feed,
            UPDATE_HOSTS,
            ghi_update::MAX_MANIFEST_BYTES,
            &t,
        )
        .map_err(|e| e.to_string())?;
        let sig = fetch_small(policy, &format!("{feed}.minisig"), UPDATE_HOSTS, 4096, &t)
            .map_err(|e| e.to_string())?;
        let sig = String::from_utf8(sig).map_err(|_| "the signature isn't text")?;
        let m =
            ghi_update::verify(&body, &sig, ghi_update::PUBLIC_KEYS).map_err(|e| e.to_string())?;
        let d = ghi_update::decide(
            &m,
            env!("CARGO_PKG_VERSION"),
            CHANNEL,
            saved.sequence,
            now_ms(),
        )
        .map_err(|e| e.to_string())?;
        saved.sequence = d.sequence;
        saved.last_check = Some(now_ms());
        store
            .set_setting(STATE_KEY, &serde_json::to_value(&saved).unwrap_or_default())
            .map_err(|e| e.to_string())?;
        {
            let mut s = lock(&self.status);
            s.running_pulled = d.running_pulled;
            s.reinstall_needed = d.reinstall_needed;
            s.available = d.available.as_ref().map(|l| l.version.clone());
            s.notes_url = d.available.as_ref().and_then(|l| l.notes_url.clone());
        }
        let Some(latest) = d.available else {
            return Ok(());
        };
        if lock(&self.archive)
            .as_ref()
            .is_some_and(|(_, v)| *v == latest.version)
        {
            return Ok(());
        }
        let dest = core
            .data_dir()
            .join("updates")
            .join(format!("Ghira-{}.app.zip", latest.version));
        fetch(
            policy,
            &latest.archive.url,
            &dest,
            &FetchOpts {
                expected_sha256: latest.archive.sha256.to_ascii_lowercase(),
                expected_size: latest.archive.size,
                resume: true,
                extra_hosts: UPDATE_HOSTS.iter().map(|h| h.to_string()).collect(),
                control: Default::default(),
            },
            &mut |_| {},
            &t,
        )
        .map_err(|e| e.to_string())?;
        *lock(&self.archive) = Some((dest, latest.version.clone()));
        lock(&self.status).ready = true;
        Ok(())
    }
}

/// Background checks: a minute after launch, then daily (when allowed).
pub fn spawn_checks(app: AppHandle) {
    if !ghi_update::configured() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("ghi-update".into())
        .spawn(move || {
            std::thread::sleep(Duration::from_secs(60));
            loop {
                let core = app.state::<Arc<Core>>().inner().clone();
                let updates = app.state::<Arc<Updates>>().inner().clone();
                let on = crate::system::load_settings(&core)
                    .map(|s| s.update_check && !s.strict_offline)
                    .unwrap_or(false);
                // Busy: try again in ten minutes rather than a day.
                let wait = if !on {
                    DAY
                } else if Updates::busy(&core) {
                    Duration::from_secs(600)
                } else {
                    let _ = updates.check(&app, &core);
                    DAY
                };
                std::thread::sleep(wait);
            }
        });
}

/// After an update: removes the previous bundle once this one runs.
pub fn cleanup_after_update() {
    if let Ok(exe) = std::env::current_exe()
        && let Ok(app) = ghi_update::install::bundle_of(&exe)
    {
        ghi_update::install::cleanup(&app);
    }
}

#[tauri::command]
#[specta::specta]
pub fn update_status(updates: tauri::State<'_, Arc<Updates>>) -> UpdateStatus {
    let mut s = lock(&updates.status).clone();
    s.configured = ghi_update::configured();
    s
}

/// "Check for updates" (works with automatic checks off, not under strict
/// offline).
#[tauri::command]
#[specta::specta]
pub async fn check_for_updates(
    app: AppHandle,
    core: crate::CoreState<'_>,
    updates: tauri::State<'_, Arc<Updates>>,
) -> Result<UpdateStatus, String> {
    let updates = updates.inner().clone();
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        updates.check(&app, &core)?;
        Ok(lock(&updates.status).clone())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// "Restart to update": swaps in the downloaded version and relaunches.
#[tauri::command]
#[specta::specta]
pub async fn install_update(
    app: AppHandle,
    core: crate::CoreState<'_>,
    updates: tauri::State<'_, Arc<Updates>>,
) -> Result<(), String> {
    let updates = updates.inner().clone();
    let core = core.inner().clone();
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        if Updates::busy(&core) {
            return Err("updates wait until the recording and processing are done".into());
        }
        let (archive, _) = lock(&updates.archive).clone().ok_or("no update is ready")?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let bundle = ghi_update::install::bundle_of(&exe).map_err(|e| e.to_string())?;
        // Unpacked and checked first; nothing changes if that fails.
        let new_app =
            ghi_update::install::prepare(&archive, &bundle, &ghi_update::install::Codesign)
                .map_err(|e| e.to_string())?;
        // Jobs and the notes model stop before the files change.
        core.shutdown(Duration::from_secs(5));
        let swapped = ghi_update::install::swap(&new_app, &bundle);
        let _ = std::fs::remove_file(&archive);
        // Swapped or not, start again (the old version if the swap failed).
        ghi_update::install::relaunch(&bundle).map_err(|e| e.to_string())?;
        handle.exit(0);
        swapped.map(|_| ()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
