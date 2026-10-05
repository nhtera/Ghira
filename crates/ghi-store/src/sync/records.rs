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
