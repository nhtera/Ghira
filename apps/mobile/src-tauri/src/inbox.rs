// SPDX-License-Identifier: Apache-2.0
//! The share-extension inbox (M5). The extension copies a shared audio file
//! into the App Group as `inbox/<uuid>/<file>` and writes
//! `inbox/<uuid>/manifest.json` last:
//!
//! ```json
//! { "file": "Memo.m4a", "lang": "auto|en|vi", "target": "phone|cloud|desktop",
//!   "source": "Voice Memos", "confirmed": true }
//! ```
//!
//! `confirmed` is true when the user pressed Import in the extension; such an
//! item is imported without asking again. Without it (the extension closed
//! early) the item waits in the app's inbox list for a language and target.
//!
//! Everything in the inbox is untrusted input from another process: the id
//! must be a UUID, the file name a plain name (no separators, no dot-files),
//! the type one the decoder reads, the size under a cap, and nothing may be a
//! symlink. A rejected item stays (with a reason code) until the user
//! dismisses it; the file is deleted as soon as it is imported.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use ghi_app::system::MeetingLanguage;
use ghi_core::import::{ImportOptions, ImportReport};
use serde::Deserialize;

use crate::cmd::events::{MobileEvent, emit};
use crate::cmd::import::{InboxItem, InboxState};
use crate::cmd::types::ProcessingTarget;

/// The largest file accepted (a 3 h 48 kHz stereo WAV is about 2 GB).
pub const MAX_BYTES: u64 = 3 << 30;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;
/// Types the decoder reads (Symphonia). CAF and the rest are rejected.
pub const EXTENSIONS: [&str; 11] = [
    "m4a", "mp3", "wav", "aac", "flac", "ogg", "oga", "opus", "aif", "aiff", "mp4",
];
/// Items nobody dealt with are removed after this long.
const STALE_AFTER: Duration = Duration::from_secs(14 * 24 * 3600);
/// The extension builds an item here before renaming it into the inbox; a
/// killed extension leaves folders behind.
const STAGING_DIR: &str = "inbox-staging";
const STAGING_STALE_AFTER: Duration = Duration::from_secs(24 * 3600);
/// Written in an item's folder when importing it failed.
const REJECTED_MARK: &str = "rejected";

/// Why an item cannot be imported (`InboxItem.reason`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    BadManifest,
    BadName,
    UnsupportedType,
    TooLarge,
    Empty,
    Missing,
    Unreadable,
}

impl Reason {
    pub fn code(self) -> &'static str {
        match self {
            Reason::BadManifest => "badManifest",
            Reason::BadName => "badName",
            Reason::UnsupportedType => "unsupportedType",
            Reason::TooLarge => "tooLarge",
            Reason::Empty => "empty",
            Reason::Missing => "missing",
            Reason::Unreadable => "unreadable",
        }
    }

    fn from_code(c: &str) -> Reason {
        [
            Reason::BadManifest,
            Reason::BadName,
            Reason::UnsupportedType,
            Reason::TooLarge,
            Reason::Empty,
            Reason::Missing,
        ]
        .into_iter()
        .find(|r| r.code() == c)
        .unwrap_or(Reason::Unreadable)
    }
}

/// `8-4-4-4-12` hex digits, either case (Swift prints UUIDs in upper case).
pub fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

#[derive(Debug, Deserialize)]
struct Manifest {
    file: String,
    #[serde(default)]
    lang: Option<String>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    confirmed: bool,
}

/// A plain file name with a type we decode; nothing that could leave the folder.
fn check_name(name: &str) -> Result<(), Reason> {
    let bad = name.is_empty()
        || name.len() > 255
        || name.starts_with('.')
        || name == REJECTED_MARK
        || name.contains(['/', '\\', '\0'])
        || name.chars().any(char::is_control);
    if bad {
        return Err(Reason::BadName);
    }
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    if EXTENSIONS.contains(&ext.as_str()) {
        Ok(())
    } else {
        Err(Reason::UnsupportedType)
    }
}

/// What the folder `inbox/<id>/` holds.
#[derive(Debug)]
pub struct Entry {
    pub id: String,
    pub dir: PathBuf,
    /// The shared file's name (empty when the manifest is unusable).
    pub name: String,
    pub size: u64,
    pub language: MeetingLanguage,
    pub target: ProcessingTarget,
    pub confirmed: bool,
    pub verdict: Result<(), Reason>,
}

impl Entry {
    pub fn file(&self) -> PathBuf {
        self.dir.join(&self.name)
    }
}

