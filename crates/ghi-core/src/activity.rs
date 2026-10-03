// SPDX-License-Identifier: Apache-2.0
//! Speech activity of one track (phase 14d, D9): where a participant of a
//! multi-track import (Zoom "record a separate audio file for each
//! participant") is talking. Pure and energy-based: 20 ms RMS against an
//! adaptive floor, a hangover, and gap merging. The spans decide who spoke;
//! the transcript still comes from the mixed audio.
//!
//! W0-B stub: the signature is final, the body is inert (slice S2).

/// Speech spans of 16 kHz mono `pcm` as `[t0_ms, t1_ms]` pairs in time order,
/// non-overlapping.
pub fn speech_spans(_pcm: &[f32]) -> Vec<[i64; 2]> {
    Vec::new()
}
