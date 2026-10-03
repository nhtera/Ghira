// SPDX-License-Identifier: Apache-2.0
//! The store facade the app uses.
//!
//! # Layout of the data directory
//!
//! ```text
//! <dir>/ghira.db                      SQLCipher database (key = HKDF(master))
//! <dir>/bundles/<meeting gid>/<kind>.ghb (+ .ghi)   encrypted audio
//! <dir>/snapshots/                    pre-migration copies of the database
//! <dir>/recovery.bin                  master key wrapped by the recovery phrase
//! ```
//!
//! # What is encrypted how
//!
//! Every meeting has its own random DEK, stored only wrapped by the master key
//! (`meetings.dek_wrapped`). It encrypts the meeting's audio pages, and its
//! text columns (`title_ct`, `segments.text_ct`, `notes_blocks.body_ct`,
//! `action_items.text_ct`) with the AAD `table.column:gid`. Text is stored as
//! NFC. The FTS indexes get the folded plaintext at insert time only.
//!
//! # Delete is a crypto-shred
//!
//! See [`Store::delete_meeting`], including the known limits.
//!
//! All methods take `&self`; the store is `Send + Sync` (one connection behind
//! a mutex). Nothing here blocks the audio thread: audio goes through the
//! [`BundleWriter`] returned by [`Store::open_track`], which never touches the
//! database.

use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::anchors::Anchor;
use crate::bundle::{self, BundleReader, BundleWriter};
use crate::fold;
use crate::keys::{KeyRing, KeyStore, Protection};
use crate::migrate::{self, Migration};
use crate::recovery::{self, RecoveryPhrase};
use crate::rowcrypt::{Dek, open_text, row_aad, seal_text};
use crate::tombstones;
use crate::{Result, StoreError, backup, db, export, new_gid};

/// File name of the database inside the data directory.
pub const DB_FILE: &str = "ghira.db";
const RECOVERY_FILE: &str = "recovery.bin";
/// Name of the key-ring entry in an export archive.
const RING_ENTRY: &str = "keyring.bin";

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

// ---------------------------------------------------------------- entities

/// Which capture a track holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackKind {
    Mic,
    System,
    File,
}

impl TrackKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TrackKind::Mic => "mic",
            TrackKind::System => "system",
            TrackKind::File => "file",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkTag {
    Star,
    Decision,
    Action,
    Question,
}

