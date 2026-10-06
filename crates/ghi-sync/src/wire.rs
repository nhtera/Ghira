// SPDX-License-Identifier: Apache-2.0
//! The messages of the session protocol (doc 07 §6, §5.2; slice 15-F).
//!
//! On the wire a message is CBOR `{ t: u8 type, id: u32 request id, b: body }`.
//! Versions: `proto = (major, minor)`; a minor adds optional fields only, and
//! **an absent field means keep the current value**. An unknown type gets
//! `Error{Unsupported}`; no common major gives `Error{UpgradeRequired}`.
//! Codes only, never content, in errors.

use ciborium::value::Value;
use serde::{Deserialize, Serialize};

use crate::{Result, SyncError};
use ghi_store::sync::records::{self, Bytes};
use zeroize::Zeroize;

pub use ghi_store::sync::apply::ApplyOutcome;
pub use ghi_store::sync::records::SyncTombstone;

/// Helpers on the store's tombstone type.
pub trait SyncTombstoneExt {
    /// A meeting delete the mass-delete guard counts (retention is exempt).
    fn is_counted_meeting_delete(&self) -> bool;
}

impl SyncTombstoneExt for SyncTombstone {
    fn is_counted_meeting_delete(&self) -> bool {
        self.kind == "meeting" && self.cause != Some(records::TombCause::Retention)
    }
}

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

/// Largest app message (the transport's cap): checked before decoding.
const MAX_MESSAGE: usize = crate::transport::MAX_MESSAGE;
/// Slack over the 1 MiB batch cap for the envelope and per-record framing.
const BATCH_SLACK: usize = 64 * 1024;
/// Sanity caps on small lists.
const MAX_LIST: usize = 1024;

#[derive(Serialize, Deserialize)]
struct ProcessAcceptBody {
    job_uuid: String,
    epoch: i64,
}

#[derive(Serialize, Deserialize)]
struct RevokedBody {
    epoch: i64,
}

#[derive(Serialize, Deserialize)]
struct AlreadyDoneBody {
    job_uuid: String,
}

#[derive(Serialize, Deserialize)]
struct PongBody {
    #[serde(default)]
    dirty: bool,
}

fn wire_err(what: &str) -> SyncError {
    SyncError::Wire(what.to_string())
}

fn to_value<T: Serialize>(body: &T) -> Result<Value> {
    Value::serialized(body).map_err(|_| wire_err("unencodable body"))
}

fn body_value(msg: &Message) -> Result<Value> {
    match msg {
        Message::Hello(b) => to_value(b),
        Message::HelloOk(b) => to_value(b),
        Message::Control(b) => to_value(b),
        Message::PushTombs(b) => to_value(b),
        Message::PushRows(b) => to_value(b),
        Message::Ack(b) => to_value(b),
        Message::ProcessRequest(b) => to_value(b),
        Message::ProcessAccept { job_uuid, epoch } => to_value(&ProcessAcceptBody {
            job_uuid: job_uuid.clone(),
            epoch: *epoch,
        }),
        Message::Refuse(b) => to_value(b),
        Message::LeaseRevoke(b) => to_value(b),
        Message::Revoked { epoch } => to_value(&RevokedBody { epoch: *epoch }),
        Message::AlreadyDone { job_uuid } => to_value(&AlreadyDoneBody {
            job_uuid: job_uuid.clone(),
        }),
        Message::LeaseStatus(b) => to_value(b),
        Message::LeaseStatusReply(b) => to_value(b),
        Message::TrackOffer(b) => to_value(b),
        Message::TrackHave(b) => to_value(b),
        Message::TrackPages(b) => to_value(b),
        Message::TrackAck(b) => to_value(b),
        Message::PullTombs(b) | Message::PullRows(b) => to_value(b),
        Message::Tombs(b) => to_value(b),
        Message::Rows(b) => to_value(b),
        Message::Pong { dirty } => to_value(&PongBody { dirty: *dirty }),
        Message::Error(b) => to_value(b),
        Message::PairHello(b) => to_value(b),
        Message::PairAccept(b) => to_value(b),
        Message::Ok | Message::WipeDone | Message::Ping | Message::Bye | Message::PairDone => {
            Ok(Value::Null)
        }
    }
}

