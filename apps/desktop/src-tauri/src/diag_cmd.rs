// SPDX-License-Identifier: Apache-2.0
//! Local crash reports and the event log (`ghi-diag`): startup wiring and the
//! three commands the UI uses. Nothing here leaves the device.
//!
//! Folder: `<app data dir>/diagnostics` (reports `*.txt`, copied macOS crash
//! reports `*.ips`, `ghira.log` and its rolls, `running.lock`).

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::SystemTime;

use ghi_core::events::{Event, SessionState};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager, Runtime};

/// Names reported in a crash report, in `SessionState` order (state NAME only).
const STATE_NAMES: [&str; 8] = [
    "idle",
    "starting",
    "recording",
    "paused",
    "stopping",
    "processing",
    "ready",
    "failed",
];
static STATE: AtomicU8 = AtomicU8::new(0);

/// Process names whose macOS crash reports are copied after a crash.
const OS_REPORT_PREFIXES: [&str; 3] = ["Ghira", "ghi-desktop", "ghi-llm-worker"];

/// Follows the session state for the panic hook. Called for every core event.
pub fn track(event: &Event) {
    if let Event::StateChanged { state, .. } = event {
        let i = match state {
            SessionState::Idle => 0,
            SessionState::Starting => 1,
            SessionState::Recording => 2,
            SessionState::Paused => 3,
            SessionState::Stopping => 4,
            SessionState::Processing => 5,
            SessionState::Ready => 6,
            SessionState::Failed => 7,
        };
        STATE.store(i, Ordering::Relaxed);
    }
}

/// Panic-hook context: the session state name.
fn context() -> String {
    let i = usize::from(STATE.load(Ordering::Relaxed));
    format!("session {}", STATE_NAMES.get(i).copied().unwrap_or("?"))
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsStatus {
    pub crashed_last_run: bool,
    pub reports: u32,
}

pub struct Diag {
    dir: PathBuf,
    crashed: Mutex<bool>,
    marker: Mutex<Option<ghi_diag::RunningMarker>>,
}

impl Diag {
    /// Installs the panic hook and the log, notes whether the previous run
    /// crashed, then marks this run as running. Never fails the launch.
    pub fn init<R: Runtime>(app: &AppHandle<R>) -> Diag {
        let dir = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| std::env::temp_dir())
            .join("diagnostics");
        ghi_diag::install_panic_hook("ghira", dir.clone(), context);
        let _ = ghi_diag::init_log(&dir);
        let crashed = ghi_diag::previous_run_crashed(&dir);
        if crashed {
            let since = ghi_diag::stale_marker_time(&dir);
            let copied = copy_os_reports(&dir, since);
            log::warn!("previous run did not exit cleanly; os reports copied={copied}");
        }
        let marker = ghi_diag::running_marker(&dir);
        log::info!("app start version={}", env!("CARGO_PKG_VERSION"));
        Diag {
            dir,
            crashed: Mutex::new(crashed),
            marker: Mutex::new(marker),
        }
    }

    /// Clean exit: removes `running.lock` (process exit runs no destructors).
    pub fn release(&self) {
        log::info!("app exit");
        if let Some(m) = self
            .marker
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            m.release();
        }
    }
}

/// `~/Library/Logs/DiagnosticReports`.
fn os_reports_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Logs/DiagnosticReports"))
}

/// Copies, per process name, the newest macOS crash report written after
/// `since` into `dest`. Returns how many were copied.
fn copy_os_reports(dest: &Path, since: Option<SystemTime>) -> usize {
    let Some(src) = os_reports_dir() else {
        return 0;
    };
    copy_newest(&src, dest, since)
}

fn copy_newest(src: &Path, dest: &Path, since: Option<SystemTime>) -> usize {
    let Ok(entries) = std::fs::read_dir(src) else {
        return 0;
    };
    let files: Vec<(String, PathBuf, SystemTime)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((name, e.path(), modified))
        })
        .filter(|(n, _, m)| n.ends_with(".ips") && since.is_none_or(|s| *m > s))
        .collect();
    let mut copied = 0;
    for prefix in OS_REPORT_PREFIXES {
        let newest = files
            .iter()
            .filter(|(n, _, _)| n.starts_with(prefix))
            .max_by_key(|(_, _, m)| *m);
        if let Some((name, path, _)) = newest
            && std::fs::copy(path, dest.join(name)).is_ok()
        {
            copied += 1;
        }
    }
    copied
}

fn count_reports(dir: &Path) -> u32 {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.ends_with(".txt") || n.ends_with(".ips")
                })
                .count() as u32
        })
        .unwrap_or(0)
}

#[tauri::command]
#[specta::specta]
pub fn diagnostics_status(diag: tauri::State<'_, std::sync::Arc<Diag>>) -> DiagnosticsStatus {
    DiagnosticsStatus {
        crashed_last_run: *diag.crashed.lock().unwrap_or_else(|e| e.into_inner()),
        reports: count_reports(&diag.dir),
    }
}

/// Opens the diagnostics folder in Finder.
#[tauri::command]
#[specta::specta]
pub fn reveal_diagnostics(diag: tauri::State<'_, std::sync::Arc<Diag>>) -> Result<(), String> {
    std::fs::create_dir_all(&diag.dir).map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("/usr/bin/open")
        .arg(&diag.dir)
        .spawn();
    #[cfg(not(target_os = "macos"))]
    let r: std::io::Result<std::process::Child> = Err(std::io::Error::other("not supported"));
    r.map(|_| ()).map_err(|e| e.to_string())
}

/// The user saw the crash notice: it does not show again this launch.
#[tauri::command]
#[specta::specta]
pub fn acknowledge_crash(diag: tauri::State<'_, std::sync::Arc<Diag>>) {
    *diag.crashed.lock().unwrap_or_else(|e| e.into_inner()) = false;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn copies_only_newer_matching_reports_newest_per_process() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        let w = |n: &str| std::fs::write(src.path().join(n), "x").unwrap();
        w("Ghira-2026-10-01-1.ips");
        std::thread::sleep(Duration::from_millis(30));
        let since = SystemTime::now();
        std::thread::sleep(Duration::from_millis(30));
        w("Ghira-2026-10-02-1.ips");
        std::thread::sleep(Duration::from_millis(30));
        w("Ghira-2026-10-02-2.ips");
        w("ghi-llm-worker-2026-10-02.ips");
        w("Safari-2026.ips");
        w("Ghira-notes.txt");
        assert_eq!(copy_newest(src.path(), dst.path(), Some(since)), 2);
        assert!(dst.path().join("ghi-llm-worker-2026-10-02.ips").exists());
        assert!(!dst.path().join("Ghira-2026-10-01-1.ips").exists());
        assert!(!dst.path().join("Safari-2026.ips").exists());
        assert_eq!(count_reports(dst.path()), 2);
    }

    #[test]
    fn context_is_a_state_name() {
        assert!(context().starts_with("session "));
    }
}