impl MarkTag {
    pub fn as_str(self) -> &'static str {
        match self {
            MarkTag::Star => "star",
            MarkTag::Decision => "decision",
            MarkTag::Action => "action",
            MarkTag::Question => "question",
        }
    }

    fn parse(s: &str) -> MarkTag {
        match s {
            "decision" => MarkTag::Decision,
            "action" => MarkTag::Action,
            "question" => MarkTag::Question,
            _ => MarkTag::Star,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NewMeeting {
    pub title: String,
    /// Unix ms; `0` means now.
    pub started_at: i64,
    /// `live` or `file` (imported recording).
    pub source: String,
    pub mode: String,
    pub lang: Option<String>,
    pub template: Option<String>,
    pub sensitive: bool,
    /// Delete the audio after this time (unix ms), keeping the text.
    pub audio_retained_until: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Meeting {
    pub gid: String,
    pub title: String,
    pub started_at: i64,
    pub duration_ms: i64,
    pub source: String,
    pub mode: String,
    pub lang: Option<String>,
    pub template: Option<String>,
    pub status: String,
    pub privacy_state: String,
    pub cloud_locked: bool,
    pub sensitive: bool,
    pub consent_confirmed: bool,
    pub cloud_used: bool,
    pub transcript_version: i64,
    pub audio_retained_until: Option<i64>,
    /// `zoom`, `teams`, `meet`, `plaud` or `voice_memos` for an import
    /// recognised as coming from there.
    pub source_app: Option<String>,
    /// The folder the meeting is in ([`Store::folders`]).
    pub folder_gid: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Word {
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub conf: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct NewSegment {
    /// A gid chosen by the caller (live lines get theirs before they are
    /// stored, so the UI can edit them at once); `None`: a new one.
    pub gid: Option<String>,
    pub speaker_gid: Option<String>,
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub text: String,
    pub lang: Option<String>,
    pub confidence: Option<f32>,
    /// Word timings; the word text is inside `text`.
    pub words: Vec<Word>,
    /// The user's text (kept as written by later passes).
    pub edited: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub gid: String,
    pub version: i64,
    pub speaker_gid: Option<String>,
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub text: String,
    pub lang: Option<String>,
    pub confidence: Option<f32>,
    pub edited: bool,
    /// Another speaker talked over this line ([`Store::mark_overlaps`]).
    pub overlap: bool,
}

#[derive(Debug, Clone, Default)]
pub struct NewSpeaker {
    pub label_idx: i64,
    pub display_name: Option<String>,
    pub person_gid: Option<String>,
    pub color_slot: i64,
    pub is_me: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Speaker {
    pub gid: String,
    pub label_idx: i64,
    pub display_name: Option<String>,
    pub person_gid: Option<String>,
    pub color_slot: i64,
    pub is_me: bool,
    pub not_person: bool,
    /// Merged into this speaker (its lines moved there).
    pub merged_into: Option<String>,
    /// "Sounds like ..." from the final pass's voice matching.
    pub suggestion: Option<SpeakerSuggestion>,
}

/// A voice-match suggestion on a speaker (never applied by itself).
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakerSuggestion {
    pub person_gid: String,
    /// The person's name (empty for Me).
    pub person_name: String,
    /// The suggested person is Me.
    pub is_me: bool,
    pub score: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Provenance {
    #[default]
    User,
    Ai,
    AiEdited,
}

impl Provenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Provenance::User => "user",
            Provenance::Ai => "ai",
            Provenance::AiEdited => "ai_edited",
        }
    }

    fn parse(s: &str) -> Provenance {
        match s {
            "ai" => Provenance::Ai,
            "ai_edited" => Provenance::AiEdited,
            _ => Provenance::User,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewNoteBlock {
    pub kind: String,
    pub provenance: Provenance,
    pub body: String,
    pub anchors: Vec<Anchor>,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NoteBlock {
    pub gid: String,
    pub kind: String,
    pub provenance: Provenance,
    pub body: String,
    pub anchors: Vec<Anchor>,
    pub pinned: bool,
}

#[derive(Debug, Clone, Default)]
pub struct NewActionItem {
    pub text: String,
    pub owner_speaker_gid: Option<String>,
    /// Due date (unix ms) when known.
    pub due: Option<i64>,
    /// The due date as spoken ("thứ Sáu"), encrypted like the text.
    pub due_text: Option<String>,
    /// Primary citation.
    pub anchor: Option<Anchor>,
    /// Every citation (the primary one first).
    pub anchors: Vec<Anchor>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActionItem {
    pub gid: String,
    pub text: String,
    pub owner_speaker_gid: Option<String>,
    pub due: Option<i64>,
    pub due_text: Option<String>,
    pub done: bool,
    pub anchor: Option<Anchor>,
    pub anchors: Vec<Anchor>,
    pub provenance: Provenance,
}

/// What [`Store::replace_ai_notes`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReplacedNotes {
    /// AI blocks and action items removed (unpinned, unedited, not done).
    pub removed: usize,
    /// User-written, pinned, edited or done items kept.
    pub kept: usize,
    pub added: usize,
}

/// Kinds of items that belong to a meeting ([`Store::meeting_of`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Segment,
    NoteBlock,
    ActionItem,
    Speaker,
}

/// One logged cloud request (no content: who, what model, how many tokens, when).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudRequest {
    pub meeting_gid: String,
    pub provider: String,
    pub model: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    /// Unix ms.
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub gid: String,
    pub t_ms: i64,
    pub tag: MarkTag,
}

// ------------------------------------------------------------------- store

pub struct Store {
    pub(crate) dir: PathBuf,
    ring: Mutex<KeyRing>,
    keystore: Arc<dyn KeyStore>,
    protection: Protection,
    conn: Mutex<Connection>,
    /// Unwrapped DEKs by meeting rowid; zeroized on drop / on shred.
    deks: Mutex<HashMap<i64, Dek>>,
    /// Unwrapped voice profile keys by profile rowid; zeroized on drop / delete.
    voice_keys: Mutex<HashMap<i64, Dek>>,
    /// Exclusive lock on `<dir>/.lock`, held for the store's lifetime.
    _lock: File,
}

/// A meeting's row id and current transcript version.
pub(crate) struct MeetingRef {
    pub id: i64,
    pub version: i64,
}

/// Job kind queued when finishing an interrupted delete failed.
pub const FINISH_DELETE_JOB: &str = "finish_delete";

impl Store {
    /// Opens the store in `dir`, creating it (and the key ring) on first run.
    ///
    /// The store keeps `keystore`: every meeting delete rotates the wrap
    /// secret and saves the ring again.
    ///
    /// If the keystore has no ring but the directory holds a database, this
    /// fails with [`StoreError::Invalid`] ("restore with the recovery phrase")
    /// instead of creating a key that opens nothing.
    pub fn open(dir: &Path, keystore: Arc<dyn KeyStore>, protection: Protection) -> Result<Store> {
        Store::open_with_migrations(dir, keystore, protection, migrate::MIGRATIONS)
    }

    /// [`open`](Store::open) with a custom migration list. This is the test
    /// hook for a failing migration: on failure the pre-migration snapshot is
    /// restored and [`StoreError::Migration`] returned; opening again with the
    /// real list works.
    pub fn open_with_migrations(
        dir: &Path,
        keystore: Arc<dyn KeyStore>,
        protection: Protection,
        migrations: &[Migration],
    ) -> Result<Store> {
        fs::create_dir_all(dir)?;
        let lock = acquire_lock(dir)?;
        let ring = match keystore.load()? {
            Some(r) => r,
            None if dir.join(DB_FILE).exists() => {
                return Err(StoreError::Invalid(
                    "the key for this store is missing: restore with the recovery phrase".into(),
                ));
            }
            None => {
                let r = KeyRing::generate();
                keystore.save(&r, protection)?;
                r
            }
        };
        fs::create_dir_all(dir.join("bundles"))?;
        fs::create_dir_all(snapshots_dir(dir))?;
        let _ = export::clean_stale_staging(dir, std::time::Duration::ZERO);
        let db_path = dir.join(DB_FILE);
        let conn = db::open(&db_path, &ring.db_key())?;
        let conn = migrate::run(
            conn,
            &db_path,
            migrations,
            &snapshots_dir(dir),
            migrate::KEEP_SNAPSHOTS,
        )?;
        let store = Store {
            dir: dir.to_path_buf(),
            ring: Mutex::new(ring),
            keystore,
            protection,
            conn: Mutex::new(conn),
            deks: Mutex::new(HashMap::new()),
            voice_keys: Mutex::new(HashMap::new()),
            _lock: lock,
        };
        let include = matches!(
            store.get_setting("include_in_backups")?,
            Some(serde_json::Value::Bool(true))
        );
        backup::exclude_from_backup(dir, !include)?;
        store.finish_pending_deletes();
        store.finish_pending_voice_deletes();
        // A crash mid-rotation left the ring holding two wrap secrets: finish
        // (best effort; both secrets keep working until it succeeds).
        if store.ring().is_rotating() {
            let _ = store.rotate_wraps(false);
        } else {
            // A crash (or a full disk) after a rotation's last ring save can
            // leave `recovery.bin` one ring behind: rewrite it (cheap, no KDF).
            let _ = store.write_recovery_file(&store.ring());
        }
        store.requeue_interrupted_jobs()?;
        store.recover_interrupted_tracks();
        Ok(store)
    }

    /// Recovery-phrase restore: unwraps the key ring from `<dir>/recovery.bin`,
    /// saves it into `keystore` (for a new device or a wiped keychain) and opens.
    pub fn restore_with_phrase(
        dir: &Path,
        phrase: &RecoveryPhrase,
        keystore: Arc<dyn KeyStore>,
        protection: Protection,
    ) -> Result<Store> {
        // Only for a lost key: an old recovery.bin (from a backup) must never
        // replace the current ring, whose wrap secret may be newer.
        if keystore.load()?.is_some() {
            return Err(StoreError::Invalid(
                "this device still has the key for this store; open it normally".into(),
            ));
        }
        let blob = fs::read(dir.join(RECOVERY_FILE))?;
        let ring = recovery::unwrap_ring(&blob, phrase)?;
        keystore.save(&ring, protection)?;
        Store::open(dir, keystore, protection)
    }

    /// Sets the recovery phrase: the ring gets the phrase-derived key (so
    /// `recovery.bin` can be rewritten after every rotation) and
    /// `recovery.bin` holds the ring wrapped by it. Show the phrase once and
    /// have the user re-enter it before calling this.
    pub fn set_recovery_phrase(&self, phrase: &RecoveryPhrase) -> Result<()> {
        let mut ring = self.ring();
        let before = ring.clone();
        ring.set_recovery_key(Some(*phrase.wrap_key()));
        if let Err(e) = self.keystore.save(&ring, self.protection) {
            // The save may have stored the new ring before failing: put the
            // old one back on top so a failed change never takes effect.
            let _ = self.keystore.save(&before, self.protection);
            *ring = before;
            return Err(e);
        }
        if let Err(e) = write_atomic(
            &self.dir.join(RECOVERY_FILE),
            &recovery::wrap_ring_with_key(&ring, &phrase.wrap_key()),
        ) {
            // Same: the old phrase (and its recovery.bin) stays in effect.
            let _ = self.keystore.save(&before, self.protection);
            *ring = before;
            return Err(e);
        }
        Ok(())
    }

    pub fn has_recovery_phrase(&self) -> bool {
        self.dir.join(RECOVERY_FILE).is_file()
    }

    /// Whether the data directory is included in OS backups (default: no).
    /// Backups keep deleted meetings, so turning this on needs a warning.
    pub fn set_include_in_backups(&self, include: bool) -> Result<()> {
        self.set_setting("include_in_backups", &serde_json::Value::Bool(include))?;
        backup::exclude_from_backup(&self.dir, !include)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub(crate) fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Lock order everywhere: connection, then ring, then DEK cache.
    pub(crate) fn ring(&self) -> MutexGuard<'_, KeyRing> {
        self.ring.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn deks(&self) -> MutexGuard<'_, HashMap<i64, Dek>> {
        self.deks.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub(crate) fn voice_keys(&self) -> MutexGuard<'_, HashMap<i64, Dek>> {
        self.voice_keys.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The meeting's DEK (from the cache or unwrapped from `dek_wrapped`).
    /// A shredded meeting gives [`StoreError::Decrypt`].
    pub(crate) fn dek(&self, conn: &Connection, meeting_id: i64) -> Result<Dek> {
        if let Some(d) = self.deks().get(&meeting_id) {
            return Ok(d.clone());
        }
        let (gid, wrapped): (String, Vec<u8>) = conn
            .query_row(
                "SELECT gid, dek_wrapped FROM meetings WHERE id = ?1",
                [meeting_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "meeting",
                gid: meeting_id.to_string(),
            })?;
        let dek = self.ring().unwrap_dek(&wrapped, &gid)?;
        self.deks().insert(meeting_id, dek.clone());
        Ok(dek)
    }

    pub(crate) fn meeting_ref(conn: &Connection, gid: &str) -> Result<MeetingRef> {
        conn.query_row(
            "SELECT id, transcript_version FROM meetings WHERE gid = ?1",
            [gid],
            |r| {
                Ok(MeetingRef {
                    id: r.get(0)?,
                    version: r.get(1)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound {
            kind: "meeting",
            gid: gid.to_string(),
        })
    }

    /// Takes `n` consecutive lamport values; returns the first.
    pub(crate) fn alloc_lamport(conn: &Connection, n: i64) -> Result<i64> {
        let last: i64 = conn.query_row(
            "UPDATE settings SET value_json = CAST(CAST(value_json AS INTEGER) + ?1 AS TEXT)
             WHERE key = 'lamport' RETURNING CAST(value_json AS INTEGER)",
            [n],
            |r| r.get(0),
        )?;
        Ok(last - n + 1)
    }

    /// `<dir>/bundles/<gid>`. The gid becomes a path, so it must be a canonical
    /// UUID; anything else (a crafted `../x` in an imported database) is
    /// [`StoreError::Invalid`].
    pub(crate) fn bundle_dir(&self, meeting_gid: &str) -> Result<PathBuf> {
        check_gid(meeting_gid)?;
        Ok(self.dir.join("bundles").join(meeting_gid))
    }

    pub fn bundle_path(&self, meeting_gid: &str, kind: TrackKind) -> Result<PathBuf> {
        Ok(self
            .bundle_dir(meeting_gid)?
            .join(format!("{}.ghb", kind.as_str())))
    }

    /// The AAD binding a track's audio pages to their track (its gid, which
    /// is in the `tracks` table).
    pub fn bundle_aad(track_gid: &str) -> Vec<u8> {
        format!("bundle:{track_gid}").into_bytes()
    }

    /// The SQLite version the database runs on (needs >= 3.43 for `contentless_delete`).
    pub fn sqlite_version(&self) -> Result<String> {
        Ok(self
            .conn()
            .query_row("SELECT sqlite_version()", [], |r| r.get(0))?)
    }

    /// Flushes the WAL into the database file (and truncates it).
    pub fn checkpoint(&self) -> Result<()> {
        db::checkpoint(&self.conn())
    }

    // ---------------------------------------------------------- settings

    pub fn get_setting(&self, key: &str) -> Result<Option<serde_json::Value>> {
        let raw: Option<String> = self
            .conn()
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                [key],
                |r| r.get(0),
            )
            .optional()?;
        raw.map(|s| serde_json::from_str(&s).map_err(|e| StoreError::Invalid(e.to_string())))
            .transpose()
    }

    pub fn set_setting(&self, key: &str, value: &serde_json::Value) -> Result<()> {
        if key == "lamport" {
            return Err(StoreError::Invalid("`lamport` is reserved".into()));
        }
        self.conn().execute(
            "INSERT INTO settings (key, value_json) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
            params![key, value.to_string()],
        )?;
        Ok(())
    }

    // ---------------------------------------------------------- meetings

    pub fn create_meeting(&self, new: NewMeeting) -> Result<Meeting> {
        let gid = new_gid();
        let dek = Dek::generate();
        // Wrap while holding the connection: a rotation (which holds it for
        // its whole run) can't drop the secret between the wrap and the insert.
        let mut conn = self.conn();
        let wrapped = self.ring().wrap_dek(&dek, &gid);
        let title_ct = seal_text(&dek, &new.title, &row_aad("meetings", "title_ct", &gid));
        let started_at = if new.started_at == 0 {
            now_ms()
        } else {
            new.started_at
        };
        let source = if new.source.is_empty() {
            "live".to_string()
        } else {
            new.source
        };
        let mode = if new.mode.is_empty() {
            "meeting".to_string()
        } else {
            new.mode
        };
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "INSERT INTO meetings (gid, title_ct, started_at, source, mode, lang, template, sensitive,
                                   dek_wrapped, audio_retained_until, lamport, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                gid,
                title_ct,
                started_at,
                source,
                mode,
                new.lang,
                new.template,
                new.sensitive,
                wrapped,
                new.audio_retained_until,
                lamport,
                now_ms(),
            ],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        self.deks().insert(id, dek);
        self.load_meeting(&conn, &gid)
    }

    pub fn get_meeting(&self, gid: &str) -> Result<Meeting> {
        self.load_meeting(&self.conn(), gid)
    }

    fn load_meeting(&self, conn: &Connection, gid: &str) -> Result<Meeting> {
        let (id, m) = conn
            .query_row(
                &format!("{MEETING_SELECT} WHERE gid = ?1"),
                [gid],
                meeting_from_row,
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "meeting",
                gid: gid.to_string(),
            })?;
        self.finish_meeting_row(conn, id, m)
    }

    fn finish_meeting_row(&self, conn: &Connection, id: i64, mut m: RawMeeting) -> Result<Meeting> {
        let dek = self.dek(conn, id)?;
        let title = match &m.title_ct {
            Some(ct) => open_text(&dek, ct, &row_aad("meetings", "title_ct", &m.gid))?,
            None => String::new(),
        };
        Ok(Meeting {
            gid: std::mem::take(&mut m.gid),
            title,
            started_at: m.started_at,
            duration_ms: m.duration_ms,
            source: m.source,
            mode: m.mode,
            lang: m.lang,
            template: m.template,
            status: m.status,
            privacy_state: m.privacy_state,
            cloud_locked: m.cloud_locked,
            sensitive: m.sensitive,
            consent_confirmed: m.consent_confirmed,
            cloud_used: m.cloud_used,
            transcript_version: m.transcript_version,
            audio_retained_until: m.audio_retained_until,
            source_app: m.source_app,
            folder_gid: m.folder_gid,
        })
    }

    /// Newest first. Meetings whose key was shredded are skipped.
    pub fn list_meetings(&self, limit: usize, offset: usize) -> Result<Vec<Meeting>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(&format!(
            "{MEETING_SELECT} WHERE dek_wrapped <> zeroblob(length(dek_wrapped))
             ORDER BY started_at DESC, id DESC LIMIT ?1 OFFSET ?2"
        ))?;
        let rows = stmt
            .query_map(params![limit as i64, offset as i64], meeting_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(id, m)| self.finish_meeting_row(&conn, id, m))
            .collect()
    }

    pub fn set_meeting_title(&self, gid: &str, title: &str) -> Result<()> {
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, gid)?;
        let dek = self.dek(&conn, m.id)?;
        let ct = seal_text(&dek, title, &row_aad("meetings", "title_ct", gid));
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "UPDATE meetings SET title_ct = ?1, lamport = ?2 WHERE id = ?3",
            params![ct, lamport, m.id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_meeting_status(&self, gid: &str, status: &str) -> Result<()> {
        self.update_meeting(gid, "status = ?1", status)
    }

    /// Records that the user confirmed everyone's consent to recording
    /// (the voiceprint and cloud gates read it).
    pub fn set_consent_confirmed(&self, gid: &str, confirmed: bool) -> Result<()> {
        self.update_meeting(gid, "consent_confirmed = ?1", i64::from(confirmed))
    }

    /// "Never send to cloud" for this meeting: every cloud request is refused.
    pub fn set_cloud_locked(&self, gid: &str, locked: bool) -> Result<()> {
        self.update_meeting(gid, "cloud_locked = ?1", i64::from(locked))
    }

    /// The notes template the meeting's notes are written with (`None`: default).
    pub fn set_meeting_template(&self, gid: &str, template: Option<&str>) -> Result<()> {
        self.update_meeting(gid, "template = ?1", template)
    }

    /// Records the final duration and marks the meeting `done`.
    pub fn finish_meeting(&self, gid: &str, duration_ms: i64) -> Result<()> {
        self.update_meeting(gid, "duration_ms = ?1, status = 'done'", duration_ms)
    }

    /// Raises the recorded duration to `duration_ms` (the final pass measures
    /// the audio; recovery of a meeting without lines left it short).
    pub fn extend_meeting_duration(&self, gid: &str, duration_ms: i64) -> Result<()> {
        self.update_meeting(gid, "duration_ms = MAX(duration_ms, ?1)", duration_ms)
    }

    /// Sets when the audio is deleted by [`Store::retention_sweep`] (`None` = keep).
    pub fn set_audio_retained_until(&self, gid: &str, until: Option<i64>) -> Result<()> {
        self.update_meeting(gid, "audio_retained_until = ?1", until)
    }

    fn update_meeting(&self, gid: &str, set: &str, value: impl rusqlite::ToSql) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        let n = tx.execute(
            &format!("UPDATE meetings SET {set}, lamport = ?2 WHERE gid = ?3"),
            params![value, lamport, gid],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound {
                kind: "meeting",
                gid: gid.to_string(),
            });
        }
        tx.commit()?;
        Ok(())
    }

    // ------------------------------------------------ persons & speakers

    /// Adds a person (named; not Me), or returns the one that already has this
    /// name (by [`crate::people::name_key`]). Production code gets persons by
    /// renaming speakers; this is for callers that hold only a name.
    pub fn add_person(&self, name: &str, color_slot: i64) -> Result<String> {
        let name = fold::nfc(name.trim());
        if name.is_empty() {
            return Err(StoreError::Invalid("a person needs a name".into()));
        }
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let key = crate::people::name_key(&name);
        let existing: Option<String> = tx
            .query_row(
                "SELECT gid FROM persons WHERE name_key = ?1 AND is_me = 0",
                [&key],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(gid) = existing {
            return Ok(gid);
        }
        let gid = new_gid();
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "INSERT INTO persons (gid, name, color_slot, lamport, created_at, name_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![gid, name, color_slot, lamport, now_ms(), key],
        )?;
        tx.commit()?;
        Ok(gid)
    }

    pub fn add_speaker(&self, meeting_gid: &str, new: NewSpeaker) -> Result<String> {
        let gid = new_gid();
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let name_ct = new
            .display_name
            .as_deref()
            .map(|n| seal_text(&dek, n, &row_aad("speakers", "display_name_ct", &gid)));
        let tx = conn.transaction()?;
        // A Me speaker is always linked to the Me person.
        let person_id = if new.is_me {
            Some(crate::people::me_id(&tx)?)
        } else {
            match &new.person_gid {
                Some(p) => Some(id_of(&tx, "persons", p)?),
                None => None,
            }
        };
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "INSERT INTO speakers (gid, meeting_id, label_idx, display_name_ct, person_id, color_slot, is_me, lamport)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![gid, m.id, new.label_idx, name_ct, person_id, new.color_slot, new.is_me, lamport],
        )?;
        tx.commit()?;
        Ok(gid)
    }

    /// Links a speaker to a person (or with `None` unlinks it). Me speakers
    /// and "not a person" speakers can't be linked, nothing links to Me this
    /// way (use [`Store::set_speaker_me`]), and a person left with no speaker
    /// and no voice profile is removed.
    pub fn set_speaker_person(&self, speaker_gid: &str, person_gid: Option<&str>) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let (sid, is_me, not_person, old): (i64, bool, bool, Option<i64>) = tx
            .query_row(
                "SELECT id, is_me, not_person, person_id FROM speakers WHERE gid = ?1",
                [speaker_gid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "speaker",
                gid: speaker_gid.to_string(),
            })?;
        if is_me {
            return Err(StoreError::Invalid(
                "a Me speaker stays linked to Me".into(),
            ));
        }
        let person_id = match person_gid {
            Some(p) => {
                let id = id_of(&tx, "persons", p)?;
                if not_person || id == crate::people::me_id(&tx)? {
                    return Err(StoreError::Invalid(
                        "this speaker can't be linked to that person".into(),
                    ));
                }
                Some(id)
            }
            None => None,
        };
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "UPDATE speakers SET person_id = ?1, lamport = ?2 WHERE id = ?3",
            params![person_id, lamport, sid],
        )?;
        if let Some(old) = old.filter(|o| Some(*o) != person_id) {
            crate::people::gc_persons(&tx, &[old])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn speakers(&self, meeting_gid: &str) -> Result<Vec<Speaker>> {
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let mut stmt = conn.prepare_cached(
            "SELECT s.gid, s.label_idx, s.display_name_ct, p.gid, s.color_slot, s.is_me,
                    s.not_person, t.gid, sp.gid, sp.name, s.suggest_score, sp.is_me
             FROM speakers s LEFT JOIN persons p ON p.id = s.person_id
             LEFT JOIN speakers t ON t.id = s.merged_into
             LEFT JOIN persons sp ON sp.id = s.suggest_person_id
             WHERE s.meeting_id = ?1 ORDER BY s.label_idx, s.id",
        )?;
        let rows = stmt
            .query_map([m.id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<Vec<u8>>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, bool>(5)?,
                    r.get::<_, bool>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                    r.get::<_, Option<String>>(9)?,
                    r.get::<_, Option<f64>>(10)?,
                    r.get::<_, Option<bool>>(11)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(
                |(
                    gid,
                    label_idx,
                    ct,
                    person_gid,
                    color_slot,
                    is_me,
                    not_person,
                    merged_into,
                    suggest_gid,
                    suggest_name,
                    suggest_score,
                    suggest_me,
                )| {
                    let display_name = ct
                        .map(|ct| {
                            open_text(&dek, &ct, &row_aad("speakers", "display_name_ct", &gid))
                        })
                        .transpose()?;
                    Ok(Speaker {
                        gid,
                        label_idx,
                        display_name,
                        person_gid,
                        color_slot,
                        is_me,
                        not_person,
                        merged_into,
                        suggestion: suggest_gid.map(|person_gid| SpeakerSuggestion {
                            person_gid,
                            person_name: suggest_name.unwrap_or_default(),
                            is_me: suggest_me.unwrap_or(false),
                            score: suggest_score.unwrap_or(0.0) as f32,
                        }),
                    })
                },
            )
            .collect()
    }

    /// The named speakers (not merged away) of each of `meeting_gids`, in
    /// label order, as `(name, color slot)`: the library's people column and
    /// filter, in one query.
    pub fn named_speakers(
        &self,
        meeting_gids: &[String],
    ) -> Result<HashMap<String, Vec<(String, i64)>>> {
        let conn = self.conn();
        let want =
            serde_json::to_string(meeting_gids).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let mut stmt = conn.prepare_cached(
            "SELECT m.id, m.gid, s.gid, s.display_name_ct, s.color_slot
             FROM speakers s JOIN meetings m ON m.id = s.meeting_id
             WHERE s.display_name_ct IS NOT NULL AND s.merged_into IS NULL
               AND m.gid IN (SELECT value FROM json_each(?1))
             ORDER BY m.id, s.label_idx, s.id",
        )?;
        let rows = stmt
            .query_map([want], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out: HashMap<String, Vec<(String, i64)>> = HashMap::new();
        for (id, meeting, gid, ct, slot) in rows {
            let dek = self.dek(&conn, id)?;
            let name = open_text(&dek, &ct, &row_aad("speakers", "display_name_ct", &gid))?;
            out.entry(meeting).or_default().push((name, slot));
        }
        Ok(out)
    }

    // ------------------------------------------------------------ tracks

    /// Registers a track and creates its audio bundle at
    /// `<dir>/bundles/<gid>/<kind>.ghb`. Feed the writer from a dedicated
    /// thread; it never touches the database. Finish with [`Store::finish_track`].
    pub fn open_track(&self, meeting_gid: &str, kind: TrackKind) -> Result<BundleWriter> {
        check_gid(meeting_gid)?;
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let tx = conn.transaction()?;
        let track_gid = new_gid();
        let inserted = tx.execute(
            "INSERT INTO tracks (gid, meeting_id, kind, page_count) VALUES (?1, ?2, ?3, 0)",
            params![track_gid, m.id, kind.as_str()],
        );
        match inserted {
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(StoreError::Invalid(format!(
                    "meeting already has a {} track",
                    kind.as_str()
                )));
            }
            other => {
                other?;
            }
        }
        let path = self.bundle_path(meeting_gid, kind)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let writer = BundleWriter::create(&path, &dek, &Store::bundle_aad(&track_gid))?;
        tx.commit()?;
        Ok(writer)
    }

    /// Finishes the bundle (final record + index + full sync) and records the page count.
    pub fn finish_track(
        &self,
        meeting_gid: &str,
        kind: TrackKind,
        mut writer: BundleWriter,
    ) -> Result<u32> {
        let pages = writer.finish()?;
        let conn = self.conn();
        conn.execute(
            "UPDATE tracks SET page_count = ?1 WHERE kind = ?2 AND meeting_id = (SELECT id FROM meetings WHERE gid = ?3)",
            params![pages, kind.as_str(), meeting_gid],
        )?;
        Ok(pages)
    }

    /// Opens a track's audio for playback. A bundle that has fewer pages
    /// than were recorded (truncated or swapped for an older file) is
    /// [`StoreError::Decrypt`].
    pub fn open_bundle(&self, meeting_gid: &str, kind: TrackKind) -> Result<BundleReader> {
        check_gid(meeting_gid)?;
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let (track_gid, recorded): (String, u32) = conn
            .query_row(
                "SELECT gid, page_count FROM tracks WHERE meeting_id = ?1 AND kind = ?2",
                params![m.id, kind.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "track",
                gid: format!("{meeting_gid}/{}", kind.as_str()),
            })?;
        let reader = BundleReader::open(
            &self.bundle_path(meeting_gid, kind)?,
            &dek,
            &Store::bundle_aad(&track_gid),
        )?;
        // `recorded` is 0 while a track is still open (or before crash recovery).
        if reader.page_count() < recorded {
            return Err(StoreError::Decrypt);
        }
        Ok(reader)
    }

    /// Kinds of the tracks that still have audio on disk.
    pub fn tracks(&self, meeting_gid: &str) -> Result<Vec<(TrackKind, u32)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT t.kind, t.page_count FROM tracks t JOIN meetings m ON m.id = t.meeting_id
             WHERE m.gid = ?1 ORDER BY t.id",
        )?;
        let rows = stmt.query_map([meeting_gid], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (k, n) = r?;
            let kind = match k.as_str() {
                "system" => TrackKind::System,
                "file" => TrackKind::File,
                _ => TrackKind::Mic,
            };
            out.push((kind, n));
        }
        Ok(out)
    }

    /// After a crash, tracks recorded with a still-open bundle have
    /// `page_count = 0`; cut them back to the last authentic page and record
    /// the count. Best effort: a bundle that can't be recovered is left alone.
    fn recover_interrupted_tracks(&self) {
        let conn = self.conn();
        let Ok(mut stmt) = conn.prepare(
            "SELECT t.id, t.kind, m.id, m.gid, t.gid FROM tracks t JOIN meetings m ON m.id = t.meeting_id
             WHERE t.page_count = 0",
        ) else {
            return;
        };
        let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        }) else {
            return;
        };
        let pending: Vec<_> = rows.filter_map(|r| r.ok()).collect();
        drop(stmt);
        for (track_id, kind, meeting_id, gid, track_gid) in pending {
            let kind = match kind.as_str() {
                "system" => TrackKind::System,
                "file" => TrackKind::File,
                _ => TrackKind::Mic,
            };
            let Ok(path) = self.bundle_path(&gid, kind) else {
                continue;
            };
            if !path.is_file() {
                continue;
            }
            let Ok(dek) = self.dek(&conn, meeting_id) else {
                continue;
            };
            if let Ok(rec) = bundle::recover(&path, &dek, &Store::bundle_aad(&track_gid)) {
                let _ = conn.execute(
                    "UPDATE tracks SET page_count = ?1 WHERE id = ?2",
                    params![rec.pages, track_id],
                );
            }
        }
    }

    // ---------------------------------------------------------- segments

    /// Adds a segment to the current transcript version.
    pub fn add_segment(&self, meeting_gid: &str, seg: NewSegment) -> Result<Segment> {
        let mut v = self.add_segments(meeting_gid, vec![seg])?;
        Ok(v.remove(0))
    }

    /// Adds many segments in one transaction (live ASR batches, imports).
    pub fn add_segments(&self, meeting_gid: &str, segs: Vec<NewSegment>) -> Result<Vec<Segment>> {
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let tx = conn.transaction()?;
        let out = insert_segments(&tx, &dek, m.id, m.version, &segs)?;
        tx.commit()?;
        Ok(out)
    }

    /// The final pass: atomically replaces the transcript with `segs` as a new
    /// version. Bumps `transcript_version`, writes tombstones for the old
    /// segments, removes their FTS rows and deletes them (word timings go with
    /// them). Citations are time anchors, so they still resolve against the new
    /// version. Returns the new version.
    pub fn replace_transcript(&self, meeting_gid: &str, segs: Vec<NewSegment>) -> Result<i64> {
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let new_version = m.version + 1;
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tombstones::write_where(
            &tx,
            "segment",
            "SELECT gid FROM segments WHERE meeting_id = ?1 AND version < ?2",
            params![m.id, new_version],
            lamport,
        )?;
        tx.execute(
            "DELETE FROM segments_fts WHERE rowid IN (SELECT id FROM segments WHERE meeting_id = ?1)",
            [m.id],
        )?;
        tx.execute("DELETE FROM segments WHERE meeting_id = ?1", [m.id])?;
        tx.execute(
            "UPDATE meetings SET transcript_version = ?1, lamport = ?2 WHERE id = ?3",
            params![new_version, lamport, m.id],
        )?;
        insert_segments(&tx, &dek, m.id, new_version, &segs)?;
        tx.commit()?;
        Ok(new_version)
    }

    /// Segments of the current transcript version, in time order.
    pub fn segments(&self, meeting_gid: &str) -> Result<Vec<Segment>> {
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        self.read_segments(&conn, &m, "", params![m.id, m.version])
    }

    /// Current-version segments of a meeting matching `extra_where` (an
    /// `AND ...` clause from this crate; `?1`, `?2` are meeting id and
    /// version), decrypted, in time order.
    pub(crate) fn read_segments(
        &self,
        conn: &Connection,
        m: &MeetingRef,
        extra_where: &str,
        args: impl rusqlite::Params,
    ) -> Result<Vec<Segment>> {
        let dek = self.dek(conn, m.id)?;
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT s.gid, s.version, sp.gid, s.t0_ms, s.t1_ms, s.text_ct, s.lang, s.confidence, s.edited,
                    s.overlap
             FROM segments s LEFT JOIN speakers sp ON sp.id = s.speaker_id
             WHERE s.meeting_id = ?1 AND s.version = ?2 {extra_where} ORDER BY s.t0_ms, s.id"
        ))?;
        let rows = stmt
            .query_map(args, |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, Vec<u8>>(5)?,
                    r.get::<_, Option<String>>(6)?,
                    r.get::<_, Option<f32>>(7)?,
                    r.get::<_, bool>(8)?,
                    r.get::<_, bool>(9)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(
                |(
                    gid,
                    version,
                    speaker_gid,
                    t0_ms,
                    t1_ms,
                    ct,
                    lang,
                    confidence,
                    edited,
                    overlap,
                )| {
                    let text = open_text(&dek, &ct, &row_aad("segments", "text_ct", &gid))?;
                    Ok(Segment {
                        gid,
                        version,
                        speaker_gid,
                        t0_ms,
                        t1_ms,
                        text,
                        lang,
                        confidence,
                        edited,
                        overlap,
                    })
                },
            )
            .collect()
    }

    /// Word timings of a segment, in order.
    /// Word timings of every current segment of a meeting, by segment gid
    /// (one query for the whole transcript).
    pub fn meeting_words(&self, meeting_gid: &str) -> Result<HashMap<String, Vec<Word>>> {
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let mut stmt = conn.prepare_cached(
            "SELECT s.gid, w.t0_ms, w.t1_ms, w.conf FROM words w JOIN segments s ON s.id = w.segment_id
             WHERE s.meeting_id = ?1 AND s.version = ?2 ORDER BY s.id, w.idx",
        )?;
        let mut out: HashMap<String, Vec<Word>> = HashMap::new();
        let rows = stmt.query_map(params![m.id, m.version], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Word {
                    t0_ms: r.get(1)?,
                    t1_ms: r.get(2)?,
                    conf: r.get(3)?,
                },
            ))
        })?;
        for row in rows {
            let (gid, w) = row?;
            out.entry(gid).or_default().push(w);
        }
        Ok(out)
    }

    /// The meeting an item belongs to (no decryption): for ownership checks.
    pub fn meeting_of(&self, item: Item, gid: &str) -> Result<String> {
        let table = match item {
            Item::Segment => "segments",
            Item::NoteBlock => "notes_blocks",
            Item::ActionItem => "action_items",
            Item::Speaker => "speakers",
        };
        self.conn()
            .query_row(
                &format!("SELECT m.gid FROM {table} t JOIN meetings m ON m.id = t.meeting_id WHERE t.gid = ?1"),
                [gid],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: table,
                gid: gid.to_string(),
            })
    }

    /// Deletes note blocks of one meeting in one transaction.
    pub fn delete_note_blocks(&self, note_gids: &[String]) -> Result<()> {
        let mut conn = self.conn();
        let ids: Vec<(i64, &String)> = note_gids
            .iter()
            .map(|g| id_of(&conn, "notes_blocks", g).map(|id| (id, g)))
            .collect::<Result<_>>()?;
        let tx = conn.transaction()?;
        let first = Store::alloc_lamport(&tx, ids.len().max(1) as i64)?;
        for (n, (id, gid)) in ids.iter().enumerate() {
            tombstones::write(&tx, gid, "note", first + n as i64)?;
            tx.execute("DELETE FROM notes_fts WHERE rowid = ?1", [id])?;
            tx.execute("DELETE FROM notes_blocks WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn segment_words(&self, segment_gid: &str) -> Result<Vec<Word>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT w.t0_ms, w.t1_ms, w.conf FROM words w JOIN segments s ON s.id = w.segment_id
             WHERE s.gid = ?1 ORDER BY w.idx",
        )?;
        let rows = stmt.query_map([segment_gid], |r| {
            Ok(Word {
                t0_ms: r.get(0)?,
                t1_ms: r.get(1)?,
                conf: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Edits a segment's text (marks it `edited`) and re-indexes it.
    pub fn update_segment_text(&self, segment_gid: &str, text: &str) -> Result<()> {
        let mut conn = self.conn();
        let (id, meeting_id): (i64, i64) = conn
            .query_row(
                "SELECT id, meeting_id FROM segments WHERE gid = ?1",
                [segment_gid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "segment",
                gid: segment_gid.to_string(),
            })?;
        let dek = self.dek(&conn, meeting_id)?;
        let text = fold::nfc(text);
        let ct = seal_text(&dek, &text, &row_aad("segments", "text_ct", segment_gid));
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "UPDATE segments SET text_ct = ?1, edited = 1, lamport = ?2 WHERE id = ?3",
            params![ct, lamport, id],
        )?;
        crate::embeddings::bump_index_gen(&tx, meeting_id)?;
        tx.execute("DELETE FROM segments_fts WHERE rowid = ?1", [id])?;
        tx.execute(
            "INSERT INTO segments_fts (rowid, text_norm) VALUES (?1, ?2)",
            params![id, fold::fold(&text)],
        )?;
        tx.commit()?;
        Ok(())
    }

    // ------------------------------------------------------------- notes

    pub fn add_note_block(&self, meeting_gid: &str, new: NewNoteBlock) -> Result<NoteBlock> {
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let tx = conn.transaction()?;
        let block = insert_note_block(&tx, &dek, m.id, new)?;
        tx.commit()?;
        Ok(block)
    }

    pub fn note_blocks(&self, meeting_gid: &str) -> Result<Vec<NoteBlock>> {
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let mut stmt = conn.prepare_cached(
            "SELECT gid, kind, provenance, body_ct, anchors_json, pinned FROM notes_blocks
             WHERE meeting_id = ?1 ORDER BY id",
        )?;
        let rows = stmt
            .query_map([m.id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, bool>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(gid, kind, prov, ct, anchors, pinned)| {
                let body = open_text(&dek, &ct, &row_aad("notes_blocks", "body_ct", &gid))?;
                let anchors = serde_json::from_str(&anchors)
                    .map_err(|e| StoreError::Invalid(e.to_string()))?;
                Ok(NoteBlock {
                    gid,
                    kind,
                    provenance: Provenance::parse(&prov),
                    body,
                    anchors,
                    pinned,
                })
            })
            .collect()
    }

    /// Edits a note body and re-indexes it. AI-written blocks become `ai_edited`.
    pub fn update_note_block(&self, note_gid: &str, body: &str) -> Result<()> {
        let mut conn = self.conn();
        let (id, meeting_id): (i64, i64) = conn
            .query_row(
                "SELECT id, meeting_id FROM notes_blocks WHERE gid = ?1",
                [note_gid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "note",
                gid: note_gid.to_string(),
            })?;
        let dek = self.dek(&conn, meeting_id)?;
        let body = fold::nfc(body);
        let ct = seal_text(&dek, &body, &row_aad("notes_blocks", "body_ct", note_gid));
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "UPDATE notes_blocks SET body_ct = ?1, lamport = ?2,
                    provenance = CASE provenance WHEN 'ai' THEN 'ai_edited' ELSE provenance END
             WHERE id = ?3",
            params![ct, lamport, id],
        )?;
        tx.execute("DELETE FROM notes_fts WHERE rowid = ?1", [id])?;
        let norm = fold::fold(&body);
        if !norm.is_empty() {
            tx.execute(
                "INSERT INTO notes_fts (rowid, body_norm) VALUES (?1, ?2)",
                params![id, norm],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_note_block(&self, note_gid: &str) -> Result<()> {
        let mut conn = self.conn();
        let id = id_of(&conn, "notes_blocks", note_gid)?;
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tombstones::write(&tx, note_gid, "note", lamport)?;
        tx.execute("DELETE FROM notes_fts WHERE rowid = ?1", [id])?;
        tx.execute("DELETE FROM notes_blocks WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    // ------------------------------------------------------ action items

    pub fn add_action_item(&self, meeting_gid: &str, new: NewActionItem) -> Result<ActionItem> {
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let tx = conn.transaction()?;
        let item = insert_action_item(&tx, &dek, m.id, new)?;
        tx.commit()?;
        Ok(item)
    }

    /// Edits an action item's text. AI-written items become `ai_edited`, so a
    /// regenerate keeps them.
    pub fn update_action_item_text(&self, action_gid: &str, text: &str) -> Result<()> {
        let mut conn = self.conn();
        let meeting_id: i64 = conn
            .query_row(
                "SELECT meeting_id FROM action_items WHERE gid = ?1",
                [action_gid],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "action item",
                gid: action_gid.to_string(),
            })?;
        let dek = self.dek(&conn, meeting_id)?;
        let text = fold::nfc(text);
        let ct = seal_text(&dek, &text, &row_aad("action_items", "text_ct", action_gid));
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "UPDATE action_items SET text_ct = ?1, lamport = ?2,
                    provenance = CASE provenance WHEN 'ai' THEN 'ai_edited' ELSE provenance END
             WHERE gid = ?3",
            params![ct, lamport, action_gid],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Regenerated notes (RT-7): in one transaction, removes the AI note
    /// blocks that aren't pinned and the AI action items that are neither
    /// done nor edited, then adds `blocks` and `actions` as AI-written.
    /// Everything the user wrote, pinned, edited or ticked off stays.
    pub fn replace_ai_notes(
        &self,
        meeting_gid: &str,
        blocks: Vec<NewNoteBlock>,
        actions: Vec<NewActionItem>,
    ) -> Result<ReplacedNotes> {
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let tx = conn.transaction()?;
        let mut out = ReplacedNotes::default();
        let old_blocks: Vec<(i64, String)> = tx
            .prepare(
                "SELECT id, gid FROM notes_blocks
                 WHERE meeting_id = ?1 AND provenance = 'ai' AND pinned = 0",
            )?
            .query_map([m.id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let old_actions: Vec<(i64, String)> = tx
            .prepare(
                "SELECT id, gid FROM action_items
                 WHERE meeting_id = ?1 AND provenance = 'ai' AND done = 0",
            )?
            .query_map([m.id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let total: i64 = tx.query_row(
            "SELECT (SELECT count(*) FROM notes_blocks WHERE meeting_id = ?1)
                  + (SELECT count(*) FROM action_items WHERE meeting_id = ?1)",
            [m.id],
            |r| r.get(0),
        )?;
        out.removed = old_blocks.len() + old_actions.len();
        out.kept = usize::try_from(total).unwrap_or(0) - out.removed;
        if out.removed > 0 {
            let first = Store::alloc_lamport(&tx, out.removed as i64)?;
            for (n, (id, gid)) in old_blocks.iter().enumerate() {
                tombstones::write(&tx, gid, "note", first + n as i64)?;
                tx.execute("DELETE FROM notes_fts WHERE rowid = ?1", [id])?;
                tx.execute("DELETE FROM notes_blocks WHERE id = ?1", [id])?;
            }
            for (n, (id, gid)) in old_actions.iter().enumerate() {
                tombstones::write(
                    &tx,
                    gid,
                    "action_item",
                    first + (old_blocks.len() + n) as i64,
                )?;
                tx.execute("DELETE FROM action_items WHERE id = ?1", [id])?;
            }
        }
        for mut b in blocks {
            b.provenance = Provenance::Ai;
            b.pinned = false;
            insert_note_block(&tx, &dek, m.id, b)?;
            out.added += 1;
        }
        for mut a in actions {
            a.provenance = Provenance::Ai;
            insert_action_item(&tx, &dek, m.id, a)?;
            out.added += 1;
        }
        tx.commit()?;
        Ok(out)
    }

    /// Records a cloud AI request in the audit log and marks the meeting as
    /// having used the cloud. Counts only; no content.
    pub fn record_cloud_request(
        &self,
        meeting_gid: &str,
        provider: &str,
        model: &str,
        tokens_in: u64,
        tokens_out: u64,
    ) -> Result<()> {
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO cloud_requests (meeting_id, provider, model, tokens_in, tokens_out, at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                m.id,
                provider,
                model,
                i64::try_from(tokens_in).unwrap_or(i64::MAX),
                i64::try_from(tokens_out).unwrap_or(i64::MAX),
                now_ms()
            ],
        )?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "UPDATE meetings SET cloud_used = 1, lamport = ?1 WHERE id = ?2",
            params![lamport, m.id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The cloud request log, newest first (the "request log" in Settings → AI).
    pub fn cloud_requests(&self, limit: usize) -> Result<Vec<CloudRequest>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT m.gid, c.provider, c.model, c.tokens_in, c.tokens_out, c.at
             FROM cloud_requests c JOIN meetings m ON m.id = c.meeting_id
             ORDER BY c.at DESC, c.id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit as i64], |r| {
                Ok(CloudRequest {
                    meeting_gid: r.get(0)?,
                    provider: r.get(1)?,
                    model: r.get(2)?,
                    tokens_in: r.get::<_, i64>(3)?.max(0) as u64,
                    tokens_out: r.get::<_, i64>(4)?.max(0) as u64,
                    at: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn action_items(&self, meeting_gid: &str) -> Result<Vec<ActionItem>> {
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let dek = self.dek(&conn, m.id)?;
        let mut stmt = conn.prepare_cached(
            "SELECT a.gid, a.text_ct, sp.gid, a.due, a.done, a.anchor_json,
                    a.due_text_ct, a.anchors_json, a.provenance
             FROM action_items a LEFT JOIN speakers sp ON sp.id = a.owner_speaker_id
             WHERE a.meeting_id = ?1 ORDER BY a.id",
        )?;
        let rows = stmt
            .query_map([m.id], |r| {
                Ok((
                    (
                        r.get::<_, String>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                        r.get::<_, bool>(4)?,
                    ),
                    (
                        r.get::<_, Option<String>>(5)?,
                        r.get::<_, Option<Vec<u8>>>(6)?,
                        r.get::<_, String>(7)?,
                        r.get::<_, String>(8)?,
                    ),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(
                |((gid, ct, owner_speaker_gid, due, done), (anchor, due_ct, anchors, prov))| {
                    let text = open_text(&dek, &ct, &row_aad("action_items", "text_ct", &gid))?;
                    let due_text = due_ct
                        .map(|c| open_text(&dek, &c, &row_aad("action_items", "due_text_ct", &gid)))
                        .transpose()?;
                    Ok(ActionItem {
                        text,
                        owner_speaker_gid,
                        due,
                        due_text,
                        done,
                        anchor: anchor.as_deref().map(from_json).transpose()?,
                        anchors: from_json(&anchors)?,
                        provenance: Provenance::parse(&prov),
                        gid,
                    })
                },
            )
            .collect()
    }

    pub fn set_action_done(&self, action_gid: &str, done: bool) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        let n = tx.execute(
            "UPDATE action_items SET done = ?1, lamport = ?2 WHERE gid = ?3",
            params![done, lamport, action_gid],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound {
                kind: "action item",
                gid: action_gid.to_string(),
            });
        }
        tx.commit()?;
        Ok(())
    }

    /// Sets who owns an action item (`None`: nobody). AI-written items become
    /// `ai_edited`, so a regenerate keeps them.
    pub fn set_action_owner(&self, action_gid: &str, speaker_gid: Option<&str>) -> Result<()> {
        let mut conn = self.conn();
        if let Some(sp) = speaker_gid {
            let same: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM speakers s JOIN action_items a
                     ON a.meeting_id = s.meeting_id WHERE s.gid = ?1 AND a.gid = ?2)",
                params![sp, action_gid],
                |r| r.get(0),
            )?;
            if !same {
                return Err(StoreError::NotFound {
                    kind: "speaker",
                    gid: sp.to_string(),
                });
            }
        }
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        let n = tx.execute(
            "UPDATE action_items SET owner_speaker_id =
                    (SELECT id FROM speakers WHERE gid = ?1
                       AND meeting_id = action_items.meeting_id),
                    lamport = ?2,
                    provenance = CASE provenance WHEN 'ai' THEN 'ai_edited' ELSE provenance END
             WHERE gid = ?3",
            params![speaker_gid, lamport, action_gid],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound {
                kind: "action item",
                gid: action_gid.to_string(),
            });
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_action_item(&self, action_gid: &str) -> Result<()> {
        let mut conn = self.conn();
        let id = id_of(&conn, "action_items", action_gid)?;
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tombstones::write(&tx, action_gid, "action_item", lamport)?;
        tx.execute("DELETE FROM action_items WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(())
    }

    // ------------------------------------------------------------- marks

    pub fn add_mark(&self, meeting_gid: &str, t_ms: i64, tag: MarkTag) -> Result<Mark> {
        let gid = new_gid();
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let tx = conn.transaction()?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "INSERT INTO marks (gid, meeting_id, t_ms, tag, lamport) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![gid, m.id, t_ms, tag.as_str(), lamport],
        )?;
        tx.commit()?;
        Ok(Mark { gid, t_ms, tag })
    }

    pub fn marks(&self, meeting_gid: &str) -> Result<Vec<Mark>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT k.gid, k.t_ms, k.tag FROM marks k JOIN meetings m ON m.id = k.meeting_id
             WHERE m.gid = ?1 ORDER BY k.t_ms, k.id",
        )?;
        let rows = stmt.query_map([meeting_gid], |r| {
            Ok(Mark {
                gid: r.get(0)?,
                t_ms: r.get(1)?,
                tag: MarkTag::parse(&r.get::<_, String>(2)?),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ------------------------------------------------------------ delete

    /// Deletes a meeting by crypto-shredding it. In order:
    ///
    /// 1. tombstones for the meeting and every child row (sync deletes);
    /// 2. `dek_wrapped` is overwritten with zeros (and committed) and the
    ///    cached DEK dropped: from here the meeting's audio, transcript, notes
    ///    and title are undecryptable even though ciphertext may linger;
    /// 3. the audio bundle files are deleted;
    /// 4. the rows and their FTS entries are deleted (`contentless_delete`);
    /// 5. pre-migration snapshots are deleted (they hold the old wrapped DEK);
    ///    both FTS indexes are `optimize`d so tokens of deleted rows are
    ///    rewritten away; `incremental_vacuum` returns the freed pages (zeroed
    ///    by `secure_delete`) to the OS; a truncating WAL checkpoint empties
    ///    `-wal`.
    ///
    /// A crash between 2 and 4 leaves a meeting with a zeroed key; the next
    /// [`Store::open`] finishes the delete.
    ///
    /// # Limits (by design, documented)
    ///
    /// - A copy of the database or bundle taken *before* the delete stays
    ///   readable for as long as the master key exists, because it still holds
    ///   the wrapped DEK. That is why the data directory is excluded from
    ///   OS backups by default. Deleting the meeting cannot reach copies.
    /// - The FTS indexes are keyed by SQLCipher, not by the per-meeting key,
    ///   and they use `detail=full`: they store every folded token *and its
    ///   position*, so a meeting's folded (accent-stripped, lowercase) text
    ///   can be reconstructed from the index until those entries are merged
    ///   away. `optimize` after each delete rewrites the index without the
    ///   deleted rows (and `secure_delete` zeroes the freed pages), but that
    ///   protection is only as strong as the SQLCipher key; it is not a
    ///   crypto-shred of the index.
    /// - On SSDs/APFS, freed blocks may survive; this is a crypto-shred,
    ///   not a physical wipe.
    pub fn delete_meeting(&self, gid: &str) -> Result<()> {
        let deleted = self.delete_meeting_before_rotation(gid);
        // The key is gone either way: rotate so no earlier copy can unwrap it.
        // `true`: if another delete's rotation finished meanwhile, this key
        // was re-wrapped under that rotation's new secret, so start another.
        let rotated = self.rotate_wraps(true);
        deleted.and(rotated)
    }

    /// [`Store::delete_meeting`] without its last step (finishing the
    /// wrap-secret rotation). Public only so tests can simulate a crash there.
    #[doc(hidden)]
    pub fn delete_meeting_before_rotation(&self, gid: &str) -> Result<()> {
        check_gid(gid)?;
        Store::meeting_ref(&self.conn(), gid)?;
        // Start the rotation first: a crash anywhere below leaves the ring
        // "rotating", and the next open finishes it.
        self.begin_wrap_rotation()?;
        {
            let mut conn = self.conn();
            let m = Store::meeting_ref(&conn, gid)?;
            let tx = conn.transaction()?;
            let lamport = Store::alloc_lamport(&tx, 1)?;
            tombstones::write_children(&tx, m.id, lamport)?;
            tombstones::write(&tx, gid, "meeting", lamport)?;
            tx.commit()?;
        }
        self.shred_key(gid)?;
        self.finish_delete(gid)
    }

    /// Step 2 of [`Store::delete_meeting`], on its own. Public only so tests
    /// can simulate a crash between the steps.
    #[doc(hidden)]
    pub fn shred_key(&self, gid: &str) -> Result<()> {
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, gid)?;
        conn.execute(
            "UPDATE meetings SET dek_wrapped = zeroblob(length(dek_wrapped)) WHERE id = ?1",
            [m.id],
        )?;
        self.deks().remove(&m.id);
        Ok(())
    }

    /// Simulates a crash in the middle of a rotation: saves the ring with a new
    /// wrap secret (old one kept) and stops. Tests only.
    #[doc(hidden)]
    pub fn begin_rotation_only(&self) -> Result<()> {
        let mut ring = self.ring();
        ring.begin_rotation();
        self.keystore.save(&ring, self.protection)
    }

    /// Steps 3-5 of [`Store::delete_meeting`] (files, rows, compaction).
    fn finish_delete(&self, gid: &str) -> Result<()> {
        remove_dir_if_exists(&self.bundle_dir(gid)?)?;
        let mut conn = self.conn();
        let m = Store::meeting_ref(&conn, gid)?;
        let tx = conn.transaction()?;
        // Persons met only in this meeting go with it (unless they hold a
        // voice profile): their names are not under the meeting key.
        let persons = crate::people::persons_of_meeting(&tx, m.id)?;
        tx.execute(
            "DELETE FROM segments_fts WHERE rowid IN (SELECT id FROM segments WHERE meeting_id = ?1)",
            [m.id],
        )?;
        tx.execute(
            "DELETE FROM notes_fts WHERE rowid IN (SELECT id FROM notes_blocks WHERE meeting_id = ?1)",
            [m.id],
        )?;
        tx.execute("DELETE FROM meetings WHERE id = ?1", [m.id])?;
        crate::people::gc_persons(&tx, &persons)?;
        tx.commit()?;
        migrate::purge_snapshots(&snapshots_dir(&self.dir))?;
        compact_locked(&conn)
    }

    /// Completes deletes interrupted after the key was shredded. Best effort
    /// (never fails an open): a delete that can't finish now is queued as a
    /// `finish_delete` job and retried on the next open.
    fn finish_pending_deletes(&self) {
        let pending: Vec<String> = {
            let conn = self.conn();
            let Ok(mut stmt) = conn.prepare(
                "SELECT gid FROM meetings WHERE dek_wrapped = zeroblob(length(dek_wrapped))",
            ) else {
                return;
            };
            let Ok(rows) = stmt.query_map([], |r| r.get(0)) else {
                return;
            };
            rows.filter_map(|r| r.ok()).collect()
        };
        let mut done = false;
        for gid in pending {
            match self.finish_delete(&gid) {
                Ok(()) => done = true,
                Err(_) => {
                    let queued = self
                        .jobs_for_meeting(&gid)
                        .map_or(true, |j| j.iter().any(|j| j.kind == FINISH_DELETE_JOB));
                    if !queued && check_gid(&gid).is_ok() {
                        let _ = self.enqueue_job(
                            Some(&gid),
                            FINISH_DELETE_JOB,
                            1,
                            &serde_json::json!({ "meeting": gid }),
                        );
                    }
                }
            }
        }
        if done {
            let _ = self.rotate_wraps(true);
        }
    }

    /// Rotates the DEK wrap secret and re-wraps every remaining meeting's key.
    ///
    /// Why: a copy of the database taken before a delete holds the deleted
    /// meeting's DEK wrapped by the old secret. Once the old secret is
    /// destroyed, that copy can't unwrap it (or any other DEK in the copy).
    ///
    /// Order (crash-safe; both secrets work while the ring is "rotating"):
    /// begin (new secret, old kept) -> save ring -> re-wrap all in ONE
    /// transaction -> finish (old secret dropped) -> save ring -> rewrite
    /// `recovery.bin` if a phrase is set -> truncate the WAL. With
    /// `begin = false` it resumes a rotation an earlier crash left open.
    ///
    /// `recovery.bin` is rewritten with the rotating ring (both secrets)
    /// before the re-wrap, so a phrase restore works at every step.
    pub(crate) fn rotate_wraps(&self, begin: bool) -> Result<()> {
        if begin {
            self.begin_wrap_rotation()?;
        }
        let mut conn = self.conn();
        let mut ring = self.ring();
        if !ring.is_rotating() {
            return Ok(());
        }
        self.write_recovery_file(&ring)?;
        let rows: Vec<(i64, String, Vec<u8>)> = {
            let mut stmt = conn.prepare(
                "SELECT id, gid, dek_wrapped FROM meetings
                 WHERE dek_wrapped <> zeroblob(length(dek_wrapped))",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let voice_rows: Vec<(i64, String, Vec<u8>)> = {
            let mut stmt = conn.prepare(
                "SELECT id, gid, key_wrapped FROM voice_profiles
                 WHERE key_wrapped <> zeroblob(length(key_wrapped))",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let tx = conn.transaction()?;
        {
            let mut upd = tx.prepare("UPDATE meetings SET dek_wrapped = ?1 WHERE id = ?2")?;
            let mut upd_voice =
                tx.prepare("UPDATE voice_profiles SET key_wrapped = ?1 WHERE id = ?2")?;
            let total = rows.len() + voice_rows.len();
            let mut rewrapped = 0;
            for (id, gid, wrapped) in voice_rows {
                // Same rule as for meeting keys below.
                let Ok(key) = ring.unwrap_voice_key(&wrapped, &gid) else {
                    continue;
                };
                upd_voice.execute(params![ring.wrap_voice_key(&key, &gid), id])?;
                rewrapped += 1;
            }
            for (id, gid, wrapped) in rows {
                // A key neither secret opens was wrapped by a secret destroyed
                // earlier: it is already unreadable, and must not block the
                // rotation (and every later delete) forever.
                let Ok(dek) = ring.unwrap_dek(&wrapped, &gid) else {
                    continue;
                };
                upd.execute(params![ring.wrap_dek(&dek, &gid), id])?;
                rewrapped += 1;
            }
            // But if NO key opens, this is the wrong ring (e.g. a stale
            // keystore item): finishing would destroy the right secret.
            if total > 0 && rewrapped == 0 {
                return Err(StoreError::Decrypt);
            }
        }
        tx.commit()?;
        ring.finish_rotation();
        self.keystore.save(&ring, self.protection)?;
        self.write_recovery_file(&ring)?;
        db::checkpoint(&conn)
    }

    /// Starts a wrap-secret rotation (new secret, old one kept) and saves the
    /// ring. No-op when one is already open.
    pub(crate) fn begin_wrap_rotation(&self) -> Result<()> {
        let mut ring = self.ring();
        if ring.is_rotating() {
            return Ok(());
        }
        let before = ring.clone();
        ring.begin_rotation();
        if let Err(e) = self.keystore.save(&ring, self.protection) {
            *ring = before;
            return Err(e);
        }
        // Meetings created from now on use the new secret: recovery.bin must
        // have it too.
        self.write_recovery_file(&ring)
    }

    /// Rewrites `recovery.bin` with `ring` when a recovery phrase is set and
    /// the file exists. Never creates it: only [`Store::set_recovery_phrase`]
    /// does, so a phrase that failed to set, or one `delete_all` removed,
    /// can't come back.
    fn write_recovery_file(&self, ring: &KeyRing) -> Result<()> {
        let path = self.dir.join(RECOVERY_FILE);
        match ring.recovery_key() {
            Some(key) if path.is_file() => {
                write_atomic(&path, &recovery::wrap_ring_with_key(ring, key))
            }
            _ => Ok(()),
        }
    }

    /// Rewrites the FTS indexes without deleted rows, vacuums and truncates
    /// the WAL (the last step of every delete; also usable when idle).
    pub fn compact_index(&self) -> Result<()> {
        compact_locked(&self.conn())
    }

    // --------------------------------------------------- export & delete all

    /// "Export everything": a password-encrypted archive (Argon2id +
    /// XChaCha20-Poly1305) of a consistent database copy (SQLite backup API),
    /// every audio bundle and the key ring. The ring is streamed from memory
    /// into the archive; it is never written to disk unencrypted. So the
    /// archive alone restores on a new device.
    ///
    /// The archive's security is exactly the password's: anyone with the file
    /// and the password gets the key ring and all data.
    pub fn export_all(&self, out: &Path, password: &str) -> Result<()> {
        let staging = self.dir.join(format!(".export-{}", new_gid()));
        fs::create_dir_all(&staging)?;
        let _guard = RemoveOnDrop(staging.clone());
        let db_copy = staging.join(DB_FILE);
        let ring_bytes = {
            let conn = self.conn();
            let ring = self.ring();
            db::backup_to(&conn, &db_copy, &ring.db_key())?;
            ring.to_bytes()
        };
        let mut entries = vec![
            export::Entry::file(DB_FILE, db_copy),
            export::Entry::bytes(RING_ENTRY, ring_bytes.to_vec()),
        ];
        let mut files = Vec::new();
        collect_files(&self.dir.join("bundles"), "bundles", &mut files)?;
        entries.extend(files.into_iter().map(|(n, p)| export::Entry::file(n, p)));
        export::export_entries(&entries, out, password)
    }

    /// Restores an [`export_all`](Store::export_all) archive into `dest` (which
    /// must be missing or empty), saves its key ring in `keystore` and opens it.
    ///
    /// Refuses when `keystore` already holds a key (that would orphan the data
    /// on this device). The archive is unpacked into a staging directory and
    /// checked (every meeting gid must be a canonical UUID); the ring is saved
    /// to the keystore first, and only then are the files moved into place, so
    /// a failure at any step leaves `dest` untouched.
    pub fn import_archive(
        archive: &Path,
        password: &str,
        dest: &Path,
        keystore: Arc<dyn KeyStore>,
        protection: Protection,
    ) -> Result<Store> {
        if keystore.load()?.is_some() {
            return Err(StoreError::Invalid(
                "this device already has a Ghira key; import into a fresh install or delete everything first"
                    .into(),
            ));
        }
        if dest.exists() && fs::read_dir(dest)?.next().is_some() {
            return Err(StoreError::Invalid(
                "the destination directory is not empty".into(),
            ));
        }
        let parent = dest
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        // Leftovers of a crashed import (an hour old: another import may be
        // running next to this one).
        let _ = export::clean_stale_imports(parent, std::time::Duration::from_secs(3600));
        let staging = parent.join(format!(".ghi-import-{}", new_gid()));
        let _guard = RemoveOnDrop(staging.clone());
        let imported = export::import_archive_entries(archive, password, &staging)?;
        let ring = KeyRing::from_bytes(
            imported
                .entry(RING_ENTRY)
                .ok_or_else(|| StoreError::Invalid("archive has no key ring".into()))?,
        )?;
        check_imported_gids(&staging, &ring)?;
        if let Err(e) = keystore.save(&ring, protection) {
            // The keystore was empty: don't leave a partial save that would
            // refuse every later import.
            let _ = keystore.delete();
            return Err(e);
        }
        let moved = (|| -> Result<()> {
            if dest.exists() {
                fs::remove_dir(dest)?;
            }
            fs::rename(&staging, dest)?;
            Ok(())
        })();
        if let Err(e) = moved {
            let _ = keystore.delete();
            return Err(e);
        }
        Store::open(dest, keystore, protection)
    }

    /// "Delete everything": shreds every meeting key, closes and removes the
    /// database, bundles, snapshots and recovery file, then **rotates the
    /// whole key ring**: the keystore item is deleted and a brand-new ring
    /// saved (with `protection`). The old recovery phrase is dead:
    /// `recovery.bin` is gone and even a copy of it unwraps a ring that no
    /// longer opens anything. The data directory itself is left empty.
    ///
    /// No tombstones are written (the database is gone); phase 15 sync needs
    /// its own "wiped" signal.
    pub fn delete_all(self, keystore: &dyn KeyStore, protection: Protection) -> Result<()> {
        {
            let conn = self.conn();
            conn.execute_batch(
                "UPDATE meetings SET dek_wrapped = zeroblob(length(dek_wrapped));
                 UPDATE voice_profiles SET key_wrapped = zeroblob(length(key_wrapped));",
            )?;
            db::checkpoint(&conn)?;
        }
        let Store { dir, conn, .. } = self;
        let conn = conn.into_inner().unwrap_or_else(|p| p.into_inner());
        let _ = conn.close();
        for name in [
            DB_FILE.to_string(),
            format!("{DB_FILE}-wal"),
            format!("{DB_FILE}-shm"),
            RECOVERY_FILE.to_string(),
        ] {
            remove_file_if_exists(&dir.join(name))?;
        }
        remove_dir_if_exists(&dir.join("bundles"))?;
        remove_dir_if_exists(&snapshots_dir(&dir))?;
        keystore.delete()?;
        keystore.save(&KeyRing::generate(), protection)
    }
}

// ------------------------------------------------------------------ helpers

/// FTS `optimize` on both indexes, then vacuum and truncate the WAL.
fn insert_note_block(
    tx: &Transaction,
    dek: &Dek,
    meeting_id: i64,
    new: NewNoteBlock,
) -> Result<NoteBlock> {
    let gid = new_gid();
    let body = fold::nfc(&new.body);
    let ct = seal_text(dek, &body, &row_aad("notes_blocks", "body_ct", &gid));
    let anchors_json = to_json(&new.anchors)?;
    let lamport = Store::alloc_lamport(tx, 1)?;
    tx.execute(
        "INSERT INTO notes_blocks (gid, meeting_id, kind, provenance, body_ct, anchors_json, pinned, lamport)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![gid, meeting_id, new.kind, new.provenance.as_str(), ct, anchors_json, new.pinned, lamport],
    )?;
    let id = tx.last_insert_rowid();
    let norm = fold::fold(&body);
    if !norm.is_empty() {
        tx.execute(
            "INSERT INTO notes_fts (rowid, body_norm) VALUES (?1, ?2)",
            params![id, norm],
        )?;
    }
    Ok(NoteBlock {
        gid,
        kind: new.kind,
        provenance: new.provenance,
        body,
        anchors: new.anchors,
        pinned: new.pinned,
    })
}

fn insert_action_item(
    tx: &Transaction,
    dek: &Dek,
    meeting_id: i64,
    new: NewActionItem,
) -> Result<ActionItem> {
    let gid = new_gid();
    let text = fold::nfc(&new.text);
    let ct = seal_text(dek, &text, &row_aad("action_items", "text_ct", &gid));
    let due_text = new.due_text.as_deref().map(fold::nfc);
    let due_ct = due_text
        .as_deref()
        .map(|d| seal_text(dek, d, &row_aad("action_items", "due_text_ct", &gid)));
    let anchor_json = new.anchor.as_ref().map(to_json).transpose()?;
    let anchors_json = to_json(&new.anchors)?;
    let owner = match &new.owner_speaker_gid {
        Some(s) => Some(id_of(tx, "speakers", s)?),
        None => None,
    };
    let lamport = Store::alloc_lamport(tx, 1)?;
    tx.execute(
        "INSERT INTO action_items (gid, meeting_id, text_ct, owner_speaker_id, due, anchor_json,
                                   lamport, provenance, due_text_ct, anchors_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            gid,
            meeting_id,
            ct,
            owner,
            new.due,
            anchor_json,
            lamport,
            new.provenance.as_str(),
            due_ct,
            anchors_json
        ],
    )?;
    Ok(ActionItem {
        gid,
        text,
        owner_speaker_gid: new.owner_speaker_gid,
        due: new.due,
        due_text,
        done: false,
        anchor: new.anchor,
        anchors: new.anchors,
        provenance: new.provenance,
    })
}

fn to_json<T: serde::Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|e| StoreError::Invalid(e.to_string()))
}

fn from_json<T: serde::de::DeserializeOwned>(s: &str) -> Result<T> {
    serde_json::from_str(s).map_err(|e| StoreError::Invalid(e.to_string()))
}

pub(crate) fn compact_locked(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT INTO segments_fts (segments_fts) VALUES ('optimize')",
        [],
    )?;
    conn.execute("INSERT INTO notes_fts (notes_fts) VALUES ('optimize')", [])?;
    db::incremental_vacuum(conn)?;
    db::checkpoint(conn)
}

pub(crate) fn snapshots_dir(dir: &Path) -> PathBuf {
    dir.join("snapshots")
}

const MEETING_SELECT: &str =
    "SELECT id, gid, title_ct, started_at, duration_ms, source, mode, lang, template, status,
        privacy_state, cloud_locked, sensitive, consent_confirmed, cloud_used, transcript_version,
        audio_retained_until, source_app,
        (SELECT f.gid FROM folders f WHERE f.id = meetings.folder_id) FROM meetings";

struct RawMeeting {
    gid: String,
    title_ct: Option<Vec<u8>>,
    started_at: i64,
    duration_ms: i64,
    source: String,
    mode: String,
    lang: Option<String>,
    template: Option<String>,
    status: String,
    privacy_state: String,
    cloud_locked: bool,
    sensitive: bool,
    consent_confirmed: bool,
    cloud_used: bool,
    transcript_version: i64,
    audio_retained_until: Option<i64>,
    source_app: Option<String>,
    folder_gid: Option<String>,
}

fn meeting_from_row(r: &rusqlite::Row) -> rusqlite::Result<(i64, RawMeeting)> {
    Ok((
        r.get(0)?,
        RawMeeting {
            gid: r.get(1)?,
            title_ct: r.get(2)?,
            started_at: r.get(3)?,
            duration_ms: r.get(4)?,
            source: r.get(5)?,
            mode: r.get(6)?,
            lang: r.get(7)?,
            template: r.get(8)?,
            status: r.get(9)?,
            privacy_state: r.get(10)?,
            cloud_locked: r.get(11)?,
            sensitive: r.get(12)?,
            consent_confirmed: r.get(13)?,
            cloud_used: r.get(14)?,
            transcript_version: r.get(15)?,
            audio_retained_until: r.get(16)?,
            source_app: r.get(17)?,
            folder_gid: r.get(18)?,
        },
    ))
}

/// Rowid of `gid` in `table` (the table name is a literal from this crate).
pub(crate) fn id_of(conn: &Connection, table: &'static str, gid: &str) -> Result<i64> {
    conn.query_row(
        &format!("SELECT id FROM {table} WHERE gid = ?1"),
        [gid],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| StoreError::NotFound {
        kind: table,
        gid: gid.to_string(),
    })
}

/// Inserts segments (+ words + FTS rows) for `meeting_id` at `version`.
pub(crate) fn insert_segments(
    tx: &rusqlite::Transaction,
    dek: &Dek,
    meeting_id: i64,
    version: i64,
    segs: &[NewSegment],
) -> Result<Vec<Segment>> {
    let mut out = Vec::with_capacity(segs.len());
    if segs.is_empty() {
        return Ok(out);
    }
    let first_lamport = Store::alloc_lamport(tx, segs.len() as i64)?;
    crate::embeddings::bump_index_gen(tx, meeting_id)?;
    let mut speakers: HashMap<String, i64> = HashMap::new();
    let mut ins = tx.prepare_cached(
        "INSERT INTO segments (gid, meeting_id, version, speaker_id, t0_ms, t1_ms, text_ct, lang, confidence, lamport, edited)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )?;
    let mut fts =
        tx.prepare_cached("INSERT INTO segments_fts (rowid, text_norm) VALUES (?1, ?2)")?;
    let mut word = tx.prepare_cached(
        "INSERT INTO words (segment_id, idx, t0_ms, t1_ms, conf) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for (i, s) in segs.iter().enumerate() {
        let speaker_id = match &s.speaker_gid {
            Some(g) => Some(match speakers.get(g) {
                Some(id) => *id,
                None => {
                    let id: i64 = tx
                        .query_row(
                            "SELECT id FROM speakers WHERE gid = ?1 AND meeting_id = ?2",
                            params![g, meeting_id],
                            |r| r.get(0),
                        )
                        .optional()?
                        .ok_or_else(|| StoreError::NotFound {
                            kind: "speaker",
                            gid: g.clone(),
                        })?;
                    speakers.insert(g.clone(), id);
                    id
                }
            }),
            None => None,
        };
        let gid = match &s.gid {
            Some(g) => {
                check_gid(g)?;
                g.clone()
            }
            None => new_gid(),
        };
        let text = fold::nfc(&s.text);
        let ct = seal_text(dek, &text, &row_aad("segments", "text_ct", &gid));
        ins.execute(params![
            gid,
            meeting_id,
            version,
            speaker_id,
            s.t0_ms,
            s.t1_ms,
            ct,
            s.lang,
            s.confidence,
            first_lamport + i as i64,
            s.edited
        ])?;
        let id = tx.last_insert_rowid();
        let norm = fold::fold(&text);
        if !norm.is_empty() {
            fts.execute(params![id, norm])?;
        }
        for (idx, w) in s.words.iter().enumerate() {
            word.execute(params![id, idx as i64, w.t0_ms, w.t1_ms, w.conf])?;
        }
        out.push(Segment {
            gid,
            version,
            speaker_gid: s.speaker_gid.clone(),
            t0_ms: s.t0_ms,
            t1_ms: s.t1_ms,
            text,
            lang: s.lang.clone(),
            confidence: s.confidence,
            edited: false,
            overlap: false,
        });
    }
    Ok(out)
}

/// Writes `bytes` to `path` via a temp file + rename, owner-only on unix.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::File::open(&tmp)?.sync_all()?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn remove_dir_if_exists(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// A gid that becomes a path component must be a canonical UUID.
pub(crate) fn check_gid(gid: &str) -> Result<()> {
    if gid.len() == 36 && uuid::Uuid::parse_str(gid).is_ok() {
        Ok(())
    } else {
        Err(StoreError::Invalid("malformed id".into()))
    }
}

/// Takes the exclusive `<dir>/.lock` for the store's lifetime.
fn acquire_lock(dir: &Path) -> Result<File> {
    let f = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(".lock"))?;
    match f.try_lock() {
        Ok(()) => Ok(f),
        Err(fs::TryLockError::WouldBlock) => Err(StoreError::Invalid(
            "store is in use by another process".into(),
        )),
        Err(fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

/// After an import: every meeting gid in the database and every bundle
/// directory name must be a canonical UUID (they become paths).
fn check_imported_gids(staging: &Path, ring: &KeyRing) -> Result<()> {
    let conn = db::open(&staging.join(DB_FILE), &ring.db_key())?;
    let mut stmt = conn.prepare("SELECT gid FROM meetings")?;
    let gids: Vec<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for g in &gids {
        check_gid(g)?;
    }
    drop(stmt);
    let _ = conn.close();
    if let Ok(rd) = fs::read_dir(staging.join("bundles")) {
        for e in rd {
            check_gid(&e?.file_name().to_string_lossy())?;
        }
    }
    Ok(())
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Lists regular files under `root` as `(prefix/relative/name, path)`.
fn collect_files(root: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
    let rd = match fs::read_dir(root) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let mut entries: Vec<_> = rd.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = format!("{prefix}/{}", e.file_name().to_string_lossy());
        let path = e.path();
        if e.file_type()?.is_dir() {
            collect_files(&path, &name, out)?;
        } else {
            out.push((name, path));
        }
    }
    Ok(())
}
