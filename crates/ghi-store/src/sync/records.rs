// SPDX-License-Identifier: Apache-2.0
//! Typed sync records (doc 07 §5.4, §7.1): one struct per row kind, as they
//! travel between devices.
//!
//! - Foreign keys are **gids**, never rowids; `origin` is a device gid.
//! - `_ct` fields are the stored ciphertext, verbatim (the AAD binds them to
//!   `table.column:gid`, which is global).
//! - Every field except the identity ones is an `Option`: **absent means
//!   keep the current value**, so a peer on an older minor version never
//!   resets a field it doesn't know (§5.2). A nullable column can therefore
//!   be set but not cleared by a record; clearing is a later minor version.
//! - Nothing here is validated; [`crate::sync::apply`] does that.

use std::cmp::Ordering;
use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Raw bytes that serialize as a CBOR byte string (a `Vec<u8>` would become an
/// array of integers). `Debug` shows the length only: it may be ciphertext or
/// a key.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Bytes(pub Vec<u8>);

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bytes({} B)", self.0.len())
    }
}

impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Bytes;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a byte string")
            }
            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Bytes, E> {
                Ok(Bytes(v.to_vec()))
            }
            fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Bytes, E> {
                Ok(Bytes(v))
            }
            // Self-describing text formats (JSON, used for parked records)
            // write bytes as an array of numbers.
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Bytes, A::Error> {
                let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(1 << 20));
                while let Some(b) = seq.next_element::<u8>()? {
                    out.push(b);
                }
                Ok(Bytes(out))
            }
        }
        d.deserialize_bytes(V)
    }
}

/// The version of a row: `(lamport, origin device gid)`, ordered that way
/// (§7.3: the deterministic tie-break between concurrent writes).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Version {
    pub lamport: i64,
    pub origin: String,
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.lamport, &self.origin).cmp(&(other.lamport, &other.origin))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Defines a record struct: the identity fields (`gid`, `version`, `base`),
/// the required ones, then the optional ones.
macro_rules! record {
    ($(#[$m:meta])* $name:ident {
        $($rf:ident : $rt:ty),* $(,)? ;
        $($of:ident : $ot:ty),* $(,)?
    }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
        pub struct $name {
            pub gid: String,
            pub version: Version,
            /// The hub's version this one was last based on (spoke pushes only).
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub base: Option<Version>,
            $(pub $rf: $rt,)*
            $(
                #[serde(default, skip_serializing_if = "Option::is_none")]
                pub $of: Option<$ot>,
            )*
        }
    };
}

record! {
    /// A meeting. `dek` rides in the record the first time it goes to a peer
    /// (inside the Noise channel only; never stored, never logged).
    MeetingRec {
        ;
        title_ct: Bytes, started_at: i64, duration_ms: i64, source: String,
        mode: String, lang: String, template: String, status: String,
        privacy_state: String, cloud_locked: bool, sensitive: bool,
        consent_confirmed: bool, cloud_used: bool, transcript_version: i64,
        transcript_epoch: i64, ai_epoch: i64, audio_retained_until: i64,
        audio_origin: String, created_at: i64, source_hash: String,
        source_app: String, folder_gid: String, calendar_ct: Bytes,
        track_speakers_ct: Bytes, dek: Bytes
    }
}

record! {
    TrackRec {
        meeting_gid: String ;
        kind: String, page_count: i64, cut_pages: i64
    }
}

record! {
    PersonRec {
        ;
        name: String, color_slot: i64, is_me: bool, created_at: i64
    }
}

record! {
    SpeakerRec {
        meeting_gid: String ;
        label_idx: i64, display_name_ct: Bytes, person_gid: String,
        color_slot: i64, is_me: bool, not_person: bool, merged_into: String
    }
}

/// Word timing; the word text lives inside the segment's `text_ct`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WordRec {
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub conf: Option<f64>,
}

record! {
    SegmentRec {
        meeting_gid: String ;
        transcript_version: i64, epoch: i64, speaker_gid: String, t0_ms: i64,
        t1_ms: i64, text_ct: Bytes, lang: String, confidence: f64,
        edited: bool, overlap: bool, words: Vec<WordRec>
    }
}

