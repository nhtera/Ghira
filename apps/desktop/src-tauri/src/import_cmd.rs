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
    Meet,
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
    /// The mixed recording of a Zoom meeting whose participant tracks are
    /// staged too: the tracks are imported instead ("Won't be imported").
    Superseded,
    /// A Zoom recording with more participant tracks than one import takes
    /// (49): on every one of its tracks; import them separately instead.
    TooManyTracks,
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
    /// The participant tracks of one Zoom recording share this id: they are
    /// imported as one meeting (start them together; the update's id is this).
    pub group: Option<String>,
    /// The participant's name from the file name (a track in a group).
    pub participant: Option<String>,
    /// A title found in the folder or file name or the file's tags.
    pub title: Option<String>,
    /// When it was recorded (unix ms): from the name or tags, else the file's
    /// modification time.
    pub started_at: Option<f64>,
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
    /// When the recording was made (from its name or tags, else the file's
    /// modification time).
    started_at: Option<i64>,
    /// SHA-256 from staging (the import doesn't hash it again); none for a
    /// track in a group (the group is hashed as a whole).
    hash: Option<String>,
    /// For a track in a group: the group's hash and how many tracks it was
    /// made from (the import reuses it if the group is unchanged).
    group_hash: Option<String>,
    group_size: usize,
    at: Instant,
}

/// Staged files not imported are forgotten after this long.
const STAGED_TTL: Duration = Duration::from_secs(3600);

struct Job {
    /// The file's staging id, or the group's id for a multi-track import.
    id: String,
    /// One file; or a group's tracks with their participants' names.
    files: Vec<(PathBuf, Option<String>)>,
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
fn source_of(path: &Path) -> ImportSource {
    match ghi_core::presets::detect_source(path) {
        Some("zoom") => ImportSource::Zoom,
        Some("teams") => ImportSource::Teams,
        Some("meet") => ImportSource::Meet,
        Some("plaud") => ImportSource::Plaud,
        Some("voice_memos") => ImportSource::VoiceMemos,
        _ => ImportSource::Other,
    }
}

/// Audio files the importer reads, by extension.
fn is_audio(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        crate::dialogs::IMPORT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str())
    })
}

/// Dropped folders expanded one level: a Zoom meeting folder's `Audio Record`
/// folder when it has one, else the folder's own audio files (by name).
fn expand(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        if !p.is_dir() {
            out.push(p);
            continue;
        }
        let record = p.join("Audio Record");
        let dir = if record.is_dir() { record } else { p };
        let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|f| f.is_file() && is_audio(f))
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        out.extend(found);
    }
    out
}

/// Looks at a file without importing it (no hashing yet).
fn probe_one(path: PathBuf) -> Staged {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let meta = std::fs::metadata(&path).ok();
    let size = meta.as_ref().map_or(0, |m| m.len());
    let modified = meta
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
        source: source_of(&path),
        problems: Vec::new(),
        duplicate_of: None,
        group: None,
        participant: None,
        title: None,
        started_at: None,
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
    // The name or tags may say what and when; the modification time is the
    // fallback for when.
    let tags = ghi_audio::decode::tags(&path);
    let found = ghi_core::presets::title_date(&path, tags.title.as_deref(), tags.date_ms);
    let started_at = found.started_at_ms.or(modified);
    info.title = found.title;
    info.started_at = started_at.map(|t| t as f64);
    Staged {
        path,
        info,
        started_at,
        hash: None,
        group_hash: None,
        group_size: 0,
        at: Instant::now(),
    }
}

fn importable(s: &Staged) -> bool {
    !s.info
        .problems
        .iter()
        .any(|p| matches!(p, ImportProblem::Unsupported | ImportProblem::Empty))
}

/// The Zoom meeting folder a track belongs to: its folder, or the one above
/// `Audio Record`.
fn meeting_dir_of(path: &Path) -> PathBuf {
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let in_record = dir
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("audio record"));
    if in_record {
        dir.parent().map(Path::to_path_buf).unwrap_or(dir)
    } else {
        dir
    }
}

