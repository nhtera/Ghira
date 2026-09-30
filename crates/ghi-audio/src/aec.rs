// SPDX-License-Identifier: Apache-2.0
//! Acoustic echo cancellation: WebRTC AEC3 (`sonora`, a pure-Rust port) at
//! 16 kHz mono on 10 ms frames.
//!
//! Far end (render) is the system track, near end (capture) is the mic. On
//! every frame the render is fed before the capture. When the system track is
//! absent the caller passes zeros. AEC3 estimates the echo path delay itself;
//! no `set_stream_delay_ms` is needed.

use sonora::config::EchoCanceller;
use sonora::{AudioProcessing, Config, StreamConfig};

use crate::{FRAME_SAMPLES, SAMPLE_RATE};

fn build() -> AudioProcessing {
    let cfg = StreamConfig::new(SAMPLE_RATE, 1);
    AudioProcessing::builder()
        .config(Config {
            echo_canceller: Some(EchoCanceller::default()),
            ..Default::default()
        })
        .capture_config(cfg)
        .render_config(cfg)
        .build()
}

/// AEC3 delays the capture signal by two 64-sample blocks (8 ms). The
/// pipeline does not compensate: ASR gets the cleaned mic 8 ms late relative to
/// the raw tracks, which is far below anything the recognizer or diarizer resolves.
pub const LATENCY_SAMPLES: usize = 128;

pub struct Aec {
    apm: AudioProcessing,
    enabled: bool,
    scratch: [f32; FRAME_SAMPLES],
}

impl Default for Aec {
    fn default() -> Self {
        Self::new()
    }
}

impl Aec {
    /// A new canceller, enabled.
    pub fn new() -> Self {
        Self {
            apm: build(),
            enabled: true,
            scratch: [0.0; FRAME_SAMPLES],
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Off: `process` copies the mic through. Turning it back on starts from a
    /// fresh filter, since the echo path may have changed while it was off.
    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled && !self.enabled {
            self.reset();
        }
        self.enabled = enabled;
    }

    /// Forgets the adapted echo path (route flip, device change).
    pub fn reset(&mut self) {
        self.apm = build();
    }

    /// Processes one 10 ms frame: `far` is the system track (zeros if absent),
    /// `near` the mic, `out` receives the cleaned mic.
    ///
    /// All three slices must hold [`FRAME_SAMPLES`] samples.
    pub fn process(&mut self, far: &[f32], near: &[f32], out: &mut [f32]) {
        debug_assert!(far.len() == FRAME_SAMPLES && near.len() == FRAME_SAMPLES);
        if !self.enabled {
            out.copy_from_slice(near);
            return;
        }
        let ok = self
            .apm
            .process_render_f32(&[far], &mut [&mut self.scratch[..]])
            .is_ok()
            && self.apm.process_capture_f32(&[near], &mut [out]).is_ok();
        if !ok {
            // Never lose mic audio to a processing error.
            out.copy_from_slice(near);
        }
    }

    /// AEC3's own ERLE estimate in dB, once it has one.
    pub fn erle_db(&self) -> Option<f64> {
        self.apm.statistics().echo_return_loss_enhancement
    }
}

/// Echo return loss enhancement measured directly: energy of `before` over
/// energy of `after`, in dB. Meaningful on echo-only stretches (no near-end
/// speech). Returns 0 for an empty or silent `before`.
pub fn measure_erle_db(before: &[f32], after: &[f32]) -> f64 {
    let e = |x: &[f32]| x.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>();
    let (b, a) = (e(before), e(after));
    if b <= 0.0 {
        return 0.0;
    }
    10.0 * (b / a.max(1e-20)).log10()
}

#[cfg(test)]
pub(crate) mod testsig {
    //! Synthetic far-end speech and room echo for AEC tests.
    use crate::SAMPLE_RATE;

    pub struct Rng(pub u64);
    impl Rng {
        pub fn next_f32(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
        }
    }

