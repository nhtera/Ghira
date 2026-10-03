// SPDX-License-Identifier: Apache-2.0
//! Speaker re-clustering without the diarizer's 8-speaker cap (phase 14d,
//! D12): when the final pass's diarizer saturates, diarize short windows
//! independently, embed each (window, label) with the speaker model and
//! cluster across windows. Vectors live in memory only and are wiped (RT-13).
//!
//! W0-B stub: the signature is final, the body is inert (slice S3).

use ghi_speech::SpeakerSegment;

use crate::engines::SpeechEngines;
use crate::profiles::VoiceEmbed;

/// Distinct labels at which the diarizer is saturated.
pub const SATURATED_AT: usize = 8;

/// Uncapped segments for `pcm` (16 kHz mono), or `None` to keep `segs`
/// (fewer than 9 clusters result, or stopped). `stop` is asked between windows.
pub fn run(
    _pcm: &[f32],
    _engines: &dyn SpeechEngines,
    _embedder: &mut dyn VoiceEmbed,
    _segs: &[SpeakerSegment],
    _stop: &dyn Fn() -> bool,
) -> Result<Option<Vec<SpeakerSegment>>, String> {
    Ok(None)
}
