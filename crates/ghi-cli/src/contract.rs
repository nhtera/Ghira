// SPDX-License-Identifier: Apache-2.0
//! JSON documents printed by `ghi`, schema version 1.
//!
//! The eval harness (`tools/eval`) is the only consumer. The contract is
//! specified in `tools/eval/docs/formats.md` §2; golden examples live in
//! `tools/eval/tests/fixtures/cli/` and are checked by the tests below and by
//! the harness's JSON schemas.

use serde::{Deserialize, Serialize};

pub const VERSION: &str = "ghi.version/1";
pub const ERROR: &str = "ghi.error/1";
pub const TRANSCRIPT: &str = "ghi.transcript/1";
pub const EVENT: &str = "ghi.event/1";
pub const DIARIZATION: &str = "ghi.diarization/1";
pub const NOTES: &str = "ghi.notes/1";
pub const BENCH: &str = "ghi.bench/1";

/// Which pipeline pass produced a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Pass {
    Live,
    Final,
}

/// Language requested for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum LangMode {
    Auto,
    Vi,
    En,
}

/// Language detected for a segment or event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Vi,
    En,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Version {
    pub schema: String,
    pub ghi: String,
    pub core: String,
    pub engines: Vec<Engine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotImplemented,
    EngineUnavailable,
    BadInput,
    Internal,
}

impl ErrorCode {
    /// Process exit code for this error (formats.md §2).
    pub fn exit_code(self) -> u8 {
        match self {
            ErrorCode::NotImplemented | ErrorCode::EngineUnavailable => 3,
            ErrorCode::BadInput | ErrorCode::Internal => 1,
        }
    }
}

/// Printed as the last line of stderr when a command fails.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorDoc {
    pub schema: String,
    pub code: ErrorCode,
    pub message: String,
}

impl ErrorDoc {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            schema: ERROR.to_owned(),
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Engine {
    pub name: String,
    pub version: String,
}

/// Self-reported timings. The harness measures its own; these are informational.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Perf {
    pub wall_s: f64,
    pub rtf: Option<f64>,
    pub peak_rss_mb: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    pub schema: String,
    pub audio: String,
    pub duration_s: f64,
    pub pass: Pass,
    pub lang: LangMode,
    pub engine: Engine,
    pub segments: Vec<Segment>,
    pub perf: Perf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    /// Unique within the transcript; notes cite segments by this id.
    pub id: u32,
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub lang: Option<Lang>,
    pub speaker: Option<String>,
    pub words: Option<Vec<Word>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    /// May still change.
    Partial,
    /// Committed caption.
    Final,
    /// Last event of the stream.
    End,
}

/// One NDJSON line of `ghi transcribe --stream`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub schema: String,
    #[serde(rename = "type")]
    pub kind: EventType,
    pub seq: u64,
    /// Seconds since the CLI started feeding audio. With `--realtime`,
    /// caption lag is `wall_s - audio_end`.
    pub wall_s: f64,
    pub audio_start: f64,
    pub audio_end: f64,
    pub text: String,
    pub lang: Option<Lang>,
    pub speaker: Option<String>,
    /// Final events only: the committed words and when each was first shown
    /// as a caption. `null` on partial and end events.
    pub words: Option<Vec<EventWord>>,
}

/// A word of a final event. Caption lag of the word is `shown_s - end`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventWord {
    pub start: f64,
    pub end: f64,
    pub text: String,
    /// `wall_s` of the first event (partial or this final) that displayed the word.
    pub shown_s: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diarization {
    pub schema: String,
    pub audio: String,
    pub duration_s: f64,
    pub pass: Pass,
    pub engine: Engine,
    pub turns: Vec<Turn>,
    pub perf: Perf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub start: f64,
    pub end: f64,
    pub speaker: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notes {
    pub schema: String,
    pub engine: Engine,
    pub summary: Vec<NoteItem>,
    pub decisions: Vec<NoteItem>,
    pub action_items: Vec<ActionItem>,
    pub perf: Perf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoteItem {
    pub text: String,
    /// `segments[].id` values of the input transcript.
    pub citations: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionItem {
    pub text: String,
    pub owner: Option<String>,
    pub due: Option<String>,
    pub citations: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bench {
    pub schema: String,
    pub audio: String,
    pub duration_s: f64,
    pub pass: Pass,
    pub stages: Vec<Stage>,
    pub perf: Perf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    pub name: String,
    pub wall_s: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;
    use serde_json::Value;

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/eval/tests/fixtures/cli/"
    );

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}{name}"))
            .unwrap_or_else(|e| panic!("read fixture {name}: {e}"))
    }

    /// The golden document parses into our type and serializes back to the
    /// same JSON: no missing, extra or renamed fields on either side.
    fn round_trip<T: DeserializeOwned + Serialize>(json: &str, schema: &str) {
        let golden: Value = serde_json::from_str(json).unwrap();
        assert_eq!(golden["schema"], schema);
        let typed: T = serde_json::from_value(golden.clone()).unwrap();
        assert_eq!(serde_json::to_value(&typed).unwrap(), golden);
    }

    #[test]
    fn goldens_match_types() {
        round_trip::<Version>(&fixture("version.json"), VERSION);
        round_trip::<ErrorDoc>(&fixture("error.json"), ERROR);
        round_trip::<Transcript>(&fixture("transcript.json"), TRANSCRIPT);
        round_trip::<Diarization>(&fixture("diarization.json"), DIARIZATION);
        round_trip::<Notes>(&fixture("notes.json"), NOTES);
        round_trip::<Bench>(&fixture("bench.json"), BENCH);
        let events = fixture("events.ndjson");
        let lines: Vec<_> = events.lines().filter(|l| !l.trim().is_empty()).collect();
        assert!(!lines.is_empty());
        for line in lines {
            round_trip::<Event>(line, EVENT);
        }
    }

    #[test]
    fn error_exit_codes() {
        assert_eq!(ErrorCode::NotImplemented.exit_code(), 3);
        assert_eq!(ErrorCode::EngineUnavailable.exit_code(), 3);
        assert_eq!(ErrorCode::BadInput.exit_code(), 1);
        assert_eq!(ErrorCode::Internal.exit_code(), 1);
    }
}
