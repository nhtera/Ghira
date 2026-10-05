// SPDX-License-Identifier: Apache-2.0
//! Applying a peer's batch (slice 15-C2, doc 07 §7.3-§7.5).
//!
//! One transaction per batch: tombstones first and absorbing, then rows;
//! every `_ct` is opened with its AAD before anything commits, and any
//! failure rejects the whole batch with no partial write
//! ([`StoreError::BadRecord`]). Rows whose parent is missing are parked
//! ([`super::pending`]).
//!
//! Who is who: `from_device` is the sender's gid. If the sender is a *hub*,
//! this device is a spoke: pulled rows are applied to clean rows only (a dirty
//! row is kept for the next push) and `base` follows the hub's version. Any
//! other sender is a spoke pushing to this hub: concurrent edits are merged
//! here by `(lamport, origin gid)`.
//!
//! `origin` columns hold `devices.id` of the device that wrote a version
//! (NULL = this device). A writer this device is not paired with (a row
//! relayed by the hub) gets a `devices` row in state `known` (no key, never
//! connects), so versions always compare as `(lamport, origin device gid)`.

use std::collections::BTreeSet;

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::observe_lamport;
use super::records::{
    ActionItemRec, Bytes, ConflictCopyRec, MarkRec, MeetingRec, MeetingTagRec, NoteRec, PersonRec,
    Record, RecordKind, SegmentRec, SettingRec, SpeakerRec, SyncTombstone, TombCause, TrackRec,
    Version, origin_gid, own_gid,
};
use super::rules::{self, Fence};
use crate::rowcrypt::{Dek, open_text, row_aad, seal_text};
use crate::store::{Store, now_ms};
use crate::tombstones::{self, Cause};
use crate::{Result, StoreError, fold, new_gid};

/// Records in one batch (doc 07 §5.3).
pub const MAX_BATCH_RECORDS: usize = 256;
/// Bytes of record content in one batch.
pub const MAX_BATCH_BYTES: usize = 1 << 20;
/// One text field, plaintext.
pub const MAX_TEXT: usize = 256 * 1024;
/// Short strings (names, kinds, keys).
const MAX_SHORT: usize = 4096;
/// A sealed text: the plaintext limit plus format byte, nonce and tag.
const MAX_CT: usize = MAX_TEXT + 64;
const MAX_WORDS: usize = 100_000;