record! {
    NoteRec {
        meeting_gid: String ;
        kind: String, provenance: String, body_ct: Bytes, anchors_json: String,
        pinned: bool, epoch: i64, ord: String
    }
}

record! {
    ActionItemRec {
        meeting_gid: String ;
        text_ct: Bytes, due_text_ct: Bytes, owner_speaker_gid: String,
        due: i64, done: bool, anchors_json: String, provenance: String,
        epoch: i64, ord: String
    }
}

record! {
    /// Insert-only; deleted by tombstone.
    MarkRec {
        meeting_gid: String ;
        t_ms: i64, tag: String
    }
}

record! {
    FolderRec {
        ;
        name: String, created_at: i64
    }
}

record! {
    TagRec {
        ;
        name: String, created_at: i64
    }
}

record! {
    MeetingTagRec {
        meeting_gid: String, tag_gid: String ;
    }
}

record! {
    /// Reserved: voice profiles do not sync in v1 (only their tombstones do).
    /// The kind number is kept so a later major can use it.
    VoiceProfileRec {
        ;
        person_gid: String
    }
}

record! {
    /// The losing side of a concurrent free-text edit (§7.5).
    ConflictCopyRec {
        meeting_gid: String, target_kind: String, target_gid: String,
        field: String ;
        value_ct: Bytes, created_at: i64
    }
}

record! {
    /// One synced setting, last writer wins per key.
    SettingRec {
        key: String ;
        value_json: String
    }
}

/// The kind number of a record on the wire. Numbers are never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum RecordKind {
    Meeting = 1,
    Track = 2,
    Person = 3,
    Speaker = 4,
    Segment = 5,
    Note = 6,
    ActionItem = 7,
    Mark = 8,
    Folder = 9,
    Tag = 10,
    MeetingTag = 11,
    /// Reserved (not synced in v1).
    VoiceProfile = 12,
    ConflictCopy = 13,
    Setting = 14,
}

impl RecordKind {
    /// The `sync_log.kind` / tombstone kind of this record kind.
    pub fn log_kind(self) -> &'static str {
        match self {
            Self::Meeting => "meeting",
            Self::Track => "track",
            Self::Person => "person",
            Self::Speaker => "speaker",
            Self::Segment => "segment",
            Self::Note => "note",
            Self::ActionItem => "action_item",
            Self::Mark => "mark",
            Self::Folder => "folder",
            Self::Tag => "tag",
            Self::MeetingTag => "meeting_tag",
            Self::VoiceProfile => "voice_profile",
            Self::ConflictCopy => "conflict_copy",
            Self::Setting => "setting",
        }
    }

    pub fn from_u8(n: u8) -> Option<Self> {
        Some(match n {
            1 => Self::Meeting,
            2 => Self::Track,
            3 => Self::Person,
            4 => Self::Speaker,
            5 => Self::Segment,
            6 => Self::Note,
            7 => Self::ActionItem,
            8 => Self::Mark,
            9 => Self::Folder,
            10 => Self::Tag,
            11 => Self::MeetingTag,
            12 => Self::VoiceProfile,
            13 => Self::ConflictCopy,
            14 => Self::Setting,
            _ => return None,
        })
    }
}

/// One syncable row as it travels. Short tags keep the CBOR small. The size
/// spread is fine: records are short-lived values in a batch `Vec`.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Record {
    #[serde(rename = "1")]
    Meeting(MeetingRec),
    #[serde(rename = "2")]
    Track(TrackRec),
    #[serde(rename = "3")]
    Person(PersonRec),
    #[serde(rename = "4")]
    Speaker(SpeakerRec),
    #[serde(rename = "5")]
    Segment(SegmentRec),
    #[serde(rename = "6")]
    Note(NoteRec),
    #[serde(rename = "7")]
    ActionItem(ActionItemRec),
    #[serde(rename = "8")]
    Mark(MarkRec),
    #[serde(rename = "9")]
    Folder(FolderRec),
    #[serde(rename = "10")]
    Tag(TagRec),
    #[serde(rename = "11")]
    MeetingTag(MeetingTagRec),
    #[serde(rename = "12")]
    VoiceProfile(VoiceProfileRec),
    #[serde(rename = "13")]
    ConflictCopy(ConflictCopyRec),
    #[serde(rename = "14")]
    Setting(SettingRec),
}