/// Finds the participant tracks of Zoom recordings among the staged files
/// and gives each recording one group id; the mixed recording next to a
/// group's tracks, in a real Zoom meeting folder, is marked "won't be
/// imported". A group over the import's limit is flagged on every track.
fn group_zoom(list: &mut [Staged]) {
    let ok: Vec<usize> = (0..list.len())
        .filter(|&i| importable(&list[i]) && list[i].info.duration_ms.is_some())
        .collect();
    let files: Vec<(PathBuf, f64)> = ok
        .iter()
        .map(|&i| {
            (
                list[i].path.clone(),
                list[i].info.duration_ms.unwrap_or(0.0) / 1000.0,
            )
        })
        .collect();
    for group in ghi_core::presets::zoom_group(&files) {
        let gid = new_id();
        let too_many = group.len() > ghi_core::import::MAX_TRACKS;
        for &g in &group {
            let s = &mut list[ok[g]];
            s.info.group = Some(gid.clone());
            s.info.participant = ghi_core::presets::zoom_participant(&s.info.name);
            s.info.source = ImportSource::Zoom;
            if too_many {
                s.info.problems.push(ImportProblem::TooManyTracks);
            }
        }
        // What the first track says about the meeting stands for all of them.
        let first = &list[ok[group[0]]];
        let (title, at, started) = (
            first.info.title.clone(),
            first.info.started_at,
            first.started_at,
        );
        for &g in &group {
            let s = &mut list[ok[g]];
            s.info.title.clone_from(&title);
            s.info.started_at = at;
            s.started_at = started;
        }
    }
    refresh_superseded(list.iter_mut());
}

/// Marks the mixed recording (audio_only, the video) of a Zoom meeting folder
/// "won't be imported" while participant tracks of that folder are staged,
/// and lets it back in when none is. Only in a folder named as Zoom names its
/// meeting folders. Returns the files whose problems changed.
fn refresh_superseded<'a>(list: impl Iterator<Item = &'a mut Staged>) -> Vec<StagedFile> {
    let list: Vec<&mut Staged> = list.collect();
    let with_tracks: Vec<PathBuf> = list
        .iter()
        .filter(|s| s.info.group.is_some())
        .map(|s| meeting_dir_of(&s.path))
        .filter(|d| ghi_core::presets::is_zoom_folder(d))
        .collect();
    let mut changed = Vec::new();
    for s in list {
        if s.info.group.is_some() || s.info.source != ImportSource::Zoom {
            continue;
        }
        let covered = s
            .path
            .parent()
            .is_some_and(|d| with_tracks.iter().any(|t| t == d));
        let marked = s.info.problems.contains(&ImportProblem::Superseded);
        if covered && !marked {
            s.info.problems.push(ImportProblem::Superseded);
            changed.push(s.info.clone());
        } else if !covered && marked {
            s.info.problems.retain(|p| *p != ImportProblem::Superseded);
            changed.push(s.info.clone());
        }
    }
    changed
}

/// Hashes what can be imported and flags what was imported before.
fn finish(core: &Core, list: &mut [Staged]) {
    let store = core.store().ok();
    let earlier = |hash: &str| -> Option<DuplicateOf> {
        let store = store.as_ref()?;
        let m = store.meeting_by_source_hash(hash).ok()??;
        let meeting = store.get_meeting(&m).ok()?;
        (meeting.status != ghi_core::import::IMPORTING).then_some(DuplicateOf {
            meeting: m,
            title: meeting.title,
        })
    };
    // Tracks of a group are hashed together (as the import will).
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, s) in list.iter().enumerate() {
        if let Some(g) = &s.info.group {
            match groups.iter_mut().find(|(id, _)| id == g) {
                Some((_, v)) => v.push(i),
                None => groups.push((g.clone(), vec![i])),
            }
        }
    }
    for (_, members) in groups {
        if members.iter().any(|&i| {
            list[i]
                .info
                .problems
                .contains(&ImportProblem::TooManyTracks)
        }) {
            continue;
        }
        let paths: Vec<PathBuf> = members.iter().map(|&i| list[i].path.clone()).collect();
        let Ok(hash) = ghi_core::import::tracks_hash(&paths, None) else {
            continue;
        };
        let dup = earlier(&hash);
        for &i in &members {
            // Kept for the import, which would hash the same files again.
            list[i].group_hash = Some(hash.clone());
            list[i].group_size = members.len();
            if let Some(d) = &dup {
                list[i].info.problems.push(ImportProblem::Duplicate);
                list[i].info.duplicate_of = Some(d.clone());
            }
        }
    }
    // Only a file that can be imported is hashed (duplicates; the import
    // reuses the hash).
    for s in list.iter_mut().filter(|s| s.info.group.is_none()) {
        if s.info.problems.is_empty() || s.info.problems == [ImportProblem::VeryLong] {
            s.hash = ghi_core::import::file_sha256(&s.path).ok();
            if let Some(d) = s.hash.as_deref().and_then(earlier) {
                s.info.problems.push(ImportProblem::Duplicate);
                s.info.duplicate_of = Some(d);
            }
        }
    }
}