fn parse_language(s: Option<&str>) -> MeetingLanguage {
    match s {
        Some("en") => MeetingLanguage::En,
        Some("vi") => MeetingLanguage::Vi,
        _ => MeetingLanguage::Auto,
    }
}

fn parse_target(s: Option<&str>) -> ProcessingTarget {
    match s {
        Some("cloud") => ProcessingTarget::Cloud,
        Some("desktop") => ProcessingTarget::Desktop,
        _ => ProcessingTarget::Phone,
    }
}

/// Opens a regular file without following a symlink (and without blocking on
/// a FIFO), checked on the open descriptor, so a swap after the check cannot
/// redirect the read.
fn open_regular(p: &Path) -> Option<(std::fs::File, u64)> {
    use std::os::unix::fs::OpenOptionsExt;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(p)
        .ok()?;
    let m = f.metadata().ok()?;
    m.is_file().then_some((f, m.len()))
}

/// At most `cap` bytes of a regular file.
fn read_small(p: &Path, cap: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let (f, len) = open_regular(p)?;
    if len > cap {
        return None;
    }
    let mut out = Vec::new();
    f.take(cap).read_to_end(&mut out).ok()?;
    Some(out)
}

/// Removes the entries of `dir` older than `age` (symlinks are unlinked,
/// never followed); returns how many went.
fn sweep_older_than(dir: &Path, age: Duration) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut n = 0;
    for e in rd.filter_map(Result::ok) {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|a| a > age);
        let Ok(ty) = e.file_type() else { continue };
        if old {
            let p = e.path();
            let r = if ty.is_dir() {
                std::fs::remove_dir_all(&p)
            } else {
                std::fs::remove_file(&p)
            };
            n += usize::from(r.is_ok());
        }
    }
    n
}

/// The inbox folder is a real directory (a symlinked root is never followed).
fn root_ok(root: &Path) -> bool {
    std::fs::symlink_metadata(root).is_ok_and(|m| m.is_dir())
}

/// Reads one item. The folder must be a real directory named by a UUID.
pub fn read_entry(root: &Path, id: &str) -> Option<Entry> {
    if !is_uuid(id) || !root_ok(root) {
        return None;
    }
    let dir = root.join(id);
    let m = std::fs::symlink_metadata(&dir).ok()?;
    if !m.is_dir() {
        return None;
    }
    let mut entry = Entry {
        id: id.to_owned(),
        dir: dir.clone(),
        name: String::new(),
        size: 0,
        language: MeetingLanguage::Auto,
        target: ProcessingTarget::Phone,
        confirmed: false,
        verdict: Err(Reason::BadManifest),
    };
    let manifest_path = dir.join("manifest.json");
    let manifest = read_small(&manifest_path, MAX_MANIFEST_BYTES)
        .and_then(|b| serde_json::from_slice::<Manifest>(&b).ok());
    let Some(manifest) = manifest else {
        return Some(entry);
    };
    entry.language = parse_language(manifest.lang.as_deref());
    entry.target = parse_target(manifest.target.as_deref());
    entry.confirmed = manifest.confirmed;
    entry.verdict = check_name(&manifest.file).and_then(|()| {
        entry.name = manifest.file.clone();
        match open_regular(&dir.join(&manifest.file)).map(|(_, n)| n) {
            None => Err(Reason::Missing),
            Some(0) => Err(Reason::Empty),
            Some(n) if n > MAX_BYTES => {
                entry.size = n;
                Err(Reason::TooLarge)
            }
            Some(n) => {
                entry.size = n;
                Ok(())
            }
        }
    });
    // A bad name is never shown or opened.
    if entry.verdict == Err(Reason::BadName) {
        entry.name.clear();
    }
    // An earlier import failed.
    if entry.verdict.is_ok()
        && let Some(code) = read_small(&dir.join(REJECTED_MARK), 64)
    {
        entry.verdict = Err(Reason::from_code(String::from_utf8_lossy(&code).trim()));
    }
    Some(entry)
}

/// Every item under `root` (folders that are not UUIDs, files and symlinks
/// are ignored), oldest first.
pub fn scan(root: &Path) -> Vec<Entry> {
    let Ok(rd) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    if !root_ok(root) {
        return Vec::new();
    }
    let mut out: Vec<(SystemTime, Entry)> = rd
        .filter_map(Result::ok)
        .filter_map(|e| {
            let id = e.file_name().into_string().ok()?;
            let entry = read_entry(root, &id)?;
            let at = e.metadata().and_then(|m| m.modified()).ok()?;
            Some((at, entry))
        })
        .collect();
    out.sort_by_key(|(at, e)| (*at, e.id.clone()));
    out.into_iter().map(|(_, e)| e).collect()
}