impl Record {
    pub fn kind(&self) -> RecordKind {
        match self {
            Self::Meeting(_) => RecordKind::Meeting,
            Self::Track(_) => RecordKind::Track,
            Self::Person(_) => RecordKind::Person,
            Self::Speaker(_) => RecordKind::Speaker,
            Self::Segment(_) => RecordKind::Segment,
            Self::Note(_) => RecordKind::Note,
            Self::ActionItem(_) => RecordKind::ActionItem,
            Self::Mark(_) => RecordKind::Mark,
            Self::Folder(_) => RecordKind::Folder,
            Self::Tag(_) => RecordKind::Tag,
            Self::MeetingTag(_) => RecordKind::MeetingTag,
            Self::VoiceProfile(_) => RecordKind::VoiceProfile,
            Self::ConflictCopy(_) => RecordKind::ConflictCopy,
            Self::Setting(_) => RecordKind::Setting,
        }
    }

    pub fn gid(&self) -> &str {
        match self {
            Self::Meeting(r) => &r.gid,
            Self::Track(r) => &r.gid,
            Self::Person(r) => &r.gid,
            Self::Speaker(r) => &r.gid,
            Self::Segment(r) => &r.gid,
            Self::Note(r) => &r.gid,
            Self::ActionItem(r) => &r.gid,
            Self::Mark(r) => &r.gid,
            Self::Folder(r) => &r.gid,
            Self::Tag(r) => &r.gid,
            Self::MeetingTag(r) => &r.gid,
            Self::VoiceProfile(r) => &r.gid,
            Self::ConflictCopy(r) => &r.gid,
            Self::Setting(r) => &r.gid,
        }
    }

    pub fn version(&self) -> &Version {
        match self {
            Self::Meeting(r) => &r.version,
            Self::Track(r) => &r.version,
            Self::Person(r) => &r.version,
            Self::Speaker(r) => &r.version,
            Self::Segment(r) => &r.version,
            Self::Note(r) => &r.version,
            Self::ActionItem(r) => &r.version,
            Self::Mark(r) => &r.version,
            Self::Folder(r) => &r.version,
            Self::Tag(r) => &r.version,
            Self::MeetingTag(r) => &r.version,
            Self::VoiceProfile(r) => &r.version,
            Self::ConflictCopy(r) => &r.version,
            Self::Setting(r) => &r.version,
        }
    }

    /// The meeting this record belongs to, for the kinds that have a parent
    /// meeting (the unit that tombstones and parking work on).
    pub fn meeting_gid(&self) -> Option<&str> {
        match self {
            Self::Track(r) => Some(&r.meeting_gid),
            Self::Speaker(r) => Some(&r.meeting_gid),
            Self::Segment(r) => Some(&r.meeting_gid),
            Self::Note(r) => Some(&r.meeting_gid),
            Self::ActionItem(r) => Some(&r.meeting_gid),
            Self::Mark(r) => Some(&r.meeting_gid),
            Self::MeetingTag(r) => Some(&r.meeting_gid),
            Self::ConflictCopy(r) => Some(&r.meeting_gid),
            _ => None,
        }
    }
}

/// Why a row was deleted (`tombstones.cause`, doc 07 §7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TombCause {
    User,
    Meeting,
    /// An AI result replaced by a regeneration (§7.5.4: the one case where an
    /// edited target survives as a new row).
    Regenerate,
    Discard,
    Retention,
    Transcript,
    Superseded,
}

impl TombCause {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Meeting => "meeting",
            Self::Regenerate => "regenerate",
            Self::Discard => "discard",
            Self::Retention => "retention",
            Self::Transcript => "transcript",
            Self::Superseded => "superseded",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "user" => Self::User,
            "meeting" => Self::Meeting,
            "regenerate" => Self::Regenerate,
            "discard" => Self::Discard,
            "retention" => Self::Retention,
            "transcript" => Self::Transcript,
            "superseded" => Self::Superseded,
            _ => return None,
        })
    }
}

/// A tombstone as it travels: no content, only what was deleted and why.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncTombstone {
    pub gid: String,
    pub kind: String,
    pub lamport: i64,
    /// Device gid of whoever deleted it.
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<TombCause>,
}