    /// Speech-like: gliding pitch, harmonic series with a formant-ish tilt,
    /// syllable-rate amplitude modulation with pauses, plus a little noise.
    pub fn speechlike(seed: u64, seconds: f32, amp: f32) -> Vec<f32> {
        let n = (seconds * SAMPLE_RATE as f32) as usize;
        let mut rng = Rng(seed);
        let mut phase = 0.0f64;
        let mut out = Vec::with_capacity(n);
        let f0_base = 110.0 + (seed % 90) as f64;
        for i in 0..n {
            let t = i as f64 / SAMPLE_RATE as f64;
            let f0 = f0_base * (1.0 + 0.2 * (2.0 * std::f64::consts::PI * 0.7 * t).sin());
            phase += 2.0 * std::f64::consts::PI * f0 / SAMPLE_RATE as f64;
            let mut s = 0.0f64;
            for h in 1..=24 {
                let fh = f0 * h as f64;
                // Two broad formants around 700 and 1800 Hz.
                let g = (-((fh - 700.0) / 500.0).powi(2)).exp()
                    + 0.6 * (-((fh - 1800.0) / 700.0).powi(2)).exp();
                s += g * (phase * h as f64).sin() / (h as f64).sqrt();
            }
            let syl = (2.0 * std::f64::consts::PI * 3.7 * t + seed as f64).sin();
            let env = (syl * 2.0).clamp(0.0, 1.0);
            let pause = if (t * 0.9 + seed as f64 * 0.31).fract() > 0.85 {
                0.0
            } else {
                1.0
            };
            out.push((amp as f64 * (0.5 * s * env * pause) + 0.002 * rng.next_f32() as f64) as f32);
        }
        out
    }

    /// Exponentially decaying noise impulse response (`rt60` seconds), with
    /// `delay_ms` of pure delay before the direct path.
    pub fn room_ir(seed: u64, taps: usize, rt60: f32, delay_ms: f32, gain: f32) -> Vec<f32> {
        let mut rng = Rng(seed);
        let delay = (delay_ms * SAMPLE_RATE as f32 / 1000.0) as usize;
        let mut ir = vec![0.0f32; delay + taps];
        for k in 0..taps {
            let t = k as f32 / SAMPLE_RATE as f32;
            let decay = (-6.9 * t / rt60).exp();
            ir[delay + k] = rng.next_f32() * decay * 0.3;
        }
        ir[delay] += 1.0;
        for v in &mut ir {
            *v *= gain;
        }
        ir
    }

    pub fn convolve(x: &[f32], ir: &[f32]) -> Vec<f32> {
        let mut y = vec![0.0f32; x.len()];
        for (i, yi) in y.iter_mut().enumerate() {
            let k = ir.len().min(i + 1);
            let xs = &x[i + 1 - k..=i];
            let mut acc = 0.0f32;
            for (a, b) in xs.iter().rev().zip(ir.iter()) {
                acc += a * b;
            }
            *yi = acc;
        }
        y
    }
}

#[cfg(test)]
mod tests {
    use super::testsig::*;
    use super::*;

    fn run(aec: &mut Aec, far: &[f32], mic: &[f32]) -> Vec<f32> {
        let mut out = vec![0.0f32; mic.len()];
        for ((f, m), o) in far
            .chunks_exact(FRAME_SAMPLES)
            .zip(mic.chunks_exact(FRAME_SAMPLES))
            .zip(out.chunks_exact_mut(FRAME_SAMPLES))
        {
            aec.process(f, m, o);
        }
        out
    }

    fn rms_db(x: &[f32]) -> f64 {
        let e = x.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64;
        10.0 * e.max(1e-20).log10()
    }

