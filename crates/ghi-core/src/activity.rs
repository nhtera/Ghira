// SPDX-License-Identifier: Apache-2.0
//! Speech activity of one track (phase 14d, D9): where a participant of a
//! multi-track import (Zoom "record a separate audio file for each
//! participant") is talking. Pure and energy-based: 20 ms RMS against an
//! adaptive floor, a hangover, and gap merging. The spans decide who spoke;
//! the transcript still comes from the mixed audio.
//!
//! [`Meter`] takes audio as it streams (it keeps one 4-byte level per 20 ms
//! frame: 0.7 MB per track-hour, 35 MB for 49 tracks of an hour);
//! [`speech_spans`] is the same over a slice.
//!
//! The quiet level the threshold stands on is measured over the frames that
//! are not digital silence ([`SILENCE_DB`]): Zoom writes exact zeros while a
//! participant is muted, which would otherwise drag the floor to -180 dB.

/// Frame length (ms).
pub const FRAME_MS: i64 = 20;
const FRAME_SAMPLES: usize = 320; // 20 ms at 16 kHz
/// Absolute floor of the threshold (dBFS): below this is never speech.
const MIN_DB: f32 = -50.0;
/// Frames at or below this level are digital silence (a muted track) and say
/// nothing about the room's noise floor.
pub const SILENCE_DB: f32 = -90.0;
/// Speech has to stand this far (dB) above the track's quiet level.
const MARGIN_DB: f32 = 12.0;
/// Frames kept active after the last loud one (300 ms).
const HANGOVER: usize = 15;
/// The same in ms: a span's end includes this much that is not speech.
pub const HANGOVER_MS: i64 = HANGOVER as i64 * FRAME_MS;
/// Shortest span kept (ms).
const MIN_SPAN_MS: i64 = 250;
/// Gaps shorter than this between spans are closed (ms).
const MERGE_GAP_MS: i64 = 400;

/// Level of one frame in dBFS.
fn frame_db(frame: &[f32]) -> f32 {
    let ms = frame.iter().map(|x| x * x).sum::<f32>() / frame.len().max(1) as f32;
    10.0 * (ms + 1e-18).log10()
}

/// Collects frame levels from streamed 16 kHz mono audio.
#[derive(Debug, Default)]
pub struct Meter {
    db: Vec<f32>,
    carry: Vec<f32>,
}

impl Meter {
    /// Adds the next samples (any length).
    pub fn push(&mut self, samples: &[f32]) {
        self.carry.extend_from_slice(samples);
        let whole = self.carry.len() / FRAME_SAMPLES * FRAME_SAMPLES;
        for f in self.carry[..whole].chunks_exact(FRAME_SAMPLES) {
            self.db.push(frame_db(f));
        }
        self.carry.drain(..whole);
    }

    /// The speech spans so far as `[t0_ms, t1_ms]` pairs (a last partial frame
    /// counts as a whole one).
    pub fn finish(mut self) -> Vec<[i64; 2]> {
        if !self.carry.is_empty() {
            let tail = std::mem::take(&mut self.carry);
            self.db.push(frame_db(&tail));
        }
        spans_from_db(&self.db)
    }
}

/// Speech spans of 16 kHz mono `pcm` as `[t0_ms, t1_ms]` pairs in time order,
/// non-overlapping.
pub fn speech_spans(pcm: &[f32]) -> Vec<[i64; 2]> {
    let mut m = Meter::default();
    m.push(pcm);
    m.finish()
}