/// What happened to one gid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplyOutcome {
    Accepted,
    /// Concurrent: merged by the field rules (a conflict copy may exist).
    Merged,
    /// The gid (or its meeting) is tombstoned here; dropped.
    Tombstoned,
    /// The parent is missing; held in `sync_pending`.
    Parked,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ApplyResult {
    /// One entry per applied record, in order.
    pub results: Vec<(String, ApplyOutcome)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TombResult {
    /// Gids that were deleted by this batch (the rest were already gone).
    pub applied: Vec<String>,
    /// Gids refused (malformed or unknown kind), reported in the ack.
    pub rejected: Vec<String>,
    /// Meeting tombstones that need the user's confirmation first (D13). This
    /// engine never holds one back: the confirmation policy sits above it.
    pub needs_confirm: usize,
}

impl Store {
    /// Applies tombstones from `from_device` (a device gid).
    pub fn apply_tombs(&self, from_device: &str, tombs: &[SyncTombstone]) -> Result<TombResult> {
        let (res, post) = {
            let mut conn = self.conn();
            let tx = conn.transaction()?;
            let mut ctx = Ctx::new(self, &tx, Some(from_device))?;
            let mut res = TombResult::default();
            for t in tombs {
                match ctx.tombstone(t)? {
                    Tomb::Applied => res.applied.push(t.gid.clone()),
                    Tomb::Present | Tomb::Skipped => {}
                    Tomb::Rejected => res.rejected.push(t.gid.clone()),
                }
            }
            // Parked rows of a meeting that just died are dropped here.
            ctx.retry_parked()?;
            ctx.finish()?;
            let post = ctx.into_post();
            tx.commit()?;
            (res, post)
        };
        self.finish_post(post)?;
        Ok(res)
    }

    /// Applies rows from `from_device`. Meeting records may carry a DEK.
    pub fn apply_rows(&self, from_device: &str, rows: &[Record]) -> Result<ApplyResult> {
        validate_batch(rows)?;
        let (res, post) = {
            let mut conn = self.conn();
            let tx = conn.transaction()?;
            let mut ctx = Ctx::new(self, &tx, Some(from_device))?;
            let mut res = ApplyResult::default();
            for rec in rows {
                let outcome = ctx.apply_one(rec)?;
                res.results.push((rec.gid().to_string(), outcome));
            }
            ctx.retry_parked()?;
            ctx.expire_pending()?;
            ctx.finish()?;
            let post = ctx.into_post();
            tx.commit()?;
            (res, post)
        };
        self.finish_post(post)?;
        Ok(res)
    }

    /// Spoke, after an ack: the row is clean again if it is still the
    /// version that was pushed (`base := (lamport, origin)`).
    pub fn mark_clean(&self, gid: &str, lamport: i64) -> Result<()> {
        let conn = self.conn();
        for (table, _) in crate::migrate::SYNC_TABLES {
            let n = conn.execute(
                &format!(
                    "UPDATE {table} SET base_lamport = lamport, base_origin = origin
                     WHERE gid = ?1 AND lamport = ?2"
                ),
                rusqlite::params![gid, lamport],
            )?;
            if n > 0 {
                break;
            }
        }
        Ok(())
    }

    /// Work that must happen after a batch committed: shredding (rotates the
    /// wrap secret), bundle files, settings, and finishing deletes a crash
    /// interrupted.
    pub(crate) fn finish_post(&self, post: Post) -> Result<()> {
        for (meeting, kind) in &post.files {
            self.remove_track_files(meeting, kind)?;
        }
        for (meeting, kind, keep) in &post.truncate {
            self.cut_track_bundle(meeting, kind, *keep)?;
        }
        for rec in &post.settings {
            match self.apply_synced(rec) {
                // A value of the wrong shape is dropped: one bad setting must
                // not wedge the session that carries the rest.
                Ok(()) | Err(StoreError::NotYet(_) | StoreError::Invalid(_)) => {}
                Err(e) => return Err(e),
            }
        }
        self.sweep_tombstoned()
    }

    /// Shreds every meeting (and voice profile) that has a tombstone but is
    /// still stored: a tombstone commits before its shred, and a crash can
    /// land between them.
    pub(crate) fn sweep_tombstoned(&self) -> Result<()> {
        let (meetings, profiles): (Vec<String>, Vec<String>) = {
            let conn = self.conn();
            let q = |sql: &str| -> Result<Vec<String>> {
                Ok(conn
                    .prepare(sql)?
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?)
            };
            (
                q("SELECT gid FROM meetings WHERE gid IN (SELECT gid FROM tombstones)")?,
                q("SELECT gid FROM voice_profiles WHERE gid IN (SELECT gid FROM tombstones)")?,
            )
        };
        for gid in meetings {
            self.delete_meeting_local(&gid)?;
        }
        for gid in profiles {
            match self.delete_voice_profile(&gid) {
                Ok(()) | Err(StoreError::NotFound { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn remove_track_files(&self, meeting_gid: &str, kind: &str) -> Result<()> {
        let path = match kind {
            "mic" => self.bundle_path(meeting_gid, crate::store::TrackKind::Mic)?,
            "system" => self.bundle_path(meeting_gid, crate::store::TrackKind::System)?,
            "file" => self.bundle_path(meeting_gid, crate::store::TrackKind::File)?,
            _ => return Ok(()),
        };
        for p in [
            crate::bundle::index_path(&path),
            crate::bundle::part_path(&path),
            path,
        ] {
            match std::fs::remove_file(p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    /// A smaller `cut_pages` arrived: cut the local bundle to it (re-sealed
    /// under this device's own prefix). Nothing happens if there is no bundle
    /// yet or it is already short enough.
    fn cut_track_bundle(&self, meeting_gid: &str, kind: &str, keep: i64) -> Result<()> {
        let tk = match kind {
            "mic" => crate::store::TrackKind::Mic,
            "system" => crate::store::TrackKind::System,
            "file" => crate::store::TrackKind::File,
            _ => return Ok(()),
        };
        let path = self.bundle_path(meeting_gid, tk)?;
        if !path.exists() {
            return Ok(());
        }
        let keep = u32::try_from(keep.max(0)).unwrap_or(u32::MAX);
        let (dek, track_gid) = {
            let conn = self.conn();
            let m = Store::meeting_ref(&conn, meeting_gid)?;
            let track_gid: String = conn.query_row(
                "SELECT gid FROM tracks WHERE meeting_id = ?1 AND kind = ?2",
                rusqlite::params![m.id, kind],
                |r| r.get(0),
            )?;
            (self.dek(&conn, m.id)?, track_gid)
        };
        let pages = crate::bundle::BundleReader::open(&path, &dek, &Store::bundle_aad(&track_gid))?
            .page_count();
        if pages > keep {
            // A local act: the version it bumps is ours (the `sync_origin_*`
            // trigger sets `origin`).
            self.truncate_track(meeting_gid, tk, keep)?;
        }
        Ok(())
    }
}

// --------------------------------------------------------------- validation

fn bad<T>(gid: &str, why: &'static str) -> Result<T> {
    Err(StoreError::BadRecord {
        gid: gid.to_string(),
        why,
    })
}

fn canonical(gid: &str) -> bool {
    gid.len() == 36 && uuid::Uuid::parse_str(gid).is_ok()
}

fn short(gid: &str, s: &str) -> Result<()> {
    if s.len() > MAX_SHORT {
        return bad(gid, "text too long");
    }
    Ok(())
}

fn short_opt(gid: &str, s: &Option<String>) -> Result<()> {
    match s {
        Some(s) => short(gid, s),
        None => Ok(()),
    }
}

fn ct_ok(gid: &str, ct: &Option<Bytes>) -> Result<()> {
    match ct {
        Some(b) if b.0.len() > MAX_CT => bad(gid, "text too long"),
        _ => Ok(()),
    }
}

fn text_ok(gid: &str, s: &Option<String>) -> Result<()> {
    match s {
        Some(s) if s.len() > MAX_TEXT => bad(gid, "text too long"),
        _ => Ok(()),
    }
}

fn ref_ok(gid: &str, r: &Option<String>) -> Result<()> {
    match r {
        Some(g) if !canonical(g) => bad(gid, "malformed reference"),
        _ => Ok(()),
    }
}

fn version_ok(gid: &str, v: &Version) -> Result<()> {
    if v.lamport < 0 || v.origin.is_empty() || v.origin.len() > 64 {
        return bad(gid, "malformed version");
    }
    Ok(())
}

/// Content bytes of a record, for the batch size limit.
fn weight(rec: &Record) -> usize {
    let b = |x: &Option<Bytes>| x.as_ref().map_or(0, |b| b.0.len());
    let s = |x: &Option<String>| x.as_ref().map_or(0, String::len);
    256 + match rec {
        Record::Meeting(r) => b(&r.title_ct) + b(&r.calendar_ct) + b(&r.track_speakers_ct),
        Record::Segment(r) => b(&r.text_ct) + r.words.as_ref().map_or(0, |w| w.len() * 24),
        Record::Note(r) => b(&r.body_ct) + s(&r.anchors_json),
        Record::ActionItem(r) => b(&r.text_ct) + b(&r.due_text_ct) + s(&r.anchors_json),
        Record::Speaker(r) => b(&r.display_name_ct),
        Record::ConflictCopy(r) => b(&r.value_ct),
        Record::Setting(r) => s(&r.value_json),
        _ => 0,
    }
}

/// Static checks of one record (no database).
fn validate(rec: &Record) -> Result<()> {
    let gid = rec.gid();
    if !canonical(gid) {
        return bad(gid, "malformed id");
    }
    version_ok(gid, rec.version())?;
    let base = match rec {
        Record::Meeting(r) => &r.base,
        Record::Track(r) => &r.base,
        Record::Person(r) => &r.base,
        Record::Speaker(r) => &r.base,
        Record::Segment(r) => &r.base,
        Record::Note(r) => &r.base,
        Record::ActionItem(r) => &r.base,
        Record::Mark(r) => &r.base,
        Record::Folder(r) => &r.base,
        Record::Tag(r) => &r.base,
        Record::MeetingTag(r) => &r.base,
        Record::VoiceProfile(r) => &r.base,
        Record::ConflictCopy(r) => &r.base,
        Record::Setting(r) => &r.base,
    };
    if let Some(b) = base {
        version_ok(gid, b)?;
    }
    if let Some(m) = rec.meeting_gid()
        && !canonical(m)
    {
        return bad(gid, "malformed reference");
    }
    match rec {
        Record::Meeting(r) => {
            ct_ok(gid, &r.title_ct)?;
            ct_ok(gid, &r.calendar_ct)?;
            ct_ok(gid, &r.track_speakers_ct)?;
            for s in [
                &r.source,
                &r.mode,
                &r.lang,
                &r.template,
                &r.status,
                &r.privacy_state,
                &r.audio_origin,
                &r.source_hash,
                &r.source_app,
            ] {
                short_opt(gid, s)?;
            }
            ref_ok(gid, &r.folder_gid)?;
            if let Some(s) = &r.status
                && rules::status_rank(s).is_none()
            {
                return bad(gid, "unknown status");
            }
            if let Some(d) = &r.dek
                && d.0.len() != 32
            {
                return bad(gid, "malformed key");
            }
        }
        Record::Track(r) => {
            short_opt(gid, &r.kind)?;
            if let Some(k) = &r.kind
                && !matches!(k.as_str(), "mic" | "system" | "file")
            {
                return bad(gid, "unknown track kind");
            }
        }
        Record::Person(r) => short_opt(gid, &r.name)?,
        Record::Speaker(r) => {
            ct_ok(gid, &r.display_name_ct)?;
            ref_ok(gid, &r.person_gid)?;
            ref_ok(gid, &r.merged_into)?;
        }
        Record::Segment(r) => {
            ct_ok(gid, &r.text_ct)?;
            short_opt(gid, &r.lang)?;
            ref_ok(gid, &r.speaker_gid)?;
            if r.words.as_ref().is_some_and(|w| w.len() > MAX_WORDS) {
                return bad(gid, "too many words");
            }
        }
        Record::Note(r) => {
            ct_ok(gid, &r.body_ct)?;
            text_ok(gid, &r.anchors_json)?;
            short_opt(gid, &r.kind)?;
            short_opt(gid, &r.ord)?;
            if let Some(p) = &r.provenance
                && !matches!(p.as_str(), "user" | "ai" | "ai_edited")
            {
                return bad(gid, "unknown provenance");
            }
            json_ok(gid, &r.anchors_json)?;
        }
        Record::ActionItem(r) => {
            ct_ok(gid, &r.text_ct)?;
            ct_ok(gid, &r.due_text_ct)?;
            text_ok(gid, &r.anchors_json)?;
            short_opt(gid, &r.ord)?;
            ref_ok(gid, &r.owner_speaker_gid)?;
            if let Some(p) = &r.provenance
                && !matches!(p.as_str(), "user" | "ai" | "ai_edited")
            {
                return bad(gid, "unknown provenance");
            }
            json_ok(gid, &r.anchors_json)?;
        }
        Record::Mark(r) => {
            if let Some(t) = &r.tag
                && !matches!(t.as_str(), "star" | "decision" | "action" | "question")
            {
                return bad(gid, "unknown mark tag");
            }
        }
        Record::Folder(r) => short_opt(gid, &r.name)?,
        Record::Tag(r) => short_opt(gid, &r.name)?,
        Record::MeetingTag(r) => {
            if !canonical(&r.tag_gid) {
                return bad(gid, "malformed reference");
            }
        }
        Record::VoiceProfile(_) => {}
        Record::ConflictCopy(r) => {
            ct_ok(gid, &r.value_ct)?;
            if !canonical(&r.target_gid) || !copy_target_ok(&r.target_kind, &r.field) {
                return bad(gid, "malformed conflict copy");
            }
        }
        Record::Setting(r) => {
            short(gid, &r.key)?;
            text_ok(gid, &r.value_json)?;
            if r.key.is_empty() {
                return bad(gid, "empty setting key");
            }
            json_ok(gid, &r.value_json)?;
        }
    }
    Ok(())
}

fn json_ok(gid: &str, s: &Option<String>) -> Result<()> {
    match s {
        Some(s) if serde_json::from_str::<serde_json::Value>(s).is_err() => {
            bad(gid, "malformed json")
        }
        _ => Ok(()),
    }
}

/// The free-text fields a conflict copy may point at.
pub(crate) fn copy_target_ok(kind: &str, field: &str) -> bool {
    matches!(
        (kind, field),
        ("meeting", "title_ct")
            | ("speaker", "display_name_ct")
            | ("segment", "text_ct")
            | ("note", "body_ct")
            | ("action_item", "text_ct")
            | ("action_item", "due_text_ct")
    )
}

fn validate_batch(rows: &[Record]) -> Result<()> {
    if rows.len() > MAX_BATCH_RECORDS {
        return bad(rows[MAX_BATCH_RECORDS].gid(), "batch too large");
    }
    let mut bytes = 0usize;
    for r in rows {
        validate(r)?;
        bytes += weight(r);
        if bytes > MAX_BATCH_BYTES {
            return bad(r.gid(), "batch too large");
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ context

/// Row values to write: `(column, value)` pairs.
#[derive(Default)]
pub(crate) struct Set {
    cols: Vec<&'static str>,
    vals: Vec<Value>,
}

impl Set {
    fn put(&mut self, col: &'static str, v: Value) {
        self.cols.push(col);
        self.vals.push(v);
    }
    fn int(&mut self, col: &'static str, v: i64) {
        self.put(col, Value::Integer(v));
    }
    fn flag(&mut self, col: &'static str, v: bool) {
        self.put(col, Value::Integer(i64::from(v)));
    }
    fn text(&mut self, col: &'static str, v: &str) {
        self.put(col, Value::Text(v.to_string()));
    }
    fn blob(&mut self, col: &'static str, v: &Bytes) {
        self.put(col, Value::Blob(v.0.clone()));
    }
    fn null_or_int(&mut self, col: &'static str, v: Option<i64>) {
        self.put(col, v.map_or(Value::Null, Value::Integer));
    }
    fn opt_int(&mut self, col: &'static str, v: Option<i64>) {
        if let Some(v) = v {
            self.int(col, v);
        }
    }
    fn opt_flag(&mut self, col: &'static str, v: Option<bool>) {
        if let Some(v) = v {
            self.flag(col, v);
        }
    }
    fn opt_text(&mut self, col: &'static str, v: &Option<String>) {
        if let Some(v) = v {
            self.text(col, v);
        }
    }
    fn opt_blob(&mut self, col: &'static str, v: &Option<Bytes>) {
        if let Some(v) = v {
            self.blob(col, v);
        }
    }
    fn is_empty(&self) -> bool {
        self.cols.is_empty()
    }
}

/// A foreign key in a record.
#[derive(Clone, Copy)]
pub(crate) enum Ref {
    /// The record says nothing: keep the column.
    Keep,
    /// The target is gone (tombstoned) or the record names none.
    Clear,
    Id(i64),
}

impl Ref {
    fn set(self, s: &mut Set, col: &'static str) {
        match self {
            Ref::Keep => {}
            Ref::Clear => s.put(col, Value::Null),
            Ref::Id(id) => s.int(col, id),
        }
    }
}

/// The version a row has, with `origin` as its column holds it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Ver {
    pub lamport: i64,
    pub origin: Option<i64>,
}

/// What a stored row looks like to the version rules.
#[derive(Clone, Copy)]
pub(crate) struct Meta {
    pub id: i64,
    pub lamport: i64,
    pub origin: Option<i64>,
    pub base_lamport: Option<i64>,
    pub base_origin: Option<i64>,
}

impl Meta {
    fn ver(&self) -> Ver {
        Ver {
            lamport: self.lamport,
            origin: self.origin,
        }
    }
    /// Spoke: changed here since the hub's version was last applied.
    pub(crate) fn dirty(&self) -> bool {
        self.base_lamport != Some(self.lamport) || self.base_origin != self.origin
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    Insert,
    /// Same version already here (or an older one from the same writer).
    NoOp,
    /// A successor of what is stored: take the record.
    Take,
    /// Both sides changed the row: the higher version wins.
    Concurrent {
        rec_wins: bool,
    },
    /// Spoke: the row has local changes; keep them for the next push.
    KeepLocal,
}

/// What one record handler decided.
pub(crate) enum Step {
    Done(ApplyOutcome),
    /// Hold the record until this gid exists (or is tombstoned).
    Park(String),
}

pub(crate) enum Flow<T> {
    Go(T),
    Stop(Step),
}

macro_rules! go {
    ($e:expr) => {
        match $e {
            Flow::Go(v) => v,
            Flow::Stop(s) => return Ok(s),
        }
    };
}

/// `settings` key prefix of "this folder/tag was folded into that one".
/// Device-local (not a synced setting); it lets late records that name the
/// dead gid land on the survivor.
const REDIRECT: &str = "sync.redirect.";

/// A meeting as its children see it.
#[derive(Clone, Copy)]
struct MInfo {
    id: i64,
    transcript: (i64, i64),
    ai_epoch: i64,
}

/// Follow-up work for after the commit.
#[derive(Default)]
pub(crate) struct Post {
    /// `(meeting gid, track kind)` bundles to delete.
    files: Vec<(String, String)>,
    /// `(meeting gid, track kind, pages to keep)`.
    truncate: Vec<(String, String, i64)>,
    settings: Vec<SettingRec>,
}

pub(crate) enum Tomb {
    Applied,
    /// Already tombstoned here.
    Present,
    /// A tombstone this device does not act on (kept out of the ack).
    Skipped,
    Rejected,
}

pub(crate) struct Ctx<'a> {
    pub(crate) store: &'a Store,
    pub(crate) conn: &'a Connection,
    pub(crate) sender_id: i64,
    pub(crate) sender_gid: String,
    /// The sender is the hub: this device is a spoke.
    pub(crate) spoke: bool,
    pub(crate) own: String,
    /// Keys of meetings inserted by this batch (not committed, so never put
    /// in the store's cache).
    new_deks: std::collections::HashMap<i64, Dek>,
    gids: std::collections::HashMap<Option<i64>, String>,
    touched: BTreeSet<i64>,
    post: Post,
}

impl<'a> Ctx<'a> {
    pub(crate) fn new(store: &'a Store, conn: &'a Connection, from: Option<&str>) -> Result<Self> {
        let own = own_gid(conn)?;
        // Tells the `sync_origin_*` triggers that `lamport` and `origin` are
        // written together here (the flag goes with the transaction).
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value_json) VALUES ('sync.applying', '1')",
            [],
        )?;
        let mut ctx = Ctx {
            store,
            conn,
            sender_id: 0,
            sender_gid: String::new(),
            spoke: false,
            own,
            new_deks: Default::default(),
            gids: Default::default(),
            touched: BTreeSet::new(),
            post: Post::default(),
        };
        if let Some(from) = from {
            ctx.set_sender(from)?;
        }
        Ok(ctx)
    }

    /// Takes `from` (a device gid) as the sender: unknown devices are refused.
    pub(crate) fn set_sender(&mut self, from: &str) -> Result<()> {
        let row: Option<(i64, String)> = self
            .conn
            .query_row("SELECT id, role FROM devices WHERE gid = ?1", [from], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let Some((id, role)) = row else {
            return Err(StoreError::NotFound {
                kind: "device",
                gid: from.to_string(),
            });
        };
        self.sender_id = id;
        self.sender_gid = from.to_string();
        self.spoke = role == "hub";
        Ok(())
    }

    pub(crate) fn into_post(self) -> Post {
        self.post
    }

    /// Bumps `index_gen` of every meeting whose indexed text changed.
    pub(crate) fn finish(&mut self) -> Result<()> {
        for id in std::mem::take(&mut self.touched) {
            crate::embeddings::bump_index_gen(self.conn, id)?;
        }
        self.conn
            .execute("DELETE FROM settings WHERE key = 'sync.applying'", [])?;
        Ok(())
    }

    fn dek_of(&self, meeting_id: i64) -> Result<Dek> {
        if let Some(d) = self.new_deks.get(&meeting_id) {
            return Ok(d.clone());
        }
        self.store.dek(self.conn, meeting_id)
    }

    /// Opens a sealed text of a meeting's row, refusing the batch if it does
    /// not authenticate under that meeting's key and the row's AAD.
    fn open(&self, dek: &Dek, table: &str, col: &str, gid: &str, ct: &Bytes) -> Result<String> {
        let text = match open_text(dek, &ct.0, &row_aad(table, col, gid)) {
            Ok(t) => t,
            Err(_) => return bad(gid, "ciphertext does not open"),
        };
        if text.len() > MAX_TEXT {
            return bad(gid, "text too long");
        }
        Ok(text)
    }

    fn tombstoned(&self, gid: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM tombstones WHERE gid = ?1)",
            [gid],
            |r| r.get(0),
        )?)
    }

    /// The `origin` column for a version written by the device `gid`; a
    /// device with no row yet gets one in state `known`.
    fn norm_origin(&self, gid: &str) -> Result<Option<i64>> {
        if gid == self.own {
            return Ok(None);
        }
        let id: Option<i64> = self
            .conn
            .query_row("SELECT id FROM devices WHERE gid = ?1", [gid], |r| r.get(0))
            .optional()?;
        if let Some(id) = id {
            return Ok(Some(id));
        }
        self.conn.execute(
            "INSERT INTO devices (gid, name, platform, role, state, paired_at)
             VALUES (?1, '', '', 'spoke', 'known', 0)",
            [gid],
        )?;
        Ok(Some(self.conn.last_insert_rowid()))
    }

    fn ver(&self, v: &Version) -> Result<Ver> {
        Ok(Ver {
            lamport: v.lamport,
            origin: self.norm_origin(&v.origin)?,
        })
    }

    pub(crate) fn gid_of(&mut self, origin: Option<i64>) -> Result<String> {
        if let Some(g) = self.gids.get(&origin) {
            return Ok(g.clone());
        }
        let g = origin_gid(self.conn, origin, &self.own)?;
        self.gids.insert(origin, g.clone());
        Ok(g)
    }

    fn meta(&self, table: &str, gid: &str) -> Result<Option<Meta>> {
        self.meta_where(table, "gid", gid)
    }

    fn meta_where(&self, table: &str, col: &str, val: &str) -> Result<Option<Meta>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT id, lamport, origin, base_lamport, base_origin
                     FROM {table} WHERE {col} = ?1"
                ),
                [val],
                |r| {
                    Ok(Meta {
                        id: r.get(0)?,
                        lamport: r.get(1)?,
                        origin: r.get(2)?,
                        base_lamport: r.get(3)?,
                        base_origin: r.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// The version rule of doc 07 §7.3.
    fn decide(&mut self, cur: Option<&Meta>, v: Ver, base: Option<&Version>) -> Result<Verdict> {
        let Some(c) = cur else {
            return Ok(Verdict::Insert);
        };
        if c.ver() == v {
            return Ok(Verdict::NoOp);
        }
        if self.spoke {
            return Ok(if c.dirty() {
                Verdict::KeepLocal
            } else {
                Verdict::Take
            });
        }
        if c.origin == v.origin {
            // One device's writes are in order: a later one replaces, a
            // redelivered earlier one changes nothing.
            return Ok(if v.lamport > c.lamport {
                Verdict::Take
            } else {
                Verdict::NoOp
            });
        }
        let cur_gid = self.gid_of(c.origin)?;
        if let Some(b) = base
            && b.lamport == c.lamport
            && b.origin == cur_gid
        {
            return Ok(Verdict::Take);
        }
        let rec_gid = self.gid_of(v.origin)?;
        Ok(Verdict::Concurrent {
            rec_wins: (v.lamport, rec_gid) > (c.lamport, cur_gid),
        })
    }

    /// `lamport`, `origin` and `base_*` of a row that takes `v`.
    fn stamp(&self, set: &mut Set, v: Ver) {
        set.int("lamport", v.lamport);
        set.null_or_int("origin", v.origin);
        if self.spoke {
            set.int("base_lamport", v.lamport);
            set.null_or_int("base_origin", v.origin);
        } else {
            set.put("base_lamport", Value::Null);
            set.put("base_origin", Value::Null);
        }
    }

    fn insert(&self, table: &str, set: &Set) -> Result<i64> {
        let marks = vec!["?"; set.cols.len()].join(", ");
        self.conn.execute(
            &format!(
                "INSERT INTO {table} ({}) VALUES ({marks})",
                set.cols.join(", ")
            ),
            rusqlite::params_from_iter(set.vals.iter()),
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn update(&self, table: &str, id: i64, set: &Set) -> Result<()> {
        if set.is_empty() {
            return Ok(());
        }
        let assigns: Vec<String> = set.cols.iter().map(|c| format!("{c} = ?")).collect();
        let mut vals: Vec<&Value> = set.vals.iter().collect();
        let id_val = Value::Integer(id);
        vals.push(&id_val);
        self.conn.execute(
            &format!("UPDATE {table} SET {} WHERE id = ?", assigns.join(", ")),
            rusqlite::params_from_iter(vals),
        )?;
        Ok(())
    }

    // ------------------------------------------------------------ one record

    /// Applies one record (doc 07 §7.3 `apply_push`).
    pub(crate) fn apply_one(&mut self, rec: &Record) -> Result<ApplyOutcome> {
        let gid = rec.gid();
        observe_lamport(self.conn, rec.version().lamport)?;
        if self.tombstoned(gid)? {
            self.unpark(gid)?;
            return Ok(ApplyOutcome::Tombstoned);
        }
        if let Some(m) = rec.meeting_gid()
            && self.tombstoned(m)?
        {
            self.unpark(gid)?;
            return Ok(ApplyOutcome::Tombstoned);
        }
        // Whatever waited under this gid is superseded by this record (a
        // handler may park a stub of its own under it).
        self.unpark(gid)?;
        let step = match rec {
            Record::Meeting(r) => self.meeting(r)?,
            Record::Track(r) => self.track(r)?,
            Record::Person(r) => self.person(r)?,
            Record::Speaker(r) => self.speaker(r)?,
            Record::Segment(r) => self.segment(r)?,
            Record::Note(r) => self.note(r)?,
            Record::ActionItem(r) => self.action(r)?,
            Record::Mark(r) => self.mark(r)?,
            Record::Folder(r) => self.named(
                Named::Folder,
                &r.gid,
                &r.version,
                &r.base,
                &r.name,
                r.created_at,
            )?,
            Record::Tag(r) => self.named(
                Named::Tag,
                &r.gid,
                &r.version,
                &r.base,
                &r.name,
                r.created_at,
            )?,
            Record::MeetingTag(r) => self.link(r)?,
            // Voice profiles do not sync in v1; the kind number is reserved.
            Record::VoiceProfile(_) => Step::Done(ApplyOutcome::Tombstoned),
            Record::ConflictCopy(r) => self.copy_row(r)?,
            Record::Setting(r) => self.setting(r)?,
        };
        match step {
            Step::Done(o) => Ok(o),
            Step::Park(parent) => {
                self.park(rec, &parent)?;
                Ok(ApplyOutcome::Parked)
            }
        }
    }

    /// The meeting a child row belongs to.
    fn meeting_of(&self, mgid: &str) -> Result<Flow<MInfo>> {
        let row: Option<MInfo> = self
            .conn
            .query_row(
                "SELECT id, transcript_version, transcript_epoch, ai_epoch
                 FROM meetings WHERE gid = ?1",
                [mgid],
                |r| {
                    Ok(MInfo {
                        id: r.get(0)?,
                        transcript: (r.get(1)?, r.get(2)?),
                        ai_epoch: r.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(match row {
            Some(m) => Flow::Go(m),
            None if self.tombstoned(mgid)? => Flow::Stop(Step::Done(ApplyOutcome::Tombstoned)),
            None => Flow::Stop(Step::Park(mgid.to_string())),
        })
    }

    /// A reference to a row of `table` (speaker, person): gone targets clear
    /// the column, unknown ones park the record.
    fn reference(&self, table: &str, gid: Option<&str>, meeting: Option<i64>) -> Result<Flow<Ref>> {
        let Some(gid) = gid else {
            return Ok(Flow::Go(Ref::Keep));
        };
        let row: Option<(i64, Option<i64>)> = if meeting.is_some() {
            self.conn
                .query_row(
                    &format!("SELECT id, meeting_id FROM {table} WHERE gid = ?1"),
                    [gid],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?
        } else {
            self.conn
                .query_row(
                    &format!("SELECT id, NULL FROM {table} WHERE gid = ?1"),
                    [gid],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?
        };
        Ok(match row {
            Some((id, m)) => {
                if meeting.is_some() && m != meeting {
                    return bad(gid, "reference to another meeting");
                }
                Flow::Go(Ref::Id(id))
            }
            None if self.tombstoned(gid)? => Flow::Go(Ref::Clear),
            None => Flow::Stop(Step::Park(gid.to_string())),
        })
    }

    // --------------------------------------------------------------- meeting

    fn meeting(&mut self, rec: &MeetingRec) -> Result<Step> {
        let gid = &rec.gid;
        #[allow(clippy::type_complexity)]
        let cur: Option<(Meta, MeetingCur)> = self
            .conn
            .query_row(
                "SELECT id, lamport, origin, base_lamport, base_origin, title_ct, started_at,
                        source, mode, created_at, source_hash, audio_origin, status, duration_ms,
                        cloud_used, consent_confirmed, transcript_version, transcript_epoch,
                        ai_epoch, folder_id
                 FROM meetings WHERE gid = ?1",
                [gid],
                |r| {
                    Ok((
                        Meta {
                            id: r.get(0)?,
                            lamport: r.get(1)?,
                            origin: r.get(2)?,
                            base_lamport: r.get(3)?,
                            base_origin: r.get(4)?,
                        },
                        MeetingCur {
                            title_ct: r.get(5)?,
                            started_at: r.get(6)?,
                            source: r.get(7)?,
                            mode: r.get(8)?,
                            created_at: r.get(9)?,
                            source_hash: r.get(10)?,
                            audio_origin: r.get(11)?,
                            status: r.get(12)?,
                            duration_ms: r.get(13)?,
                            cloud_used: r.get(14)?,
                            consent: r.get(15)?,
                            tv: r.get(16)?,
                            te: r.get(17)?,
                            ai: r.get(18)?,
                            folder_id: r.get(19)?,
                        },
                    ))
                },
            )
            .optional()?;

        // The key: new meetings need it, known ones must agree with it.
        let sent: Option<Zeroizing<[u8; 32]>> = match &rec.dek {
            Some(d) => Some(Zeroizing::new(d.0.as_slice().try_into().map_err(|_| {
                StoreError::BadRecord {
                    gid: gid.clone(),
                    why: "malformed key",
                }
            })?)),
            None => None,
        };
        let mut wrapped_new: Option<Vec<u8>> = None;
        let dek = match (&cur, &sent) {
            (Some((m, _)), sent) => {
                let have = match self.dek_of(m.id) {
                    Ok(d) => d,
                    // Shredded: its delete is in progress.
                    Err(StoreError::Decrypt) => return Ok(Step::Done(ApplyOutcome::Tombstoned)),
                    Err(e) => return Err(e),
                };
                if let Some(k) = sent {
                    match self.store.incoming_dek_wrapped(self.conn, gid, k) {
                        Ok(_) => {}
                        Err(StoreError::Invalid(_)) => {
                            return bad(gid, "a different key for a known meeting");
                        }
                        Err(StoreError::Tombstoned { .. }) => {
                            return Ok(Step::Done(ApplyOutcome::Tombstoned));
                        }
                        Err(e) => return Err(e),
                    }
                }
                have
            }
            (None, Some(k)) => match self.store.incoming_dek_wrapped(self.conn, gid, k) {
                Ok(Some(w)) => {
                    wrapped_new = Some(w);
                    Dek::from_bytes(**k)
                }
                Ok(None) => return bad(gid, "meeting key out of step"),
                Err(StoreError::Tombstoned { .. }) => {
                    return Ok(Step::Done(ApplyOutcome::Tombstoned));
                }
                Err(e) => return Err(e),
            },
            (None, None) => return bad(gid, "new meeting without a key"),
        };

        let title = match &rec.title_ct {
            Some(ct) => Some(self.open(&dek, "meetings", "title_ct", gid, ct)?),
            None => None,
        };
        for (col, ct) in [
            ("calendar_ct", &rec.calendar_ct),
            ("track_speakers_ct", &rec.track_speakers_ct),
        ] {
            if let Some(ct) = ct {
                self.open(&dek, "meetings", col, gid, ct)?;
            }
        }
        let audio_origin = match &rec.audio_origin {
            Some(g) => Some(self.norm_origin(g)?),
            None => None,
        };

        // Immutable fields never change after create.
        if let Some((_, c)) = &cur {
            let differs = rec.started_at.is_some_and(|v| v != c.started_at)
                || rec.source.as_ref().is_some_and(|v| *v != c.source)
                || rec.mode.as_ref().is_some_and(|v| *v != c.mode)
                || matches!((rec.created_at, c.created_at), (Some(a), Some(b)) if a != b)
                || matches!((&rec.source_hash, &c.source_hash), (Some(a), Some(b)) if a != b)
                || matches!((audio_origin, c.audio_origin), (Some(a), Some(b)) if a != Some(b));
            if differs {
                return bad(gid, "immutable field changed");
            }
        }

        // Folder: carried as a gid.
        let folder = match &rec.folder_gid {
            None => Ref::Keep,
            Some(fg) => {
                let id: Option<i64> = self
                    .conn
                    .query_row("SELECT id FROM folders WHERE gid = ?1", [fg], |r| r.get(0))
                    .optional()?;
                match id {
                    Some(id) => Ref::Id(id),
                    None if self.tombstoned(fg)? => {
                        let target: Option<i64> = match self.redirect(fg)? {
                            Some(t) => self
                                .conn
                                .query_row("SELECT id FROM folders WHERE gid = ?1", [t], |r| {
                                    r.get(0)
                                })
                                .optional()?,
                            None => None,
                        };
                        target.map_or(Ref::Clear, Ref::Id)
                    }
                    None => Ref::Keep,
                }
            }
        };
        let folder_missing = match (&rec.folder_gid, folder) {
            (Some(fg), Ref::Keep) => Some(fg.clone()),
            _ => None,
        };

        let v = self.ver(&rec.version)?;
        let verdict = self.decide(cur.as_ref().map(|c| &c.0), v, rec.base.as_ref())?;
        let mut merged = matches!(verdict, Verdict::Concurrent { .. } | Verdict::KeepLocal);
        let wrote;
        let id;

        match verdict {
            Verdict::Insert => {
                let Some(started_at) = rec.started_at else {
                    return bad(gid, "meeting without a start");
                };
                let mut s = Set::default();
                s.text("gid", gid);
                s.int("started_at", started_at);
                let Some(wrapped) = wrapped_new else {
                    return bad(gid, "new meeting without a key");
                };
                s.put("dek_wrapped", Value::Blob(wrapped));
                s.opt_blob("title_ct", &rec.title_ct);
                s.opt_int("duration_ms", rec.duration_ms);
                s.opt_text("source", &rec.source);
                s.opt_text("mode", &rec.mode);
                s.opt_text("lang", &rec.lang);
                s.opt_text("template", &rec.template);
                s.opt_text("status", &rec.status);
                s.opt_text("privacy_state", &rec.privacy_state);
                s.opt_flag("cloud_locked", rec.cloud_locked);
                s.opt_flag("sensitive", rec.sensitive);
                s.opt_flag("consent_confirmed", rec.consent_confirmed);
                s.opt_flag("cloud_used", rec.cloud_used);
                s.opt_int("transcript_version", rec.transcript_version);
                s.opt_int("transcript_epoch", rec.transcript_epoch);
                s.opt_int("ai_epoch", rec.ai_epoch);
                s.opt_int("audio_retained_until", rec.audio_retained_until);
                s.opt_int("created_at", rec.created_at);
                s.opt_text("source_hash", &rec.source_hash);
                s.opt_text("source_app", &rec.source_app);
                s.opt_blob("calendar_ct", &rec.calendar_ct);
                s.opt_blob("track_speakers_ct", &rec.track_speakers_ct);
                if let Some(o) = audio_origin {
                    s.null_or_int("audio_origin", o);
                }
                folder.set(&mut s, "folder_id");
                self.stamp(&mut s, v);
                id = self.insert("meetings", &s)?;
                self.new_deks.insert(id, dek.clone());
                wrote = true;
            }
            _ => {
                let (m, c) = cur.as_ref().expect("a verdict other than Insert has a row");
                id = m.id;
                let take = matches!(
                    verdict,
                    Verdict::Take | Verdict::Concurrent { rec_wins: true }
                );
                let mut s = Set::default();
                if take {
                    s.opt_text("lang", &rec.lang);
                    s.opt_text("template", &rec.template);
                    s.opt_text("privacy_state", &rec.privacy_state);
                    s.opt_flag("cloud_locked", rec.cloud_locked);
                    s.opt_flag("sensitive", rec.sensitive);
                    s.opt_int("audio_retained_until", rec.audio_retained_until);
                    s.opt_text("source_app", &rec.source_app);
                    s.opt_blob("calendar_ct", &rec.calendar_ct);
                    s.opt_blob("track_speakers_ct", &rec.track_speakers_ct);
                    folder.set(&mut s, "folder_id");
                    self.stamp(&mut s, v);
                }
                if let (Some(text), Some(ct)) = (&title, &rec.title_ct) {
                    self.text_field(
                        verdict,
                        &mut s,
                        TextField {
                            kind: "meeting",
                            table: "meetings",
                            col: "title_ct",
                            target: gid,
                            meeting: id,
                            dek: &dek,
                            rec_ct: ct,
                            rec_text: text,
                            cur_ct: c.title_ct.as_deref(),
                            cur: m,
                            rec_ver: v,
                        },
                    )?;
                }
                // Facts that are set once: fill, never change.
                if c.created_at.is_none() {
                    s.opt_int("created_at", rec.created_at);
                }
                if c.source_hash.is_none() {
                    s.opt_text("source_hash", &rec.source_hash);
                }
                if c.audio_origin.is_none()
                    && let Some(Some(o)) = audio_origin
                {
                    s.int("audio_origin", o);
                }
                // A folder that arrived after the meeting.
                if matches!(verdict, Verdict::NoOp) && c.folder_id.is_none() {
                    folder.set(&mut s, "folder_id");
                }
                // Monotone fields merge whatever the verdict (§7.4).
                let status = rec
                    .status
                    .as_deref()
                    .map_or(c.status.as_str(), |n| rules::merge_status(&c.status, n));
                if status != c.status {
                    s.text("status", status);
                }
                if let Some(d) = rec.duration_ms
                    && d > c.duration_ms
                {
                    s.int("duration_ms", d);
                }
                if rec.cloud_used == Some(true) && !c.cloud_used {
                    s.flag("cloud_used", true);
                }
                if rec.consent_confirmed == Some(true) && !c.consent {
                    s.flag("consent_confirmed", true);
                }
                let tuple = (
                    rec.transcript_version.unwrap_or(c.tv),
                    rec.transcript_epoch.unwrap_or(c.te),
                );
                if tuple > (c.tv, c.te) {
                    s.int("transcript_version", tuple.0);
                    s.int("transcript_epoch", tuple.1);
                }
                if let Some(a) = rec.ai_epoch
                    && a > c.ai
                {
                    s.int("ai_epoch", a);
                }
                // Monotone-only changes are a merge, not a plain accept.
                if !s.is_empty() && !take && matches!(verdict, Verdict::NoOp | Verdict::Take) {
                    merged = true;
                }
                wrote = !s.is_empty();
                self.update("meetings", id, &s)?;
            }
        }
        if wrote {
            self.touched.insert(id);
        }
        if self.sender_id != 0 {
            // The sender holds this meeting's key: part of its Wipe scope.
            self.conn.execute(
                "INSERT INTO peer_meetings (device_id, meeting_gid, key_sent) VALUES (?1, ?2, 1)
                 ON CONFLICT (device_id, meeting_gid) DO UPDATE SET key_sent = 1",
                rusqlite::params![self.sender_id, gid],
            )?;
        }
        if let Some(fg) = folder_missing {
            // The record stays out of its folder until the folder arrives.
            let stub = MeetingRec {
                gid: gid.clone(),
                version: rec.version.clone(),
                base: rec.base.clone(),
                folder_gid: Some(fg.clone()),
                ..Default::default()
            };
            self.park(&Record::Meeting(stub), &fg)?;
        }
        Ok(Step::Done(if merged {
            ApplyOutcome::Merged
        } else {
            ApplyOutcome::Accepted
        }))
    }
}

/// The stored side of a meeting record.
struct MeetingCur {
    title_ct: Option<Vec<u8>>,
    started_at: i64,
    source: String,
    mode: String,
    created_at: Option<i64>,
    source_hash: Option<String>,
    audio_origin: Option<i64>,
    status: String,
    duration_ms: i64,
    cloud_used: bool,
    consent: bool,
    tv: i64,
    te: i64,
    ai: i64,
    folder_id: Option<i64>,
}

/// One free-text field of a row that the record carries.
pub(crate) struct TextField<'a> {
    /// Conflict copy target kind (`meeting`, `segment`, ...).
    pub kind: &'static str,
    pub table: &'static str,
    pub col: &'static str,
    pub target: &'a str,
    pub meeting: i64,
    pub dek: &'a Dek,
    pub rec_ct: &'a Bytes,
    pub rec_text: &'a str,
    pub cur_ct: Option<&'a [u8]>,
    pub cur: &'a Meta,
    pub rec_ver: Ver,
}

impl Ctx<'_> {
    /// Last writer wins, and a different text that loses is kept as a
    /// conflict copy (§7.4). Adds the record's value to `set` when it wins.
    pub(crate) fn text_field(
        &mut self,
        verdict: Verdict,
        set: &mut Set,
        f: TextField,
    ) -> Result<()> {
        match verdict {
            Verdict::Take => set.blob(f.col, f.rec_ct),
            Verdict::Concurrent { rec_wins } => {
                let cur_text = f
                    .cur_ct
                    .and_then(|ct| open_text(f.dek, ct, &row_aad(f.table, f.col, f.target)).ok());
                if cur_text.as_deref() != Some(f.rec_text) {
                    if rec_wins {
                        if let Some(t) = &cur_text {
                            self.make_copy(&f, t, f.cur.lamport, f.cur.origin)?;
                        }
                    } else {
                        self.make_copy(&f, f.rec_text, f.rec_ver.lamport, f.rec_ver.origin)?;
                    }
                }
                if rec_wins {
                    set.blob(f.col, f.rec_ct);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Stores the losing `text` as the conflict copy named by
    /// `H(gid, field, loser version)`. A redelivery names the same copy and
    /// changes nothing; a copy the user already resolved stays gone.
    fn make_copy(
        &mut self,
        f: &TextField,
        text: &str,
        lamport: i64,
        origin: Option<i64>,
    ) -> Result<()> {
        let loser_gid = self.gid_of(origin)?;
        let gid = rules::copy_gid(f.target, f.col, lamport, &loser_gid);
        if self.tombstoned(&gid)? {
            return Ok(());
        }
        let ct = seal_text(f.dek, text, &row_aad("conflict_copies", "value_ct", &gid));
        let l = Store::alloc_lamport(self.conn, 1)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO conflict_copies
                 (gid, meeting_id, target_kind, target_gid, field, value_ct, lamport, origin,
                  created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                gid,
                f.meeting,
                f.kind,
                f.target,
                f.col,
                ct,
                l,
                origin,
                now_ms()
            ],
        )?;
        Ok(())
    }
}

/// Folders and tags: a name, unique by its folded key.
#[derive(Clone, Copy)]
pub(crate) enum Named {
    Folder,
    Tag,
}

impl Named {
    fn table(self) -> &'static str {
        match self {
            Named::Folder => "folders",
            Named::Tag => "tags",
        }
    }
    /// The kind name of its tombstones.
    fn kind(self) -> &'static str {
        match self {
            Named::Folder => "folder",
            Named::Tag => "tag",
        }
    }
}

impl Ctx<'_> {
    // ----------------------------------------------------------------- track

    fn track(&mut self, rec: &TrackRec) -> Result<Step> {
        let gid = &rec.gid;
        let m = go!(self.meeting_of(&rec.meeting_gid)?);
        let cur: Option<(Meta, String, Option<i64>)> = self
            .conn
            .query_row(
                "SELECT id, lamport, origin, base_lamport, base_origin, kind, cut_pages
                 FROM tracks WHERE gid = ?1",
                [gid],
                |r| {
                    Ok((
                        Meta {
                            id: r.get(0)?,
                            lamport: r.get(1)?,
                            origin: r.get(2)?,
                            base_lamport: r.get(3)?,
                            base_origin: r.get(4)?,
                        },
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .optional()?;
        if let (Some((_, kind, _)), Some(k)) = (&cur, &rec.kind)
            && kind != k
        {
            return bad(gid, "immutable field changed");
        }
        let v = self.ver(&rec.version)?;
        let verdict = self.decide(cur.as_ref().map(|c| &c.0), v, rec.base.as_ref())?;
        let kind = match (&cur, &rec.kind) {
            (Some((_, k, _)), _) => k.clone(),
            (None, Some(k)) => k.clone(),
            (None, None) => return bad(gid, "track without a kind"),
        };
        let id = match verdict {
            Verdict::Insert => {
                // One track per kind: another gid already holds it.
                let taken: bool = self.conn.query_row(
                    "SELECT EXISTS (SELECT 1 FROM tracks WHERE meeting_id = ?1 AND kind = ?2)",
                    rusqlite::params![m.id, kind],
                    |r| r.get(0),
                )?;
                if taken {
                    return Ok(Step::Done(ApplyOutcome::Merged));
                }
                let mut s = Set::default();
                s.text("gid", gid);
                s.int("meeting_id", m.id);
                s.text("kind", &kind);
                s.opt_int("page_count", rec.page_count);
                s.opt_int("cut_pages", rec.cut_pages);
                self.stamp(&mut s, v);
                self.insert("tracks", &s)?
            }
            _ => {
                let (meta, _, _) = cur.as_ref().expect("a row");
                if matches!(
                    verdict,
                    Verdict::Take | Verdict::Concurrent { rec_wins: true }
                ) {
                    let mut s = Set::default();
                    s.opt_int("page_count", rec.page_count);
                    self.stamp(&mut s, v);
                    self.update("tracks", meta.id, &s)?;
                }
                meta.id
            }
        };
        // `cut_pages` only ever gets smaller; the bundle follows it.
        let had = cur.as_ref().and_then(|c| c.2);
        let cut = rules::merge_cut(had, rec.cut_pages);
        if let Some(cut) = cut {
            // The count never exceeds the cut (a taken record may carry the
            // old count).
            self.conn.execute(
                "UPDATE tracks SET cut_pages = ?2, page_count = MIN(page_count, ?2)
                 WHERE id = ?1 AND (cut_pages IS NOT ?2 OR page_count > ?2)",
                rusqlite::params![id, cut],
            )?;
            if cut != had.unwrap_or(i64::MAX) {
                self.post
                    .truncate
                    .push((rec.meeting_gid.clone(), kind, cut));
            }
        }
        Ok(Step::Done(match verdict {
            Verdict::Concurrent { .. } | Verdict::KeepLocal => ApplyOutcome::Merged,
            _ => ApplyOutcome::Accepted,
        }))
    }

    // ---------------------------------------------------------------- person

    fn person(&mut self, rec: &PersonRec) -> Result<Step> {
        let gid = &rec.gid;
        let cur = self.meta("persons", gid)?;
        let v = self.ver(&rec.version)?;
        let verdict = self.decide(cur.as_ref(), v, rec.base.as_ref())?;
        match verdict {
            Verdict::Insert => {
                let Some(name) = &rec.name else {
                    return bad(gid, "person without a name");
                };
                let name = fold::nfc(name.trim());
                if name.is_empty() {
                    return bad(gid, "person without a name");
                }
                // Whoever the sender calls Me is just a person here.
                let key = self.unique_key("persons", &crate::people::name_key(&name), gid)?;
                let mut s = Set::default();
                s.text("gid", gid);
                s.text("name", &name);
                s.text("name_key", &key);
                s.int("is_me", 0);
                s.opt_int("color_slot", rec.color_slot);
                s.int("created_at", rec.created_at.unwrap_or_else(now_ms));
                self.stamp(&mut s, v);
                self.insert("persons", &s)?;
            }
            Verdict::Take | Verdict::Concurrent { rec_wins: true } => {
                let meta = cur.expect("a row");
                let mut s = Set::default();
                if let Some(name) = &rec.name {
                    let name = fold::nfc(name.trim());
                    if name.is_empty() {
                        return bad(gid, "person without a name");
                    }
                    let is_me: bool = self.conn.query_row(
                        "SELECT is_me FROM persons WHERE id = ?1",
                        [meta.id],
                        |r| r.get(0),
                    )?;
                    if !is_me {
                        let key =
                            self.unique_key("persons", &crate::people::name_key(&name), gid)?;
                        s.text("name_key", &key);
                    }
                    s.text("name", &name);
                }
                s.opt_int("color_slot", rec.color_slot);
                self.stamp(&mut s, v);
                self.update("persons", meta.id, &s)?;
            }
            _ => {}
        }
        Ok(Step::Done(outcome_of(verdict)))
    }

    /// The `name_key` a person takes. Same-name people made on two devices
    /// stay two people (doc 07 Q6: "Linh" twice is not auto-merged), so the
    /// unique index is kept by a per-gid suffix on the key only; the visible
    /// name is never touched. The lower gid holds the plain key, whatever the
    /// arrival order (the holder is demoted when a lower gid shows up).
    fn unique_key(&self, table: &str, key: &str, gid: &str) -> Result<String> {
        let Some((id, other)) = self.key_holder(table, key, gid)? else {
            return Ok(key.to_string());
        };
        if gid < other.as_str() {
            self.conn.execute(
                &format!("UPDATE {table} SET name_key = ?1 WHERE id = ?2"),
                rusqlite::params![format!("{key}\u{1f}{other}"), id],
            )?;
            Ok(key.to_string())
        } else {
            Ok(format!("{key}\u{1f}{gid}"))
        }
    }

    /// The other live row of `table` (not Me, for persons) that holds `key`.
    fn key_holder(&self, table: &str, key: &str, gid: &str) -> Result<Option<(i64, String)>> {
        let filter = if table == "persons" {
            " AND is_me = 0"
        } else {
            ""
        };
        Ok(self
            .conn
            .query_row(
                &format!("SELECT id, gid FROM {table} WHERE name_key = ?1 AND gid <> ?2{filter}"),
                [key, gid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    // ----------------------------------------------------------- folder / tag

    /// Folders and tags: two devices that made the same name (equal
    /// `name_key`) merge into the row with the lower gid. The higher gid is
    /// tombstoned (`superseded`) and everything that pointed at it is
    /// re-pointed ([`Ctx::merge_named`]). The survivor depends only on the
    /// two gids, so every device converges whatever the arrival order.
    fn named(
        &mut self,
        kind: Named,
        gid: &str,
        version: &Version,
        base: &Option<Version>,
        name: &Option<String>,
        created_at: Option<i64>,
    ) -> Result<Step> {
        let table = kind.table();
        let cur = self.meta(table, gid)?;
        let v = self.ver(version)?;
        let verdict = self.decide(cur.as_ref(), v, base.as_ref())?;
        let clean = |n: &str| fold::nfc(n.trim());
        match verdict {
            Verdict::Insert => {
                let Some(name) = name.as_deref().map(clean).filter(|n| !n.is_empty()) else {
                    return bad(gid, "a name is required");
                };
                let key = crate::people::name_key(&name);
                let other = self.key_holder(table, &key, gid)?;
                if let Some((_, og)) = &other
                    && gid > og.as_str()
                {
                    // Not even inserted: the other row stands for it.
                    self.fold_away(kind, gid, og)?;
                    return Ok(Step::Done(ApplyOutcome::Tombstoned));
                }
                if let Some((oid, og)) = &other {
                    self.park_key(table, *oid, &key, og)?;
                }
                let mut s = Set::default();
                s.text("gid", gid);
                s.text("name", &name);
                s.text("name_key", &key);
                s.int("created_at", created_at.unwrap_or_else(now_ms));
                self.stamp(&mut s, v);
                let id = self.insert(table, &s)?;
                if let Some((oid, og)) = other {
                    self.fold_away(kind, &og, gid)?;
                    self.repoint(kind, oid, id, gid)?;
                    self.drop_row(kind, oid)?;
                }
            }
            Verdict::Take | Verdict::Concurrent { rec_wins: true } => {
                let meta = cur.expect("a row");
                let mut s = Set::default();
                let mut other = None;
                if let Some(name) = name.as_deref().map(clean) {
                    if name.is_empty() {
                        return bad(gid, "a name is required");
                    }
                    let key = crate::people::name_key(&name);
                    other = self.key_holder(table, &key, gid)?;
                    if let Some((oid, og)) = &other {
                        if gid > og.as_str() {
                            // Renamed onto a lower gid's name: this row folds
                            // into it.
                            self.fold_away(kind, gid, og)?;
                            self.repoint(kind, meta.id, *oid, og)?;
                            self.drop_row(kind, meta.id)?;
                            return Ok(Step::Done(outcome_of(verdict)));
                        }
                        self.park_key(table, *oid, &key, og)?;
                    }
                    s.text("name_key", &key);
                    s.text("name", &name);
                }
                self.stamp(&mut s, v);
                self.update(table, meta.id, &s)?;
                if let Some((oid, og)) = other {
                    self.fold_away(kind, &og, gid)?;
                    self.repoint(kind, oid, meta.id, gid)?;
                    self.drop_row(kind, oid)?;
                }
            }
            _ => {}
        }
        Ok(Step::Done(outcome_of(verdict)))
    }

    /// Moves `id`'s `name_key` out of the way (it is about to be deleted).
    fn park_key(&self, table: &str, id: i64, key: &str, gid: &str) -> Result<()> {
        self.conn.execute(
            &format!("UPDATE {table} SET name_key = ?1 WHERE id = ?2"),
            rusqlite::params![format!("{key}\u{1f}{gid}"), id],
        )?;
        Ok(())
    }

    /// Tombstones the folder/tag `loser` (`superseded`, a local write, so it
    /// syncs) and remembers who stands for it: later records that name the
    /// loser (a link, a meeting's folder) are read as naming the survivor.
    fn fold_away(&mut self, kind: Named, loser: &str, survivor: &str) -> Result<()> {
        let lamport = Store::alloc_lamport(self.conn, 1)?;
        tombstones::write_from(
            self.conn,
            loser,
            kind.kind(),
            lamport,
            Cause::Superseded,
            None,
        )?;
        let json = serde_json::to_string(survivor)
            .map_err(|e| crate::StoreError::Invalid(e.to_string()))?;
        self.conn.execute(
            "INSERT OR REPLACE INTO settings (key, value_json) VALUES (?1, ?2)",
            rusqlite::params![format!("{REDIRECT}{loser}"), json],
        )?;
        log::info!("sync: {} {loser} folded into {survivor}", kind.kind());
        Ok(())
    }

    /// Points everything that referenced the row `from` at the row `to`
    /// (gid `to_gid`): a tag's links become new links with deterministic gids
    /// (the old ones are tombstoned `superseded`); a folder's meetings move
    /// and their lamport moves with them (as `delete_folder` does), so the
    /// change syncs.
    fn repoint(&mut self, kind: Named, from: i64, to: i64, to_gid: &str) -> Result<()> {
        let lamport = Store::alloc_lamport(self.conn, 1)?;
        match kind {
            Named::Folder => {
                self.conn.execute(
                    "UPDATE meetings SET folder_id = ?1, lamport = ?2, origin = NULL
                     WHERE folder_id = ?3",
                    rusqlite::params![to, lamport, from],
                )?;
            }
            Named::Tag => {
                let links: Vec<(String, i64)> = self
                    .conn
                    .prepare("SELECT gid, meeting_id FROM meeting_tags WHERE tag_id = ?1")?
                    .query_map([from], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                for (old, meeting) in links {
                    self.conn
                        .execute("DELETE FROM meeting_tags WHERE gid = ?1", [&old])?;
                    tombstones::write_from(
                        self.conn,
                        &old,
                        "meeting_tag",
                        lamport,
                        Cause::Superseded,
                        None,
                    )?;
                    self.relink(&old, meeting, to, to_gid, lamport)?;
                }
            }
        }
        Ok(())
    }

    /// A link `meeting` -> tag `tag_id`, standing for the link `old`.
    fn relink(
        &mut self,
        old: &str,
        meeting: i64,
        tag_id: i64,
        tag_gid: &str,
        lamport: i64,
    ) -> Result<()> {
        let gid = rules::relink_gid(old, tag_gid);
        let have: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM meeting_tags WHERE meeting_id = ?1 AND tag_id = ?2)",
            rusqlite::params![meeting, tag_id],
            |r| r.get(0),
        )?;
        if have || self.tombstoned(&gid)? {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO meeting_tags (gid, meeting_id, tag_id, lamport) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![gid, meeting, tag_id, lamport],
        )?;
        Ok(())
    }

    fn drop_row(&self, kind: Named, id: i64) -> Result<()> {
        self.conn
            .execute(&format!("DELETE FROM {} WHERE id = ?1", kind.table()), [id])?;
        Ok(())
    }

    /// Who stands for the folded-away folder/tag `gid` (following a chain of
    /// folds), if anyone.
    fn redirect(&self, gid: &str) -> Result<Option<String>> {
        let mut cur = gid.to_string();
        for _ in 0..8 {
            let next: Option<String> = self
                .conn
                .query_row(
                    "SELECT value_json FROM settings WHERE key = ?1",
                    [format!("{REDIRECT}{cur}")],
                    |r| r.get::<_, String>(0),
                )
                .optional()?
                .and_then(|j| serde_json::from_str(&j).ok());
            match next {
                Some(n) => cur = n,
                None => break,
            }
        }
        Ok((cur != gid).then_some(cur))
    }

    // --------------------------------------------------------------- speaker

    fn speaker(&mut self, rec: &SpeakerRec) -> Result<Step> {
        let gid = &rec.gid;
        let m = go!(self.meeting_of(&rec.meeting_gid)?);
        let cur: Option<(Meta, Option<Vec<u8>>)> = self
            .conn
            .query_row(
                "SELECT id, lamport, origin, base_lamport, base_origin, display_name_ct
                 FROM speakers WHERE gid = ?1",
                [gid],
                |r| {
                    Ok((
                        Meta {
                            id: r.get(0)?,
                            lamport: r.get(1)?,
                            origin: r.get(2)?,
                            base_lamport: r.get(3)?,
                            base_origin: r.get(4)?,
                        },
                        r.get(5)?,
                    ))
                },
            )
            .optional()?;
        let dek = self.dek_of(m.id)?;
        let name = match &rec.display_name_ct {
            Some(ct) => Some(self.open(&dek, "speakers", "display_name_ct", gid, ct)?),
            None => None,
        };
        let person = go!(self.reference("persons", rec.person_gid.as_deref(), None)?);
        let merged = match rec.merged_into.as_deref() {
            Some(g) if g == gid => Flow::Go(Ref::Clear),
            other => self.reference("speakers", other, Some(m.id))?,
        };
        let merged = go!(merged);
        let v = self.ver(&rec.version)?;
        let verdict = self.decide(cur.as_ref().map(|c| &c.0), v, rec.base.as_ref())?;
        match verdict {
            Verdict::Insert => {
                let Some(label) = rec.label_idx else {
                    return bad(gid, "speaker without a label");
                };
                let mut s = Set::default();
                s.text("gid", gid);
                s.int("meeting_id", m.id);
                s.int("label_idx", label);
                s.opt_blob("display_name_ct", &rec.display_name_ct);
                person.set(&mut s, "person_id");
                merged.set(&mut s, "merged_into");
                s.opt_int("color_slot", rec.color_slot);
                s.opt_flag("is_me", rec.is_me);
                s.opt_flag("not_person", rec.not_person);
                self.stamp(&mut s, v);
                let id = self.insert("speakers", &s)?;
                self.touched.insert(m.id);
                self.break_merge_cycle(id, m.id)?;
            }
            Verdict::Take | Verdict::Concurrent { .. } => {
                let (meta, cur_ct) = cur.as_ref().expect("a row");
                let take = matches!(
                    verdict,
                    Verdict::Take | Verdict::Concurrent { rec_wins: true }
                );
                let mut s = Set::default();
                if take {
                    s.opt_int("label_idx", rec.label_idx);
                    person.set(&mut s, "person_id");
                    merged.set(&mut s, "merged_into");
                    s.opt_int("color_slot", rec.color_slot);
                    s.opt_flag("is_me", rec.is_me);
                    s.opt_flag("not_person", rec.not_person);
                    self.stamp(&mut s, v);
                }
                let cycle_check = take && !s.is_empty();
                if let (Some(text), Some(ct)) = (&name, &rec.display_name_ct) {
                    self.text_field(
                        verdict,
                        &mut s,
                        TextField {
                            kind: "speaker",
                            table: "speakers",
                            col: "display_name_ct",
                            target: gid,
                            meeting: m.id,
                            dek: &dek,
                            rec_ct: ct,
                            rec_text: text,
                            cur_ct: cur_ct.as_deref(),
                            cur: meta,
                            rec_ver: v,
                        },
                    )?;
                }
                if !s.is_empty() {
                    self.touched.insert(m.id);
                }
                self.update("speakers", meta.id, &s)?;
                if cycle_check {
                    self.break_merge_cycle(meta.id, m.id)?;
                }
            }
            _ => {}
        }
        Ok(Step::Done(outcome_of(verdict)))
    }

    /// A `merged_into` cycle through `start` (S1 -> S2 -> S1, or longer) is
    /// broken by clearing the edge whose row has the lowest version
    /// `(lamport, origin gid)` (doc 07 §7.4).
    ///
    /// The clear is derived, not a new write: the row keeps its version, and
    /// every device that holds the whole cycle clears the same edge, so the
    /// result is the same on the hub and every spoke whatever the arrival
    /// order, with no extra lamport (which would let several devices race
    /// three versions of the same clear). A later version of a cleared row
    /// that still carries the edge meets the same rule again.
    fn break_merge_cycle(&mut self, start: i64, meeting: i64) -> Result<()> {
        let mut path: Vec<(i64, i64, Option<i64>)> = Vec::new();
        let mut cur = start;
        loop {
            let row: Option<(Option<i64>, i64, Option<i64>)> = self
                .conn
                .query_row(
                    "SELECT merged_into, lamport, origin FROM speakers WHERE id = ?1",
                    [cur],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((next, lamport, origin)) = row else {
                return Ok(());
            };
            path.push((cur, lamport, origin));
            match next {
                None => return Ok(()),
                Some(n) if n == start => break,
                // Into some other chain (cycles never survive a write).
                Some(n) if path.iter().any(|p| p.0 == n) => return Ok(()),
                Some(n) => cur = n,
            }
            if path.len() > 100_000 {
                return Ok(());
            }
        }
        let mut versions = Vec::with_capacity(path.len());
        for (_, lamport, origin) in &path {
            versions.push(Version {
                lamport: *lamport,
                origin: self.gid_of(*origin)?,
            });
        }
        let (id, ..) = path[rules::weakest_edge(&versions)];
        self.conn
            .execute("UPDATE speakers SET merged_into = NULL WHERE id = ?1", [id])?;
        self.touched.insert(meeting);
        log::info!("sync: a speaker merge cycle of {} was broken", path.len());
        Ok(())
    }

    // --------------------------------------------------------------- segment

    fn segment(&mut self, rec: &SegmentRec) -> Result<Step> {
        let gid = &rec.gid;
        let m = go!(self.meeting_of(&rec.meeting_gid)?);
        let tuple = (
            rec.transcript_version.unwrap_or(m.transcript.0),
            rec.epoch.unwrap_or(m.transcript.1),
        );
        match rules::fence(tuple, m.transcript) {
            Fence::Accept => {}
            // The meeting row may come in a later batch.
            Fence::Park => return Ok(Step::Park(rec.meeting_gid.clone())),
            Fence::Superseded => {
                self.supersede("segment", gid, rec.version.lamport)?;
                return Ok(Step::Done(ApplyOutcome::Tombstoned));
            }
        }
        let cur = self.meta("segments", gid)?;
        let dek = self.dek_of(m.id)?;
        let text = match &rec.text_ct {
            Some(ct) => Some(self.open(&dek, "segments", "text_ct", gid, ct)?),
            None => None,
        };
        let speaker = go!(self.reference("speakers", rec.speaker_gid.as_deref(), Some(m.id))?);
        let v = self.ver(&rec.version)?;
        let verdict = self.decide(cur.as_ref(), v, rec.base.as_ref())?;
        match verdict {
            Verdict::Insert => {
                let (Some(t0), Some(t1), Some(ct)) = (rec.t0_ms, rec.t1_ms, &rec.text_ct) else {
                    return bad(gid, "incomplete segment");
                };
                let mut s = Set::default();
                s.text("gid", gid);
                s.int("meeting_id", m.id);
                s.int("version", tuple.0);
                s.int("epoch", tuple.1);
                speaker.set(&mut s, "speaker_id");
                s.int("t0_ms", t0);
                s.int("t1_ms", t1);
                s.blob("text_ct", ct);
                s.opt_text("lang", &rec.lang);
                if let Some(c) = rec.confidence {
                    s.put("confidence", Value::Real(c));
                }
                s.opt_flag("edited", rec.edited);
                s.opt_flag("overlap", rec.overlap);
                self.stamp(&mut s, v);
                let id = self.insert("segments", &s)?;
                self.put_words(id, rec.words.as_deref())?;
                if let Some(t) = &text {
                    self.refresh_fts("segments_fts", "text_norm", id, t)?;
                }
                self.touched.insert(m.id);
            }
            Verdict::Take | Verdict::Concurrent { .. } => {
                let meta = cur.expect("a row");
                let cur_ct: Option<Vec<u8>> = self.conn.query_row(
                    "SELECT text_ct FROM segments WHERE id = ?1",
                    [meta.id],
                    |r| r.get(0),
                )?;
                let take = matches!(
                    verdict,
                    Verdict::Take | Verdict::Concurrent { rec_wins: true }
                );
                let mut s = Set::default();
                if take {
                    speaker.set(&mut s, "speaker_id");
                    s.opt_int("t0_ms", rec.t0_ms);
                    s.opt_int("t1_ms", rec.t1_ms);
                    s.opt_text("lang", &rec.lang);
                    if let Some(c) = rec.confidence {
                        s.put("confidence", Value::Real(c));
                    }
                    s.opt_flag("edited", rec.edited);
                    s.opt_flag("overlap", rec.overlap);
                    self.stamp(&mut s, v);
                }
                if let (Some(t), Some(ct)) = (&text, &rec.text_ct) {
                    self.text_field(
                        verdict,
                        &mut s,
                        TextField {
                            kind: "segment",
                            table: "segments",
                            col: "text_ct",
                            target: gid,
                            meeting: m.id,
                            dek: &dek,
                            rec_ct: ct,
                            rec_text: t,
                            cur_ct: cur_ct.as_deref(),
                            cur: &meta,
                            rec_ver: v,
                        },
                    )?;
                }
                let text_won = s.cols.contains(&"text_ct");
                if !s.is_empty() {
                    self.touched.insert(m.id);
                }
                self.update("segments", meta.id, &s)?;
                if take {
                    self.put_words(meta.id, rec.words.as_deref())?;
                }
                if let (true, Some(t)) = (text_won, &text) {
                    self.refresh_fts("segments_fts", "text_norm", meta.id, t)?;
                }
            }
            _ => {}
        }
        Ok(Step::Done(outcome_of(verdict)))
    }

    /// Replaces a segment's words (when the record has them).
    fn put_words(&self, segment_id: i64, words: Option<&[super::records::WordRec]>) -> Result<()> {
        let Some(words) = words else {
            return Ok(());
        };
        self.conn
            .execute("DELETE FROM words WHERE segment_id = ?1", [segment_id])?;
        let mut ins = self.conn.prepare_cached(
            "INSERT INTO words (segment_id, idx, t0_ms, t1_ms, conf) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (i, w) in words.iter().enumerate() {
            ins.execute(rusqlite::params![
                segment_id, i as i64, w.t0_ms, w.t1_ms, w.conf
            ])?;
        }
        Ok(())
    }

    /// Rebuilds one FTS row from the decrypted text.
    fn refresh_fts(&self, fts: &str, col: &str, id: i64, text: &str) -> Result<()> {
        self.conn
            .execute(&format!("DELETE FROM {fts} WHERE rowid = ?1"), [id])?;
        let norm = fold::fold(&fold::nfc(text));
        if !norm.is_empty() {
            self.conn.execute(
                &format!("INSERT INTO {fts} (rowid, {col}) VALUES (?1, ?2)"),
                rusqlite::params![id, norm],
            )?;
        }
        Ok(())
    }

    /// An older generation's row: dropped, and tombstoned so it can't return.
    fn supersede(&mut self, kind: &str, gid: &str, lamport: i64) -> Result<()> {
        self.delete_row(kind, gid)?;
        tombstones::write_from(self.conn, gid, kind, lamport, Cause::Superseded, None)
    }

    // ------------------------------------------------------------------ note

    fn note(&mut self, rec: &NoteRec) -> Result<Step> {
        let gid = &rec.gid;
        let m = go!(self.meeting_of(&rec.meeting_gid)?);
        // An AI block of an older generation lost to a regeneration.
        if rec.provenance.as_deref() == Some("ai")
            && rec.pinned != Some(true)
            && let Some(e) = rec.epoch
        {
            match rules::fence((0, e), (0, m.ai_epoch)) {
                Fence::Accept => {}
                Fence::Park => return Ok(Step::Park(rec.meeting_gid.clone())),
                Fence::Superseded => {
                    self.supersede("note", gid, rec.version.lamport)?;
                    return Ok(Step::Done(ApplyOutcome::Tombstoned));
                }
            }
        }
        let cur = self.meta("notes_blocks", gid)?;
        let dek = self.dek_of(m.id)?;
        let body = match &rec.body_ct {
            Some(ct) => Some(self.open(&dek, "notes_blocks", "body_ct", gid, ct)?),
            None => None,
        };
        let v = self.ver(&rec.version)?;
        let verdict = self.decide(cur.as_ref(), v, rec.base.as_ref())?;
        match verdict {
            Verdict::Insert => {
                let (Some(kind), Some(prov), Some(ct)) = (&rec.kind, &rec.provenance, &rec.body_ct)
                else {
                    return bad(gid, "incomplete note");
                };
                let mut s = Set::default();
                s.text("gid", gid);
                s.int("meeting_id", m.id);
                s.text("kind", kind);
                s.text("provenance", prov);
                s.blob("body_ct", ct);
                s.opt_text("anchors_json", &rec.anchors_json);
                s.opt_flag("pinned", rec.pinned);
                s.int("epoch", rec.epoch.unwrap_or(m.ai_epoch));
                s.text("ord", rec.ord.as_deref().unwrap_or(gid));
                self.stamp(&mut s, v);
                let id = self.insert("notes_blocks", &s)?;
                if let Some(b) = &body {
                    self.refresh_fts("notes_fts", "body_norm", id, b)?;
                }
                self.touched.insert(m.id);
            }
            Verdict::Take | Verdict::Concurrent { .. } => {
                let meta = cur.expect("a row");
                let cur_ct: Option<Vec<u8>> = self.conn.query_row(
                    "SELECT body_ct FROM notes_blocks WHERE id = ?1",
                    [meta.id],
                    |r| r.get(0),
                )?;
                let take = matches!(
                    verdict,
                    Verdict::Take | Verdict::Concurrent { rec_wins: true }
                );
                let mut s = Set::default();
                if take {
                    s.opt_text("kind", &rec.kind);
                    s.opt_text("provenance", &rec.provenance);
                    s.opt_text("anchors_json", &rec.anchors_json);
                    s.opt_flag("pinned", rec.pinned);
                    s.opt_int("epoch", rec.epoch);
                    s.opt_text("ord", &rec.ord);
                    self.stamp(&mut s, v);
                }
                if let (Some(t), Some(ct)) = (&body, &rec.body_ct) {
                    self.text_field(
                        verdict,
                        &mut s,
                        TextField {
                            kind: "note",
                            table: "notes_blocks",
                            col: "body_ct",
                            target: gid,
                            meeting: m.id,
                            dek: &dek,
                            rec_ct: ct,
                            rec_text: t,
                            cur_ct: cur_ct.as_deref(),
                            cur: &meta,
                            rec_ver: v,
                        },
                    )?;
                }
                let text_won = s.cols.contains(&"body_ct");
                if !s.is_empty() {
                    self.touched.insert(m.id);
                }
                self.update("notes_blocks", meta.id, &s)?;
                if let (true, Some(b)) = (text_won, &body) {
                    self.refresh_fts("notes_fts", "body_norm", meta.id, b)?;
                }
            }
            _ => {}
        }
        Ok(Step::Done(outcome_of(verdict)))
    }

    // ---------------------------------------------------------------- action

    fn action(&mut self, rec: &ActionItemRec) -> Result<Step> {
        let gid = &rec.gid;
        let m = go!(self.meeting_of(&rec.meeting_gid)?);
        if rec.provenance.as_deref() == Some("ai")
            && rec.done != Some(true)
            && let Some(e) = rec.epoch
        {
            match rules::fence((0, e), (0, m.ai_epoch)) {
                Fence::Accept => {}
                Fence::Park => return Ok(Step::Park(rec.meeting_gid.clone())),
                Fence::Superseded => {
                    self.supersede("action_item", gid, rec.version.lamport)?;
                    return Ok(Step::Done(ApplyOutcome::Tombstoned));
                }
            }
        }
        let cur = self.meta("action_items", gid)?;
        let dek = self.dek_of(m.id)?;
        let text = match &rec.text_ct {
            Some(ct) => Some(self.open(&dek, "action_items", "text_ct", gid, ct)?),
            None => None,
        };
        let due_text = match &rec.due_text_ct {
            Some(ct) => Some(self.open(&dek, "action_items", "due_text_ct", gid, ct)?),
            None => None,
        };
        let owner =
            go!(self.reference("speakers", rec.owner_speaker_gid.as_deref(), Some(m.id))?);
        let v = self.ver(&rec.version)?;
        let verdict = self.decide(cur.as_ref(), v, rec.base.as_ref())?;
        match verdict {
            Verdict::Insert => {
                let (Some(ct), Some(prov)) = (&rec.text_ct, &rec.provenance) else {
                    return bad(gid, "incomplete action item");
                };
                let mut s = Set::default();
                s.text("gid", gid);
                s.int("meeting_id", m.id);
                s.blob("text_ct", ct);
                s.opt_blob("due_text_ct", &rec.due_text_ct);
                owner.set(&mut s, "owner_speaker_id");
                s.opt_int("due", rec.due);
                s.opt_flag("done", rec.done);
                s.opt_text("anchors_json", &rec.anchors_json);
                s.text("provenance", prov);
                s.int("epoch", rec.epoch.unwrap_or(m.ai_epoch));
                s.text("ord", rec.ord.as_deref().unwrap_or(gid));
                self.stamp(&mut s, v);
                self.insert("action_items", &s)?;
                self.touched.insert(m.id);
            }
            Verdict::Take | Verdict::Concurrent { .. } => {
                let meta = cur.expect("a row");
                let (cur_text, cur_due): (Option<Vec<u8>>, Option<Vec<u8>>) = self.conn.query_row(
                    "SELECT text_ct, due_text_ct FROM action_items WHERE id = ?1",
                    [meta.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                let take = matches!(
                    verdict,
                    Verdict::Take | Verdict::Concurrent { rec_wins: true }
                );
                let mut s = Set::default();
                if take {
                    owner.set(&mut s, "owner_speaker_id");
                    s.opt_int("due", rec.due);
                    s.opt_flag("done", rec.done);
                    s.opt_text("anchors_json", &rec.anchors_json);
                    s.opt_text("provenance", &rec.provenance);
                    s.opt_int("epoch", rec.epoch);
                    s.opt_text("ord", &rec.ord);
                    self.stamp(&mut s, v);
                }
                for (col, rec_ct, rec_text, cur_ct) in [
                    ("text_ct", &rec.text_ct, &text, &cur_text),
                    ("due_text_ct", &rec.due_text_ct, &due_text, &cur_due),
                ] {
                    if let (Some(t), Some(ct)) = (rec_text, rec_ct) {
                        self.text_field(
                            verdict,
                            &mut s,
                            TextField {
                                kind: "action_item",
                                table: "action_items",
                                col,
                                target: gid,
                                meeting: m.id,
                                dek: &dek,
                                rec_ct: ct,
                                rec_text: t,
                                cur_ct: cur_ct.as_deref(),
                                cur: &meta,
                                rec_ver: v,
                            },
                        )?;
                    }
                }
                if !s.is_empty() {
                    self.touched.insert(m.id);
                }
                self.update("action_items", meta.id, &s)?;
            }
            _ => {}
        }
        Ok(Step::Done(outcome_of(verdict)))
    }

    // ------------------------------------------------------------------ mark

    fn mark(&mut self, rec: &MarkRec) -> Result<Step> {
        let gid = &rec.gid;
        let m = go!(self.meeting_of(&rec.meeting_gid)?);
        // Insert-only: a mark that is here stays as it is.
        if self.meta("marks", gid)?.is_some() {
            return Ok(Step::Done(ApplyOutcome::Accepted));
        }
        let (Some(t_ms), Some(tag)) = (rec.t_ms, &rec.tag) else {
            return bad(gid, "incomplete mark");
        };
        let v = self.ver(&rec.version)?;
        let mut s = Set::default();
        s.text("gid", gid);
        s.int("meeting_id", m.id);
        s.int("t_ms", t_ms);
        s.text("tag", tag);
        self.stamp(&mut s, v);
        self.insert("marks", &s)?;
        Ok(Step::Done(ApplyOutcome::Accepted))
    }

    // ----------------------------------------------------------- meeting tag

    fn link(&mut self, rec: &MeetingTagRec) -> Result<Step> {
        let gid = &rec.gid;
        let m = go!(self.meeting_of(&rec.meeting_gid)?);
        let tag: Option<i64> = self
            .conn
            .query_row("SELECT id FROM tags WHERE gid = ?1", [&rec.tag_gid], |r| {
                r.get(0)
            })
            .optional()?;
        let tag = match tag {
            Some(t) => t,
            None if self.tombstoned(&rec.tag_gid)? => {
                // A tag folded into another: the link follows it.
                let target: Option<(i64, String)> = match self.redirect(&rec.tag_gid)? {
                    Some(t) => self
                        .conn
                        .query_row("SELECT id, gid FROM tags WHERE gid = ?1", [t], |r| {
                            Ok((r.get(0)?, r.get(1)?))
                        })
                        .optional()?,
                    None => None,
                };
                if let Some((tag_id, tag_gid)) = target {
                    let lamport = Store::alloc_lamport(self.conn, 1)?;
                    tombstones::write_from(
                        self.conn,
                        gid,
                        "meeting_tag",
                        lamport,
                        Cause::Superseded,
                        None,
                    )?;
                    self.relink(gid, m.id, tag_id, &tag_gid, lamport)?;
                }
                return Ok(Step::Done(ApplyOutcome::Tombstoned));
            }
            None => return Ok(Step::Park(rec.tag_gid.clone())),
        };
        let have: bool = self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM meeting_tags WHERE gid = ?1)",
            [gid],
            |r| r.get(0),
        )?;
        if have {
            return Ok(Step::Done(ApplyOutcome::Accepted));
        }
        // The same tag added on two devices: two link gids for one pair. The
        // lower gid stays; the other is tombstoned, which relays to whoever
        // made it.
        let other: Option<String> = self
            .conn
            .query_row(
                "SELECT gid FROM meeting_tags WHERE meeting_id = ?1 AND tag_id = ?2",
                rusqlite::params![m.id, tag],
                |r| r.get(0),
            )
            .optional()?;
        let v = self.ver(&rec.version)?;
        if let Some(other) = other {
            let lamport = Store::alloc_lamport(self.conn, 1)?;
            if *gid < other {
                self.conn
                    .execute("DELETE FROM meeting_tags WHERE gid = ?1", [&other])?;
                tombstones::write_from(
                    self.conn,
                    &other,
                    "meeting_tag",
                    lamport,
                    Cause::Superseded,
                    None,
                )?;
            } else {
                tombstones::write_from(
                    self.conn,
                    gid,
                    "meeting_tag",
                    lamport,
                    Cause::Superseded,
                    None,
                )?;
                return Ok(Step::Done(ApplyOutcome::Tombstoned));
            }
        }
        let mut s = Set::default();
        s.text("gid", gid);
        s.int("meeting_id", m.id);
        s.int("tag_id", tag);
        self.stamp(&mut s, v);
        self.insert("meeting_tags", &s)?;
        Ok(Step::Done(ApplyOutcome::Accepted))
    }

    // --------------------------------------------------------- conflict copy

    fn copy_row(&mut self, rec: &ConflictCopyRec) -> Result<Step> {
        let gid = &rec.gid;
        let m = go!(self.meeting_of(&rec.meeting_gid)?);
        if self.meta("conflict_copies", gid)?.is_some() {
            return Ok(Step::Done(ApplyOutcome::Accepted));
        }
        let Some(ct) = &rec.value_ct else {
            return bad(gid, "conflict copy without a value");
        };
        let dek = self.dek_of(m.id)?;
        self.open(&dek, "conflict_copies", "value_ct", gid, ct)?;
        let v = self.ver(&rec.version)?;
        let mut s = Set::default();
        s.text("gid", gid);
        s.int("meeting_id", m.id);
        s.text("target_kind", &rec.target_kind);
        s.text("target_gid", &rec.target_gid);
        s.text("field", &rec.field);
        s.blob("value_ct", ct);
        s.int("created_at", rec.created_at.unwrap_or_else(now_ms));
        self.stamp(&mut s, v);
        self.insert("conflict_copies", &s)?;
        Ok(Step::Done(ApplyOutcome::Accepted))
    }

    // --------------------------------------------------------------- setting

    /// Synced settings are last-writer-wins per key in
    /// [`Store::apply_synced`] (slice 15-C1), which takes its own transaction:
    /// it runs once this batch has committed.
    fn setting(&mut self, rec: &SettingRec) -> Result<Step> {
        self.post.settings.push(rec.clone());
        Ok(Step::Done(ApplyOutcome::Accepted))
    }
}

fn outcome_of(v: Verdict) -> ApplyOutcome {
    match v {
        Verdict::Concurrent { .. } | Verdict::KeepLocal => ApplyOutcome::Merged,
        _ => ApplyOutcome::Accepted,
    }
}

// ---------------------------------------------------------------- tombstones

fn cause_of(c: TombCause) -> Cause {
    match c {
        TombCause::User => Cause::User,
        TombCause::Meeting => Cause::Meeting,
        TombCause::Regenerate => Cause::Regenerate,
        TombCause::Discard => Cause::Discard,
        TombCause::Retention => Cause::Retention,
        TombCause::Transcript => Cause::Transcript,
        TombCause::Superseded => Cause::Superseded,
    }
}

impl Ctx<'_> {
    /// Applies one tombstone: absorbing, whatever the version of the row.
    pub(crate) fn tombstone(&mut self, t: &SyncTombstone) -> Result<Tomb> {
        let known = [
            RecordKind::Meeting,
            RecordKind::Track,
            RecordKind::Person,
            RecordKind::Speaker,
            RecordKind::Segment,
            RecordKind::Note,
            RecordKind::ActionItem,
            RecordKind::Mark,
            RecordKind::Folder,
            RecordKind::Tag,
            RecordKind::MeetingTag,
            RecordKind::VoiceProfile,
            RecordKind::ConflictCopy,
        ];
        let Some(kind) = known.iter().find(|k| k.log_kind() == t.kind) else {
            return Ok(Tomb::Rejected);
        };
        if !canonical(&t.gid) || t.lamport < 0 || t.origin.is_empty() || t.origin.len() > 64 {
            return Ok(Tomb::Rejected);
        }
        let kind = kind.log_kind();
        observe_lamport(self.conn, t.lamport)?;
        if self.tombstoned(&t.gid)? {
            return Ok(Tomb::Present);
        }
        let cause = t.cause.map_or(Cause::User, cause_of);
        let origin = self.norm_origin(&t.origin)?;
        self.unpark(&t.gid)?;
        match kind {
            "meeting" => {
                let id: Option<i64> = self
                    .conn
                    .query_row("SELECT id FROM meetings WHERE gid = ?1", [&t.gid], |r| {
                        r.get(0)
                    })
                    .optional()?;
                if let Some(id) = id {
                    // Every child dies with it, even ones that never had a
                    // tombstone of their own; the row itself is shredded once
                    // this commits.
                    tombstones::write_children(self.conn, id, t.lamport, Cause::Meeting)?;
                }
            }
            "person" => {
                // Me never goes, and a person with a voice profile stays until
                // the profile is deleted (the store refuses otherwise).
                let keep: bool = self.conn.query_row(
                    "SELECT EXISTS (
                         SELECT 1 FROM persons p WHERE p.gid = ?1
                           AND (p.is_me = 1 OR EXISTS (
                                SELECT 1 FROM voice_profiles v WHERE v.person_id = p.id
                                  AND v.key_wrapped <> zeroblob(length(v.key_wrapped)))))",
                    [&t.gid],
                    |r| r.get(0),
                )?;
                if keep {
                    return Ok(Tomb::Skipped);
                }
                self.delete_row(kind, &t.gid)?;
            }
            "voice_profile" => {} // shredded after the commit
            "note" | "action_item" if cause == Cause::Regenerate => {
                self.keep_edited(kind, &t.gid)?;
                self.delete_row(kind, &t.gid)?;
            }
            "track" => {
                let row: Option<(String, String)> = self
                    .conn
                    .query_row(
                        "SELECT m.gid, t.kind FROM tracks t JOIN meetings m ON m.id = t.meeting_id
                         WHERE t.gid = ?1",
                        [&t.gid],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?;
                if let Some(row) = row {
                    self.post.files.push(row);
                }
                self.delete_row(kind, &t.gid)?;
            }
            _ => self.delete_row(kind, &t.gid)?,
        }
        tombstones::write_from(self.conn, &t.gid, kind, t.lamport, cause, origin)?;
        Ok(Tomb::Applied)
    }

    /// Deletes the row of `gid` of `kind` (FTS rows, words and links go with
    /// it); marks its meeting as touched.
    fn delete_row(&mut self, kind: &str, gid: &str) -> Result<()> {
        let (table, fts) = match kind {
            "segment" => ("segments", Some("segments_fts")),
            "note" => ("notes_blocks", Some("notes_fts")),
            "action_item" => ("action_items", None),
            "speaker" => ("speakers", None),
            "mark" => ("marks", None),
            "track" => ("tracks", None),
            "meeting_tag" => ("meeting_tags", None),
            "conflict_copy" => ("conflict_copies", None),
            "folder" => ("folders", None),
            "tag" => ("tags", None),
            "person" => ("persons", None),
            // Meetings and voice profiles are shredded after the commit.
            _ => return Ok(()),
        };
        if matches!(kind, "segment" | "note" | "action_item" | "speaker") {
            let row: Option<(i64, i64)> = self
                .conn
                .query_row(
                    &format!("SELECT id, meeting_id FROM {table} WHERE gid = ?1"),
                    [gid],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((id, meeting)) = row {
                if let Some(fts) = fts {
                    self.conn
                        .execute(&format!("DELETE FROM {fts} WHERE rowid = ?1"), [id])?;
                }
                self.touched.insert(meeting);
            }
        }
        self.conn
            .execute(&format!("DELETE FROM {table} WHERE gid = ?1"), [gid])?;
        Ok(())
    }

    /// A regeneration's tombstone met a block or item this side edited
    /// (§7.5.4): its text lives on as a new user row, and the old gid stays
    /// dead. Edited = not plain AI output, pinned, done, or (spoke) changed
    /// since the hub's version.
    fn keep_edited(&mut self, kind: &str, gid: &str) -> Result<()> {
        let table = if kind == "note" {
            "notes_blocks"
        } else {
            "action_items"
        };
        let Some(meta) = self.meta(table, gid)? else {
            return Ok(());
        };
        let (meeting, provenance, flag): (i64, String, bool) = self.conn.query_row(
            &format!(
                "SELECT meeting_id, provenance, {} FROM {table} WHERE id = ?1",
                if kind == "note" { "pinned" } else { "done" }
            ),
            [meta.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let edited = provenance != "ai" || flag || (self.spoke && meta.dirty());
        if !edited {
            return Ok(());
        }
        let dek = self.dek_of(meeting)?;
        let new_gid = new_gid();
        let lamport = Store::alloc_lamport(self.conn, 1)?;
        let reseal = |col: &str, ct: &[u8]| -> Result<Vec<u8>> {
            let text = open_text(&dek, ct, &row_aad(table, col, gid))?;
            Ok(seal_text(&dek, &text, &row_aad(table, col, &new_gid)))
        };
        if kind == "note" {
            let (nk, body, anchors, pinned, ord, epoch): (
                String,
                Vec<u8>,
                String,
                bool,
                String,
                i64,
            ) = self.conn.query_row(
                "SELECT kind, body_ct, anchors_json, pinned, ord, epoch
                     FROM notes_blocks WHERE id = ?1",
                [meta.id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )?;
            let ct = reseal("body_ct", &body)?;
            self.conn.execute(
                "INSERT INTO notes_blocks
                     (gid, meeting_id, kind, provenance, body_ct, anchors_json, pinned, lamport,
                      ord, epoch)
                 VALUES (?1, ?2, ?3, 'user', ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    new_gid, meeting, nk, ct, anchors, pinned, lamport, ord, epoch
                ],
            )?;
            let id = self.conn.last_insert_rowid();
            let text = open_text(&dek, &body, &row_aad(table, "body_ct", gid))?;
            self.refresh_fts("notes_fts", "body_norm", id, &text)?;
        } else {
            #[allow(clippy::type_complexity)]
            let (text, due_text, owner, due, done, anchors, ord, epoch): (
                Vec<u8>,
                Option<Vec<u8>>,
                Option<i64>,
                Option<i64>,
                bool,
                String,
                String,
                i64,
            ) = self.conn.query_row(
                "SELECT text_ct, due_text_ct, owner_speaker_id, due, done, anchors_json, ord, epoch
                 FROM action_items WHERE id = ?1",
                [meta.id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                },
            )?;
            let text_ct = reseal("text_ct", &text)?;
            let due_ct = match &due_text {
                Some(d) => Some(reseal("due_text_ct", d)?),
                None => None,
            };
            self.conn.execute(
                "INSERT INTO action_items
                     (gid, meeting_id, text_ct, due_text_ct, owner_speaker_id, due, done,
                      anchors_json, provenance, lamport, ord, epoch)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'user', ?9, ?10, ?11)",
                rusqlite::params![
                    new_gid, meeting, text_ct, due_ct, owner, due, done, anchors, lamport, ord,
                    epoch
                ],
            )?;
        }
        self.touched.insert(meeting);
        Ok(())
    }
}
