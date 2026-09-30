// SPDX-License-Identifier: Apache-2.0
//! Per-track resampling from the device rate to the shared 16 kHz timeline.
//!
//! Each captured block carries the host time of its first sample. The timeline
//! is anchored at one host time for the whole session, so a sample's timeline
//! index is `(host_ns - anchor) * 16000`. A block's expected index is compared
//! with where the resampler has actually got to; the difference `err` drives
//!
//! * a small PI controller on the resampling ratio (clamped to +-0.5%), which
//!   tracks the true device rate against the host clock (crystal drift of tens
//!   to hundreds of ppm) without ever inserting or dropping a sample, and
//! * for jumps beyond [`GAP_SAMPLES`] (sleep, device rebuild, blocks dropped by
//!   a ring overflow): silence padding (`err > 0`) or discarding the overlap
//!   (`err < 0`), then a fresh resampler segment.
//!
//! A rate change mid-stream (new device) rebuilds the resampler the same way.
//! Both tracks therefore stay on one clock and aligned to each other.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Adjustable, Async, FixedAsync, PolynomialDegree, Resampler, SincInterpolationParameters,
    WindowFunction,
};

use crate::SAMPLE_RATE;

/// Timeline jumps larger than this (30 ms) are treated as gaps, smaller ones
/// are absorbed by the ratio controller.
pub const GAP_SAMPLES: f64 = 0.030 * SAMPLE_RATE as f64;
/// Input frames per resampler call.
const CHUNK: usize = 480;
/// The controller may move the ratio by at most this fraction (+-0.5%).
const MAX_TRIM: f64 = 0.005;
/// Ratio headroom passed to rubato (must exceed `1 + MAX_TRIM`).
const MAX_RELATIVE: f64 = 1.02;
/// Controller time constant in seconds.
const TAU_S: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    /// Windowed-sinc, alias-free. Used for capture.
    #[default]
    Speech,
    /// Linear interpolation, cheap but aliases when decimating. For simulations.
    Fast,
}

/// What [`TrackResampler::push`] did besides producing samples.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PushReport {
    /// Silence inserted on the timeline `(from, to)`, only when at least
    /// [`GAP_SAMPLES`] long.
    pub gap: Option<(u64, u64)>,
    /// The resampler was rebuilt because the device rate changed.
    pub rate_changed: bool,
    /// Input frames discarded because the block overlapped audio already emitted.
    pub dropped_input: usize,
}

pub struct TrackResampler {
    quality: Quality,
    anchor_ns: Option<u64>,
    rate: f64,
    rs: Option<Async<f32>>,
    in_buf: Vec<f32>,
    out_buf: Vec<f32>,
    /// Output frames of resampler start-up delay still to drop.
    skip: usize,
    /// Timeline index of the next sample to be emitted.
    emitted: u64,
    /// Timeline position the input consumed so far maps to (fractional).
    virt: f64,
    /// Relative ratio currently in use (`1.0` = the nominal ratio).
    rel: f64,
    integ: f64,
    paused: bool,
}

impl Default for TrackResampler {
    fn default() -> Self {
        Self::new(Quality::default())
    }
}

impl TrackResampler {
    pub fn new(quality: Quality) -> Self {
        Self {
            quality,
            anchor_ns: None,
            rate: 0.0,
            rs: None,
            in_buf: Vec::new(),
            out_buf: Vec::new(),
            skip: 0,
            emitted: 0,
            virt: 0.0,
            rel: 1.0,
            integ: 0.0,
            paused: false,
        }
    }

    /// Sets timeline zero (a host time in ns). Unset, the first block's host
    /// time is used.
    pub fn set_anchor(&mut self, host_ns: u64) {
        self.anchor_ns = Some(host_ns);
    }

    pub fn anchor(&self) -> Option<u64> {
        self.anchor_ns
    }

    /// Timeline index just past the last emitted sample.
    pub fn end_pos(&self) -> u64 {
        self.emitted
    }

    /// Device rate in Hz of the current segment (0 before the first block).
    pub fn device_rate(&self) -> f64 {
        self.rate
    }

    /// The controller's estimate of the device clock error against the host
    /// clock, in ppm (positive: the device runs fast).
    pub fn drift_ppm(&self) -> f64 {
        -(self.rel - 1.0) * 1e6
    }

    /// Ignores input until [`resume`](Self::resume). Buffered input is dropped.
    pub fn pause(&mut self) {
        self.paused = true;
        self.in_buf.clear();
        self.rs = None;
    }

