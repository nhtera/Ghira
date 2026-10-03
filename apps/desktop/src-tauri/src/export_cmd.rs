// SPDX-License-Identifier: Apache-2.0
//! Export (doc 02 §N): one meeting or several, to Markdown, text, SRT/VTT or
//! Word, to an Obsidian vault, or as text for the clipboard. The files are
//! rendered by `ghi_core::export` and written by Rust after a native save or
//! folder dialog; the webview gets file names, never paths.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ghi_core::export::{self, Format};
use tauri::AppHandle;

pub use ghi_app::export_cmd::*;

use crate::dialogs;
use crate::{CoreState, blocking};

/// Where the Obsidian vault folder is remembered (a local path, never content).
const OBSIDIAN_SETTING: &str = "export.obsidian_vault";
/// The folder of the last export, offered first next time (where a dialog
/// starts).
const EXPORT_LAST_DIR_SETTING: &str = "export.last_dir";
/// The export destination ("Save to: <folder>"): set only by
/// `choose_export_folder` or by the first export's dialog.
const EXPORT_DIR_SETTING: &str = "export.dir";

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

/// The export destination, if one was chosen and it still exists.
fn remembered_dir(store: &ghi_store::store::Store) -> Option<PathBuf> {
    setting_path(store, EXPORT_DIR_SETTING).filter(|d| d.is_dir())
}

/// Where a dialog starts: the last export's folder.
fn dialog_start(store: &ghi_store::store::Store) -> Option<PathBuf> {
    setting_path(store, EXPORT_LAST_DIR_SETTING)
        .filter(|d| d.is_dir())
        .or_else(|| remembered_dir(store))
}

/// Makes `dir` the destination and where dialogs start next time.
fn remember_destination(store: &ghi_store::store::Store, dir: &Path) {
    remember(store, EXPORT_DIR_SETTING, dir);
    remember(store, EXPORT_LAST_DIR_SETTING, dir);
}

/// The name (not the path) of the folder exports go to, or `None` when none
/// was chosen yet or it is gone.
#[tauri::command]
#[specta::specta]
pub async fn export_destination(core: CoreState<'_>) -> Result<Option<String>, String> {
    blocking(&core, |c| {
        Ok(remembered_dir(&*c.store()?).map(|d| file_name(&d)))
    })
    .await
}

/// Asks for the export folder (a native dialog starting at the current one),
/// remembers it and returns its name; `None` if the user cancelled.
#[tauri::command]
#[specta::specta]
pub async fn choose_export_folder(
    app: AppHandle,
    core: CoreState<'_>,
    title: String,
) -> Result<Option<String>, String> {
    let start = blocking(&core, |c| {
        let store = c.store()?;
        Ok(remembered_dir(&store).or_else(|| dialog_start(&store)))
    })
    .await?;
    let Some(dir) = dialogs::pick_folder(&app, title, start).await? else {
        return Ok(None);
    };
    let name = file_name(&dir);
    blocking(&core, move |c| {
        remember_destination(&*c.store()?, &dir);
        Ok(Some(name))
    })
    .await
}