fn spans_from_db(db: &[f32]) -> Vec<[i64; 2]> {
    if db.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<f32> = db.iter().copied().filter(|d| *d > SILENCE_DB).collect();
    if sorted.is_empty() {
        return Vec::new();
    }
    sorted.sort_by(f32::total_cmp);
    let floor = sorted[sorted.len() / 5];
    let peak = sorted[sorted.len() - 1];
    // A track with no dynamics (always talking, or steady noise) has no floor
    // to stand above: only the absolute level counts.
    let thr = if peak - floor < MARGIN_DB {
        MIN_DB
    } else {
        (floor + MARGIN_DB).max(MIN_DB)
    };
    let mut active = vec![false; db.len()];
    let mut left = 0;
    for (i, &d) in db.iter().enumerate() {
        if d >= thr {
            left = HANGOVER + 1;
        }
        if left > 0 {
            active[i] = true;
            left -= 1;
        }
    }
    let mut spans: Vec<[i64; 2]> = Vec::new();
    let mut start: Option<usize> = None;
    for i in 0..=active.len() {
        let on = active.get(i).copied().unwrap_or(false);
        match (on, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                spans.push([s as i64 * FRAME_MS, i as i64 * FRAME_MS]);
                start = None;
            }
            _ => {}
        }
    }
    let mut merged: Vec<[i64; 2]> = Vec::new();
    for s in spans {
        match merged.last_mut() {
            Some(last) if s[0] - last[1] < MERGE_GAP_MS => last[1] = s[1],
            _ => merged.push(s),
        }
    }
    merged.retain(|s| s[1] - s[0] >= MIN_SPAN_MS);
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(secs: f32, amp: f32) -> Vec<f32> {
        (0..(16_000.0 * secs) as usize)
            .map(|i| (i as f32 * 0.07).sin() * amp)
            .collect()
    }

    /// Cheap deterministic noise.
    fn noise(secs: f32, amp: f32) -> Vec<f32> {
        let mut x = 12_345u32;
        (0..(16_000.0 * secs) as usize)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((x >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 2.0 * amp
            })
            .collect()
    }

    fn near(a: i64, b: i64) -> bool {
        (a - b).abs() <= 40
    }

    #[test]
    fn finds_talk_between_silences_within_a_frame_or_two() {
        let mut pcm = vec![0.0; 16_000 * 2];
        pcm.extend(tone(3.0, 0.2));
        pcm.extend(vec![0.0; 16_000 * 4]);
        pcm.extend(tone(1.0, 0.2));
        pcm.extend(vec![0.0; 16_000]);
        let s = speech_spans(&pcm);
        assert_eq!(s.len(), 2, "{s:?}");
        // The hangover extends the end by 300 ms.
        assert!(near(s[0][0], 2_000) && near(s[0][1], 5_300), "{s:?}");
        assert!(near(s[1][0], 9_000) && near(s[1][1], 10_300), "{s:?}");
    }

    #[test]
    fn a_muted_participant_does_not_drag_the_floor_down() {
        // Zoom: exact zeros while muted (-180 dB), then an open mic: room noise
        // at about -50 dBFS with talk at about -15 dBFS.
        let mut pcm = vec![0.0; 16_000 * 30];
        pcm.extend(noise(2.0, 0.005));
        let talk: Vec<f32> = tone(2.0, 0.25)
            .iter()
            .zip(noise(2.0, 0.005))
            .map(|(a, b)| a + b)
            .collect();
        pcm.extend(talk);
        pcm.extend(noise(2.0, 0.005));
        pcm.extend(vec![0.0; 16_000 * 10]);
        let s = speech_spans(&pcm);
        assert_eq!(
            s.len(),
            1,
            "the room noise after the mute is not speech: {s:?}"
        );
        assert!(near(s[0][0], 32_000) && near(s[0][1], 34_300), "{s:?}");
        // A track that is muted throughout has nothing.
        assert!(speech_spans(&vec![0.0; 16_000 * 60]).is_empty());
    }

    #[test]
    fn silence_and_blips_are_not_speech() {
        assert!(speech_spans(&vec![0.0; 16_000 * 5]).is_empty());
        assert!(speech_spans(&[]).is_empty());
        // 100 ms click + hangover is under the 250 ms minimum? It is 400 ms
        // with the hangover: kept. A single frame (20 ms + 300 ms) is 320 ms.
        let mut pcm = vec![0.0; 16_000 * 2];
        pcm.extend(tone(0.02, 0.3));
        pcm.extend(vec![0.0; 16_000 * 2]);
        let s = speech_spans(&pcm);
        assert!(s.iter().all(|x| x[1] - x[0] >= 250), "{s:?}");
    }

    #[test]
    fn short_gaps_close_and_long_ones_stay() {
        let mut pcm = tone(1.0, 0.2);
        pcm.extend(vec![0.0; 16_000 * 6 / 10]); // 600 ms gap - 300 ms hangover = 300 ms open
        pcm.extend(tone(1.0, 0.2));
        pcm.extend(vec![0.0; 16_000 * 2]);
        pcm.extend(tone(1.0, 0.2));
        let s = speech_spans(&pcm);
        assert_eq!(s.len(), 2, "{s:?}");
        assert!(near(s[0][0], 0) && near(s[0][1], 2_600 + 300), "{s:?}");
    }

    #[test]
    fn a_noisy_floor_is_not_speech_but_louder_talk_is() {
        // Steady room noise at about -45 dBFS with talk at about -15 dBFS.
        let mut pcm = noise(3.0, 0.01);
        let talk: Vec<f32> = tone(2.0, 0.25)
            .iter()
            .zip(noise(2.0, 0.01))
            .map(|(a, b)| a + b)
            .collect();
        pcm.extend(talk);
        pcm.extend(noise(3.0, 0.01));
        let s = speech_spans(&pcm);
        assert_eq!(s.len(), 1, "{s:?}");
        assert!(near(s[0][0], 3_000) && near(s[0][1], 5_300), "{s:?}");
    }

    #[test]
    fn a_track_that_never_stops_is_all_speech() {
        let s = speech_spans(&tone(4.0, 0.2));
        assert_eq!(s.len(), 1);
        assert!(near(s[0][0], 0) && s[0][1] >= 3_980, "{s:?}");
    }

    #[test]
    fn streaming_in_odd_chunks_matches_the_slice() {
        let mut pcm = vec![0.0; 16_000];
        pcm.extend(tone(2.0, 0.2));
        pcm.extend(vec![0.0; 16_000 * 2]);
        let mut m = Meter::default();
        for c in pcm.chunks(777) {
            m.push(c);
        }
        assert_eq!(m.finish(), speech_spans(&pcm));
    }
}
