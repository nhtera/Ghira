// SPDX-License-Identifier: Apache-2.0
//! Speech engines behind backend-neutral stream traits (RT-15).
//!
//! Backends: `nemo` (NeMo-Speech.cpp over FFI, feature `nemo`); `whisper`
//! (whisper.cpp, static, the optional final-pass ASR, feature `whisper`); `voice`
//! (speaker embeddings, CAM++ over tract, feature `voice`). The `sherpa`
//! backend (sherpa-onnx: zipformer-vi fallback, speaker embeddings) is not
//! built yet. Audio is mono `f32` PCM; the engines resample 8–96 kHz input.

#[cfg(feature = "nemo")]
pub mod nemo;
#[cfg(feature = "voice")]
pub mod voice;
#[cfg(feature = "whisper")]
pub mod whisper;

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// An engine call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeechError {
    /// The engine function that failed.
    pub op: &'static str,
    pub message: String,
}

impl std::fmt::Display for SpeechError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.op, self.message)
    }
}

impl std::error::Error for SpeechError {}

pub type Result<T> = std::result::Result<T, SpeechError>;

/// One recognized word. Times are seconds from the start of the stream.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub confidence: f32,
    /// 1-based diarization speaker, when the recognizer was asked to tag words.
    pub speaker: Option<u32>,
}

/// A partial or final ASR hypothesis for the current utterance.
#[derive(Debug, Clone, PartialEq)]
pub struct AsrResult {
    pub is_final: bool,
    pub text: String,
    pub words: Vec<Word>,
    /// Detected languages as BCP-47 codes (e.g. `vi-VN`), in the engine's order.
    pub languages: Vec<String>,
    /// Seconds of audio the engine had consumed when it produced this result.
    pub audio_processed: f64,
}

/// A speaker turn. `speaker` is 1-based.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpeakerSegment {
    pub start: f64,
    pub end: f64,
    pub speaker: u32,
}

/// A streaming recognizer: push audio, pull results; `finish` flushes the tail.
pub trait AsrStream {
    fn push(&mut self, pcm: &[f32], sample_rate: u32) -> Result<()>;
    fn finish(&mut self) -> Result<()>;
    /// [`AsrStream::finish`] for engines that work at the end (Whisper decodes
    /// everything then): stops early and returns `false` once `abort` says so,
    /// leaving no results. Streaming engines finish at once.
    fn finish_abortable(&mut self, _abort: &(dyn Fn() -> bool + Sync)) -> Result<bool> {
        self.finish().map(|()| true)
    }
    /// The next available result, or `None` when more audio is needed.
    fn next_result(&mut self) -> Result<Option<AsrResult>>;
}

/// A streaming diarizer: push audio; speaker segments so far at any time.
pub trait DiarStream {
    fn push(&mut self, pcm: &[f32], sample_rate: u32) -> Result<()>;
    fn finish(&mut self) -> Result<()>;
    fn segments(&self) -> Result<Vec<SpeakerSegment>>;
}
