// SPDX-License-Identifier: Apache-2.0
//! Pocket heuristic: is the microphone muffled (a pocket, a bag, a hand over it)?
//!
//! Speech that reaches a covered microphone loses its upper frequencies. Per
//! second of 16 kHz audio the detector compares the energy of the first
//! difference (a crude high-pass, gain rising with frequency) with that of the
//! two-sample mean (a crude low-pass), over frames loud enough to be speech.
//! Seconds with too little speech say nothing and change no count. Ten
//! muffled seconds in a row raise the warning; five normal seconds clear it.
//!
//! The threshold is a first guess from synthetic signals; tune it on a device
//! (owner checklist, iOS).

use crate::{FRAME_SAMPLES, SAMPLE_RATE};

/// Frames per analysis window (1 s).
const WINDOW_FRAMES: usize = (SAMPLE_RATE as usize) / FRAME_SAMPLES;
/// A frame counts as speech above this RMS (about -42 dBFS).
const SPEECH_RMS: f32 = 0.008;
/// A window needs this many speech frames to say anything (0.3 s).
const MIN_SPEECH_FRAMES: usize = 30;
/// High-band / low-band energy ratio below which a window is muffled.
pub const RATIO_THRESHOLD: f64 = 0.03;
/// Consecutive muffled seconds that raise the warning.
pub const WARN_AFTER_S: u32 = 10;
/// Consecutive normal seconds that clear it.
pub const CLEAR_AFTER_S: u32 = 5;

#[derive(Debug, Default)]
pub struct MuffleDetector {
    frames: usize,
    speech_frames: usize,
    hi: f64,
    lo: f64,
    prev: f32,
    muffled_s: u32,
    normal_s: u32,
    warned: bool,
}

impl MuffleDetector {
    pub fn new() -> MuffleDetector {
        MuffleDetector::default()
    }

    /// Whether the warning is up.
    pub fn muffled(&self) -> bool {
        self.warned
    }

    /// Feeds one 10 ms frame of the mic track; `Some(state)` when the warning
    /// goes up or clears.
    pub fn push(&mut self, frame: &[f32]) -> Option<bool> {
        let rms = (frame.iter().map(|s| s * s).sum::<f32>() / frame.len().max(1) as f32).sqrt();
        if rms >= SPEECH_RMS {
            self.speech_frames += 1;
            for &s in frame {
                let d = f64::from(s - self.prev);
                let m = f64::from(s + self.prev) / 2.0;
                self.hi += d * d;
                self.lo += m * m;
                self.prev = s;
            }
        } else if let Some(&last) = frame.last() {
            self.prev = last;
        }
        self.frames += 1;
        if self.frames < WINDOW_FRAMES {
            return None;
        }
        let verdict = (self.speech_frames >= MIN_SPEECH_FRAMES && self.lo > 0.0)
            .then(|| self.hi / self.lo < RATIO_THRESHOLD);
        (self.frames, self.speech_frames, self.hi, self.lo) = (0, 0, 0.0, 0.0);
        match verdict {
            Some(true) => {
                self.normal_s = 0;
                self.muffled_s += 1;
                if !self.warned && self.muffled_s >= WARN_AFTER_S {
                    self.warned = true;
                    return Some(true);
                }
            }
            Some(false) => {
                self.muffled_s = 0;
                self.normal_s += 1;
                if self.warned && self.normal_s >= CLEAR_AFTER_S {
                    self.warned = false;
                    return Some(false);
                }
            }
            None => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Speech-like: a 150 Hz voiced source with harmonics falling off as 1/k
    /// up to 4 kHz, plus a little noise. Deterministic.
    fn speech(seconds: usize) -> Vec<f32> {
        let mut seed = 12345u32;
        (0..seconds * SAMPLE_RATE as usize)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                let voiced: f32 = (1..=26)
                    .map(|k| (std::f32::consts::TAU * 150.0 * k as f32 * t).sin() / k as f32)
                    .sum();
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (seed >> 16) as f32 / 32768.0 - 1.0;
                (voiced * 0.12 + noise * 0.03).clamp(-1.0, 1.0)
            })
            .collect()
    }

    /// Two cascaded one-pole low-passes at about 500 Hz: a pocket.
    fn muffle(x: &[f32]) -> Vec<f32> {
        let a = 0.18;
        let (mut y1, mut y2) = (0.0f32, 0.0f32);
        x.iter()
            .map(|&s| {
                y1 += a * (s - y1);
                y2 += a * (y1 - y2);
                y2 * 3.0
            })
            .collect()
    }

    fn run(d: &mut MuffleDetector, pcm: &[f32]) -> Vec<(usize, bool)> {
        pcm.chunks(FRAME_SAMPLES)
            .enumerate()
            .filter_map(|(i, f)| d.push(f).map(|s| (i / 100, s)))
            .collect()
    }

    #[test]
    fn clean_speech_never_warns() {
        let mut d = MuffleDetector::new();
        assert!(run(&mut d, &speech(30)).is_empty());
        assert!(!d.muffled());
    }

    #[test]
    fn muffled_speech_warns_after_ten_seconds_and_clears_after_five() {
        let mut d = MuffleDetector::new();
        let clean = speech(20);
        let events = run(&mut d, &muffle(&clean));
        assert_eq!(events, [(9, true)], "the tenth muffled second");
        let events = run(&mut d, &clean);
        assert_eq!(events, [(4, false)], "the fifth normal second");
        assert!(!d.muffled());
    }

    #[test]
    fn silence_changes_nothing() {
        let mut d = MuffleDetector::new();
        run(&mut d, &muffle(&speech(8)));
        assert!(run(&mut d, &vec![0.0; 16_000 * 30]).is_empty());
        // Two more muffled seconds complete the ten.
        let events = run(&mut d, &muffle(&speech(2)));
        assert_eq!(events, [(1, true)]);
    }
}