/// Encodes `{ t, id, b }` as CBOR. Refuses a message over the 4 MiB cap.
pub fn encode(id: u32, msg: &Message) -> Result<Vec<u8>> {
    let envelope = Value::Map(vec![
        (
            Value::Text("t".into()),
            Value::Integer((msg.msg_type() as u8).into()),
        ),
        (Value::Text("id".into()), Value::Integer(id.into())),
        (Value::Text("b".into()), body_value(msg)?),
    ]);
    let mut out = Vec::new();
    ciborium::into_writer(&envelope, &mut out).map_err(|_| wire_err("unencodable message"))?;
    if out.len() > MAX_MESSAGE {
        return Err(wire_err("message too large"));
    }
    Ok(out)
}

/// What a received frame was.
#[derive(Debug, Clone, PartialEq)]
pub enum Decoded {
    Known(Message),
    /// A type this build doesn't know (a newer minor): answer
    /// `Error{Unsupported}` and carry on.
    Unknown(u8),
}

fn de<T: serde::de::DeserializeOwned>(body: &Value) -> Result<T> {
    body.deserialized::<T>()
        .map_err(|_| wire_err("malformed body"))
}

/// Overwrites every byte string in a decoded CBOR value (a received body may
/// carry a meeting key or a pair PSK).
fn wipe_value(v: &mut Value) {
    match v {
        Value::Bytes(b) => b.zeroize(),
        Value::Array(a) => a.iter_mut().for_each(wipe_value),
        Value::Map(m) => m.iter_mut().for_each(|(k, v)| {
            wipe_value(k);
            wipe_value(v);
        }),
        Value::Tag(_, inner) => wipe_value(inner),
        _ => {}
    }
}

/// Overwrites the meeting keys carried by `rows` (after they were applied or
/// sent): a key must not outlive its use in memory.
pub fn wipe_records(rows: &mut [records::Record]) {
    for r in rows {
        if let records::Record::Meeting(m) = r
            && let Some(d) = m.dek.as_mut()
        {
            d.0.zeroize();
        }
    }
}

impl Message {
    /// Overwrites the secrets a message carries (meeting keys in row batches,
    /// the pair PSK of `PairAccept`). Call after it was sent.
    pub fn wipe_secrets(&mut self) {
        match self {
            Message::PushRows(b) => b
                .rows
                .iter_mut()
                .for_each(|r| wipe_records(std::slice::from_mut(&mut r.0))),
            Message::Rows(b) => b
                .rows
                .iter_mut()
                .for_each(|r| wipe_records(std::slice::from_mut(&mut r.0))),
            Message::PairAccept(a) => a.pair_psk.0.zeroize(),
            _ => {}
        }
    }
}

/// Most CBOR items one message may hold (a 4 MiB message of one-byte items
/// would otherwise expand about 50x into [`Value`]s).
const MAX_ITEMS: u64 = 1_000_000;
/// Deepest nesting a message may have.
const MAX_DEPTH: usize = 24;
/// Containers at the envelope, body and list-field levels may not declare
/// more than this many entries (the exact caps are checked after decoding).
const MAX_TOP_LEN: u64 = MAX_LIST as u64;

/// Walks the CBOR headers of `bytes` without building anything and refuses
/// what would be costly to decode: too many items, too deep, a top-level list
/// declared far over its cap, indefinite lengths, a length that runs past the
/// input, or trailing bytes.
fn prescan(bytes: &[u8]) -> Result<()> {
    let bad = || wire_err("not a CBOR message");
    let mut pos = 0usize;
    let mut items = 0u64;
    // Entries still to read in each open container.
    let mut open: Vec<u64> = Vec::new();
    loop {
        // A finished container pops (also for the root once it is done).
        while open.last() == Some(&0) {
            open.pop();
        }
        if pos == bytes.len() {
            return if open.is_empty() && items > 0 {
                Ok(())
            } else {
                Err(bad())
            };
        }
        if open.is_empty() && items > 0 {
            return Err(bad()); // trailing bytes
        }
        let head = bytes[pos];
        pos += 1;
        let (major, info) = (head >> 5, head & 0x1f);
        let arg_len = match info {
            0..=23 => 0,
            24 => 1,
            25 => 2,
            26 => 4,
            27 => 8,
            _ => return Err(bad()), // indefinite lengths and reserved values
        };
        let end = pos
            .checked_add(arg_len)
            .filter(|e| *e <= bytes.len())
            .ok_or_else(bad)?;
        let arg = if info < 24 {
            u64::from(info)
        } else {
            bytes[pos..end]
                .iter()
                .fold(0u64, |a, b| (a << 8) | u64::from(*b))
        };
        pos = end;
        if let Some(n) = open.last_mut() {
            *n -= 1;
        }
        items += 1;
        if items > MAX_ITEMS {
            return Err(wire_err("message has too many items"));
        }
        match major {
            // Strings: the payload is skipped, and must be there.
            2 | 3 => {
                let stop = usize::try_from(arg)
                    .ok()
                    .and_then(|l| pos.checked_add(l))
                    .filter(|e| *e <= bytes.len())
                    .ok_or_else(bad)?;
                pos = stop;
            }
            4 | 5 => {
                let n = if major == 5 {
                    arg.checked_mul(2).ok_or_else(bad)?
                } else {
                    arg
                };
                // Every entry takes at least one byte.
                if n > (bytes.len() - pos) as u64 {
                    return Err(bad());
                }
                if open.len() < 3 && arg > MAX_TOP_LEN {
                    return Err(wire_err("list over its cap"));
                }
                if open.len() >= MAX_DEPTH {
                    return Err(wire_err("message nests too deep"));
                }
                open.push(n);
            }
            // A tag wraps the next item.
            6 => {
                open.push(1);
                items -= 1;
            }
            _ => {}
        }
    }
}

