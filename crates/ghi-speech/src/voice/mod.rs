// SPDX-License-Identifier: Apache-2.0
//! Speaker embeddings for voice profiles (phase 14c).
//!
//! [`VoiceEmbedder`] is backend-neutral. [`TractVoice`] runs CAM++ zh_en
//! (3D-Speaker, ONNX) on `tract`; [`fbank`] is its Kaldi-compatible front end.

pub mod fbank;
mod tract;

pub use tract::TractVoice;

use crate::Result;

/// Embedding size of the CAM++ model.
pub const EMBEDDING_DIM: usize = 192;
/// Model input frame buckets (10 ms each). The model is only exact for whole
/// 200-frame multiples, so a window is cropped to the largest bucket it fills.
pub const BUCKETS: [usize; 3] = [200, 400, 600];

/// Windows quieter than this RMS (samples in [-1, 1]; about -60 dBFS) are
/// treated as silence and not embedded.
pub const MIN_RMS: f64 = 1e-3;

/// Turns speech from one speaker into a voice vector.
pub trait VoiceEmbedder {
    /// Embeds mono 16 kHz `f32` PCM in [-1, 1].
    ///
    /// Returns `Ok(None)` when there is nothing to embed: the audio is under
    /// 200 fbank frames (2.015 s), or the window it is cropped to is near-silent
    /// (RMS below [`MIN_RMS`]). Non-finite input samples or output values are
    /// an error, never NaN. Longer windows are cropped to 200, 400 or 600 frames from
    /// the start, so pass 2, 4 or 6 s windows. The vector is the raw model
    /// output (192 values), not normalised.
    fn embed(&mut self, pcm16k: &[f32]) -> Result<Option<Vec<f32>>>;
}