/// Copies the shared file into `scratch` (never a symlink, never more than
/// the size cap) and returns the copy's path.
fn copy_in(entry: &Entry, scratch: &Path) -> Result<PathBuf, String> {
    use std::io::Read;
    let (src, len) = open_regular(&entry.file()).ok_or("missing")?;
    if len == 0 || len > MAX_BYTES {
        return Err("size".into());
    }
    std::fs::create_dir_all(scratch).map_err(|e| e.kind().to_string())?;
    let ext = entry
        .name
        .rsplit_once('.')
        .map_or("bin", |(_, e)| e)
        .to_ascii_lowercase();
    let dest = scratch.join(format!("{}.{ext}", entry.id));
    let result = (|| {
        let mut out = std::fs::File::create(&dest)?;
        let n = std::io::copy(&mut src.take(MAX_BYTES + 1), &mut out)?;
        if n > MAX_BYTES {
            return Err(std::io::Error::other("size"));
        }
        out.sync_all()
    })();
    match result {
        Ok(()) => Ok(dest),
        Err(e) => {
            let _ = std::fs::remove_file(&dest);
            Err(e.kind().to_string())
        }
    }
}

/// Writes the "rejected" marker (replacing, never following, a link).
fn mark_rejected(dir: &Path) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let p = dir.join(REJECTED_MARK);
    let _ = std::fs::remove_file(&p);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&p)
    {
        let _ = f.write_all(Reason::Unreadable.code().as_bytes());
    }
}

/// Runs one import: the app gives the core's `import`, tests a store's.
pub type Importer = dyn Fn(&Path, ImportOptions) -> Result<ImportReport, String> + Send + Sync;

/// An item is given up on (marked unreadable) after this many failed imports,
/// unless the file itself cannot be opened by the decoder (then at once).
const MAX_FAILURES: u32 = 3;

pub struct Inbox {
    root: PathBuf,
    /// Where an item is copied (from an open descriptor) before it is decoded.
    scratch: PathBuf,
    /// Items being imported now (an item is imported once).
    importing: Mutex<HashSet<String>>,
    /// Failed imports per item (store or disk trouble is retried).
    failures: Mutex<std::collections::HashMap<String, u32>>,
}

