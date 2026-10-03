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
    /// Sent once when a session reaches Recording, right before the
    /// `StateChanged { Recording }` that follows it. `mode` is "call" or
    /// "room" as recorded (a call without system audio is a room).
    SessionStarted {
        meeting: String,
        mode: String,
        language: Option<String>,
        title: String,
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
    /// `lines`: segment gids moved from `from` to the new speaker (each once).
    /// If the store refuses the move, an `Error` event follows and the lines
    /// stay with `from`.
    SpeakerSplit {
        meeting: String,
        from: u32,
        speaker: SpeakerInfo,
        lines: Vec<String>,
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
    /// RMS level of each track over the last ~100 ms (at most 10 per second,
    /// only while audio flows). `None`: the track is not captured or no audio
    /// passed in the window (paused, asleep); digital silence reads -100.
    LevelMeter {
        meeting: String,
        mic_dbfs: Option<f32>,
        system_dbfs: Option<f32>,
    },
    /// The Mac went to sleep; the timeline stops (the gap is not recorded as
    /// audio) until `Woke` [RT-10].
    Slept {
        meeting: String,
    },
    Woke {
        meeting: String,
    },
    /// The system-audio tap was restarted (an output device change).
    SystemAudioRestarted {
        meeting: String,
    },
    /// The system track has been digital silence for `silent_s` seconds in a
    /// call: a denied or broken tap (hint: record the room instead).
    SilentSystemTrack {
        meeting: String,
        silent_s: f32,
    },
    DiskLow {
        meeting: String,
        #[cfg_attr(feature = "specta", specta(type = f64))]
        free_bytes: u64,
    },
    DiskFull {
        meeting: String,
    },
    /// A capture device vanished (0 = mic, 1 = system).
    TrackLost {
        meeting: String,
        track: u8,
    },
    /// "Only the meeting app's audio" was on but no meeting app was in a call
    /// when recording started: all system audio is recorded instead.
    AppAudioFallback {
        meeting: String,
    },
    /// After `retry_capture`: the devices are back (also when nothing had
    /// been lost).
    CaptureRecovered {
        meeting: String,
    },
    /// After `retry_capture`: the devices could not be rebuilt.
    CaptureRetryFailed {
        meeting: String,
        message: String,
    },
    /// The audio route changed; `bluetooth_hfp`: the input is a Bluetooth
    /// headset, whose mic drops the whole link to call quality.
    RouteChanged {
        meeting: String,
        bluetooth_hfp: bool,
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

/// Everything a reloaded webview needs to redraw a running session. `seq` is
/// the bus sequence number read before the rest was gathered: events with a
/// greater `seq` may repeat what is here (lines are told apart by `gid`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    #[cfg_attr(feature = "specta", specta(type = f64))]
    pub seq: u64,
    pub meeting: String,
    pub state: SessionState,
    #[cfg_attr(feature = "specta", specta(type = f64))]
    pub now_ms: i64,
    /// A live transcript is being made (false: models missing).
    pub transcribing: bool,
    /// "call" or "room" (after the call-without-system-audio fallback).
    pub mode: String,
    pub language: Option<String>,
    pub title: String,
    /// Everyone's consent to recording was confirmed (the live toggle).
    pub consent_confirmed: bool,
    /// Speakers still in play (merged ones are gone).
    pub speakers: Vec<SpeakerInfo>,
    /// Final lines stored so far, in time order.
    pub lines: Vec<LineInfo>,
    /// Marks (ms), in time order.
    #[cfg_attr(feature = "specta", specta(type = Vec<f64>))]
    pub marks: Vec<i64>,
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
    /// Sequence number of the last event sent (0: none yet).
    pub fn last_seq(&self) -> u64 {
        *self.seq.lock().unwrap_or_else(|e| e.into_inner())
    }

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
        let e = Event::SessionStarted {
            meeting: "m1".into(),
            mode: "room".into(),
            language: None,
            title: "Standup".into(),
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"type": "sessionStarted", "meeting": "m1", "mode": "room",
                               "language": null, "title": "Standup"})
        );
        let e = Event::SilentSystemTrack {
            meeting: "m1".into(),
            silent_s: 30.0,
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({"type": "silentSystemTrack", "meeting": "m1", "silentS": 30.0})
        );
        let e = Event::RouteChanged {
            meeting: "m1".into(),
            bluetooth_hfp: true,
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap()["bluetoothHfp"],
            serde_json::json!(true)
        );
    }
}