/// Decodes one frame, keeping unknown types apart. The size cap and the list
/// caps are checked before anything is acted on.
pub fn decode_any(bytes: &[u8]) -> Result<(u32, Decoded)> {
    if bytes.len() > MAX_MESSAGE {
        return Err(wire_err("message too large"));
    }
    prescan(bytes)?;
    let value: Value = ciborium::from_reader(bytes).map_err(|_| wire_err("not a CBOR message"))?;
    let Value::Map(entries) = value else {
        return Err(wire_err("envelope is not a map"));
    };
    let (mut t, mut id, mut body) = (None, None, Value::Null);
    // Unknown envelope keys are skipped (a later minor may add some).
    for (k, v) in entries {
        match (k, v) {
            (Value::Text(k), Value::Integer(n)) if k == "t" => t = u8::try_from(n).ok(),
            (Value::Text(k), Value::Integer(n)) if k == "id" => id = u32::try_from(n).ok(),
            (Value::Text(k), v) if k == "b" => body = v,
            _ => {}
        }
    }
    let t = t.ok_or_else(|| wire_err("missing type"))?;
    let id = id.ok_or_else(|| wire_err("missing id"))?;
    let msg = message_from(t, &body);
    wipe_value(&mut body);
    let Some(msg) = msg? else {
        return Ok((id, Decoded::Unknown(t)));
    };
    validate(&msg, bytes.len())?;
    Ok((id, Decoded::Known(msg)))
}

fn message_from(t: u8, b: &Value) -> Result<Option<Message>> {
    Ok(Some(match t {
        1 => Message::Hello(de(b)?),
        2 => Message::HelloOk(de(b)?),
        3 => Message::Control(de(b)?),
        4 => Message::Ok,
        5 => Message::WipeDone,
        6 => Message::PushTombs(de(b)?),
        7 => Message::PushRows(de(b)?),
        8 => Message::Ack(de(b)?),
        9 => Message::ProcessRequest(de(b)?),
        10 => {
            let p: ProcessAcceptBody = de(b)?;
            Message::ProcessAccept {
                job_uuid: p.job_uuid,
                epoch: p.epoch,
            }
        }
        11 => Message::Refuse(de(b)?),
        12 => Message::LeaseRevoke(de(b)?),
        13 => Message::Revoked {
            epoch: de::<RevokedBody>(b)?.epoch,
        },
        14 => Message::AlreadyDone {
            job_uuid: de::<AlreadyDoneBody>(b)?.job_uuid,
        },
        15 => Message::LeaseStatus(de(b)?),
        16 => Message::LeaseStatusReply(de(b)?),
        17 => Message::TrackOffer(de(b)?),
        18 => Message::TrackHave(de(b)?),
        19 => Message::TrackPages(de(b)?),
        20 => Message::TrackAck(de(b)?),
        21 => Message::PullTombs(de(b)?),
        22 => Message::PullRows(de(b)?),
        23 => Message::Tombs(de(b)?),
        24 => Message::Rows(de(b)?),
        25 => Message::Ping,
        26 => Message::Pong {
            dirty: de::<PongBody>(b)?.dirty,
        },
        27 => Message::Bye,
        28 => Message::Error(de(b)?),
        29 => Message::PairHello(de(b)?),
        30 => Message::PairAccept(de(b)?),
        31 => Message::PairDone,
        _ => return Ok(None),
    }))
}

