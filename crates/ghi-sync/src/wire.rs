// SPDX-License-Identifier: Apache-2.0
//! The messages of the session protocol (doc 07 §6, §5.2; slice 15-F).
//!
//! On the wire a message is CBOR `{ t: u8 type, id: u32 request id, b: body }`.
//! Versions: `proto = (major, minor)`; a minor adds optional fields only, and
//! **an absent field means keep the current value**. An unknown type gets
//! `Error{Unsupported}`; no common major gives `Error{UpgradeRequired}`.
//! Codes only, never content, in errors.

use serde::{Deserialize, Serialize};

use crate::{Result, not_yet};
use ghi_store::sync::records::{self, Bytes, SyncTombstone};

pub use ghi_store::sync::apply::ApplyOutcome;

/// The protocol version this build speaks.
pub const PROTO: Proto = Proto { major: 1, minor: 0 };
/// Majors this build offers (and puts in the prologue).
pub const MAJORS: [u8; 1] = [1];

/// Most records in a batch, and their bytes (doc 07 §5.3).
pub const MAX_BATCH_RECORDS: usize = 256;
pub const MAX_BATCH_BYTES: usize = 1024 * 1024;
/// One text field, plaintext.
pub const MAX_TEXT_FIELD: usize = 256 * 1024;
/// Audio pages per `TrackPages`, and bytes per message.
pub const MAX_TRACK_PAGES: usize = 64;
pub const MAX_TRACK_PAGES_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proto {
    pub major: u8,
    pub minor: u8,
}

/// One syncable row on the wire: the store's typed record (a meeting record
/// may carry the DEK, inside the Noise channel only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Record(pub records::Record);

impl From<records::Record> for Record {
    fn from(r: records::Record) -> Self {
        Self(r)
    }
}

/// Machine-readable error codes (never content).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// An unknown message type; ignored.
    Unsupported,
    /// No common major version.
    UpgradeRequired,
    /// A record failed validation; the whole batch was rejected.
    BadRecord,
    StorageFull,
    /// More than 10 meeting deletes arrived: the user must confirm (D13).
    NeedsConfirm,
    /// Too many sessions or a session already running for this device.
    Busy,
    Internal,
}

/// Why a track or lease request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefuseReason {
    Deleted,
    StorageFull,
    NoKey,
    Unsupported,
}

/// The type byte of a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    Hello = 1,
    HelloOk = 2,
    Control = 3,
    Ok = 4,
    WipeDone = 5,
    PushTombs = 6,
    PushRows = 7,
    Ack = 8,
    ProcessRequest = 9,
    ProcessAccept = 10,
    Refuse = 11,
    LeaseRevoke = 12,
    Revoked = 13,
    AlreadyDone = 14,
    LeaseStatus = 15,
    LeaseStatusReply = 16,
    TrackOffer = 17,
    TrackHave = 18,
    TrackPages = 19,
    TrackAck = 20,
    PullTombs = 21,
    PullRows = 22,
    Tombs = 23,
    Rows = 24,
    Ping = 25,
    Pong = 26,
    Bye = 27,
    Error = 28,
    PairHello = 29,
    PairAccept = 30,
    PairDone = 31,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    /// Every major the sender speaks; the highest common one wins.
    pub proto: Vec<Proto>,
    pub app_version: String,
    pub device_gid: String,
    pub feed_id: String,
    /// The sender's cursor into the receiver's feed.
    pub pull_cursor: i64,
    pub caps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloOk {
    pub proto: Proto,
    pub device_gid: String,
    pub feed_id: String,
    /// Control commands waiting for the spoke.
    pub pending: Vec<Control>,
}