impl Inbox {
    pub fn new(root: PathBuf, scratch: PathBuf) -> Arc<Inbox> {
        Arc::new(Inbox {
            root,
            scratch,
            importing: Mutex::new(HashSet::new()),
            failures: Mutex::new(Default::default()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// An import is running.
    pub fn importing(&self) -> bool {
        !self.busy().is_empty()
    }

    fn busy(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.importing.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn list(&self) -> Vec<InboxItem> {
        let busy = self.busy().clone();
        scan(&self.root)
            .into_iter()
            .map(|e| {
                let (state, reason) = match &e.verdict {
                    _ if busy.contains(&e.id) => (InboxState::Importing, None),
                    Ok(()) => (InboxState::Pending, None),
                    Err(r) => (InboxState::Rejected, Some(r.code().to_owned())),
                };
                InboxItem {
                    id: e.id,
                    name: e.name,
                    size_bytes: e.size as f64,
                    language: e.language,
                    target: e.target,
                    state,
                    reason,
                }
            })
            .collect()
    }

    /// Imports item `id` with the user's choices; returns the meeting id.
    /// The inbox copy is deleted when the import is stored (or was a
    /// duplicate); on failure it stays, marked rejected.
    pub fn import(
        &self,
        id: &str,
        language: MeetingLanguage,
        target: ProcessingTarget,
        hold: Option<ghi_core::import::Hold>,
        importer: &Importer,
    ) -> Result<String, String> {
        // The `Desktop` target imports like the phone's: an imported file's
        // audio never travels to the computer and so is never leased (doc 07
        // §8); its text syncs like any meeting's.
        let _ = target;
        let entry = read_entry(&self.root, id).ok_or("notFound")?;
        if let Err(r) = entry.verdict {
            return Err(r.code().into());
        }
        if !self.busy().insert(entry.id.clone()) {
            return Err("busy".into());
        }
        // Another import may have finished between the read and the claim.
        let Some(entry) = read_entry(&self.root, id).filter(|e| e.verdict.is_ok()) else {
            self.busy().remove(id);
            return Err("notFound".into());
        };
        emit(MobileEvent::InboxChanged);
        let out = self.import_claimed(&entry, language, hold, importer);
        self.busy().remove(&entry.id);
        emit(MobileEvent::InboxChanged);
        out
    }

    fn import_claimed(
        &self,
        entry: &Entry,
        language: MeetingLanguage,
        hold: Option<ghi_core::import::Hold>,
        importer: &Importer,
    ) -> Result<String, String> {
        let opts = ImportOptions {
            language: language.hint(),
            hold,
            ..ImportOptions::default()
        };
        // Decode a private copy made from the open, checked descriptor: the
        // shared folder can change under us at any time.
        let result = copy_in(entry, &self.scratch).and_then(|copy| {
            let r = importer(&copy, opts);
            // A file the decoder cannot even open is the file's fault.
            let bad_file = r.is_err() && ghi_audio::decode::Decoder::open(&copy).is_err();
            let _ = std::fs::remove_file(&copy);
            r.map_err(|_| if bad_file { "format" } else { "failed" }.to_owned())
        });
        match result {
            Ok(report) => {
                self.failures().remove(&entry.id);
                let _ = std::fs::remove_dir_all(&entry.dir);
                Ok(report.meeting)
            }
            Err(kind) => {
                // Never the error text: it names the file's path.
                log::warn!("an inbox import failed ({kind})");
                let n = {
                    let mut f = self.failures();
                    let n = f.entry(entry.id.clone()).or_default();
                    *n += 1;
                    *n
                };
                if kind == "format" || n >= MAX_FAILURES {
                    self.failures().remove(&entry.id);
                    mark_rejected(&entry.dir);
                    Err(Reason::Unreadable.code().to_owned())
                } else {
                    // Store or disk trouble: the item stays as it was.
                    Err("retry".into())
                }
            }
        }
    }

    fn failures(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, u32>> {
        self.failures.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Deletes an item and its file.
    pub fn dismiss(&self, id: &str) -> Result<(), String> {
        let entry = read_entry(&self.root, id).ok_or("notFound")?;
        if self.busy().contains(&entry.id) {
            return Err("busy".into());
        }
        std::fs::remove_dir_all(&entry.dir).map_err(|e| e.to_string())?;
        emit(MobileEvent::InboxChanged);
        Ok(())
    }

    /// Items the extension's user confirmed, ready to import with the
    /// extension's choices.
    pub fn confirmed(&self) -> Vec<Entry> {
        let busy = self.busy().clone();
        scan(&self.root)
            .into_iter()
            .filter(|e| e.confirmed && e.verdict.is_ok() && !busy.contains(&e.id))
            .collect()
    }

    /// Removes what a killed share extension left in `inbox-staging/`
    /// (a sibling of the inbox) for over a day.
    fn sweep_staging(&self) -> usize {
        let Some(dir) = self.root.parent().map(|p| p.join(STAGING_DIR)) else {
            return 0;
        };
        sweep_older_than(&dir, STAGING_STALE_AFTER)
    }

    /// Removes items nobody dealt with for two weeks, and stray files.
    pub fn sweep(&self) -> usize {
        let staging = self.sweep_staging();
        let busy = self.busy().clone();
        // Copies a crash left behind.
        if busy.is_empty() {
            let _ = std::fs::remove_dir_all(&self.scratch);
        }
        let Ok(rd) = std::fs::read_dir(&self.root) else {
            return 0;
        };
        if !root_ok(&self.root) {
            return 0;
        }
        let mut n = staging;
        for e in rd.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().into_owned();
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > STALE_AFTER);
            let Ok(ty) = e.file_type() else { continue };
            // Symlinks are unlinked, never followed.
            let stray = !ty.is_dir() || !is_uuid(&name);
            if (stray || old) && !busy.contains(&name) {
                let p = e.path();
                let r = if ty.is_dir() {
                    std::fs::remove_dir_all(&p)
                } else {
                    std::fs::remove_file(&p)
                };
                n += usize::from(r.is_ok());
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_core::events::bus;
    use ghi_store::keys::MemoryKeyStore;
    use ghi_store::store::Store;
    use std::process::Command;

    const ID: &str = "0F8FAD5B-D9CB-469F-A165-70867728950E";

    fn write_item(root: &Path, id: &str, manifest: &str, file: Option<(&str, &[u8])>) -> PathBuf {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some((name, data)) = file {
            std::fs::write(dir.join(name), data).unwrap();
        }
        std::fs::write(dir.join("manifest.json"), manifest).unwrap();
        dir
    }

    fn manifest(file: &str) -> String {
        serde_json::json!({"file": file, "lang": "vi", "target": "cloud", "confirmed": true})
            .to_string()
    }

    fn verdict(root: &Path, id: &str) -> Result<(), Reason> {
        read_entry(root, id).unwrap().verdict
    }

    #[test]
    fn ids_must_be_uuids() {
        assert!(is_uuid(ID));
        assert!(is_uuid(&ID.to_lowercase()));
        for bad in [
            "",
            "..",
            "../x",
            "0F8FAD5B-D9CB-469F-A165-70867728950",
            "0F8FAD5BD9CB469FA16570867728950E1234",
            "0F8FAD5B-D9CB-469F-A165-70867728950G",
            "0F8FAD5B/D9CB-469F-A165-70867728950E",
        ] {
            assert!(!is_uuid(bad), "{bad}");
        }
        let t = tempfile::tempdir().unwrap();
        assert!(read_entry(t.path(), "../etc").is_none());
        assert!(read_entry(t.path(), "not-a-uuid").is_none());
        assert!(read_entry(t.path(), ID).is_none(), "no folder");
    }

    #[test]
    fn a_good_item_is_read_with_its_choices() {
        let t = tempfile::tempdir().unwrap();
        write_item(
            t.path(),
            ID,
            &manifest("Memo.M4A"),
            Some(("Memo.M4A", b"abc")),
        );
        let e = read_entry(t.path(), ID).unwrap();
        assert_eq!(e.verdict, Ok(()));
        assert_eq!((e.name.as_str(), e.size), ("Memo.M4A", 3));
        assert_eq!(e.language, MeetingLanguage::Vi);
        assert_eq!(e.target, ProcessingTarget::Cloud);
        assert!(e.confirmed);
    }

    #[test]
    fn names_that_could_leave_the_folder_are_refused() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("secret.m4a"), b"x").unwrap();
        for name in [
            "../secret.m4a",
            "sub/secret.m4a",
            "..\\secret.m4a",
            "/etc/passwd.m4a",
            ".hidden.m4a",
            "a\0b.m4a",
            "line\nbreak.m4a",
            "",
            "rejected",
        ] {
            write_item(t.path(), ID, &manifest(name), Some(("x.m4a", b"x")));
            let e = read_entry(t.path(), ID).unwrap();
            assert_eq!(e.verdict, Err(Reason::BadName), "{name:?}");
            assert!(e.name.is_empty(), "a bad name is not echoed");
            std::fs::remove_dir_all(t.path().join(ID)).unwrap();
        }
    }

    #[test]
    fn unknown_types_and_missing_or_empty_files_are_refused() {
        let t = tempfile::tempdir().unwrap();
        write_item(t.path(), ID, &manifest("a.caf"), Some(("a.caf", b"x")));
        assert_eq!(verdict(t.path(), ID), Err(Reason::UnsupportedType));
        write_item(t.path(), ID, &manifest("a"), Some(("a", b"x")));
        assert_eq!(verdict(t.path(), ID), Err(Reason::UnsupportedType));
        write_item(t.path(), ID, &manifest("a.sh"), Some(("a.sh", b"x")));
        assert_eq!(verdict(t.path(), ID), Err(Reason::UnsupportedType));
        write_item(t.path(), ID, &manifest("a.mp3"), None);
        assert_eq!(verdict(t.path(), ID), Err(Reason::Missing));
        write_item(t.path(), ID, &manifest("a.mp3"), Some(("a.mp3", b"")));
        assert_eq!(verdict(t.path(), ID), Err(Reason::Empty));
    }

    #[test]
    fn a_bad_or_oversize_manifest_is_refused() {
        let t = tempfile::tempdir().unwrap();
        write_item(t.path(), ID, "not json", Some(("a.mp3", b"x")));
        assert_eq!(verdict(t.path(), ID), Err(Reason::BadManifest));
        write_item(t.path(), ID, "{}", Some(("a.mp3", b"x")));
        assert_eq!(verdict(t.path(), ID), Err(Reason::BadManifest));
        let huge = format!(
            "{{\"file\":\"a.mp3\",\"source\":\"{}\"}}",
            "x".repeat(MAX_MANIFEST_BYTES as usize)
        );
        write_item(t.path(), ID, &huge, Some(("a.mp3", b"x")));
        assert_eq!(verdict(t.path(), ID), Err(Reason::BadManifest));
    }

    #[test]
    fn an_oversize_file_is_refused_without_reading_it() {
        let t = tempfile::tempdir().unwrap();
        let dir = write_item(t.path(), ID, &manifest("big.wav"), None);
        let f = std::fs::File::create(dir.join("big.wav")).unwrap();
        f.set_len(MAX_BYTES + 1).unwrap(); // sparse
        let e = read_entry(t.path(), ID).unwrap();
        assert_eq!(e.verdict, Err(Reason::TooLarge));
        assert_eq!(e.size, MAX_BYTES + 1);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_never_followed() {
        use std::os::unix::fs::symlink;
        let t = tempfile::tempdir().unwrap();
        let outside = t.path().join("outside.m4a");
        std::fs::write(&outside, b"secret").unwrap();
        let root = t.path().join("inbox");
        // The file is a link.
        let dir = write_item(&root, ID, &manifest("a.m4a"), None);
        symlink(&outside, dir.join("a.m4a")).unwrap();
        assert_eq!(verdict(&root, ID), Err(Reason::Missing));
        // The manifest is a link.
        std::fs::remove_file(dir.join("manifest.json")).unwrap();
        std::fs::write(dir.join("real.json"), manifest("a.m4a")).unwrap();
        symlink(dir.join("real.json"), dir.join("manifest.json")).unwrap();
        assert_eq!(verdict(&root, ID), Err(Reason::BadManifest));
        // The folder is a link.
        let id2 = "1F8FAD5B-D9CB-469F-A165-70867728950E";
        let real = t.path().join("elsewhere");
        write_item(
            t.path(),
            "elsewhere",
            &manifest("a.m4a"),
            Some(("a.m4a", b"x")),
        );
        symlink(&real, root.join(id2)).unwrap();
        assert!(read_entry(&root, id2).is_none());
        // Sweeping unlinks the link, not its target.
        Inbox::new(root.clone(), t.path().join("scratch")).sweep();
        assert!(!root.join(id2).exists() && real.exists());
        assert!(outside.exists());
    }

    #[test]
    fn listing_shows_states_and_reasons() {
        let t = tempfile::tempdir().unwrap();
        write_item(t.path(), ID, &manifest("a.mp3"), Some(("a.mp3", b"x")));
        let bad = "1F8FAD5B-D9CB-469F-A165-70867728950E";
        write_item(t.path(), bad, &manifest("a.caf"), Some(("a.caf", b"x")));
        std::fs::create_dir(t.path().join("not-a-uuid")).unwrap();
        let inbox = Inbox::new(t.path().to_path_buf(), t.path().join("scratch"));
        let list = inbox.list();
        assert_eq!(list.len(), 2);
        let good = list.iter().find(|i| i.id == ID).unwrap();
        assert_eq!(good.state, InboxState::Pending);
        let rej = list.iter().find(|i| i.id == bad).unwrap();
        assert_eq!(rej.state, InboxState::Rejected);
        assert_eq!(rej.reason.as_deref(), Some("unsupportedType"));
        assert_eq!(inbox.confirmed().len(), 1);
        assert_eq!(inbox.sweep(), 1, "the stray folder goes");
        assert!(!t.path().join("not-a-uuid").exists());
    }

    #[test]
    fn dismissing_deletes_the_folder_and_checks_the_id() {
        let t = tempfile::tempdir().unwrap();
        let dir = write_item(t.path(), ID, &manifest("a.mp3"), Some(("a.mp3", b"x")));
        let inbox = Inbox::new(t.path().to_path_buf(), t.path().join("scratch"));
        assert!(inbox.dismiss("../..").is_err());
        assert!(dir.exists());
        inbox.dismiss(ID).unwrap();
        assert!(!dir.exists());
        assert_eq!(inbox.dismiss(ID), Err("notFound".into()));
    }

    // -- importing real files -------------------------------------------------

    fn store(dir: &Path) -> Arc<Store> {
        Arc::new(Store::open(dir, Arc::new(MemoryKeyStore::default()), Default::default()).unwrap())
    }

    /// A 2 s 16 kHz mono sine WAV.
    fn make_wav(path: &Path) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..32_000 {
            let v = (i as f32 * 0.05).sin() * 0.3;
            w.write_sample((v * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();
    }

    fn convert(tool: &str, args: &[&str]) -> bool {
        Command::new(tool)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// Imports `file` through the inbox into a fresh store; `None` when the
    /// converter for this format is not installed here.
    fn import_through_inbox(make: impl Fn(&Path, &Path) -> bool, name: &str) -> Option<()> {
        let t = tempfile::tempdir().unwrap();
        let wav = t.path().join("src.wav");
        make_wav(&wav);
        let root = t.path().join("inbox");
        let dir = write_item(&root, ID, &manifest(name), None);
        if !make(&wav, &dir.join(name)) {
            eprintln!("skipped {name}: no converter on this machine");
            return None;
        }
        let bytes = std::fs::read(dir.join(name)).unwrap();
        let st = store(&t.path().join("store"));
        let (events, _rx) = bus();
        let inbox = Inbox::new(root, t.path().join("scratch"));
        let importer = {
            let st = st.clone();
            move |p: &Path, o: ImportOptions| ghi_core::import::import_file(&st, p, &o, &events)
        };
        let id = inbox
            .import(
                ID,
                MeetingLanguage::Vi,
                ProcessingTarget::Phone,
                None,
                &importer,
            )
            .unwrap();
        assert!(!dir.exists(), "the inbox copy is deleted after the import");
        let m = st.list_meetings(10, 0).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].gid, id);
        // The same file again is a duplicate of the first.
        write_item(inbox.root(), ID, &manifest(name), Some((name, &bytes)));
        let again = inbox
            .import(
                ID,
                MeetingLanguage::Auto,
                ProcessingTarget::Phone,
                None,
                &importer,
            )
            .unwrap();
        assert_eq!(again, id);
        assert_eq!(st.list_meetings(10, 0).unwrap().len(), 1);
        Some(())
    }

    #[test]
    fn a_wav_is_imported_and_its_copy_deleted() {
        import_through_inbox(|wav, out| std::fs::copy(wav, out).is_ok(), "Voice 001.wav").unwrap();
    }

    #[test]
    fn an_m4a_is_imported_when_afconvert_exists() {
        import_through_inbox(
            |wav, out| {
                convert(
                    "afconvert",
                    &[
                        "-f",
                        "m4af",
                        "-d",
                        "aac",
                        wav.to_str().unwrap(),
                        out.to_str().unwrap(),
                    ],
                )
            },
            "Memo.m4a",
        );
    }

    #[test]
    fn an_mp3_is_imported_when_ffmpeg_exists() {
        import_through_inbox(
            |wav, out| {
                convert(
                    "ffmpeg",
                    &[
                        "-v",
                        "error",
                        "-y",
                        "-i",
                        wav.to_str().unwrap(),
                        out.to_str().unwrap(),
                    ],
                )
            },
            "Call.mp3",
        );
    }

    #[test]
    fn a_file_the_decoder_cannot_open_is_marked_unreadable_at_once() {
        let t = tempfile::tempdir().unwrap();
        let dir = write_item(
            t.path(),
            ID,
            &manifest("junk.mp3"),
            Some(("junk.mp3", b"nope")),
        );
        let st = store(&t.path().join("store"));
        let (events, _rx) = bus();
        let inbox = Inbox::new(t.path().to_path_buf(), t.path().join("scratch"));
        let importer =
            move |p: &Path, o: ImportOptions| ghi_core::import::import_file(&st, p, &o, &events);
        let r = inbox.import(
            ID,
            MeetingLanguage::En,
            ProcessingTarget::Phone,
            None,
            &importer,
        );
        assert_eq!(r, Err("unreadable".into()));
        assert!(dir.exists());
        let item = inbox.list().remove(0);
        assert_eq!(item.state, InboxState::Rejected);
        assert_eq!(item.reason.as_deref(), Some("unreadable"));
        // And it is not offered for automatic import again.
        assert!(inbox.confirmed().is_empty());
        // The private copy is gone.
        assert_eq!(
            std::fs::read_dir(t.path().join("scratch")).unwrap().count(),
            0
        );
    }

    #[test]
    fn store_trouble_retries_and_never_marks_a_good_file() {
        let t = tempfile::tempdir().unwrap();
        let dir = write_item(t.path(), ID, &manifest("ok.wav"), None);
        make_wav(&dir.join("ok.wav"));
        let inbox = Inbox::new(t.path().to_path_buf(), t.path().join("scratch"));
        let down = |_: &Path, _: ImportOptions| -> Result<ImportReport, String> {
            Err("all data is being deleted".into())
        };
        for _ in 0..MAX_FAILURES - 1 {
            let r = inbox.import(
                ID,
                MeetingLanguage::Auto,
                ProcessingTarget::Phone,
                None,
                &down,
            );
            assert_eq!(r, Err("retry".into()));
            assert_eq!(inbox.list()[0].state, InboxState::Pending);
        }
        // Still failing at the limit: given up on, so it cannot loop for ever.
        let r = inbox.import(
            ID,
            MeetingLanguage::Auto,
            ProcessingTarget::Phone,
            None,
            &down,
        );
        assert_eq!(r, Err("unreadable".into()));
        assert_eq!(inbox.list()[0].state, InboxState::Rejected);
    }

    #[test]
    fn an_item_imported_meanwhile_is_not_found_not_unreadable() {
        let t = tempfile::tempdir().unwrap();
        let inbox = Inbox::new(t.path().to_path_buf(), t.path().join("scratch"));
        let never = |_: &Path, _: ImportOptions| -> Result<ImportReport, String> { unreachable!() };
        assert_eq!(
            inbox.import(
                ID,
                MeetingLanguage::Auto,
                ProcessingTarget::Phone,
                None,
                &never
            ),
            Err("notFound".into())
        );
        assert!(!inbox.importing(), "the claim is released");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_root_a_linked_marker_and_a_fifo_are_not_followed() {
        use std::os::unix::fs::symlink;
        let t = tempfile::tempdir().unwrap();
        let real = t.path().join("elsewhere");
        write_item(&real, ID, &manifest("a.mp3"), Some(("a.mp3", b"x")));
        let root = t.path().join("inbox");
        symlink(&real, &root).unwrap();
        assert!(scan(&root).is_empty());
        assert!(read_entry(&root, ID).is_none());
        let inbox = Inbox::new(root.clone(), t.path().join("scratch"));
        assert_eq!(inbox.sweep(), 0);
        assert!(
            real.join(ID).exists(),
            "nothing inside the link was removed"
        );

        // A marker that is a link to a secret reads as no marker, and is
        // replaced (not written through) when the item is marked.
        let dir = write_item(t.path(), ID, &manifest("a.mp3"), Some(("a.mp3", b"x")));
        let secret = t.path().join("secret");
        std::fs::write(&secret, b"keep").unwrap();
        symlink(&secret, dir.join(REJECTED_MARK)).unwrap();
        assert_eq!(verdict(t.path(), ID), Ok(()));
        mark_rejected(&dir);
        assert_eq!(std::fs::read(&secret).unwrap(), b"keep");
        assert_eq!(verdict(t.path(), ID), Err(Reason::Unreadable));

        // A FIFO as the marker does not block the read.
        std::fs::remove_file(dir.join(REJECTED_MARK)).unwrap();
        let fifo = std::ffi::CString::new(dir.join(REJECTED_MARK).to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert_eq!(verdict(t.path(), ID), Ok(()));
    }

    #[test]
    fn the_desktop_target_imports_like_the_phones() {
        let t = tempfile::tempdir().unwrap();
        write_item(t.path(), ID, &manifest("a.mp3"), Some(("a.mp3", b"x")));
        let inbox = Inbox::new(t.path().to_path_buf(), t.path().join("scratch"));
        let imported = |_: &Path, _: ImportOptions| -> Result<ImportReport, String> {
            Ok(ImportReport {
                meeting: "m1".into(),
                duplicate: false,
                duration_ms: 1,
                channels: 1,
                tracks: 1,
                jobs: vec![],
            })
        };
        assert_eq!(
            inbox.import(
                ID,
                MeetingLanguage::Auto,
                ProcessingTarget::Desktop,
                None,
                &imported
            ),
            Ok("m1".into())
        );
    }

    #[test]
    fn staging_leftovers_are_swept_when_old() {
        let t = tempfile::tempdir().unwrap();
        let staging = t.path().join(STAGING_DIR);
        std::fs::create_dir_all(staging.join(ID)).unwrap();
        std::fs::write(staging.join(ID).join("a.m4a"), b"x").unwrap();
        assert_eq!(sweep_older_than(&staging, STAGING_STALE_AFTER), 0);
        assert!(staging.join(ID).exists());
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(sweep_older_than(&staging, Duration::from_millis(1)), 1);
        assert!(!staging.join(ID).exists());
        // The inbox's own sweep looks next to its root.
        std::fs::create_dir_all(staging.join(ID)).unwrap();
        let inbox = Inbox::new(t.path().join("inbox"), t.path().join("scratch"));
        assert_eq!(inbox.sweep(), 0, "a fresh folder stays");
    }
}