/// Caps on counts and sizes (doc 07 §5.3), applied to what a peer sent.
fn validate(msg: &Message, frame_len: usize) -> Result<()> {
    let batch = |n: usize| -> Result<()> {
        if n > MAX_BATCH_RECORDS || frame_len > MAX_BATCH_BYTES + BATCH_SLACK {
            Err(wire_err("batch too large"))
        } else {
            Ok(())
        }
    };
    match msg {
        Message::PushTombs(b) => batch(b.tombs.len()),
        Message::PushRows(b) => batch(b.rows.len()),
        Message::Tombs(b) => batch(b.tombs.len()),
        Message::Rows(b) => batch(b.rows.len()),
        Message::Ack(b)
            if b.results.len() > MAX_BATCH_RECORDS || b.rejected.len() > MAX_BATCH_RECORDS =>
        {
            Err(wire_err("ack too large"))
        }
        Message::TrackPages(b) => {
            let bytes: usize = b.records.iter().map(|r| r.0.len()).sum();
            if b.records.len() > MAX_TRACK_PAGES || bytes > MAX_TRACK_PAGES_BYTES {
                Err(wire_err("too many pages"))
            } else {
                Ok(())
            }
        }
        Message::Hello(h) if h.proto.len() > MAX_LIST || h.caps.len() > MAX_LIST => {
            Err(wire_err("list too long"))
        }
        Message::PairHello(h) if h.proto.len() > MAX_LIST => Err(wire_err("list too long")),
        Message::HelloOk(h) if h.pending.len() > MAX_LIST => Err(wire_err("list too long")),
        Message::LeaseStatus(l) if l.len() > MAX_LIST => Err(wire_err("list too long")),
        Message::LeaseStatusReply(l) if l.len() > MAX_LIST => Err(wire_err("list too long")),
        _ => Ok(()),
    }
}

/// Decodes one message. An unknown type is an error here (use
/// [`decode_any`] to answer it with `Error{Unsupported}`).
pub fn decode(bytes: &[u8]) -> Result<(u32, Message)> {
    match decode_any(bytes)? {
        (id, Decoded::Known(m)) => Ok((id, m)),
        (_, Decoded::Unknown(_)) => Err(wire_err("unsupported message type")),
    }
}

/// The highest major both sides speak; its minor is the lower of the two (a
/// minor only adds optional fields, so the older one's set is the common one).
pub fn negotiate(ours: &[Proto], theirs: &[Proto]) -> Option<Proto> {
    ours.iter()
        .filter_map(|o| {
            theirs
                .iter()
                .filter(|t| t.major == o.major)
                .map(|t| Proto {
                    major: o.major,
                    minor: o.minor.min(t.minor),
                })
                .min_by_key(|p| p.minor)
        })
        .max_by_key(|p| p.major)
}