    #[test]
    fn erle_at_least_20_db_and_double_talk_preserved() {
        let sr = SAMPLE_RATE as usize;
        // 0-10 s echo only, 10-14 s double talk, 14-17 s echo only.
        let secs = 17;
        let far = speechlike(7, secs as f32, 0.6);
        let ir = room_ir(3, 1000, 0.12, 60.0, 0.25);
        let echo = convolve(&far, &ir);
        let mut near = vec![0.0f32; far.len()];
        let near_sig = speechlike(31, 4.0, 0.8);
        near[10 * sr..14 * sr].copy_from_slice(&near_sig);
        let mic: Vec<f32> = echo.iter().zip(&near).map(|(e, n)| e + n).collect();

        let mut aec = Aec::new();
        let out = run(&mut aec, &far, &mic);

        // After 5 s of convergence, echo-only stretches.
        let seg1 = 5 * sr..10 * sr;
        let seg2 = 15 * sr..17 * sr;
        let erle1 = measure_erle_db(&mic[seg1.clone()], &out[seg1]);
        let erle2 = measure_erle_db(&mic[seg2.clone()], &out[seg2]);
        eprintln!(
            "ERLE echo-only: {erle1:.1} dB, after double talk: {erle2:.1} dB, stat {:?}",
            aec.erle_db()
        );
        assert!(erle1 >= 20.0, "ERLE {erle1:.1} dB");
        assert!(erle2 >= 20.0, "ERLE after double talk {erle2:.1} dB");

        // Double talk: near-end energy survives within a few dB (compare
        // against the near-end signal alone, over its active stretch).
        let dt = 10 * sr..14 * sr;
        let kept = rms_db(&out[dt.clone()]);
        let want = rms_db(&near[dt.clone()]);
        eprintln!("double talk: out {kept:.1} dB vs near {want:.1} dB");
        // Correlation with the true near-end signal: it is still there, not
        // replaced by residual echo.
        let lag = LATENCY_SAMPLES;
        let (o, n) = (&out[dt.start + lag..dt.end], &near[dt.start..dt.end - lag]);
        let dot: f64 = o
            .iter()
            .zip(n)
            .map(|(a, b)| (*a as f64) * (*b as f64))
            .sum();
        let norm = |x: &[f32]| x.iter().map(|&v| (v as f64).powi(2)).sum::<f64>().sqrt();
        let corr = dot / (norm(o) * norm(n));
        eprintln!("double talk correlation with near end: {corr:.3}");
        assert!(
            (kept - want).abs() <= 6.0,
            "near-end level moved {:.1} dB",
            kept - want
        );
        assert!(corr > 0.3, "near-end correlation {corr:.2}");
    }

    #[test]
    fn disabled_is_transparent_and_reenable_resets() {
        let far = speechlike(1, 1.0, 0.5);
        let mic = speechlike(2, 1.0, 0.5);
        let mut aec = Aec::new();
        aec.set_enabled(false);
        assert!(!aec.enabled());
        assert_eq!(run(&mut aec, &far, &mic), mic);
        aec.set_enabled(true);
        let out = run(&mut aec, &far, &mic);
        assert_ne!(out, mic);
    }

    #[test]
    fn silent_far_end_keeps_the_mic() {
        let sr = SAMPLE_RATE as usize;
        let mic = speechlike(9, 4.0, 0.4);
        let far = vec![0.0f32; mic.len()];
        let mut aec = Aec::new();
        let out = run(&mut aec, &far, &mic);
        let d = rms_db(&out[sr..]) - rms_db(&mic[sr..]);
        assert!(d > -3.0, "mic attenuated by {d:.1} dB with no far end");
    }

    #[test]
    fn measure_erle_edge_cases() {
        assert_eq!(measure_erle_db(&[], &[]), 0.0);
        assert_eq!(measure_erle_db(&[0.0; 4], &[1.0; 4]), 0.0);
        let d = measure_erle_db(&[1.0; 4], &[0.1; 4]);
        assert!((d - 20.0).abs() < 1e-6);
    }
}
