// SPDX-License-Identifier: Apache-2.0
//! Kaldi-compatible 80-mel log filterbank (what CAM++ was trained on).
//!
//! 25 ms frames every 10 ms (snip edges), per-frame DC removal, pre-emphasis
//! 0.97, Povey window, 512-point power spectrum, 80 triangular mel bins from
//! 20 Hz to Nyquist, natural log with a float-epsilon floor, no dither.
//! Samples are floats in [-1, 1] (model metadata `normalize_samples=1`).

use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};

pub const SAMPLE_RATE: usize = 16_000;
pub const FRAME_LEN: usize = 400;
pub const FRAME_SHIFT: usize = 160;
pub const NUM_MEL: usize = 80;
const FFT_LEN: usize = 512;
const PREEMPH: f32 = 0.97;
const LOW_HZ: f64 = 20.0;

/// Number of frames `n_samples` yield (snip edges).
pub fn num_frames(n_samples: usize) -> usize {
    if n_samples < FRAME_LEN {
        0
    } else {
        1 + (n_samples - FRAME_LEN) / FRAME_SHIFT
    }
}

/// Samples needed for exactly `frames` frames.
pub fn samples_for_frames(frames: usize) -> usize {
    if frames == 0 {
        0
    } else {
        FRAME_LEN + (frames - 1) * FRAME_SHIFT
    }
}

fn mel(hz: f64) -> f64 {
    1127.0 * (1.0 + hz / 700.0).ln()
}

pub struct Fbank {
    fft: Arc<dyn RealToComplex<f32>>,
    window: [f32; FRAME_LEN],
    /// `NUM_MEL` triangles over the `FFT_LEN / 2` bins below Nyquist.
    filters: Vec<[f32; FFT_LEN / 2]>,
}

impl Default for Fbank {
    fn default() -> Self {
        Self::new()
    }
}

impl Fbank {
    pub fn new() -> Self {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(FFT_LEN);
        let mut window = [0.0f32; FRAME_LEN];
        for (i, w) in window.iter_mut().enumerate() {
            let a = 2.0 * std::f64::consts::PI * i as f64 / (FRAME_LEN - 1) as f64;
            *w = (0.5 - 0.5 * a.cos()).powf(0.85) as f32;
        }
        let nyquist = SAMPLE_RATE as f64 / 2.0;
        let (mel_lo, mel_hi) = (mel(LOW_HZ), mel(nyquist));
        let delta = (mel_hi - mel_lo) / (NUM_MEL + 1) as f64;
        let bin_hz = SAMPLE_RATE as f64 / FFT_LEN as f64;
        let filters = (0..NUM_MEL)
            .map(|b| {
                let left = mel_lo + b as f64 * delta;
                let center = left + delta;
                let right = center + delta;
                let mut f = [0.0f32; FFT_LEN / 2];
                for (i, w) in f.iter_mut().enumerate() {
                    let m = mel(bin_hz * i as f64);
                    if m > left && m < right {
                        *w = if m <= center {
                            (m - left) / (center - left)
                        } else {
                            (right - m) / (right - center)
                        } as f32;
                    }
                }
                f
            })
            .collect();
        Self {
            fft,
            window,
            filters,
        }
    }

    /// Raw log-mel frames, row-major `num_frames x NUM_MEL`.
    pub fn compute(&self, pcm: &[f32]) -> Vec<f32> {
        let frames = num_frames(pcm.len());
        let mut out = Vec::with_capacity(frames * NUM_MEL);
        let mut buf = self.fft.make_input_vec();
        let mut spec = self.fft.make_output_vec();
        let mut scratch = self.fft.make_scratch_vec();
        for t in 0..frames {
            let frame = &pcm[t * FRAME_SHIFT..t * FRAME_SHIFT + FRAME_LEN];
            let mean = frame.iter().map(|&x| f64::from(x)).sum::<f64>() / FRAME_LEN as f64;
            let mut x = [0.0f32; FRAME_LEN];
            for (d, &s) in x.iter_mut().zip(frame) {
                *d = s - mean as f32;
            }
            buf.fill(0.0);
            // Pre-emphasis; the first sample is emphasised against itself.
            buf[0] = (x[0] - PREEMPH * x[0]) * self.window[0];
            for i in 1..FRAME_LEN {
                buf[i] = (x[i] - PREEMPH * x[i - 1]) * self.window[i];
            }
            self.fft
                .process_with_scratch(&mut buf, &mut spec, &mut scratch)
                .expect("fft buffer sizes");
            let power: Vec<f32> = spec[..FFT_LEN / 2].iter().map(|c| c.norm_sqr()).collect();
            for f in &self.filters {
                let e: f32 = f.iter().zip(&power).map(|(w, p)| w * p).sum();
                out.push(e.max(f32::EPSILON).ln());
            }
        }
        out
    }

    /// [`Fbank::compute`] minus the per-bin mean over the window (the model's
    /// `global-mean` normalisation).
    pub fn features(&self, pcm: &[f32]) -> Vec<f32> {
        let mut f = self.compute(pcm);
        subtract_mean(&mut f);
        f
    }
}

/// Subtracts each mel bin's mean over time, in place.
pub fn subtract_mean(frames: &mut [f32]) {
    let n = frames.len() / NUM_MEL;
    if n == 0 {
        return;
    }
    let mut mean = [0.0f64; NUM_MEL];
    for row in frames.chunks_exact(NUM_MEL) {
        for (m, &v) in mean.iter_mut().zip(row) {
            *m += f64::from(v);
        }
    }
    for row in frames.chunks_exact_mut(NUM_MEL) {
        for (v, m) in row.iter_mut().zip(&mean) {
            *v -= (m / n as f64) as f32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_counts() {
        assert_eq!(num_frames(399), 0);
        assert_eq!(num_frames(400), 1);
        assert_eq!(num_frames(560), 2);
        assert_eq!(num_frames(samples_for_frames(200)), 200);
        assert_eq!(num_frames(samples_for_frames(200) - 1), 199);
    }

    #[test]
    fn silence_hits_the_floor_and_mean_is_removed() {
        let fb = Fbank::new();
        let f = fb.compute(&[0.0; 1000]);
        assert!(f.iter().all(|&v| (v - f32::EPSILON.ln()).abs() < 1e-6));
        let g = fb.features(
            &(0..4000)
                .map(|i| (i as f32 * 0.05).sin())
                .collect::<Vec<_>>(),
        );
        let n = g.len() / NUM_MEL;
        for b in 0..NUM_MEL {
            let m: f32 = (0..n).map(|t| g[t * NUM_MEL + b]).sum::<f32>() / n as f32;
            assert!(m.abs() < 1e-3, "bin {b} mean {m}");
        }
    }
}