// ------------------------------------------------------------------ encode

use rusqlite::{Connection, OptionalExtension, Row};

use crate::store::Store;
use crate::{Result, StoreError};

/// This device's own gid (`settings['sync.device_gid']`).
pub(crate) fn own_gid(conn: &Connection) -> Result<String> {
    let json: Option<String> = conn
        .query_row(
            "SELECT value_json FROM settings WHERE key = 'sync.device_gid'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    json.and_then(|j| serde_json::from_str::<String>(&j).ok())
        .ok_or_else(|| StoreError::Invalid("this device has no sync identity".into()))
}

/// The gid of the device an `origin` column names (`None` = this device, and
/// also a device that is no longer paired).
pub(crate) fn origin_gid(conn: &Connection, origin: Option<i64>, own: &str) -> Result<String> {
    let Some(id) = origin else {
        return Ok(own.to_string());
    };
    let gid: Option<String> = conn
        .query_row("SELECT gid FROM devices WHERE id = ?1", [id], |r| r.get(0))
        .optional()?;
    Ok(gid.unwrap_or_else(|| own.to_string()))
}

/// `(lamport, origin, base_lamport, base_origin)` of a row, as the columns
/// hold them, at `at..at + 4` of `r`.
fn stamp(conn: &Connection, own: &str, r: &Row, at: usize) -> Result<(Version, Option<Version>)> {
    let lamport: i64 = r.get(at)?;
    let origin: Option<i64> = r.get(at + 1)?;
    let base_lamport: Option<i64> = r.get(at + 2)?;
    let base_origin: Option<i64> = r.get(at + 3)?;
    let version = Version {
        lamport,
        origin: origin_gid(conn, origin, own)?,
    };
    let base = match base_lamport {
        Some(l) => Some(Version {
            lamport: l,
            origin: origin_gid(conn, base_origin, own)?,
        }),
        None => None,
    };
    Ok((version, base))
}

/// Every table's rows are addressed by gid, and `kind` is its `sync_log` kind.
pub(crate) fn table_of(kind: &str) -> Option<&'static str> {
    crate::migrate::SYNC_TABLES
        .iter()
        .find(|(_, k)| *k == kind)
        .map(|(t, _)| *t)
}

impl Store {
    /// The record to send for the row `gid` of `kind` (a `sync_log` kind):
    /// `_ct` values verbatim, foreign keys as gids, and for a spoke push the
    /// `base`. `with_dek` adds the meeting's key (first send to a peer only;
    /// it must travel inside the Noise channel). `None` if the row is gone,
    /// shredded, or of a kind that does not sync (voice profiles).
    pub fn encode_record(&self, kind: &str, gid: &str, with_dek: bool) -> Result<Option<Record>> {
        let conn = self.conn();
        self.encode_record_on(&conn, kind, gid, with_dek)
    }

    /// [`Store::encode_record`] on an open connection.
    pub(crate) fn encode_record_on(
        &self,
        conn: &Connection,
        kind: &str,
        gid: &str,
        with_dek: bool,
    ) -> Result<Option<Record>> {
        let own = own_gid(conn)?;
        let rec = match kind {
            "meeting" => self
                .encode_meeting(conn, &own, gid, with_dek)?
                .map(Record::Meeting),
            "track" => encode_track(conn, &own, gid)?.map(Record::Track),
            "person" => encode_person(conn, &own, gid)?.map(Record::Person),
            "speaker" => encode_speaker(conn, &own, gid)?.map(Record::Speaker),
            "segment" => encode_segment(conn, &own, gid)?.map(Record::Segment),
            "note" => encode_note(conn, &own, gid)?.map(Record::Note),
            "action_item" => encode_action(conn, &own, gid)?.map(Record::ActionItem),
            "mark" => encode_mark(conn, &own, gid)?.map(Record::Mark),
            "folder" => encode_folder(conn, &own, gid)?.map(Record::Folder),
            "tag" => encode_tag(conn, &own, gid)?.map(Record::Tag),
            "meeting_tag" => encode_link(conn, &own, gid)?.map(Record::MeetingTag),
            "conflict_copy" => encode_copy(conn, &own, gid)?.map(Record::ConflictCopy),
            "setting" => encode_setting(conn, &own, gid)?.map(Record::Setting),
            _ => None,
        };
        Ok(rec)
    }

