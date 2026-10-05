// SPDX-License-Identifier: Apache-2.0
//! Native open/save/folder dialogs, shown by Rust (`rfd`) as sheets of the
//! main window. Chosen paths stay in Rust: the webview only
//! ever sees file names [RT-6].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager};

/// One dialog at a time (a second would stack another sheet).
static OPEN: AtomicBool = AtomicBool::new(false);

struct Open;

impl Open {
    fn take() -> Result<Open, String> {
        if OPEN.swap(true, Ordering::AcqRel) {
            Err("a file dialog is already open".into())
        } else {
            Ok(Open)
        }
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        OPEN.store(false, Ordering::Release);
    }
}

/// A sheet on the main window. The async dialogs run on the main thread by
/// themselves, with a completion handler (no nested modal loop holding up
/// the menu, the tray or the panels).
fn dialog(app: &AppHandle) -> rfd::AsyncFileDialog {
    let d = rfd::AsyncFileDialog::new();
    match app.get_webview_window("main") {
        Some(w) => d.set_parent(&w),
        None => d,
    }
}

pub use ghi_app::import_cmd::IMPORT_EXTENSIONS;

pub async fn pick_audio_files(app: &AppHandle, title: String) -> Result<Vec<PathBuf>, String> {
    let _open = Open::take()?;
    Ok(dialog(app)
        .set_title(title)
        .add_filter("Audio", IMPORT_EXTENSIONS)
        .pick_files()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|f| f.path().to_path_buf())
        .collect())
}

pub async fn save_file(
    app: &AppHandle,
    file_name: String,
    extension: &'static str,
    start: Option<PathBuf>,
) -> Result<Option<PathBuf>, String> {
    let _open = Open::take()?;
    let mut d = dialog(app)
        .set_file_name(file_name)
        .add_filter(extension.to_uppercase(), &[extension]);
    if let Some(dir) = start.filter(|d| d.is_dir()) {
        d = d.set_directory(dir);
    }
    Ok(d.save_file().await.map(|f| f.path().to_path_buf()))
}

/// One existing file with the given extension (an archive to import).
pub async fn pick_file(
    app: &AppHandle,
    title: String,
    extension: &'static str,
) -> Result<Option<PathBuf>, String> {
    let _open = Open::take()?;
    Ok(dialog(app)
        .set_title(title)
        .add_filter(extension.to_uppercase(), &[extension])
        .pick_file()
        .await
        .map(|f| f.path().to_path_buf()))
}

pub async fn pick_folder(
    app: &AppHandle,
    title: String,
    start: Option<PathBuf>,
) -> Result<Option<PathBuf>, String> {
    let _open = Open::take()?;
    let mut d = dialog(app)
        .set_title(title)
        .set_can_create_directories(true);
    if let Some(dir) = start.filter(|d| d.is_dir()) {
        d = d.set_directory(dir);
    }
    Ok(d.pick_folder().await.map(|f| f.path().to_path_buf()))
}

pub use ghi_app::export_cmd::LastExport;

/// Shows a file in Finder / Explorer (selected).
pub fn reveal(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("/usr/bin/open")
        .arg("-R")
        .arg(path)
        .spawn();
    #[cfg(windows)]
    let r = std::process::Command::new("explorer")
        .arg(format!("/select,{}", path.display()))
        .spawn();
    #[cfg(not(any(target_os = "macos", windows)))]
    let r: std::io::Result<std::process::Child> =
        Err(std::io::Error::other(path.display().to_string()));
    r.map(|_| ()).map_err(|e| e.to_string())
}