/// Which side must update when [`negotiate`] fails: `"hub"` or `"spoke"`.
/// `we_are_hub` says which one `ours` is.
pub fn upgrade_side(ours: &[Proto], theirs: &[Proto], we_are_hub: bool) -> &'static str {
    let top = |l: &[Proto]| l.iter().map(|p| p.major).max().unwrap_or(0);
    let we_are_older = top(ours) < top(theirs);
    match (we_are_hub, we_are_older) {
        (true, true) | (false, false) => "hub",
        _ => "spoke",
    }
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

    fn sample_messages() -> Vec<Message> {
        let tomb = SyncTombstone {
            gid: "t".into(),
            kind: "meeting".into(),
            lamport: 4,
            origin: "d".into(),
            cause: Some(records::TombCause::Retention),
        };
        let mark = Record(records::Record::Mark(records::MarkRec {
            gid: "g".into(),
            meeting_gid: "m".into(),
            t_ms: Some(5),
            ..Default::default()
        }));
        let q = LeaseQuery {
            meeting_gid: "m".into(),
            epoch: 3,
        };
        vec![
            Message::Hello(Hello {
                proto: vec![PROTO],
                app_version: "1".into(),
                device_gid: "d".into(),
                feed_id: "f".into(),
                pull_cursor: 7,
                caps: vec!["x".into()],
            }),
            Message::HelloOk(HelloOk {
                proto: PROTO,
                device_gid: "d".into(),
                feed_id: "f".into(),
                pending: vec![Control::Wipe { reason: "r".into() }, Control::Unpair],
            }),
            Message::Control(Control::Unpair),
            Message::Control(Control::Wipe { reason: "r".into() }),
            Message::Ok,
            Message::WipeDone,
            Message::PushTombs(PushTombs {
                tombs: vec![tomb.clone()],
                upto_seq: 9,
            }),
            Message::PushRows(PushRows {
                rows: vec![mark.clone()],
                upto_seq: 9,
            }),
            Message::Ack(Ack {
                upto_seq: 9,
                rejected: vec!["x".into()],
                results: vec![("g".into(), ApplyOutcome::Merged)],
            }),
            Message::ProcessRequest(ProcessRequest {
                job_uuid: "j".into(),
                meeting_gid: "m".into(),
                epoch: 1,
                kinds: vec!["final_pass".into()],
                ttl_ms: 5,
            }),
            Message::ProcessAccept {
                job_uuid: "j".into(),
                epoch: 1,
            },
            Message::Refuse(RefuseReason::StorageFull),
            Message::LeaseRevoke(q.clone()),
            Message::Revoked { epoch: 2 },
            Message::AlreadyDone {
                job_uuid: "j".into(),
            },
            Message::LeaseStatus(vec![q]),
            Message::LeaseStatusReply(vec![LeaseInfo {
                meeting_gid: "m".into(),
                state: "running".into(),
                progress: 0.5,
                job_uuid: "j".into(),
            }]),
            Message::TrackOffer(TrackOffer {
                track_gid: "t".into(),
                meeting_gid: "m".into(),
                header: BundleHeader {
                    magic: Bytes(b"GHB".to_vec()),
                    version: 1,
                    prefix: Bytes(vec![1; 19]),
                },
                pages: 10,
                bytes: 100,
                complete: true,
            }),
            Message::TrackHave(TrackHave {
                have: 3,
                complete: false,
            }),
            Message::TrackPages(TrackPages {
                track_gid: "t".into(),
                prefix: Bytes(vec![1; 19]),
                first: 3,
                records: vec![Bytes(vec![9; 40])],
            }),
            Message::TrackAck(TrackAck { have: 4 }),
            Message::PullTombs(Pull {
                feed_id: "f".into(),
                since_seq: 1,
                max: 256,
            }),
            Message::PullRows(Pull {
                feed_id: "f".into(),
                since_seq: 1,
                max: 0,
            }),
            Message::Tombs(TombsBatch {
                tombs: vec![tomb],
                upto_seq: 3,
                more: true,
            }),
            Message::Rows(RowsBatch {
                rows: vec![mark],
                upto_seq: 3,
                more: false,
            }),
            Message::Ping,
            Message::Pong { dirty: true },
            Message::Bye,
            Message::Error(ErrorBody {
                code: ErrorCode::NeedsConfirm,
                detail: Some("11".into()),
            }),
            Message::PairHello(PairHello {
                device_gid: "d".into(),
                name: "n".into(),
                platform: "ios".into(),
                proto: vec![PROTO],
            }),
            Message::PairAccept(PairAccept {
                device_gid: "d".into(),
                name: "n".into(),
                pair_psk: Bytes(vec![7; 32]),
                port: 4000,
            }),
            Message::PairDone,
        ]
    }

    #[test]
    fn every_message_round_trips_with_its_id() {
        let all = sample_messages();
        let mut types: Vec<u8> = all.iter().map(|m| m.msg_type() as u8).collect();
        types.sort_unstable();
        types.dedup();
        assert_eq!(types.len(), 31, "every type is covered");
        for (i, m) in all.iter().enumerate() {
            let bytes = encode(i as u32 + 1, m).unwrap();
            let (id, back) = decode(&bytes).unwrap_or_else(|e| panic!("{:?}: {e}", m.msg_type()));
            assert_eq!((id, &back), (i as u32 + 1, m));
        }
    }

    fn envelope(t: u64, extra: Vec<(Value, Value)>, body: Value) -> Vec<u8> {
        let mut entries = vec![
            (Value::Text("t".into()), Value::Integer(t.into())),
            (Value::Text("id".into()), Value::Integer(1.into())),
            (Value::Text("b".into()), body),
        ];
        entries.extend(extra);
        let mut out = Vec::new();
        ciborium::into_writer(&Value::Map(entries), &mut out).unwrap();
        out
    }

    #[test]
    fn an_unknown_type_is_reported_not_fatal() {
        let bytes = envelope(200, vec![], Value::Null);
        assert_eq!(decode_any(&bytes).unwrap(), (1, Decoded::Unknown(200)));
        assert!(decode(&bytes).is_err());
    }

    #[test]
    fn unknown_fields_are_ignored_and_absent_optional_fields_are_none() {
        // A newer minor adds a body field and an envelope key.
        let body = Value::Map(vec![
            (Value::Text("have".into()), Value::Integer(5.into())),
            (Value::Text("complete".into()), Value::Bool(true)),
            (
                Value::Text("added_in_minor_1".into()),
                Value::Integer(1.into()),
            ),
        ]);
        let bytes = envelope(18, vec![(Value::Text("x".into()), Value::Null)], body);
        let (_, m) = decode(&bytes).unwrap();
        assert_eq!(
            m,
            Message::TrackHave(TrackHave {
                have: 5,
                complete: true
            })
        );
        // A record from an older minor lacks a field: it decodes as absent
        // (keep), not as zero.
        let old = Value::Map(vec![
            (Value::Text("gid".into()), Value::Text("g".into())),
            (
                Value::Text("version".into()),
                Value::Map(vec![
                    (Value::Text("lamport".into()), Value::Integer(1.into())),
                    (Value::Text("origin".into()), Value::Text("o".into())),
                ]),
            ),
            (Value::Text("meeting_gid".into()), Value::Text("m".into())),
            (Value::Text("future_field".into()), Value::Bool(true)),
        ]);
        let rec: records::MarkRec = old.deserialized().unwrap();
        assert_eq!((rec.t_ms, rec.tag), (None, None));
    }

    #[test]
    fn malformed_and_oversized_input_is_refused_before_it_is_used() {
        assert!(decode(b"").is_err());
        assert!(decode(&[0xff, 0x00]).is_err());
        // Not a map.
        let mut arr = Vec::new();
        ciborium::into_writer(&Value::Array(vec![]), &mut arr).unwrap();
        assert!(decode(&arr).is_err());
        // Missing id.
        let mut no_id = Vec::new();
        ciborium::into_writer(
            &Value::Map(vec![(Value::Text("t".into()), Value::Integer(4.into()))]),
            &mut no_id,
        )
        .unwrap();
        assert!(decode(&no_id).is_err());
        // Wrong body type.
        assert!(decode(&envelope(18, vec![], Value::Text("x".into()))).is_err());
        // Over the 4 MiB cap.
        assert!(decode(&vec![0u8; MAX_MESSAGE + 1]).is_err());
    }

    /// `{ t: 7, id: 1, b: { rows: <rows> } }` by hand, around raw CBOR.
    fn raw_push_rows(rows: &[u8]) -> Vec<u8> {
        let mut out = vec![0xa3, 0x61, b't', 0x07, 0x62, b'i', b'd', 0x01, 0x61, b'b'];
        out.extend_from_slice(&[0xa1, 0x64, b'r', b'o', b'w', b's']);
        out.extend_from_slice(rows);
        out
    }

    #[test]
    fn the_item_count_nesting_and_list_lengths_are_checked_before_decoding() {
        // A row list declared far over its cap, with the bytes to back it.
        let mut over = vec![0x99, 0x10, 0x00]; // array of 4096
        over.extend(std::iter::repeat(0x80).take(4096));
        let err = decode(&raw_push_rows(&over)).unwrap_err();
        assert!(err.to_string().contains("cap"), "{err}");

        // A tiny message of nested arrays: too deep.
        let mut deep = vec![0x81; 200];
        deep.push(0x00);
        assert!(prescan(&deep).is_err());

        // 256 rows of 12 000 one-byte items: 3 M items in 3 MB, refused by
        // count before a single Value exists.
        let mut rows = vec![0x99, 0x01, 0x00];
        for _ in 0..256 {
            rows.extend_from_slice(&[0x99, 0x2e, 0xe0]);
            rows.extend(std::iter::repeat(0x00).take(12_000));
        }
        let msg = raw_push_rows(&rows);
        assert!(msg.len() < MAX_MESSAGE);
        let err = prescan(&msg).unwrap_err();
        assert!(err.to_string().contains("items"), "{err}");

        // Indefinite lengths, lengths past the input, and trailing bytes.
        assert!(prescan(&[0x9f, 0x00, 0xff]).is_err());
        assert!(prescan(&[0x82, 0x00]).is_err());
        assert!(prescan(&[0x65, b'a']).is_err());
        assert!(prescan(&[0x00, 0x00]).is_err());
        // What the encoder writes passes.
        assert!(prescan(&encode(3, &Message::Ping).unwrap()).is_ok());
        assert!(
            prescan(&[0xc1, 0x1a, 0, 0, 0, 1]).is_ok(),
            "a tag wraps one item"
        );
    }

    #[test]
    fn keys_in_a_sent_or_received_batch_are_overwritten() {
        let mut m = records::MeetingRec {
            gid: "m".into(),
            dek: Some(Bytes(vec![7; 32])),
            ..Default::default()
        };
        let mut msg = Message::PushRows(PushRows {
            rows: vec![Record(records::Record::Meeting(m.clone()))],
            upto_seq: 1,
        });
        msg.wipe_secrets();
        let Message::PushRows(b) = &msg else { panic!() };
        let records::Record::Meeting(w) = &b.rows[0].0 else {
            panic!()
        };
        assert!(w.dek.as_ref().unwrap().0.is_empty(), "wiped and emptied");
        let mut rows = [records::Record::Meeting(std::mem::take(&mut m))];
        wipe_records(&mut rows);
        let records::Record::Meeting(w) = &rows[0] else {
            panic!()
        };
        assert!(w.dek.as_ref().unwrap().0.is_empty(), "wiped and emptied");
        let mut v = Value::Array(vec![Value::Bytes(vec![9; 8]), Value::Text("x".into())]);
        wipe_value(&mut v);
        assert_eq!(
            v,
            Value::Array(vec![Value::Bytes(vec![]), Value::Text("x".into())])
        );
    }

    #[test]
    fn batch_caps_are_enforced_on_receipt() {
        let row = Record(records::Record::Mark(records::MarkRec {
            gid: "g".into(),
            ..Default::default()
        }));
        let big = Message::PushRows(PushRows {
            rows: vec![row.clone(); MAX_BATCH_RECORDS + 1],
            upto_seq: 1,
        });
        assert!(decode(&encode(1, &big).unwrap()).is_err());
        let ok = Message::PushRows(PushRows {
            rows: vec![row; MAX_BATCH_RECORDS],
            upto_seq: 1,
        });
        assert!(decode(&encode(1, &ok).unwrap()).is_ok());
        let pages = |n: usize, len: usize| {
            Message::TrackPages(TrackPages {
                track_gid: "t".into(),
                prefix: Bytes(vec![0; 19]),
                first: 0,
                records: vec![Bytes(vec![0; len]); n],
            })
        };
        assert!(decode(&encode(1, &pages(MAX_TRACK_PAGES + 1, 10)).unwrap()).is_err());
        assert!(decode(&encode(1, &pages(2, MAX_TRACK_PAGES_BYTES / 2 + 1)).unwrap()).is_err());
        assert!(decode(&encode(1, &pages(MAX_TRACK_PAGES, 1000)).unwrap()).is_ok());
    }

    #[test]
    fn the_highest_common_major_wins_with_the_lower_minor() {
        let p = |major, minor| Proto { major, minor };
        assert_eq!(negotiate(&[p(1, 0)], &[p(1, 3)]), Some(p(1, 0)));
        assert_eq!(negotiate(&[p(1, 2)], &[p(1, 0)]), Some(p(1, 0)));
        assert_eq!(
            negotiate(&[p(1, 1), p(2, 0)], &[p(2, 4), p(1, 9)]),
            Some(p(2, 0))
        );
        assert_eq!(negotiate(&[p(1, 0)], &[p(2, 0)]), None);
        assert_eq!(negotiate(&[p(1, 0)], &[]), None);
        // Which side must update.
        assert_eq!(upgrade_side(&[p(1, 0)], &[p(2, 0)], true), "hub");
        assert_eq!(upgrade_side(&[p(1, 0)], &[p(2, 0)], false), "spoke");
        assert_eq!(upgrade_side(&[p(2, 0)], &[p(1, 0)], true), "spoke");
    }
}
