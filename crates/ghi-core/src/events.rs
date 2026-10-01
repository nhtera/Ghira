// SPDX-License-Identifier: Apache-2.0
//! The event contract between the core and the UIs (phases 10, 11, 14, 16)
//! [RT-14]. Serialized as `{"type": "...", ...}` (camelCase fields); with the
//! `specta` feature the types are exported to TypeScript by the desktop app.
//!
//! Times are milliseconds from the start of the meeting. Events never carry
//! audio; transcript text only goes to the local UI. 64-bit numbers are
//! exported to TypeScript as `number` (all stay far below 2^53).

use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// SessionManager states (brief §7, the Record control).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum SessionState {
    Idle,
    Starting,
    Recording,
    Paused,
    Stopping,
    Processing,
    Ready,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SpeakerInfo {
    /// Session speaker id (stable for the meeting).
    pub id: u32,
    /// What the UI shows: a name, "Me", "Speaker N" or "Identifying…".
    pub label: String,
    /// 1..=8, or 0 for the shared Others lane.
    pub color_slot: u8,
    pub is_me: bool,
    pub provisional: bool,
    pub not_person: bool,
    pub others: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct WordInfo {
    pub text: String,
    #[cfg_attr(feature = "specta", specta(type = f64))]
    pub t0_ms: i64,
    #[cfg_attr(feature = "specta", specta(type = f64))]
    pub t1_ms: i64,
    pub low_confidence: bool,
}

/// A final transcript line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct LineInfo {
    /// Store segment gid (empty until persisted).
    pub gid: String,
    pub speaker: Option<u32>,
    #[cfg_attr(feature = "specta", specta(type = f64))]
    pub t0_ms: i64,
    #[cfg_attr(feature = "specta", specta(type = f64))]
    pub t1_ms: i64,
    pub text: String,
    /// Two speakers talked over each other.
    pub overlap: bool,
    pub words: Vec<WordInfo>,
}

/// Final-pass stages, in order (shown as progress in the UI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum Stage {
    Decoding,
    RefiningSpeakers,
    MatchingVoices,
    ImprovingTranscript,
    WritingNotes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    Permission,
    Capture,
    Engine,
    ModelsMissing,
    Storage,
    Job,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Event {
    StateChanged {
        meeting: String,
        state: SessionState,
    },
    TranscriptPartial {
        meeting: String,
        /// 0 = mic, 1 = system.
        track: u8,
        text: String,
    },
    TranscriptFinal {
        meeting: String,
        line: LineInfo,
    },
    SpeakerArrived {
        meeting: String,
        speaker: SpeakerInfo,
    },
    /// A provisional speaker became "Speaker N".
    SpeakerConfirmed {
        meeting: String,
        speaker: SpeakerInfo,
    },
    SpeakerRenamed {
        meeting: String,
        speaker: SpeakerInfo,
    },
    SpeakersMerged {
        meeting: String,
        from: u32,
        into: u32,
    },
    SpeakerSplit {
        meeting: String,
        from: u32,
        speaker: SpeakerInfo,
    },
    SpeakerNotAPerson {
        meeting: String,
        id: u32,
    },
    MarkAdded {
        meeting: String,
        #[cfg_attr(feature = "specta", specta(type = f64))]
        t_ms: i64,
    },
    /// Everything from `from_ms` on was removed (audio, lines, marks, notes).
    DiscardApplied {
        meeting: String,
        #[cfg_attr(feature = "specta", specta(type = f64))]
        from_ms: i64,
    },
    LevelMeter {
        meeting: String,
        mic_dbfs: Option<f32>,
        system_dbfs: Option<f32>,
    },
    Health {
        meeting: String,
        /// Seconds the live transcript trails the audio.
        asr_lag_s: f32,
        /// Audio skipped by ASR so far (seconds; the final pass fills it).
        asr_skipped_s: f32,
        aec: bool,
    },
    JobProgress {
        meeting: Option<String>,
        #[cfg_attr(feature = "specta", specta(type = f64))]
        job: i64,
        kind: String,
        stage: Option<Stage>,
        /// 0..1 within the job.
        progress: f32,
    },
    NotesReady {
        meeting: String,
        /// 1 = from the live transcript, 2 = after the final pass.
        version: u32,
    },
    Error {
        meeting: Option<String>,
        kind: ErrorKind,
        message: String,
    },
}

/// An event with its sequence number (gap-free per bus) and wall time, so a
/// late subscriber (a reloaded webview) can tell what it missed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    #[cfg_attr(feature = "specta", specta(type = f64))]
    pub seq: u64,
    /// Unix ms.
    #[cfg_attr(feature = "specta", specta(type = f64))]
    pub at_ms: i64,
    pub event: Event,
}

/// The sending side of the event bus (cheap to clone; every thread has one).
#[derive(Clone)]
pub struct EventTx {
    tx: crossbeam_channel::Sender<Envelope>,
    /// Held while numbering and sending, so `seq` order is delivery order.
    seq: Arc<Mutex<u64>>,
}

/// The receiving side of the bus.
pub type EventRx = crossbeam_channel::Receiver<Envelope>;

/// A bus: give the [`EventTx`] to the core, read the receiver in the UI layer.
pub fn bus() -> (EventTx, EventRx) {
    let (tx, rx) = crossbeam_channel::unbounded();
    (
        EventTx {
            tx,
            seq: Arc::new(Mutex::new(0)),
        },
        rx,
    )
}

impl EventTx {
    /// Sends an event; a bus nobody listens to drops it.
    pub fn emit(&self, event: Event) {
        let at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let mut seq = self.seq.lock().unwrap_or_else(|e| e.into_inner());
        *seq += 1;
        let _ = self.tx.send(Envelope {
            seq: *seq,
            at_ms,
            event,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialize_with_a_type_tag_and_camel_case() {
        let e = Event::MarkAdded {
            meeting: "m1".into(),
            t_ms: 1500,
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"type": "markAdded", "meeting": "m1", "tMs": 1500})
        );
        let e = Event::StateChanged {
            meeting: "m1".into(),
            state: SessionState::Recording,
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap()["state"],
            serde_json::json!("recording")
        );
    }
}