    /// Spoke: whether the row has local changes the hub has not acknowledged
    /// (never pushed, or its version differs from its `base`). A missing row
    /// is not dirty.
    pub fn sync_dirty(&self, kind: &str, gid: &str) -> Result<bool> {
        let Some(table) = table_of(kind) else {
            return Ok(false);
        };
        let dirty: Option<bool> = self
            .conn()
            .query_row(
                &format!(
                    "SELECT base_lamport IS NULL OR base_lamport <> lamport
                            OR base_origin IS NOT origin
                     FROM {table} WHERE gid = ?1"
                ),
                [gid],
                |r| r.get(0),
            )
            .optional()?;
        Ok(dirty.unwrap_or(false))
    }

    fn encode_meeting(
        &self,
        conn: &Connection,
        own: &str,
        gid: &str,
        with_dek: bool,
    ) -> Result<Option<MeetingRec>> {
        let row = conn
            .query_row(
                "SELECT m.id, m.title_ct, m.started_at, m.duration_ms, m.source, m.mode, m.lang,
                        m.template, m.status, m.privacy_state, m.cloud_locked, m.sensitive,
                        m.consent_confirmed, m.cloud_used, m.transcript_version,
                        m.transcript_epoch, m.ai_epoch, m.audio_retained_until, m.audio_origin,
                        m.created_at, m.source_hash, m.source_app, f.gid, m.calendar_ct,
                        m.track_speakers_ct, m.lamport, m.origin, m.base_lamport, m.base_origin
                 FROM meetings m LEFT JOIN folders f ON f.id = m.folder_id
                 WHERE m.gid = ?1 AND m.dek_wrapped <> zeroblob(length(m.dek_wrapped))",
                [gid],
                |r| {
                    let id: i64 = r.get(0)?;
                    let audio_origin: Option<i64> = r.get(18)?;
                    Ok((
                        id,
                        MeetingRec {
                            gid: gid.to_string(),
                            title_ct: r.get::<_, Option<Vec<u8>>>(1)?.map(Bytes),
                            started_at: Some(r.get(2)?),
                            duration_ms: Some(r.get(3)?),
                            source: Some(r.get(4)?),
                            mode: Some(r.get(5)?),
                            lang: r.get(6)?,
                            template: r.get(7)?,
                            status: Some(r.get(8)?),
                            privacy_state: Some(r.get(9)?),
                            cloud_locked: Some(r.get(10)?),
                            sensitive: Some(r.get(11)?),
                            consent_confirmed: Some(r.get(12)?),
                            cloud_used: Some(r.get(13)?),
                            transcript_version: Some(r.get(14)?),
                            transcript_epoch: Some(r.get(15)?),
                            ai_epoch: Some(r.get(16)?),
                            audio_retained_until: r.get(17)?,
                            created_at: r.get(19)?,
                            source_hash: r.get(20)?,
                            source_app: r.get(21)?,
                            folder_gid: r.get(22)?,
                            calendar_ct: r.get::<_, Option<Vec<u8>>>(23)?.map(Bytes),
                            track_speakers_ct: r.get::<_, Option<Vec<u8>>>(24)?.map(Bytes),
                            ..Default::default()
                        },
                        audio_origin,
                        (r.get::<_, i64>(25)?, r.get::<_, Option<i64>>(26)?),
                        (r.get::<_, Option<i64>>(27)?, r.get::<_, Option<i64>>(28)?),
                    ))
                },
            )
            .optional()?;
        let Some((id, mut rec, audio_origin, (lamport, origin), (bl, bo))) = row else {
            return Ok(None);
        };
        rec.version = Version {
            lamport,
            origin: origin_gid(conn, origin, own)?,
        };
        rec.base = match bl {
            Some(l) => Some(Version {
                lamport: l,
                origin: origin_gid(conn, bo, own)?,
            }),
            None => None,
        };
        // NULL: recorded here (or imported here).
        rec.audio_origin = Some(origin_gid(conn, audio_origin, own)?);
        if with_dek {
            let dek = self.dek(conn, id)?;
            rec.dek = Some(Bytes(dek.as_bytes().to_vec()));
        }
        Ok(Some(rec))
    }
}