    /// Starts a new segment on the next block. The caller re-anchors the
    /// timeline first so that the block lands at [`end_pos`](Self::end_pos).
    pub fn resume(&mut self) {
        self.paused = false;
        self.rs = None;
    }

    fn nominal_ratio(&self) -> f64 {
        SAMPLE_RATE as f64 / self.rate
    }

    fn build(&self) -> Option<Async<f32>> {
        let ratio = self.nominal_ratio();
        match self.quality {
            Quality::Speech => {
                let params = SincInterpolationParameters::new(128, WindowFunction::BlackmanHarris2)
                    .oversampling_factor(128);
                Async::new_sinc(ratio, MAX_RELATIVE, &params, CHUNK, 1, FixedAsync::Input).ok()
            }
            Quality::Fast => Async::new_poly(
                ratio,
                MAX_RELATIVE,
                PolynomialDegree::Linear,
                CHUNK,
                1,
                FixedAsync::Input,
            )
            .ok(),
        }
    }

    /// Appends `n` zeros to `out` and advances the timeline.
    fn pad(&mut self, n: u64, out: &mut Vec<f32>) {
        out.resize(out.len() + n as usize, 0.0);
        self.emitted += n;
        self.virt += n as f64;
    }

    /// Pads with silence up to timeline index `pos` (a stalled track).
    pub fn pad_to(&mut self, pos: u64, out: &mut Vec<f32>) -> Option<(u64, u64)> {
        if pos <= self.emitted {
            return None;
        }
        let from = self.emitted;
        self.in_buf.clear();
        self.rs = None; // the resampler's tail is stale now
        self.pad(pos - from, out);
        Some((from, pos))
    }

    /// Consumes one captured block and appends its 16 kHz samples to `out`.
    ///
    /// `host_ns` is the capture time of the block's first sample.
    pub fn push(
        &mut self,
        samples: &[f32],
        rate: f64,
        host_ns: u64,
        out: &mut Vec<f32>,
    ) -> PushReport {
        let mut rep = PushReport::default();
        if self.paused || samples.is_empty() || rate < 1000.0 {
            return rep;
        }
        let anchor = *self.anchor_ns.get_or_insert(host_ns);
        let expected = (host_ns as i128 - anchor as i128) as f64 * 1e-9 * SAMPLE_RATE as f64;
        let mut samples = samples;

        let rate_changed = self.rs.is_none() && self.rate != 0.0 && (rate - self.rate).abs() > 0.5
            || self.rs.is_some() && (rate - self.rate).abs() > 0.5;
        let mut restart = self.rs.is_none() || rate_changed;

        if !restart {
            let virt_end = self.virt + self.in_buf.len() as f64 * self.nominal_ratio() * self.rel;
            let err = expected - virt_end;
            if err > GAP_SAMPLES {
                restart = true;
            } else if err < -GAP_SAMPLES {
                let n = ((-err) / (self.nominal_ratio() * self.rel)).ceil() as usize;
                let n = n.min(samples.len());
                rep.dropped_input = n;
                samples = &samples[n..];
                if samples.is_empty() {
                    return rep;
                }
            } else {
                self.control(err, samples.len() as f64 / rate);
            }
        }

        if restart {
            if rate_changed {
                self.integ = 0.0;
                self.rel = 1.0;
                rep.rate_changed = true;
            }
            self.rate = rate;
            self.in_buf.clear();
            let target = expected.round().max(0.0) as u64;
            if target > self.emitted {
                let from = self.emitted;
                self.pad(target - from, out);
                if (target - from) as f64 >= GAP_SAMPLES {
                    rep.gap = Some((from, target));
                }
            } else if (self.emitted as f64) > expected + 1.0 {
                // Overlaps audio already emitted (or precedes the anchor).
                let lead = self.emitted as f64 - expected;
                let n = (lead / self.nominal_ratio()).ceil() as usize;
                let n = n.min(samples.len());
                rep.dropped_input = n;
                samples = &samples[n..];
                if samples.is_empty() {
                    self.rs = None;
                    return rep;
                }
            }
            let Some(rs) = self.build() else {
                return rep;
            };
            self.skip = rs.output_delay();
            self.out_buf = vec![0.0; rs.output_frames_max()];
            self.rs = Some(rs);
            self.virt = self.emitted as f64;
        }

        self.in_buf.extend_from_slice(samples);
        self.run(out);
        rep
    }

    /// PI update of the relative ratio from the timeline error (in samples).
    fn control(&mut self, err: f64, dt: f64) {
        let err_s = err / SAMPLE_RATE as f64;
        let kp = 1.0 / TAU_S;
        let ki = kp * kp / 4.0;
        self.integ = (self.integ + ki * err_s * dt).clamp(-MAX_TRIM, MAX_TRIM);
        self.rel = 1.0 + (kp * err_s + self.integ).clamp(-MAX_TRIM, MAX_TRIM);
    }

