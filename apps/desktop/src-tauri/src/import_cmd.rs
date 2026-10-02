// SPDX-License-Identifier: Apache-2.0
//! Import (D10): files come from the open dialog, a drop on the window or on
//! the Dock icon; Rust keeps their paths and the webview gets staging ids
//! with what was found (name, size, length, likely source, problems). The
//! chosen ones go into a queue that imports one at a time (waiting while a
//! recording runs), reporting each step as an [`ImportUpdate`]. The old
//! "import this path" command is gone: the webview never names a file.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use ghi_core::import::{CANCELLED, Hold, ImportOptions, OnProgress};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::core::Core;
use crate::{CoreState, blocking};

/// Longer than this asks for a second look (hours of decoding and notes).
const VERY_LONG_MS: u64 = 4 * 3600 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ImportSource {
    Plaud,
    Zoom,
    Teams,
    VoiceMemos,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ImportProblem {
    /// Not audio we can read.
    Unsupported,
    /// An empty file.
    Empty,
    /// Over 4 hours: allowed, with a warning.
    VeryLong,
    /// Imported before (see `duplicateOf`).
    Duplicate,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateOf {
    pub meeting: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StagedFile {
    pub id: String,
    pub name: String,
    pub size_bytes: f64,
    pub duration_ms: Option<f64>,
    /// Two or more: "split channels" is offered (a Zoom/Teams stereo file).
    pub channels: u32,
    pub source: ImportSource,
    pub problems: Vec<ImportProblem>,
    pub duplicate_of: Option<DuplicateOf>,
}

/// Files dropped on the window or the Dock icon were staged.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct ImportStaged {
    pub files: Vec<StagedFileEvent>,
}

/// [`StagedFile`] as carried by an event (events need `Deserialize`).
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StagedFileEvent {
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ImportState {
    Queued,
    Decoding,
    Done,
    Failed,
    Cancelled,
}

/// One step of a queued import.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct ImportUpdate {
    pub id: String,
    pub state: ImportState,
    /// The meeting being made (once decoding started).
    pub meeting: Option<String>,
    /// 0..1 while decoding.
    pub progress: f64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportChoice {
    /// The transcript language (`en`, `vi`), or detect.
    pub language: Option<String>,
    /// Keep a stereo file's two channels as you / the others.
    pub split_channels: bool,
}

struct Staged {
    path: PathBuf,
    info: StagedFile,
    /// When the recording was made (the file's modification time).
    started_at: Option<i64>,
    /// SHA-256 from staging (the import doesn't hash it again).
    hash: Option<String>,
    at: Instant,
}

/// Staged files not imported are forgotten after this long.
const STAGED_TTL: Duration = Duration::from_secs(3600);

struct Job {
    id: String,
    path: PathBuf,
    opts: ImportOptions,
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    cancels: HashMap<String, Arc<AtomicBool>>,
    worker: bool,
}

#[derive(Default)]
pub struct Imports {
    staged: Mutex<HashMap<String, Staged>>,
    /// Ids staged by a drop, until the import screen reads them (at a cold
    /// launch from the Dock the event comes before the page listens).
    dropped: Mutex<Vec<String>>,
    queue: Mutex<Queue>,
    wake: Condvar,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn new_id() -> String {
    let mut b = [0u8; 12];
    OsRng.fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A likely source from the file's name and folder.
fn detect_source(path: &Path) -> ImportSource {
    let full = path.to_string_lossy().to_lowercase();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if full.contains("voicememos")
        || full.contains("voice memos")
        || name.starts_with("new recording")
    {
        ImportSource::VoiceMemos
    } else if full.contains("/zoom/")
        || name.starts_with("gmt")
        || name.starts_with("audio_only")
        || (name.starts_with("audio")
            && name.ends_with(".m4a")
            && name[5..].starts_with(|c: char| c.is_ascii_digit()))
    {
        ImportSource::Zoom
    } else if full.contains("teams") || name.contains("meeting recording") {
        ImportSource::Teams
    } else if full.contains("plaud") {
        ImportSource::Plaud
    } else {
        ImportSource::Other
    }
}

/// Looks at a file without importing it.
fn stage_one(core: &Core, path: PathBuf) -> Staged {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let meta = std::fs::metadata(&path).ok();
    let size = meta.as_ref().map_or(0, |m| m.len());
    let started_at = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    let mut info = StagedFile {
        id: new_id(),
        name,
        size_bytes: size as f64,
        duration_ms: None,
        channels: 0,
        source: detect_source(&path),
        problems: Vec::new(),
        duplicate_of: None,
    };
    if !meta.as_ref().is_some_and(|m| m.is_file()) || size == 0 {
        info.problems.push(ImportProblem::Empty);
    } else {
        match ghi_audio::decode::Decoder::open(&path) {
            Ok(dec) => {
                let i = dec.info();
                info.channels = u32::from(i.channels);
                info.duration_ms = i.duration_ms.map(|d| d as f64);
                if i.duration_ms.is_some_and(|d| d > VERY_LONG_MS) {
                    info.problems.push(ImportProblem::VeryLong);
                }
            }
            Err(_) => info.problems.push(ImportProblem::Unsupported),
        }
    }
    // Only a file that can be imported is hashed (duplicates; the import
    // reuses the hash).
    let mut hash = None;
    if info.problems.is_empty() || info.problems == [ImportProblem::VeryLong] {
        hash = ghi_core::import::file_sha256(&path).ok();
        if let (Ok(store), Some(hash)) = (core.store(), hash.as_ref())
            && let Ok(Some(m)) = store.meeting_by_source_hash(hash)
            && let Ok(meeting) = store.get_meeting(&m)
            && meeting.status != ghi_core::import::IMPORTING
        {
            info.problems.push(ImportProblem::Duplicate);
            info.duplicate_of = Some(DuplicateOf {
                meeting: m,
                title: meeting.title,
            });
        }
    }
    Staged {
        path,
        info,
        started_at,
        hash,
        at: Instant::now(),
    }
}

impl Imports {
    /// Stops every queued and running import ("Delete all data").
    pub fn cancel_all(&self) {
        let q = lock(&self.queue);
        for c in q.cancels.values() {
            c.store(true, Ordering::Relaxed);
        }
    }

    /// Stages files (blocking: decodes headers and hashes).
    pub fn stage(&self, core: &Core, paths: Vec<PathBuf>) -> Vec<StagedFile> {
        let mut out = Vec::new();
        lock(&self.staged).retain(|_, s| s.at.elapsed() < STAGED_TTL);
        for p in paths.into_iter().filter(|p| p.is_absolute()).take(100) {
            let s = stage_one(core, p);
            out.push(s.info.clone());
            lock(&self.staged).insert(s.info.id.clone(), s);
        }
        out
    }

    fn emit(app: &AppHandle, u: ImportUpdate) {
        let _ = u.emit_to(app, "main");
    }

    fn worker(self: Arc<Self>, app: AppHandle) {
        let core = app.state::<Arc<Core>>().inner().clone();
        loop {
            let job = {
                let mut q = lock(&self.queue);
                loop {
                    if let Some(j) = q.jobs.pop_front() {
                        break j;
                    }
                    let (g, timeout) = self
                        .wake
                        .wait_timeout(q, Duration::from_secs(60))
                        .unwrap_or_else(|e| e.into_inner());
                    q = g;
                    if timeout.timed_out() && q.jobs.is_empty() {
                        q.worker = false;
                        return;
                    }
                }
            };
            // A recording comes first.
            while core.recording() {
                std::thread::sleep(Duration::from_secs(1));
            }
            let cancel = job.opts.cancel.clone();
            if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
                lock(&self.queue).cancels.remove(&job.id);
                Self::emit(
                    &app,
                    update(&job.id, ImportState::Cancelled, None, 0.0, None),
                );
                continue;
            }
            Self::emit(
                &app,
                update(&job.id, ImportState::Decoding, None, 0.0, None),
            );
            let name = job
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            // A decoder bug on a malformed file must not stop the queue.
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                core.import(&job.path, job.opts)
            }))
            .unwrap_or_else(|_| Err("this file could not be read".into()));
            lock(&self.queue).cancels.remove(&job.id);
            let u = match r {
                Ok(r) => update(&job.id, ImportState::Done, Some(r.meeting), 1.0, None),
                Err(e) if e == CANCELLED => {
                    update(&job.id, ImportState::Cancelled, None, 0.0, None)
                }
                // The webview gets names, never paths.
                Err(e) => {
                    let msg = e.replace(&job.path.display().to_string(), &name);
                    update(&job.id, ImportState::Failed, None, 0.0, Some(msg))
                }
            };
            Self::emit(&app, u);
        }
    }
}

fn update(
    id: &str,
    state: ImportState,
    meeting: Option<String>,
    progress: f64,
    error: Option<String>,
) -> ImportUpdate {
    ImportUpdate {
        id: id.to_string(),
        state,
        meeting,
        progress,
        error,
    }
}

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

/// What was staged by a drop (the event carries ids only).
#[tauri::command]
#[specta::specta]
pub fn staged_files(imports: tauri::State<'_, Arc<Imports>>, ids: Vec<String>) -> Vec<StagedFile> {
    let staged = lock(&imports.staged);
    ids.iter()
        .filter_map(|id| staged.get(id).map(|s| s.info.clone()))
        .collect()
}

/// Files staged by a drop that the import screen hasn't shown yet (read once).
#[tauri::command]
#[specta::specta]
pub fn take_dropped_files(imports: tauri::State<'_, Arc<Imports>>) -> Vec<StagedFile> {
    let ids = std::mem::take(&mut *lock(&imports.dropped));
    let staged = lock(&imports.staged);
    ids.iter()
        .filter_map(|id| staged.get(id).map(|s| s.info.clone()))
        .collect()
}

/// Removes files from the staging list.
#[tauri::command]
#[specta::specta]
pub fn unstage_files(imports: tauri::State<'_, Arc<Imports>>, ids: Vec<String>) {
    let mut staged = lock(&imports.staged);
    for id in ids {
        staged.remove(&id);
    }
}

/// Queues staged files for import (unsupported or empty ones are refused).
#[tauri::command]
#[specta::specta]
pub fn start_import(
    app: AppHandle,
    imports: tauri::State<'_, Arc<Imports>>,
    ids: Vec<String>,
    choice: ImportChoice,
) -> Result<(), String> {
    let imports = imports.inner().clone();
    let mut staged = lock(&imports.staged);
    let mut q = lock(&imports.queue);
    for id in ids {
        let Some(s) = staged.remove(&id) else {
            continue;
        };
        if s.info
            .problems
            .iter()
            .any(|p| matches!(p, ImportProblem::Unsupported | ImportProblem::Empty))
        {
            continue;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        q.cancels.insert(id.clone(), cancel.clone());
        let hold_core = app.state::<Arc<Core>>().inner().clone();
        let progress_app = app.clone();
        let progress_id = id.clone();
        // At most a few updates a second.
        let last = Arc::new(Mutex::new((Instant::now(), -1.0f32)));
        let on_progress = OnProgress(Arc::new(move |meeting: &str, p: f32| {
            let mut l = lock(&last);
            if p >= 1.0 || (l.0.elapsed() >= Duration::from_millis(250) && p - l.1 >= 0.01) {
                *l = (Instant::now(), p);
                Imports::emit(
                    &progress_app,
                    update(
                        &progress_id,
                        ImportState::Decoding,
                        Some(meeting.to_string()),
                        f64::from(p),
                        None,
                    ),
                );
            }
        }));
        Imports::emit(&app, update(&id, ImportState::Queued, None, 0.0, None));
        q.jobs.push_back(Job {
            id,
            path: s.path,
            opts: ImportOptions {
                title: None,
                language: choice.language.clone().filter(|l| l == "en" || l == "vi"),
                split_channels: choice.split_channels && s.info.channels >= 2,
                started_at: s.started_at,
                cancel: Some(cancel),
                on_progress: Some(on_progress),
                source_hash: s.hash,
                // A recording that starts meanwhile goes first.
                hold: Some(Hold(Arc::new(move || hold_core.recording()))),
            },
        });
    }
    if !q.worker && !q.jobs.is_empty() {
        q.worker = true;
        let (worker, app) = (imports.clone(), app.clone());
        std::thread::Builder::new()
            .name("ghi-import".into())
            .spawn(move || worker.worker(app))
            .map_err(|e| e.to_string())?;
    }
    drop(q);
    imports.wake.notify_all();
    Ok(())
}

/// Stops a queued or running import (its half-made meeting is removed).
#[tauri::command]
#[specta::specta]
pub fn cancel_import(imports: tauri::State<'_, Arc<Imports>>, id: String) {
    if let Some(c) = lock(&imports.queue).cancels.get(&id) {
        c.store(true, Ordering::Relaxed);
    }
}

/// Files dropped on the main window or the Dock icon: staged, then the
/// import screen opens with them.
pub fn dropped(app: &AppHandle, paths: Vec<PathBuf>) {
    let paths: Vec<PathBuf> = paths
        .into_iter()
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                crate::dialogs::IMPORT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())
            })
        })
        .collect();
    if paths.is_empty() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let core = app.state::<Arc<Core>>().inner().clone();
        let imports = app.state::<Arc<Imports>>().inner().clone();
        let files = imports.stage(&core, paths);
        lock(&imports.dropped).extend(files.iter().map(|f| f.id.clone()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_guessed_from_names() {
        let s = |p: &str| detect_source(Path::new(p));
        assert_eq!(
            s(
                "/Users/a/Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings/x.m4a"
            ),
            ImportSource::VoiceMemos
        );
        assert_eq!(
            s("/Users/a/Desktop/New Recording 12.m4a"),
            ImportSource::VoiceMemos
        );
        assert_eq!(
            s("/Users/a/Documents/Zoom/2026-10-01 Sync/audio1123456789.m4a"),
            ImportSource::Zoom
        );
        assert_eq!(
            s("/Users/a/Downloads/GMT20261001-030000_Recording.m4a"),
            ImportSource::Zoom
        );
        assert_eq!(s("/Users/a/Downloads/audio.m4a"), ImportSource::Other);
        assert_eq!(
            s("/Users/a/PLAUD/2026-10-01 10_00.mp3"),
            ImportSource::Plaud
        );
        assert_eq!(
            s("/Users/a/Downloads/Weekly-20261001_Meeting Recording.mp4"),
            ImportSource::Teams
        );
    }
}