/// An error message with the file paths replaced by their names: the webview
/// never gets a path.
fn scrub(message: &str, files: &[(PathBuf, Option<String>)]) -> String {
    files.iter().fold(message.to_string(), |m, (p, _)| {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        m.replace(&p.display().to_string(), &name)
    })
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
        let paths = expand(paths.into_iter().filter(|p| p.is_absolute()).collect());
        let mut list: Vec<Staged> = paths.into_iter().take(100).map(probe_one).collect();
        group_zoom(&mut list);
        finish(core, &mut list);
        for s in list {
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
            let files = job.files.clone();
            // A decoder bug on a malformed file must not stop the queue.
            let r =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match files.as_slice() {
                    [(path, _)] => core.import(path, job.opts),
                    _ => core.import_tracks(&files, job.opts),
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
                    let msg = scrub(&e, &job.files);
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

/// Removes files from the staging list. Returns the staged files whose
/// problems changed because of it: the mixed Zoom recording comes back once
/// its participant tracks are all gone.
#[tauri::command]
#[specta::specta]
pub fn unstage_files(imports: tauri::State<'_, Arc<Imports>>, ids: Vec<String>) -> Vec<StagedFile> {
    let mut staged = lock(&imports.staged);
    for id in ids {
        staged.remove(&id);
    }
    refresh_superseded(staged.values_mut())
}

/// Imports a staged Zoom recording's tracks as separate meetings instead of
/// one: the tracks lose their group (each is checked for duplicates as a file
/// of its own) and the mixed recording is no longer left out. Returns every
/// staged file that changed. The way out for a recording with more than 49
/// tracks. Errors: `storage`.
#[tauri::command]
#[specta::specta]
pub async fn import_tracks_separately(
    core: CoreState<'_>,
    imports: tauri::State<'_, Arc<Imports>>,
    group: String,
) -> Result<Vec<StagedFile>, String> {
    let imports = imports.inner().clone();
    blocking(&core, move |c| Ok(imports.ungroup(c, &group))).await
}

impl Imports {
    fn ungroup(&self, core: &Core, group: &str) -> Vec<StagedFile> {
        let mut members: Vec<Staged> = {
            let mut staged = lock(&self.staged);
            let ids: Vec<String> = staged
                .values()
                .filter(|s| s.info.group.as_deref() == Some(group))
                .map(|s| s.info.id.clone())
                .collect();
            ids.iter().filter_map(|id| staged.remove(id)).collect()
        };
        for s in &mut members {
            s.info.group = None;
            s.info.participant = None;
            s.group_hash = None;
            s.group_size = 0;
            s.info
                .problems
                .retain(|p| !matches!(p, ImportProblem::Duplicate | ImportProblem::TooManyTracks));
            s.info.duplicate_of = None;
        }
        finish(core, &mut members);
        let mut changed: Vec<StagedFile> = members.iter().map(|s| s.info.clone()).collect();
        let mut staged = lock(&self.staged);
        for s in members {
            staged.insert(s.info.id.clone(), s);
        }
        changed.extend(refresh_superseded(staged.values_mut()));
        changed
    }
}

/// Queues staged files for import (unsupported, empty and superseded ones
/// are skipped). The tracks of a group, if all are passed, become one import
/// whose id is the group's.
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
    // What to import: single files, and the tracks of each group together.
    let mut singles: Vec<Staged> = Vec::new();
    let mut groups: Vec<(String, Vec<Staged>)> = Vec::new();
    for id in ids {
        let Some(s) = staged.remove(&id) else {
            continue;
        };
        if s.info.problems.iter().any(|p| {
            matches!(
                p,
                ImportProblem::Unsupported
                    | ImportProblem::Empty
                    | ImportProblem::Superseded
                    | ImportProblem::TooManyTracks
            )
        }) {
            continue;
        }
        match s.info.group.clone() {
            Some(g) => match groups.iter_mut().find(|(id, _)| *id == g) {
                Some((_, v)) => v.push(s),
                None => groups.push((g, vec![s])),
            },
            None => singles.push(s),
        }
    }
    // A group of one is just a file.
    let (whole, lone): (Vec<_>, Vec<_>) = groups.into_iter().partition(|(_, v)| v.len() >= 2);
    singles.extend(lone.into_iter().flat_map(|(_, v)| v));
    let groups = whole;
    let mut work: Vec<(String, Vec<Staged>)> = singles
        .into_iter()
        .map(|s| (s.info.id.clone(), vec![s]))
        .collect();
    work.extend(groups);
    for (id, members) in work {
        let first = &members[0];
        let single = members.len() == 1;
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
            files: members
                .iter()
                .map(|m| (m.path.clone(), m.info.participant.clone()))
                .collect(),
            opts: ImportOptions {
                // What the name or tags said (the core would find the same).
                title: first.info.title.clone(),
                language: choice.language.clone().filter(|l| l == "en" || l == "vi"),
                split_channels: single && choice.split_channels && first.info.channels >= 2,
                started_at: first.started_at,
                cancel: Some(cancel),
                on_progress: Some(on_progress),
                // The hash staging made: of the file, or of the group if it
                // is the one that was hashed.
                source_hash: if single {
                    first.hash.clone()
                } else if first.group_size == members.len() {
                    first.group_hash.clone()
                } else {
                    None
                },
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

    fn wav(path: &Path, secs: f32, hz: f32) {
        let mut w = hound::WavWriter::create(
            path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for i in 0..(16_000.0 * secs) as usize {
            let x = (i as f32 * hz * std::f32::consts::TAU / 16_000.0).sin() * 0.2;
            w.write_sample((x * 32_767.0) as i16).unwrap();
        }
        w.finalize().unwrap();
    }

    /// `<tmp>/2026-07-03 14.05.02 Sprint 81234567890/{audio_only.wav,Audio Record/...}`
    fn zoom_folder(tmp: &Path) -> PathBuf {
        let meeting = tmp.join("2026-07-03 14.05.02 Sprint 81234567890");
        let rec = meeting.join("Audio Record");
        std::fs::create_dir_all(&rec).unwrap();
        wav(&meeting.join("audio_only.wav"), 3.0, 200.0);
        wav(&rec.join("audioLinh1111.wav"), 3.0, 300.0);
        wav(&rec.join("audioMinh2222.wav"), 3.0, 700.0);
        wav(&rec.join("audio_recording_3.wav"), 3.0, 900.0);
        // A different length is not part of the recording.
        wav(&rec.join("audioStray9.wav"), 9.0, 500.0);
        std::fs::write(rec.join("notes.txt"), "not audio").unwrap();
        meeting
    }

    #[test]
    fn sources_are_guessed_from_names() {
        let s = |p: &str| source_of(Path::new(p));
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
        assert_eq!(
            s("/Users/a/Downloads/Weekly sync (2026-10-01 14:05 GMT+7) - Recording.mp4"),
            ImportSource::Meet
        );
    }

    #[test]
    fn a_dropped_folder_gives_its_audio_one_level_down() {
        let tmp = tempfile::tempdir().unwrap();
        let meeting = zoom_folder(tmp.path());
        // A Zoom meeting folder: the participant tracks, not the mixed file.
        let got: Vec<String> = expand(vec![meeting.clone()])
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            got,
            [
                "audioLinh1111.wav",
                "audioMinh2222.wav",
                "audioStray9.wav",
                "audio_recording_3.wav"
            ]
        );
        // Any other folder: its audio files; plain files pass through.
        let plain = tmp.path().join("memos");
        std::fs::create_dir_all(plain.join("deeper")).unwrap();
        wav(&plain.join("a.wav"), 1.0, 300.0);
        wav(&plain.join("deeper/b.wav"), 1.0, 300.0);
        std::fs::write(plain.join("c.txt"), "x").unwrap();
        let single = tmp.path().join("single.wav");
        wav(&single, 1.0, 300.0);
        let got = expand(vec![plain.clone(), single.clone()]);
        assert_eq!(got, vec![plain.join("a.wav"), single]);
        assert!(
            expand(vec![tmp.path().join("missing")]).len() == 1,
            "a missing path stays for staging to reject"
        );
    }

    #[test]
    fn zoom_tracks_are_grouped_named_and_the_mixed_file_is_superseded() {
        let tmp = tempfile::tempdir().unwrap();
        let meeting = zoom_folder(tmp.path());
        let mut paths = expand(vec![meeting.clone()]);
        paths.push(meeting.join("audio_only.wav"));
        paths.push(tmp.path().join("elsewhere.wav"));
        wav(&tmp.path().join("elsewhere.wav"), 3.0, 450.0);
        let mut list: Vec<Staged> = paths.into_iter().map(probe_one).collect();
        group_zoom(&mut list);
        let by = |n: &str| list.iter().find(|s| s.info.name == n).unwrap();
        let g = by("audioLinh1111.wav").info.group.clone().expect("grouped");
        assert_eq!(
            by("audioMinh2222.wav").info.group.as_deref(),
            Some(g.as_str())
        );
        assert_eq!(
            by("audio_recording_3.wav").info.group.as_deref(),
            Some(g.as_str())
        );
        assert_eq!(
            by("audioLinh1111.wav").info.participant.as_deref(),
            Some("Linh")
        );
        assert_eq!(by("audio_recording_3.wav").info.participant, None);
        assert_eq!(by("audioStray9.wav").info.group, None, "9 s is not 3 s");
        // The title and time come from the Zoom folder, for every track.
        for n in ["audioLinh1111.wav", "audioMinh2222.wav"] {
            assert_eq!(by(n).info.title.as_deref(), Some("Sprint"));
            assert_eq!(
                by(n).started_at,
                ghi_core::presets::local_ms(2026, 7, 3, 14, 5, 2)
            );
            assert_eq!(by(n).info.source, ImportSource::Zoom);
        }
        // The mixed recording of the same folder is not imported; others are.
        assert!(
            by("audio_only.wav")
                .info
                .problems
                .contains(&ImportProblem::Superseded)
        );
        assert!(by("elsewhere.wav").info.problems.is_empty());
        assert!(by("audioStray9.wav").info.problems.is_empty());
    }

    #[test]
    fn two_files_that_are_not_zoom_tracks_are_not_a_group() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("one.wav"), tmp.path().join("two.wav"));
        wav(&a, 2.0, 300.0);
        wav(&b, 2.0, 400.0);
        let mut list = vec![probe_one(a), probe_one(b)];
        group_zoom(&mut list);
        assert!(
            list.iter()
                .all(|s| s.info.group.is_none() && s.info.problems.is_empty())
        );
        // Unreadable files are never grouped either.
        let junk = tmp.path().join("audioNobody1.wav");
        std::fs::write(&junk, "junk").unwrap();
        let mut one = vec![probe_one(junk)];
        group_zoom(&mut one);
        assert!(one[0].info.group.is_none());
        assert!(one[0].info.problems.contains(&ImportProblem::Unsupported));
    }

    #[test]
    fn errors_never_carry_a_path() {
        let files = vec![
            (PathBuf::from("/Users/a/Zoom/x/audioLinh1111.m4a"), None),
            (PathBuf::from("/Users/a/Zoom/x/audioMinh2222.m4a"), None),
        ];
        let m = scrub(
            "/Users/a/Zoom/x/audioLinh1111.m4a: corrupt; /Users/a/Zoom/x/audioMinh2222.m4a: gone",
            &files,
        );
        assert_eq!(m, "audioLinh1111.m4a: corrupt; audioMinh2222.m4a: gone");
        assert!(!m.contains("/Users"));
    }

    /// A recording imported once is flagged when its tracks are staged again,
    /// as a group, in any order.
    #[cfg(unix)]
    #[test]
    fn staged_again_a_group_is_a_duplicate() {
        let tmp = tempfile::tempdir().unwrap();
        let meeting = zoom_folder(tmp.path());
        let (core, _rx) = Core::for_test(tmp.path().join("data"));
        let imports = Imports::default();
        let first = imports.stage(&core, vec![meeting.clone()]);
        assert!(first.iter().filter(|f| f.group.is_some()).count() >= 3);
        assert!(
            first
                .iter()
                .all(|f| !f.problems.contains(&ImportProblem::Duplicate))
        );
        // Import the group as the queue would.
        let files: Vec<(PathBuf, Option<String>)> = first
            .iter()
            .filter(|f| f.group.is_some())
            .map(|f| {
                let p = meeting.join("Audio Record").join(&f.name);
                (p, f.participant.clone())
            })
            .collect();
        let r = core
            .import_tracks(&files, ImportOptions::default())
            .unwrap();
        assert_eq!(r.tracks, files.len());
        // Staging the same folder, or the same files listed in reverse, flags
        // every track of the group with the meeting it made.
        for again in [
            imports.stage(&core, vec![meeting.clone()]),
            imports.stage(&core, files.iter().rev().map(|(p, _)| p.clone()).collect()),
        ] {
            let tracks: Vec<_> = again.iter().filter(|f| f.group.is_some()).collect();
            assert!(!tracks.is_empty());
            for f in tracks {
                assert!(f.problems.contains(&ImportProblem::Duplicate), "{f:?}");
                assert_eq!(f.duplicate_of.as_ref().unwrap().meeting, r.meeting);
            }
        }
    }

    #[test]
    fn the_mixed_recording_is_left_out_only_in_a_zoom_folder_and_comes_back() {
        let tmp = tempfile::tempdir().unwrap();
        let meeting = zoom_folder(tmp.path());
        let mut list: Vec<Staged> = expand(vec![meeting.clone()])
            .into_iter()
            .map(probe_one)
            .collect();
        list.push(probe_one(meeting.join("audio_only.wav")));
        group_zoom(&mut list);
        let mixed = |l: &[Staged]| {
            l.iter()
                .find(|s| s.info.name == "audio_only.wav")
                .unwrap()
                .info
                .problems
                .clone()
        };
        assert_eq!(mixed(&list), [ImportProblem::Superseded]);
        // Take every track away: the mixed file is importable again.
        let mut rest: Vec<Staged> = list
            .into_iter()
            .filter(|s| s.info.name == "audio_only.wav")
            .collect();
        let changed = refresh_superseded(rest.iter_mut());
        assert_eq!(changed.len(), 1);
        assert!(mixed(&rest).is_empty());
        // The same layout in a folder that is not a Zoom meeting folder: never.
        let other = tmp.path().join("my recordings");
        std::fs::create_dir_all(other.join("Audio Record")).unwrap();
        wav(&other.join("audio_only.wav"), 3.0, 200.0);
        wav(&other.join("Audio Record/audioA1.wav"), 3.0, 300.0);
        wav(&other.join("Audio Record/audioB2.wav"), 3.0, 400.0);
        let mut list: Vec<Staged> = expand(vec![other.clone()])
            .into_iter()
            .map(probe_one)
            .collect();
        list.push(probe_one(other.join("audio_only.wav")));
        group_zoom(&mut list);
        assert!(
            list.iter().any(|s| s.info.group.is_some()),
            "the tracks still group"
        );
        assert!(
            mixed(&list).is_empty(),
            "but nothing is left out in a folder Zoom did not name"
        );
    }

    #[test]
    fn other_audio_names_are_not_grouped_or_superseded() {
        let tmp = tempfile::tempdir().unwrap();
        let mut list = Vec::new();
        for (n, hz) in [
            ("audio-en.wav", 300.0),
            ("audio-vi.wav", 400.0),
            ("audiobook1.wav", 500.0),
            ("audiobook2.wav", 600.0),
        ] {
            let p = tmp.path().join(n);
            wav(&p, 2.0, hz);
            list.push(probe_one(p));
        }
        group_zoom(&mut list);
        assert!(
            list.iter()
                .all(|s| s.info.group.is_none() && s.info.problems.is_empty())
        );
    }

    #[test]
    fn a_recording_with_too_many_tracks_is_flagged_when_staged() {
        let tmp = tempfile::tempdir().unwrap();
        let rec = tmp
            .path()
            .join("2026-07-03 14.05.02 Big 81234567890/Audio Record");
        std::fs::create_dir_all(&rec).unwrap();
        for i in 0..50 {
            wav(
                &rec.join(format!("audioP{i:02}{:08}.wav", 10_000_000 + i)),
                0.5,
                200.0 + 5.0 * i as f32,
            );
        }
        let mut list: Vec<Staged> =
            expand(vec![tmp.path().join("2026-07-03 14.05.02 Big 81234567890")])
                .into_iter()
                .map(probe_one)
                .collect();
        group_zoom(&mut list);
        assert_eq!(list.len(), 50);
        assert!(
            list.iter().all(|s| s.info.group.is_some()
                && s.info.problems.contains(&ImportProblem::TooManyTracks))
        );
        // 49 is fine.
        let mut list: Vec<Staged> = list.into_iter().take(49).collect();
        for s in &mut list {
            s.info.group = None;
            s.info.problems.clear();
        }
        group_zoom(&mut list);
        assert!(
            list.iter()
                .all(|s| s.info.group.is_some() && s.info.problems.is_empty())
        );
    }

    /// Staging a group hashes it once; separating it makes each track a file
    /// of its own again and brings the mixed recording back.
    #[cfg(unix)]
    #[test]
    fn a_group_can_be_imported_separately() {
        let tmp = tempfile::tempdir().unwrap();
        let meeting = zoom_folder(tmp.path());
        let (core, _rx) = Core::for_test(tmp.path().join("data"));
        let imports = Imports::default();
        let mut paths = vec![meeting.clone()];
        paths.push(meeting.join("audio_only.wav"));
        let staged = imports.stage(&core, paths);
        let group = staged
            .iter()
            .find_map(|f| f.group.clone())
            .expect("grouped");
        {
            let st = lock(&imports.staged);
            let member = st
                .values()
                .find(|s| s.info.group.as_deref() == Some(group.as_str()))
                .unwrap();
            assert!(
                member.group_hash.is_some() && member.group_size >= 3,
                "kept for the import"
            );
            assert!(member.hash.is_none());
        }
        assert!(
            staged
                .iter()
                .any(|f| f.problems.contains(&ImportProblem::Superseded))
        );
        let changed = imports.ungroup(&core, &group);
        assert!(changed.iter().all(|f| f.group.is_none()));
        assert!(
            changed
                .iter()
                .all(|f| !f.problems.contains(&ImportProblem::Superseded)),
            "the mixed file is back"
        );
        let st = lock(&imports.staged);
        assert!(st.values().all(|s| s.info.group.is_none()));
        assert!(
            st.values()
                .filter(|s| s.info.name != "audio_only.wav" && s.info.problems.is_empty())
                .all(|s| s.hash.is_some()),
            "each former track is hashed on its own (the restored mixed file by the import): {:?}",
            st.values()
                .map(|s| (
                    s.info.name.clone(),
                    s.info.problems.clone(),
                    s.hash.is_some()
                ))
                .collect::<Vec<_>>()
        );
    }
}