/// Unpair and wipe commands (doc 07 §3.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Control {
    Wipe { reason: String },
    Unpair,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushTombs {
    pub tombs: Vec<SyncTombstone>,
    pub upto_seq: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PushRows {
    pub rows: Vec<Record>,
    pub upto_seq: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ack {
    pub upto_seq: i64,
    /// Tombstone gids that were refused.
    pub rejected: Vec<String>,
    /// Per-row outcome of a `PushRows`.
    pub results: Vec<(String, ApplyOutcome)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessRequest {
    pub job_uuid: String,
    pub meeting_gid: String,
    pub epoch: i64,
    /// `final_pass`, `notes_live`, `notes_final`.
    pub kinds: Vec<String>,
    pub ttl_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaseQuery {
    pub meeting_gid: String,
    pub epoch: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaseInfo {
    pub meeting_gid: String,
    /// `queued | running | done | revoked | expired`.
    pub state: String,
    pub progress: f64,
    pub job_uuid: String,
}

/// Header of a bundle: magic, version and the 19-byte nonce prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleHeader {
    pub magic: Bytes,
    pub version: u8,
    pub prefix: Bytes,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackOffer {
    pub track_gid: String,
    pub meeting_gid: String,
    pub header: BundleHeader,
    pub pages: u64,
    pub bytes: u64,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackHave {
    /// Contiguous verified pages already held.
    pub have: u64,
    /// The receiver holds the whole track whatever the prefix (advisor 3).
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackPages {
    pub track_gid: String,
    pub prefix: Bytes,
    pub first: u64,
    pub records: Vec<Bytes>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackAck {
    pub have: u64,
}

/// `PullTombs` / `PullRows`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pull {
    pub feed_id: String,
    pub since_seq: i64,
    pub max: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TombsBatch {
    pub tombs: Vec<SyncTombstone>,
    pub upto_seq: i64,
    pub more: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RowsBatch {
    pub rows: Vec<Record>,
    pub upto_seq: i64,
    pub more: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    /// A second, finer code (never content): which side must update, a gid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairHello {
    pub device_gid: String,
    pub name: String,
    pub platform: String,
    pub proto: Vec<Proto>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairAccept {
    pub device_gid: String,
    pub name: String,
    /// The pair's long-term PSK, from now on mixed into every handshake.
    pub pair_psk: Bytes,
    pub port: u16,
}

/// A session message.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Hello(Hello),
    HelloOk(HelloOk),
    Control(Control),
    Ok,
    WipeDone,
    PushTombs(PushTombs),
    PushRows(PushRows),
    Ack(Ack),
    ProcessRequest(ProcessRequest),
    ProcessAccept {
        job_uuid: String,
        epoch: i64,
    },
    Refuse(RefuseReason),
    LeaseRevoke(LeaseQuery),
    Revoked {
        epoch: i64,
    },
    AlreadyDone {
        job_uuid: String,
    },
    LeaseStatus(Vec<LeaseQuery>),
    LeaseStatusReply(Vec<LeaseInfo>),
    TrackOffer(TrackOffer),
    TrackHave(TrackHave),
    TrackPages(TrackPages),
    TrackAck(TrackAck),
    PullTombs(Pull),
    PullRows(Pull),
    Tombs(TombsBatch),
    Rows(RowsBatch),
    Ping,
    /// `dirty`: the hub has new changes for the spoke.
    Pong {
        dirty: bool,
    },
    Bye,
    Error(ErrorBody),
    PairHello(PairHello),
    PairAccept(PairAccept),
    PairDone,
}

impl Message {
    /// The type byte this message is sent with.
    pub fn msg_type(&self) -> MsgType {
        match self {
            Message::Hello(_) => MsgType::Hello,
            Message::HelloOk(_) => MsgType::HelloOk,
            Message::Control(_) => MsgType::Control,
            Message::Ok => MsgType::Ok,
            Message::WipeDone => MsgType::WipeDone,
            Message::PushTombs(_) => MsgType::PushTombs,
            Message::PushRows(_) => MsgType::PushRows,
            Message::Ack(_) => MsgType::Ack,
            Message::ProcessRequest(_) => MsgType::ProcessRequest,
            Message::ProcessAccept { .. } => MsgType::ProcessAccept,
            Message::Refuse(_) => MsgType::Refuse,
            Message::LeaseRevoke(_) => MsgType::LeaseRevoke,
            Message::Revoked { .. } => MsgType::Revoked,
            Message::AlreadyDone { .. } => MsgType::AlreadyDone,
            Message::LeaseStatus(_) => MsgType::LeaseStatus,
            Message::LeaseStatusReply(_) => MsgType::LeaseStatusReply,
            Message::TrackOffer(_) => MsgType::TrackOffer,
            Message::TrackHave(_) => MsgType::TrackHave,
            Message::TrackPages(_) => MsgType::TrackPages,
            Message::TrackAck(_) => MsgType::TrackAck,
            Message::PullTombs(_) => MsgType::PullTombs,
            Message::PullRows(_) => MsgType::PullRows,
            Message::Tombs(_) => MsgType::Tombs,
            Message::Rows(_) => MsgType::Rows,
            Message::Ping => MsgType::Ping,
            Message::Pong { .. } => MsgType::Pong,
            Message::Bye => MsgType::Bye,
            Message::Error(_) => MsgType::Error,
            Message::PairHello(_) => MsgType::PairHello,
            Message::PairAccept(_) => MsgType::PairAccept,
            Message::PairDone => MsgType::PairDone,
        }
    }
}

/// Encodes `{ t, id, b }` as CBOR.
pub fn encode(_id: u32, _msg: &Message) -> Result<Vec<u8>> {
    not_yet("wire::encode")
}

/// Decodes one message. An unknown type is an error the caller answers with
/// `Error{Unsupported}`; the size caps are checked before allocating.
pub fn decode(_bytes: &[u8]) -> Result<(u32, Message)> {
    not_yet("wire::decode")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_is_cbor_serializable_without_loss() {
        // The wrapper is transparent: the store's record encodes as itself.
        let rec = Record(records::Record::Mark(records::MarkRec {
            gid: "g".into(),
            meeting_gid: "m".into(),
            t_ms: Some(5),
            ..Default::default()
        }));
        let mut bytes = Vec::new();
        ciborium::into_writer(&rec, &mut bytes).unwrap();
        let back: Record = ciborium::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(back, rec);
    }

    #[test]
    fn absent_optional_fields_stay_absent_and_cost_nothing() {
        let tombless = records::MarkRec {
            gid: "g".into(),
            meeting_gid: "m".into(),
            ..Default::default()
        };
        let mut a = Vec::new();
        ciborium::into_writer(&tombless, &mut a).unwrap();
        let back: records::MarkRec = ciborium::from_reader(a.as_slice()).unwrap();
        assert_eq!((back.t_ms, back.tag), (None, None));
        let with_tag = records::MarkRec {
            tag: Some("star".into()),
            ..tombless
        };
        let mut b = Vec::new();
        ciborium::into_writer(&with_tag, &mut b).unwrap();
        assert!(b.len() > a.len());
    }

    #[test]
    fn ciphertext_is_a_cbor_byte_string() {
        let one = Bytes(vec![0xAB; 100]);
        let mut out = Vec::new();
        ciborium::into_writer(&one, &mut out).unwrap();
        // 0x58 0x64 = byte string of 100 bytes: no per-byte overhead.
        assert_eq!(&out[..2], &[0x58, 100]);
        assert_eq!(out.len(), 102);
    }

    #[test]
    fn message_types_are_distinct() {
        let msgs = [
            Message::Ok,
            Message::WipeDone,
            Message::Ping,
            Message::Bye,
            Message::PairDone,
            Message::Pong { dirty: true },
        ];
        let mut seen: Vec<u8> = msgs.iter().map(|m| m.msg_type() as u8).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), msgs.len());
    }
}