    fn run(&mut self, out: &mut Vec<f32>) {
        let Some(rs) = self.rs.as_mut() else {
            return;
        };
        let nominal = SAMPLE_RATE as f64 / self.rate;
        while self.in_buf.len() >= rs.input_frames_next() {
            if rs.set_resample_ratio_relative(self.rel, true).is_err() {
                self.rel = 1.0;
                let _ = rs.set_resample_ratio_relative(1.0, false);
            }
            let n_in = rs.input_frames_next();
            let (Ok(inp), Ok(mut outp)) = (
                InterleavedSlice::new(&self.in_buf[..n_in], 1, n_in),
                InterleavedSlice::new_mut(&mut self.out_buf, 1, rs.output_frames_max()),
            ) else {
                return;
            };
            let Ok((used, made)) = rs.process_into_buffer(&inp, &mut outp, None) else {
                self.in_buf.clear();
                return;
            };
            self.virt += used as f64 * nominal * self.rel;
            let skip = self.skip.min(made);
            self.skip -= skip;
            out.extend_from_slice(&self.out_buf[skip..made]);
            self.emitted += (made - skip) as u64;
            self.in_buf.drain(..used);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE_NS: u64 = 5_000_000_000_000;

    /// Simulated capture: the device clock runs `1 + drift` times faster than
    /// the host clock says. Returns `(click position seen, samples emitted)`.
    struct Sim {
        rs: TrackResampler,
        rate: f64,
        drift: f64,
        block: usize,
        /// Device sample index of the next block.
        n: u64,
        click_at: u64,
        peak: (f32, u64),
        out: Vec<f32>,
        produced: u64,
        clock_offset_ns: u64,
    }

    impl Sim {
        fn new(rate: f64, drift_ppm: f64, block: usize, click_s: f64, q: Quality) -> Self {
            let drift = drift_ppm * 1e-6;
            Sim {
                rs: TrackResampler::new(q),
                rate,
                drift,
                block,
                n: 0,
                click_at: (click_s * rate * (1.0 + drift)).round() as u64,
                peak: (0.0, 0),
                out: Vec::new(),
                produced: 0,
                clock_offset_ns: 0,
            }
        }

        fn host_ns(&self, n: u64) -> u64 {
            BASE_NS
                + self.clock_offset_ns
                + (n as f64 / (self.rate * (1.0 + self.drift)) * 1e9) as u64
        }

        fn feed(&mut self, buf: &mut Vec<f32>) -> PushReport {
            buf.clear();
            buf.resize(self.block, 0.0);
            // A triangular pulse, wide enough to survive cheap interpolation.
            if self.n.abs_diff(self.click_at) < 2 * self.block as u64 + 200 {
                for (i, v) in buf.iter_mut().enumerate() {
                    let d = (self.n + i as u64).abs_diff(self.click_at) as f32;
                    *v = (1.0 - d / 100.0).max(0.0);
                }
            }
            let h = self.host_ns(self.n);
            self.out.clear();
            let rep = self.rs.push(buf, self.rate, h, &mut self.out);
            for (i, &v) in self.out.iter().enumerate() {
                if v.abs() > self.peak.0 {
                    self.peak = (v.abs(), self.produced + i as u64);
                }
            }
            self.produced += self.out.len() as u64;
            self.n += self.block as u64;
            rep
        }

        fn run_seconds(&mut self, secs: f64) {
            let mut buf = Vec::new();
            let target = (secs * self.rate) as u64;
            while self.n < target {
                self.feed(&mut buf);
            }
        }
    }

    fn timeline_secs(pos: u64) -> f64 {
        pos as f64 / SAMPLE_RATE as f64
    }

    #[test]
    fn two_tracks_align_after_simulated_hour_with_drift() {
        // 60 min: 48 kHz mic running +100 ppm fast, 44.1 kHz system -37 ppm slow,
        // click at 3599 s on both. Cheap interpolation and big blocks keep the
        // debug-build test fast; the alignment logic is the same as `Speech`.
        let mut mic = Sim::new(48_000.0, 100.0, 4800, 3599.0, Quality::Fast);
        let mut sys = Sim::new(44_100.0, -37.0, 4410, 3599.0, Quality::Fast);
        mic.rs.set_anchor(BASE_NS);
        sys.rs.set_anchor(BASE_NS);
        std::thread::scope(|sc| {
            sc.spawn(|| mic.run_seconds(3600.0));
            sc.spawn(|| sys.run_seconds(3600.0));
        });
        let want = 3599 * SAMPLE_RATE as u64;
        let (em, es) = (mic.peak.1, sys.peak.1);
        let err_m = timeline_secs(em.abs_diff(want)) * 1e3;
        let err_s = timeline_secs(es.abs_diff(want)) * 1e3;
        let rel = timeline_secs(em.abs_diff(es)) * 1e3;
        eprintln!(
            "60 min drift: mic click err {err_m:.2} ms, system {err_s:.2} ms, mutual {rel:.2} ms; \
             est drift mic {:.0} ppm sys {:.0} ppm",
            mic.rs.drift_ppm(),
            sys.rs.drift_ppm()
        );
        assert!(err_m < 20.0 && err_s < 20.0 && rel < 20.0);
        // The controller found the drift.
        assert!((mic.rs.drift_ppm() - 100.0).abs() < 20.0);
        assert!((sys.rs.drift_ppm() + 37.0).abs() < 20.0);
        // Total output length matches wall time.
        // Emitted length matches the host time span of the last sample.
        let total = mic.rs.end_pos() as f64 / SAMPLE_RATE as f64;
        let host_span = 3600.0 / (1.0 + 100e-6);
        assert!(
            (total - host_span).abs() < 0.05,
            "emitted {total} s, host span {host_span} s"
        );
    }

    #[test]
    fn sinc_quality_tracks_drift_too() {
        let mut s = Sim::new(48_000.0, 250.0, 512, 28.0, Quality::Speech);
        s.rs.set_anchor(BASE_NS);
        s.run_seconds(30.0);
        let err = timeline_secs(s.peak.1.abs_diff(28 * SAMPLE_RATE as u64)) * 1e3;
        eprintln!("sinc 30 s +250 ppm: click err {err:.2} ms");
        assert!(err < 5.0);
    }

    #[test]
    fn host_jitter_does_not_disturb_alignment() {
        // +-1.5 ms of timestamp noise on every 10 ms block.
        let mut s = Sim::new(48_000.0, 60.0, 480, 118.0, Quality::Fast);
        s.rs.set_anchor(BASE_NS);
        let mut buf = Vec::new();
        let mut rng = crate::aec::testsig::Rng(5);
        while s.n < (120.0 * 48_000.0) as u64 {
            s.clock_offset_ns = ((rng.next_f32() * 1.5e6) as i64 + 1_500_000) as u64;
            s.feed(&mut buf);
        }
        let err = timeline_secs(s.peak.1.abs_diff(118 * SAMPLE_RATE as u64)) * 1e3;
        eprintln!("jitter: click err {err:.2} ms");
        assert!(err < 10.0);
    }

    #[test]
    fn gap_is_filled_with_silence_and_reported() {
        // Blocks vanish between 10.0 s and 10.5 s (sleep, device rebuild).
        let mut s = Sim::new(48_000.0, 0.0, 480, 20.0, Quality::Speech);
        s.rs.set_anchor(BASE_NS);
        let mut buf = Vec::new();
        s.run_seconds(10.0);
        let before = s.rs.end_pos();
        s.n += (0.5 * 48_000.0) as u64;
        let rep = s.feed(&mut buf);
        let (from, to) = rep.gap.expect("gap reported");
        assert!(to - from >= (0.49 * 16_000.0) as u64 && to - from <= (0.51 * 16_000.0) as u64);
        assert!(from >= before.saturating_sub(200) && from <= before + 200);
        s.run_seconds(25.0);
        let err = timeline_secs(s.peak.1.abs_diff(20 * SAMPLE_RATE as u64)) * 1e3;
        eprintln!("after a 500 ms gap: click err {err:.2} ms");
        assert!(err < 5.0);
        let total = timeline_secs(s.rs.end_pos());
        assert!((total - 25.0).abs() < 0.05, "{total}");
    }

    #[test]
    fn rate_change_mid_stream_rebuilds_and_stays_aligned() {
        let mut s = Sim::new(48_000.0, 0.0, 480, 1e9, Quality::Speech);
        s.rs.set_anchor(BASE_NS);
        s.run_seconds(5.0);
        // Device switches to 44.1 kHz; host time continues.
        let host_now = s.host_ns(s.n);
        let mut buf = vec![0.0f32; 441];
        buf[100] = 1.0;
        let mut out = Vec::new();
        let rep = s.rs.push(&buf, 44_100.0, host_now, &mut out);
        assert!(rep.rate_changed);
        let mut all = out.clone();
        for i in 1..300u64 {
            let mut b = vec![0.0f32; 441];
            b.fill(0.0);
            let h = host_now + i * 10_000_000;
            s.rs.push(&b, 44_100.0, h, &mut all);
        }
        let idx = all
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap()
            .0 as u64;
        let click_pos = s.rs.end_pos() - all.len() as u64 + idx;
        let want = 5 * SAMPLE_RATE as u64 + (100.0 / 44_100.0 * 16_000.0) as u64;
        let err = timeline_secs(click_pos.abs_diff(want)) * 1e3;
        eprintln!("rate change: click err {err:.2} ms");
        assert!(err < 5.0);
        assert_eq!(s.rs.device_rate(), 44_100.0);
    }

    #[test]
    fn tone_keeps_its_amplitude_and_frequency() {
        // 1 kHz at 44.1 kHz down to 16 kHz with the sinc resampler.
        let mut rs = TrackResampler::new(Quality::Speech);
        rs.set_anchor(BASE_NS);
        let mut out = Vec::new();
        let mut n = 0u64;
        while n < 44_100 * 3 {
            let blk: Vec<f32> = (0..441)
                .map(|i| {
                    (2.0 * std::f64::consts::PI * 1000.0 * (n + i) as f64 / 44_100.0).sin() as f32
                })
                .collect();
            let h = BASE_NS + (n as f64 / 44_100.0 * 1e9) as u64;
            rs.push(&blk, 44_100.0, h, &mut out);
            n += 441;
        }
        // Least-squares fit of a 1 kHz sinusoid on the 16 kHz timeline: the
        // residual measures distortion, the fitted phase the time offset.
        let w = 2.0 * std::f64::consts::PI * 1000.0 / 16_000.0;
        let seg = &out[2000..22_000];
        let (mut sc, mut ss, mut cc, mut sn, mut ssn) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (k, &v) in seg.iter().enumerate() {
            let t = (k + 2000) as f64 * w;
            let (s_, c_) = t.sin_cos();
            sc += s_ * c_;
            ss += s_ * s_;
            cc += c_ * c_;
            sn += v as f64 * s_;
            ssn += v as f64 * c_;
        }
        let det = ss * cc - sc * sc;
        let a_ = (sn * cc - ssn * sc) / det;
        let b_ = (ssn * ss - sn * sc) / det;
        let amp = a_.hypot(b_);
        let phase = b_.atan2(a_);
        let shift_samples = phase / w;
        let resid: f64 = seg
            .iter()
            .enumerate()
            .map(|(k, &v)| {
                let t = (k + 2000) as f64 * w;
                (v as f64 - a_ * t.sin() - b_ * t.cos()).powi(2)
            })
            .sum();
        let sig: f64 = seg.iter().map(|&v| (v as f64).powi(2)).sum();
        let snr = 10.0 * (sig / resid).log10();
        eprintln!(
            "tone: amp {amp:.4}, offset {shift_samples:.2} samples, distortion SNR {snr:.1} dB"
        );
        assert!((amp - 1.0).abs() < 0.02);
        assert!(shift_samples.abs() < 1.5);
        assert!(snr > 40.0);
    }

    #[test]
    fn pause_resume_continues_the_timeline() {
        let mut s = Sim::new(48_000.0, 0.0, 480, 1e9, Quality::Fast);
        s.rs.set_anchor(BASE_NS);
        s.run_seconds(2.0);
        let end = s.rs.end_pos();
        s.rs.pause();
        let mut out = Vec::new();
        assert_eq!(
            s.rs.push(&[0.0; 480], 48_000.0, BASE_NS + 3_000_000_000, &mut out),
            PushReport::default()
        );
        assert!(out.is_empty());
        // Resume 60 s later; the caller moves the anchor so this block lands at `end`.
        let h = BASE_NS + 62_000_000_000;
        s.rs.set_anchor(h - (end as f64 / SAMPLE_RATE as f64 * 1e9) as u64);
        s.rs.resume();
        s.rs.push(&[0.0; 4800], 48_000.0, h, &mut out);
        assert!(s.rs.end_pos() >= end);
        assert!(s.rs.end_pos() < end + 2000);
    }

    #[test]
    fn pad_to_fills_stalled_track() {
        let mut rs = TrackResampler::new(Quality::Fast);
        rs.set_anchor(BASE_NS);
        let mut out = Vec::new();
        assert_eq!(rs.pad_to(1600, &mut out), Some((0, 1600)));
        assert_eq!(out.len(), 1600);
        assert_eq!(rs.pad_to(100, &mut out), None);
        assert_eq!(rs.end_pos(), 1600);
    }
}