fn encode_track(conn: &Connection, own: &str, gid: &str) -> Result<Option<TrackRec>> {
    let row = conn
        .query_row(
            "SELECT m.gid, t.kind, t.page_count, t.cut_pages, t.lamport, t.origin,
                    t.base_lamport, t.base_origin
             FROM tracks t JOIN meetings m ON m.id = t.meeting_id WHERE t.gid = ?1",
            [gid],
            |r| {
                let (v, b) = stamp(conn, own, r, 4).map_err(to_sql)?;
                Ok(TrackRec {
                    gid: gid.to_string(),
                    version: v,
                    base: b,
                    meeting_gid: r.get(0)?,
                    kind: Some(r.get(1)?),
                    page_count: Some(r.get(2)?),
                    cut_pages: r.get(3)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn encode_person(conn: &Connection, own: &str, gid: &str) -> Result<Option<PersonRec>> {
    let row = conn
        .query_row(
            "SELECT name, color_slot, is_me, created_at, lamport, origin, base_lamport, base_origin
             FROM persons WHERE gid = ?1 AND is_me = 0",
            [gid],
            |r| {
                let (v, b) = stamp(conn, own, r, 4).map_err(to_sql)?;
                Ok(PersonRec {
                    gid: gid.to_string(),
                    version: v,
                    base: b,
                    name: Some(r.get(0)?),
                    color_slot: Some(r.get(1)?),
                    is_me: Some(r.get(2)?),
                    created_at: r.get(3)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn encode_speaker(conn: &Connection, own: &str, gid: &str) -> Result<Option<SpeakerRec>> {
    let row = conn
        .query_row(
            "SELECT m.gid, s.label_idx, s.display_name_ct,
                    CASE WHEN p.is_me = 0 THEN p.gid END, s.color_slot, s.is_me,
                    s.not_person, t.gid, s.lamport, s.origin, s.base_lamport, s.base_origin
             FROM speakers s JOIN meetings m ON m.id = s.meeting_id
             LEFT JOIN persons p ON p.id = s.person_id
             LEFT JOIN speakers t ON t.id = s.merged_into
             WHERE s.gid = ?1",
            [gid],
            |r| {
                let (v, b) = stamp(conn, own, r, 8).map_err(to_sql)?;
                Ok(SpeakerRec {
                    gid: gid.to_string(),
                    version: v,
                    base: b,
                    meeting_gid: r.get(0)?,
                    label_idx: Some(r.get(1)?),
                    display_name_ct: r.get::<_, Option<Vec<u8>>>(2)?.map(Bytes),
                    person_gid: r.get(3)?,
                    color_slot: Some(r.get(4)?),
                    is_me: Some(r.get(5)?),
                    not_person: Some(r.get(6)?),
                    merged_into: r.get(7)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn encode_segment(conn: &Connection, own: &str, gid: &str) -> Result<Option<SegmentRec>> {
    let row = conn
        .query_row(
            "SELECT s.id, m.gid, s.version, s.epoch, sp.gid, s.t0_ms, s.t1_ms, s.text_ct,
                    s.lang, s.confidence, s.edited, s.overlap, s.lamport, s.origin,
                    s.base_lamport, s.base_origin
             FROM segments s JOIN meetings m ON m.id = s.meeting_id
             LEFT JOIN speakers sp ON sp.id = s.speaker_id
             WHERE s.gid = ?1",
            [gid],
            |r| {
                let (v, b) = stamp(conn, own, r, 12).map_err(to_sql)?;
                Ok((
                    r.get::<_, i64>(0)?,
                    SegmentRec {
                        gid: gid.to_string(),
                        version: v,
                        base: b,
                        meeting_gid: r.get(1)?,
                        transcript_version: Some(r.get(2)?),
                        epoch: Some(r.get(3)?),
                        speaker_gid: r.get(4)?,
                        t0_ms: Some(r.get(5)?),
                        t1_ms: Some(r.get(6)?),
                        text_ct: Some(Bytes(r.get(7)?)),
                        lang: r.get(8)?,
                        confidence: r.get(9)?,
                        edited: Some(r.get(10)?),
                        overlap: Some(r.get(11)?),
                        words: None,
                    },
                ))
            },
        )
        .optional()?;
    let Some((id, mut rec)) = row else {
        return Ok(None);
    };
    let words = conn
        .prepare_cached("SELECT t0_ms, t1_ms, conf FROM words WHERE segment_id = ?1 ORDER BY idx")?
        .query_map([id], |r| {
            Ok(WordRec {
                t0_ms: r.get(0)?,
                t1_ms: r.get(1)?,
                conf: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rec.words = Some(words);
    Ok(Some(rec))
}

fn encode_note(conn: &Connection, own: &str, gid: &str) -> Result<Option<NoteRec>> {
    let row = conn
        .query_row(
            "SELECT m.gid, n.kind, n.provenance, n.body_ct, n.anchors_json, n.pinned, n.epoch,
                    n.ord, n.lamport, n.origin, n.base_lamport, n.base_origin
             FROM notes_blocks n JOIN meetings m ON m.id = n.meeting_id WHERE n.gid = ?1",
            [gid],
            |r| {
                let (v, b) = stamp(conn, own, r, 8).map_err(to_sql)?;
                Ok(NoteRec {
                    gid: gid.to_string(),
                    version: v,
                    base: b,
                    meeting_gid: r.get(0)?,
                    kind: Some(r.get(1)?),
                    provenance: Some(r.get(2)?),
                    body_ct: Some(Bytes(r.get(3)?)),
                    anchors_json: Some(r.get(4)?),
                    pinned: Some(r.get(5)?),
                    epoch: Some(r.get(6)?),
                    ord: r.get(7)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn encode_action(conn: &Connection, own: &str, gid: &str) -> Result<Option<ActionItemRec>> {
    let row = conn
        .query_row(
            "SELECT m.gid, a.text_ct, a.due_text_ct, s.gid, a.due, a.done, a.anchors_json,
                    a.provenance, a.epoch, a.ord, a.lamport, a.origin, a.base_lamport,
                    a.base_origin
             FROM action_items a JOIN meetings m ON m.id = a.meeting_id
             LEFT JOIN speakers s ON s.id = a.owner_speaker_id
             WHERE a.gid = ?1",
            [gid],
            |r| {
                let (v, b) = stamp(conn, own, r, 10).map_err(to_sql)?;
                Ok(ActionItemRec {
                    gid: gid.to_string(),
                    version: v,
                    base: b,
                    meeting_gid: r.get(0)?,
                    text_ct: Some(Bytes(r.get(1)?)),
                    due_text_ct: r.get::<_, Option<Vec<u8>>>(2)?.map(Bytes),
                    owner_speaker_gid: r.get(3)?,
                    due: r.get(4)?,
                    done: Some(r.get(5)?),
                    anchors_json: Some(r.get(6)?),
                    provenance: Some(r.get(7)?),
                    epoch: Some(r.get(8)?),
                    ord: r.get(9)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn encode_mark(conn: &Connection, own: &str, gid: &str) -> Result<Option<MarkRec>> {
    let row = conn
        .query_row(
            "SELECT m.gid, k.t_ms, k.tag, k.lamport, k.origin, k.base_lamport, k.base_origin
             FROM marks k JOIN meetings m ON m.id = k.meeting_id WHERE k.gid = ?1",
            [gid],
            |r| {
                let (v, b) = stamp(conn, own, r, 3).map_err(to_sql)?;
                Ok(MarkRec {
                    gid: gid.to_string(),
                    version: v,
                    base: b,
                    meeting_gid: r.get(0)?,
                    t_ms: Some(r.get(1)?),
                    tag: Some(r.get(2)?),
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn encode_folder(conn: &Connection, own: &str, gid: &str) -> Result<Option<FolderRec>> {
    let row = conn
        .query_row(
            "SELECT name, created_at, lamport, origin, base_lamport, base_origin
             FROM folders WHERE gid = ?1",
            [gid],
            |r| {
                let (v, b) = stamp(conn, own, r, 2).map_err(to_sql)?;
                Ok(FolderRec {
                    gid: gid.to_string(),
                    version: v,
                    base: b,
                    name: Some(r.get(0)?),
                    created_at: Some(r.get(1)?),
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn encode_tag(conn: &Connection, own: &str, gid: &str) -> Result<Option<TagRec>> {
    let row = conn
        .query_row(
            "SELECT name, created_at, lamport, origin, base_lamport, base_origin
             FROM tags WHERE gid = ?1",
            [gid],
            |r| {
                let (v, b) = stamp(conn, own, r, 2).map_err(to_sql)?;
                Ok(TagRec {
                    gid: gid.to_string(),
                    version: v,
                    base: b,
                    name: Some(r.get(0)?),
                    created_at: Some(r.get(1)?),
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn encode_link(conn: &Connection, own: &str, gid: &str) -> Result<Option<MeetingTagRec>> {
    let row = conn
        .query_row(
            "SELECT m.gid, t.gid, l.lamport, l.origin, l.base_lamport, l.base_origin
             FROM meeting_tags l JOIN meetings m ON m.id = l.meeting_id
             JOIN tags t ON t.id = l.tag_id WHERE l.gid = ?1",
            [gid],
            |r| {
                let (v, b) = stamp(conn, own, r, 2).map_err(to_sql)?;
                Ok(MeetingTagRec {
                    gid: gid.to_string(),
                    version: v,
                    base: b,
                    meeting_gid: r.get(0)?,
                    tag_gid: r.get(1)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn encode_copy(conn: &Connection, own: &str, gid: &str) -> Result<Option<ConflictCopyRec>> {
    let row = conn
        .query_row(
            "SELECT m.gid, c.target_kind, c.target_gid, c.field, c.value_ct, c.created_at,
                    c.lamport, c.origin, c.base_lamport, c.base_origin
             FROM conflict_copies c JOIN meetings m ON m.id = c.meeting_id WHERE c.gid = ?1",
            [gid],
            |r| {
                let (v, b) = stamp(conn, own, r, 6).map_err(to_sql)?;
                Ok(ConflictCopyRec {
                    gid: gid.to_string(),
                    version: v,
                    base: b,
                    meeting_gid: r.get(0)?,
                    target_kind: r.get(1)?,
                    target_gid: r.get(2)?,
                    field: r.get(3)?,
                    value_ct: Some(Bytes(r.get(4)?)),
                    created_at: Some(r.get(5)?),
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn encode_setting(conn: &Connection, own: &str, gid: &str) -> Result<Option<SettingRec>> {
    let row = conn
        .query_row(
            "SELECT key, value_json, lamport, origin, base_lamport, base_origin
             FROM synced_settings WHERE gid = ?1",
            [gid],
            |r| {
                let (v, b) = stamp(conn, own, r, 2).map_err(to_sql)?;
                Ok(SettingRec {
                    gid: gid.to_string(),
                    version: v,
                    base: b,
                    key: r.get(0)?,
                    value_json: Some(r.get(1)?),
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// Carries a store error through a rusqlite row closure.
fn to_sql(e: StoreError) -> rusqlite::Error {
    match e {
        StoreError::Db(e) => e,
        other => rusqlite::Error::ToSqlConversionFailure(Box::new(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_order_by_lamport_then_origin() {
        let v = |lamport, origin: &str| Version {
            lamport,
            origin: origin.into(),
        };
        assert!(v(2, "a") > v(1, "z"));
        assert!(v(2, "b") > v(2, "a"));
        assert_eq!(v(2, "a"), v(2, "a"));
    }

    #[test]
    fn kind_numbers_round_trip() {
        for n in 0..=255u8 {
            if let Some(k) = RecordKind::from_u8(n) {
                assert_eq!(k as u8, n);
            }
        }
        assert_eq!(RecordKind::from_u8(14), Some(RecordKind::Setting));
        assert_eq!(RecordKind::from_u8(15), None);
    }

    #[test]
    fn cause_names_round_trip() {
        for c in [
            TombCause::User,
            TombCause::Meeting,
            TombCause::Regenerate,
            TombCause::Discard,
            TombCause::Retention,
            TombCause::Transcript,
            TombCause::Superseded,
        ] {
            assert_eq!(TombCause::parse(c.as_str()), Some(c));
        }
    }

    #[test]
    fn bytes_debug_hides_content() {
        assert_eq!(format!("{:?}", Bytes(vec![1, 2, 3])), "Bytes(3 B)");
    }
}