/// Saves one meeting into the remembered export folder (nothing is
/// overwritten); with none chosen yet, a save dialog first. Returns the file
/// name, or `None` if the user cancelled.
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
    let (m, dest, start) = blocking(&core, move |c| {
        let store = c.store()?;
        let m = store.get_meeting(&id).map_err(|e| e.to_string())?;
        Ok((m, remembered_dir(&store), dialog_start(&store)))
    })
    .await?;
    if let Some(dir) = dest {
        let last = last.inner().clone();
        return blocking(&core, move |c| {
            let store = c.store()?;
            let bytes = export::render(&store, &meeting, f, &content.into())?;
            let name = export::write_new(&dir, &export::file_stem(&m), ext, &bytes)?;
            *last.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir.join(&name));
            Ok(Some(name))
        })
        .await;
    }
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
        // The first export's dialog also sets the destination.
        if let Some(dir) = path.parent() {
            remember(&store, EXPORT_LAST_DIR_SETTING, dir);
            if remembered_dir(&store).is_none() {
                remember(&store, EXPORT_DIR_SETTING, dir);
            }
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
    let (dest, start) = blocking(&core, |c| {
        let store = c.store()?;
        Ok((remembered_dir(&store), dialog_start(&store)))
    })
    .await?;
    // A chosen destination is used as is (Save to: <folder> · Change…).
    let dir = match dest {
        Some(d) => d,
        None => match dialogs::pick_folder(&app, title, start).await? {
            Some(d) => d,
            None => return Ok(None),
        },
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
        remember(&store, EXPORT_LAST_DIR_SETTING, &dir);
        if remembered_dir(&store).is_none() {
            remember(&store, EXPORT_DIR_SETTING, &dir);
        }
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

/// Percent-encodes everything but unreserved characters (RFC 3986) and the
/// ones in `keep`.
fn pct(s: &str, keep: &[u8]) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') || keep.contains(&b)
        {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The longest `mailto:` URL handed to the OS (mail apps and ShellExecute
/// refuse or cut longer ones).
#[cfg(windows)]
const MAILTO_MAX: usize = 2_000;
#[cfg(not(windows))]
const MAILTO_MAX: usize = 8_000;

/// The `mailto:` URL for a draft, at most `max` characters, and whether the
/// body was cut to fit. Only plausible addresses are kept (`@ . - _ +` stay
/// literal in them).
fn mailto(to: &[String], subject: &str, body: &str, max: usize) -> (String, bool) {
    let to: Vec<String> = to
        .iter()
        .map(|a| a.trim())
        .filter(|a| {
            a.len() <= 254
                && a.contains('@')
                && !a
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || c == ',')
        })
        .take(100)
        .map(|a| pct(a, b"@+"))
        .collect();
    let clip = |s: &str, n: usize| s.chars().take(n).collect::<String>();
    let head = format!(
        "mailto:{}?subject={}&body=",
        to.join(","),
        pct(&clip(subject, 300), b"")
    );
    // Whole characters of the body while the encoded URL fits.
    let mut url = head;
    let mut truncated = false;
    for c in body.chars() {
        let enc = pct(c.encode_utf8(&mut [0; 4]), b"");
        if url.len() + enc.len() > max {
            truncated = true;
            break;
        }
        url.push_str(&enc);
    }
    (url, truncated)
}

/// What `open_mail_draft` did.
#[derive(Debug, Clone, Copy, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct MailDraft {
    /// The body was cut to fit the OS's limit: offer "Copy the full email".
    pub truncated: bool,
}

/// Opens the default mail app with a new message (nothing is sent, and the
/// app makes no network request).
#[tauri::command]
#[specta::specta]
pub fn open_mail_draft(
    to: Vec<String>,
    subject: String,
    body: String,
) -> Result<MailDraft, String> {
    let (url, truncated) = mailto(&to, &subject, &body, MAILTO_MAX);
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("/usr/bin/open");
        c.arg(&url);
        c
    };
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        let mut c =
            std::process::Command::new(std::path::Path::new(&root).join(r"System32\rundll32.exe"));
        c.args(["url.dll,FileProtocolHandler"])
            .arg(&url)
            .creation_flags(0x0800_0000);
        c
    };
    #[cfg(not(any(target_os = "macos", windows)))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(&url);
        c
    };
    // Started, not awaited: a command must not block on another process.
    cmd.spawn()
        .map(|_| MailDraft { truncated })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod mail_tests {
    use super::*;

    #[test]
    fn the_draft_url_is_encoded_and_drops_bad_addresses() {
        let (u, cut) = mailto(
            &[
                "a.b+c@b.co".into(),
                "no at".into(),
                "x,y@z.io".into(),
                "c@d-e.vn".into(),
            ],
            "Họp & chốt",
            "Line 1\nLine 2",
            8_000,
        );
        assert_eq!(
            u,
            "mailto:a.b+c@b.co,c@d-e.vn?subject=H%E1%BB%8Dp%20%26%20ch%E1%BB%91t&body=Line%201%0ALine%202"
        );
        assert!(!cut);
    }

    #[test]
    fn a_long_body_is_cut_to_the_limit_on_a_character() {
        let body = "ộ".repeat(5_000);
        for max in [2_000, 8_000] {
            let (u, cut) = mailto(&["a@b.co".into()], "s", &body, max);
            assert!(
                cut && u.len() <= max && u.len() > max - 9,
                "{} {max}",
                u.len()
            );
            assert!(u.ends_with("%E1%BB%99"), "no half characters");
        }
        let (_, cut) = mailto(&["a@b.co".into()], "s", "short", 2_000);
        assert!(!cut);
    }
}
