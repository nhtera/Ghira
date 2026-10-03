// SPDX-License-Identifier: Apache-2.0
//! Import, the window side: the open dialog and files dropped on the window
//! or the Dock icon. The staging, the queue and the import itself are in
//! `ghi-app`.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Manager};
use tauri_specta::Event;

pub use ghi_app::import_cmd::*;

use crate::core::Core;
use crate::{CoreState, blocking};

/// The open dialog: stages the chosen files.
#[tauri::command]
#[specta::specta]
pub async fn pick_import_files(
    app: AppHandle,
    core: CoreState<'_>,
    imports: tauri::State<'_, Arc<Imports>>,
    title: String,
) -> Result<Vec<StagedFile>, String> {
    let paths = crate::dialogs::pick_audio_files(&app, title).await?;
    let imports = imports.inner().clone();
    blocking(&core, move |c| Ok(imports.stage(c, paths))).await
}

/// Files dropped on the main window or the Dock icon: staged, then the
/// import screen opens with them.
pub fn dropped(app: &AppHandle, paths: Vec<PathBuf>) {
    let paths: Vec<PathBuf> = paths
        .into_iter()
        .filter(|p| p.is_dir() || is_audio(p))
        .collect();
    if paths.is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let core = app.state::<Arc<Core>>().inner().clone();
        let imports = app.state::<Arc<Imports>>().inner().clone();
        let files = imports.stage(&core, paths);
        imports.remember_dropped(files.iter().map(|f| f.id.clone()));
        if let Ok(w) = crate::windows::main(&app, Some("/import")) {
            let _ = w.show();
            let _ = w.set_focus();
        }
        let _ = crate::Navigate {
            route: "/import".into(),
        }
        .emit_to(&app, "main");
        let _ = ImportStaged {
            files: files
                .into_iter()
                .map(|f| StagedFileEvent { id: f.id })
                .collect(),
        }
        .emit_to(&app, "main");
    });
}
