// SPDX-License-Identifier: Apache-2.0
//! Export (doc 02 §N): one meeting or several, to Markdown, text, SRT/VTT or
//! Word, to an Obsidian vault, or as text for the clipboard. The files are
//! rendered by `ghi_core::export` and written by Rust after a native save or
//! folder dialog; the webview gets file names, never paths.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ghi_core::export::{self, ExportOptions, Format, Lang};
use serde::Deserialize;
use specta::Type;
use tauri::AppHandle;

use crate::dialogs::{self, LastExport};
use crate::{CoreState, blocking};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    Markdown,
    Text,
    Srt,
    Vtt,
    Docx,
}

impl From<ExportFormat> for Format {
    fn from(f: ExportFormat) -> Format {
        match f {
            ExportFormat::Markdown => Format::Markdown,
            ExportFormat::Text => Format::Text,
            ExportFormat::Srt => Format::Srt,
            ExportFormat::Vtt => Format::Vtt,
            ExportFormat::Docx => Format::Docx,
        }
    }
}

/// What goes into the file; headings in the app's language.
#[derive(Debug, Clone, Copy, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportContent {
    pub notes: bool,
    pub transcript: bool,
    /// Headings in Vietnamese (else English).
    pub vietnamese: bool,
}

impl From<ExportContent> for ExportOptions {
    fn from(c: ExportContent) -> ExportOptions {
        ExportOptions {
            include_notes: c.notes,
            include_transcript: c.transcript,
            ui_lang: if c.vietnamese { Lang::Vi } else { Lang::En },
        }
    }
}

/// Where the Obsidian vault folder is remembered (a local path, never content).
const OBSIDIAN_SETTING: &str = "export.obsidian_vault";
/// The folder of the last export, offered first next time.
const EXPORT_DIR_SETTING: &str = "export.last_dir";

fn setting_path(store: &ghi_store::store::Store, key: &str) -> Option<PathBuf> {
    store
        .get_setting(key)
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(PathBuf::from))
}

fn remember(store: &ghi_store::store::Store, key: &str, dir: &Path) {
    if let Some(s) = dir.to_str() {
        let _ = store.set_setting(key, &serde_json::json!(s));
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Saves one meeting (a save dialog first). Returns the file name, or `None`
/// if the user cancelled.
#[tauri::command]
#[specta::specta]
pub async fn export_meeting(
    app: AppHandle,
    core: CoreState<'_>,
    last: tauri::State<'_, Arc<LastExport>>,
    meeting: String,
    format: ExportFormat,
    content: ExportContent,
) -> Result<Option<String>, String> {
    let f: Format = format.into();
    let ext = export::extension(f);
    let id = meeting.clone();
    let (m, start) = blocking(&core, move |c| {
        let store = c.store()?;
        let m = store.get_meeting(&id).map_err(|e| e.to_string())?;
        Ok((m, setting_path(&store, EXPORT_DIR_SETTING)))
    })
    .await?;
    let Some(path) =
        dialogs::save_file(&app, format!("{}.{ext}", export::file_stem(&m)), ext, start).await?
    else {
        return Ok(None);
    };
    let last = last.inner().clone();
    blocking(&core, move |c| {
        let store = c.store()?;
        let bytes = export::render(&store, &meeting, f, &content.into())?;
        // The user chose this path (and confirmed any replace).
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        if let Some(dir) = path.parent() {
            remember(&store, EXPORT_DIR_SETTING, dir);
        }
        let name = file_name(&path);
        *last.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
        Ok(Some(name))
    })
    .await
}

/// Saves several meetings into a folder (one file each). Returns how many
/// were written, or `None` if the user cancelled.
#[tauri::command]
#[specta::specta]
pub async fn export_meetings(
    app: AppHandle,
    core: CoreState<'_>,
    last: tauri::State<'_, Arc<LastExport>>,
    meetings: Vec<String>,
    format: ExportFormat,
    content: ExportContent,
    title: String,
) -> Result<Option<u32>, String> {
    if meetings.is_empty() {
        return Ok(Some(0));
    }
    let start = blocking(&core, |c| {
        Ok(setting_path(&*c.store()?, EXPORT_DIR_SETTING))
    })
    .await?;
    let Some(dir) = dialogs::pick_folder(&app, title, start).await? else {
        return Ok(None);
    };
    let last = last.inner().clone();
    blocking(&core, move |c| {
        let store = c.store()?;
        let f: Format = format.into();
        let mut n = 0;
        let mut first = None;
        for gid in &meetings {
            let m = store.get_meeting(gid).map_err(|e| e.to_string())?;
            let bytes = export::render(&store, gid, f, &content.into())?;
            let name =
                export::write_new(&dir, &export::file_stem(&m), export::extension(f), &bytes)?;
            first.get_or_insert(dir.join(name));
            n += 1;
        }
        remember(&store, EXPORT_DIR_SETTING, &dir);
        *last.0.lock().unwrap_or_else(|e| e.into_inner()) = first;
        Ok(Some(n))
    })
    .await
}

/// Writes the meeting as a note into an Obsidian vault folder (chosen once,
/// then remembered; `chooseFolder` asks again). Returns the note's name, or
/// `None` if the user cancelled.
#[tauri::command]
#[specta::specta]
pub async fn export_obsidian(
    app: AppHandle,
    core: CoreState<'_>,
    last: tauri::State<'_, Arc<LastExport>>,
    meeting: String,
    content: ExportContent,
    choose_folder: bool,
    title: String,
) -> Result<Option<String>, String> {
    let known = blocking(&core, |c| Ok(setting_path(&*c.store()?, OBSIDIAN_SETTING)))
        .await?
        .filter(|d| d.is_dir());
    let dir = match known {
        Some(d) if !choose_folder => d,
        start => match dialogs::pick_folder(&app, title, start).await? {
            Some(d) => d,
            None => return Ok(None),
        },
    };
    let last = last.inner().clone();
    blocking(&core, move |c| {
        let store = c.store()?;
        let name = export::write_obsidian(&store, &meeting, &dir, &content.into())?;
        remember(&store, OBSIDIAN_SETTING, &dir);
        *last.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir.join(&name));
        Ok(Some(name))
    })
    .await
}

/// The meeting as Markdown or plain text, for the clipboard.
#[tauri::command]
#[specta::specta]
pub async fn meeting_as_text(
    core: CoreState<'_>,
    meeting: String,
    markdown: bool,
    content: ExportContent,
) -> Result<String, String> {
    blocking(&core, move |c| {
        let f = if markdown {
            Format::Markdown
        } else {
            Format::Text
        };
        let bytes = export::render(&*c.store()?, &meeting, f, &content.into())?;
        String::from_utf8(bytes).map_err(|e| e.to_string())
    })
    .await
}

/// Shows the last exported file in Finder.
#[tauri::command]
#[specta::specta]
pub fn reveal_last_export(last: tauri::State<'_, Arc<LastExport>>) -> Result<(), String> {
    let path = last
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .ok_or("nothing was exported yet")?;
    dialogs::reveal(&path)
}
