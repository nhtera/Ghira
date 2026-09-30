// SPDX-License-Identifier: Apache-2.0
//! The capture pipeline: rings in, two consumers out.
//!
//! ```text
//! mic ring ──► resample ─┐                      ┌─► FrameSink   raw mic + raw system, markers
//! system ring ► resample ┴─► 10 ms frames ─► AEC ┤   (the writer: never waits on ASR)
//!                        on one 16 kHz timeline  └─► ASR ring    cleaned mic + system
//!                                                    (bounded, skips ahead when full)
//! ```
//!
//! [`Pipeline::step`] is meant to run on one non-realtime thread every 10-20 ms.
//! It has no clock of its own: time comes from the host stamps of the blocks,
//! so it runs the same on devices, on a replayed file and in tests.
//!
//! The writer stores the raw tracks; the echo-cancelled mic exists only for
//! ASR and diarization, and [`MarkerKind::AecOn`]/[`MarkerKind::AecOff`] mark
//! the spans so a final pass can redo it from the raw audio.

use std::collections::VecDeque;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rtrb::{Consumer, Producer, RingBuffer};

use crate::aec::Aec;
use crate::resample::{Quality, TrackResampler};
use crate::ring::RingConsumer;
use crate::{CaptureEvent, FRAME_SAMPLES, Marker, MarkerKind, Route, SAMPLE_RATE, Track};

/// The writer side. Implemented by [`crate::encoder::OpusRecorder`] and by
/// the WAV/test sinks.
pub trait FrameSink {
    /// One 10 ms frame of a raw 16 kHz track. `pos` is its timeline index.
    /// Called for every enabled track, in timeline order, without gaps.
    fn frame(&mut self, track: Track, pos: u64, samples: &[f32]) -> io::Result<()>;
    fn marker(&mut self, marker: &Marker) -> io::Result<()>;
}

/// A sink that discards everything.
pub struct NullSink;

impl FrameSink for NullSink {
    fn frame(&mut self, _: Track, _: u64, _: &[f32]) -> io::Result<()> {
        Ok(())
    }
    fn marker(&mut self, _: &Marker) -> io::Result<()> {
        Ok(())
    }
}

/// One 10 ms frame for ASR.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AsrFrame {
    /// Timeline index of the first sample. A jump between consecutive frames
    /// means the ring overflowed and the frames in between were skipped.
    pub pos: u64,
    /// The mic, echo-cancelled while `aec` is set.
    pub mic: [f32; FRAME_SAMPLES],
    /// The system track (zeros when not captured).
    pub system: [f32; FRAME_SAMPLES],
    pub aec: bool,
}

/// Consumer end of the ASR ring. When ASR falls behind, [`pop`](Self::pop)
/// skips ahead: once more than `high_water` frames wait, the oldest are
/// dropped down to `keep` (about 1 s behind real time) and counted. The
/// reader sees the `pos` jump and the final pass fills the hole from the
/// stored audio. The writer path is never involved. If the ring is full
/// before the reader runs, the pipeline drops the newest frame instead.
pub struct AsrConsumer {
    inner: Consumer<AsrFrame>,
    skipped: Arc<AtomicU64>,
    high_water: usize,
    keep: usize,
}

impl AsrConsumer {
    pub fn pop(&mut self) -> Option<AsrFrame> {
        let n = self.inner.slots();
        if n > self.high_water {
            for _ in 0..n - self.keep {
                let _ = self.inner.pop();
            }
            self.skipped
                .fetch_add((n - self.keep) as u64, Ordering::Relaxed);
        }
        self.inner.pop().ok()
    }

    /// Frames waiting.
    pub fn len(&self) -> usize {
        self.inner.slots()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total frames skipped (dropped oldest, or newest if the ring was full).
    pub fn skipped_frames(&self) -> u64 {
        self.skipped.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Host time (ns) of timeline zero. `None`: the first block that arrives.
    pub start_host_ns: Option<u64>,
    pub route: Route,
    /// The user's echo-cancellation setting; it only runs when the route also
    /// wants it and both tracks are captured.
    pub aec_enabled: bool,
    /// Seconds of digital silence on the system track before
    /// [`CaptureEvent::SilentSystemTrack`] (5 in the CLI, 30 in the app).
    pub silent_system_secs: f32,
    /// A call is known to be in progress (auto-detect hit, or `--mode call`);
    /// the silent-system check only runs while this is set.
    pub call_detected: bool,
    /// ASR ring size in 10 ms frames.
    pub asr_capacity_frames: usize,
    pub quality: Quality,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            start_host_ns: None,
            route: Route::Unknown,
            aec_enabled: true,
            silent_system_secs: 30.0,
            call_detected: false,
            asr_capacity_frames: 3000,
            quality: Quality::Speech,
        }
    }
}

/// What one [`Pipeline::step`] did.
#[derive(Debug, Default)]
pub struct StepReport {
    /// 10 ms frames processed.
    pub frames: usize,
    pub events: Vec<CaptureEvent>,
}

/// A track that trails the leader by more than this is padded with silence.
const STALL_SAMPLES: u64 = SAMPLE_RATE as u64 / 4;
/// A track that never delivered gets this long before it counts as stalled.
const STARTUP_GRACE_SAMPLES: u64 = 2 * SAMPLE_RATE as u64;
/// Frame RMS below this is treated as digital silence (about -80 dBFS).
const SILENCE_RMS: f32 = 1e-4;

pub struct Pipeline<W: FrameSink> {
    cfg: PipelineConfig,
    writer: W,
    writer_failed: bool,
    rx: [Option<RingConsumer>; 2],
    rs: [TrackResampler; 2],
    buf: [VecDeque<f32>; 2],
    started: [bool; 2],
    lost: [bool; 2],
    /// Timeline index of the next frame to emit (a multiple of 160).
    next_pos: u64,
    paused: bool,
    /// Between Sleep and Wake: like a pause, but driven by the platform.
    sleeping: bool,
    /// End of the last padded stretch on the system track (not real silence).
    sys_pad_end: u64,
    call_detected: bool,
    /// The next block sets the timeline anchor (start, or after a resume).
    rebase: bool,
    aec: Aec,
    aec_active: bool,
    aec_user: bool,
    route: Route,
    asr: Producer<AsrFrame>,
    asr_skipped: Arc<AtomicU64>,
    asr_reported: u64,
    events: Vec<CaptureEvent>,
    scratch: Vec<f32>,
    out: Vec<f32>,
    silent_frames: u64,
    silent_reported: bool,
}

impl<W: FrameSink> Pipeline<W> {
    /// Builds a pipeline over the two rings (`None`: that track is not
    /// captured, e.g. room mode has no system ring) and its ASR consumer.
    pub fn new(
        cfg: PipelineConfig,
        mic: Option<RingConsumer>,
        system: Option<RingConsumer>,
        writer: W,
    ) -> (Self, AsrConsumer) {
        let cfg_capacity = cfg.asr_capacity_frames.max(1);
        let (asr, asr_rx) = RingBuffer::<AsrFrame>::new(cfg_capacity);
        let skipped = Arc::new(AtomicU64::new(0));
        let mut rs = [
            TrackResampler::new(cfg.quality),
            TrackResampler::new(cfg.quality),
        ];
        let rebase = cfg.start_host_ns.is_none();
        if let Some(ns) = cfg.start_host_ns {
            for r in &mut rs {
                r.set_anchor(ns);
            }
        }
        let cfg_call = cfg.call_detected;
        let p = Self {
            route: cfg.route,
            aec_user: cfg.aec_enabled,
            cfg,
            writer,
            writer_failed: false,
            rx: [mic, system],
            rs,
            buf: [VecDeque::new(), VecDeque::new()],
            started: [false; 2],
            lost: [false; 2],
            next_pos: 0,
            paused: false,
            sleeping: false,
            sys_pad_end: 0,
            call_detected: cfg_call,
            rebase,
            aec: Aec::new(),
            aec_active: false,
            asr,
            asr_skipped: skipped.clone(),
            asr_reported: 0,
            events: Vec::new(),
            scratch: Vec::new(),
            out: Vec::new(),
            silent_frames: 0,
            silent_reported: false,
        };
        let high_water = (cfg_capacity / 2).clamp(2, 300);
        (
            p,
            AsrConsumer {
                inner: asr_rx,
                skipped,
                high_water,
                keep: (high_water / 3).max(1),
            },
        )
    }

    pub fn writer(&self) -> &W {
        &self.writer
    }

    pub fn writer_mut(&mut self) -> &mut W {
        &mut self.writer
    }

    /// The writer returned an error (disk full...) and is no longer called.
    /// ASR keeps running; the caller decides whether to stop the session.
    pub fn writer_failed(&self) -> bool {
        self.writer_failed
    }

    /// Timeline position of the next frame, in samples. Divide by
    /// [`SAMPLE_RATE`] for seconds of recorded (unpaused) time.
    pub fn position(&self) -> u64 {
        self.next_pos
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn aec_active(&self) -> bool {
        self.aec_active
    }

    /// AEC3's own ERLE estimate while it runs.
    pub fn erle_db(&self) -> Option<f64> {
        self.aec_active.then(|| self.aec.erle_db()).flatten()
    }

    fn enabled(&self, t: Track) -> bool {
        self.rx[t.index()].is_some()
    }

    fn marker(&mut self, pos: u64, kind: MarkerKind) {
        if self.writer_failed {
            return;
        }
        if let Err(e) = self.writer.marker(&Marker { pos, kind }) {
            self.fail(e);
        }
    }

    fn fail(&mut self, e: io::Error) {
        self.writer_failed = true;
        self.events.push(
            if e.kind() == io::ErrorKind::StorageFull || e.raw_os_error() == Some(28) {
                CaptureEvent::DiskFull
            } else {
                CaptureEvent::Error {
                    code: e.raw_os_error().map_or(-1, i64::from),
                    detail: format!("writer: {e}"),
                }
            },
        );
    }

    // --- control ------------------------------------------------------------

    /// The output route changed. Echo cancellation follows the route and
    /// restarts from a fresh filter.
    pub fn set_route(&mut self, route: Route) {
        if route != self.route {
            self.route = route;
            self.aec.reset();
        }
    }

    pub fn set_aec_enabled(&mut self, on: bool) {
        self.aec_user = on;
    }

    /// Stops the timeline. Whatever is buffered is flushed, the pause marker is
    /// written and later blocks are discarded until [`resume`](Self::resume).
    pub fn pause(&mut self) {
        if self.paused {
            return;
        }
        self.step();
        self.flush_partial();
        self.marker(self.next_pos, MarkerKind::Pause);
        self.paused = true;
        for r in &mut self.rs {
            r.pause();
        }
    }

    /// Continues the timeline where it stopped: paused time is not recorded.
    pub fn resume(&mut self) {
        if !self.paused {
            return;
        }
        self.paused = false;
        self.rebase = true;
        for r in &mut self.rs {
            r.resume();
        }
        self.marker(self.next_pos, MarkerKind::Resume);
    }

    /// Marks the last `last_s` seconds as discarded (capture layer only).
    pub fn discard(&mut self, last_s: f32) {
        self.marker(self.next_pos, MarkerKind::Discard { last_s });
    }

    /// Tells the silent-system check whether a call is in progress.
    pub fn set_call_detected(&mut self, on: bool) {
        self.call_detected = on;
        self.silent_frames = 0;
        self.silent_reported = false;
    }

    fn inactive(&self) -> bool {
        self.paused || self.sleeping
    }

    /// The Mac is going to sleep. Like a pause: buffered audio is flushed, a
    /// `Sleep` marker is written and the timeline stops until [`wake`](Self::wake).
    pub fn sleep(&mut self) {
        if self.inactive() {
            return;
        }
        self.step();
        self.flush_partial();
        self.marker(self.next_pos, MarkerKind::Sleep);
        self.sleeping = true;
        for r in &mut self.rs {
            r.pause();
        }
    }

    /// The Mac woke after `slept_wall` of wall time (recorded in the marker
    /// only). The timeline continues where it stopped: the next block sets a
    /// new anchor, nothing is padded.
    pub fn wake(&mut self, slept_wall: Option<Duration>) {
        if !self.sleeping {
            return;
        }
        self.sleeping = false;
        self.silent_frames = 0;
        self.silent_reported = false;
        if !self.paused {
            self.rebase = true;
            for r in &mut self.rs {
                r.resume();
            }
        }
        let slept_s = slept_wall.map_or(0.0, |d| d.as_secs_f32());
        self.marker(self.next_pos, MarkerKind::Wake { slept_s });
        self.events.push(CaptureEvent::Wake);
    }

    /// Applies an event from the platform layer and queues it for the caller.
    pub fn handle_event(&mut self, ev: CaptureEvent) {
        match &ev {
            CaptureEvent::RouteChanged { route, .. } => self.set_route(*route),
            CaptureEvent::MicDeviceChanged { .. } | CaptureEvent::SystemRestarted => {
                self.aec.reset();
            }
            CaptureEvent::TrackLost { track } => self.lost[track.index()] = true,
            CaptureEvent::Sleep => self.sleep(),
            // Use `wake` directly to record the wall-clock sleep length.
            CaptureEvent::Wake => {
                self.wake(None);
                return;
            }
            _ => {}
        }
        self.events.push(ev);
    }

    // --- processing ---------------------------------------------------------

    /// Drains the rings and processes every complete frame.
    pub fn step(&mut self) -> StepReport {
        self.drain();
        let frames = self.emit_frames();
        let skipped = self.asr_skipped.load(Ordering::Relaxed);
        if skipped > self.asr_reported {
            self.events.push(CaptureEvent::AsrSkipped {
                frames: skipped - self.asr_reported,
            });
            self.asr_reported = skipped;
        }
        StepReport {
            frames,
            events: std::mem::take(&mut self.events),
        }
    }

    fn drain(&mut self) {
        for t in Track::ALL {
            let i = t.index();
            while let Some((rate, host)) = self.rx[i]
                .as_mut()
                .and_then(|rx| rx.pop_into(&mut self.scratch))
            {
                if self.inactive() {
                    continue;
                }
                if self.rebase {
                    // Timeline zero (or the resume point) is this block's time.
                    let anchor = host.saturating_sub(
                        (self.next_pos as u128 * 1_000_000_000 / SAMPLE_RATE as u128) as u64,
                    );
                    for r in &mut self.rs {
                        r.set_anchor(anchor);
                    }
                    self.rebase = false;
                }
                self.started[i] = true;
                self.lost[i] = false;
                self.out.clear();
                let rep = self.rs[i].push(&self.scratch, rate, host, &mut self.out);
                self.buf[i].extend(self.out.iter().copied());
                if let Some((from, to)) = rep.gap {
                    if t == Track::System {
                        self.sys_pad_end = self.sys_pad_end.max(to);
                    }
                    self.marker(from, MarkerKind::Gap { track: t, from, to });
                }
            }
            if let Some(rx) = self.rx[i].as_mut() {
                let dropped = rx.take_new_drops();
                if dropped > 0 {
                    self.events
                        .push(CaptureEvent::Overrun { track: t, dropped });
                }
            }
        }
    }

    /// End of buffered audio of `t` on the timeline.
    fn end_of(&self, t: Track) -> u64 {
        self.rs[t.index()].end_pos()
    }

    /// Pads tracks that fell behind and returns how far all tracks have data.
    fn ready_end(&mut self) -> Option<u64> {
        let live: Vec<Track> = Track::ALL
            .into_iter()
            .filter(|&t| self.enabled(t) && !self.inactive())
            .collect();
        let lead = live
            .iter()
            .filter(|&&t| !self.lost[t.index()])
            .map(|&t| self.end_of(t))
            .max();
        let lead = lead.or_else(|| live.iter().map(|&t| self.end_of(t)).max())?;
        for &t in &live {
            let i = t.index();
            let behind = lead.saturating_sub(self.end_of(t));
            let limit = if self.started[i] {
                STALL_SAMPLES
            } else {
                STARTUP_GRACE_SAMPLES
            };
            if self.lost[i] || behind > limit {
                self.out.clear();
                if let Some((from, to)) = self.rs[i].pad_to(lead, &mut self.out) {
                    if t == Track::System {
                        self.sys_pad_end = self.sys_pad_end.max(to);
                    }
                    self.buf[i].extend(self.out.iter().copied());
                    self.marker(from, MarkerKind::Gap { track: t, from, to });
                }
            }
        }
        live.iter().map(|&t| self.end_of(t)).min()
    }

    fn emit_frames(&mut self) -> usize {
        let Some(end) = self.ready_end() else {
            return 0;
        };
        let mut n = 0;
        while self.next_pos + FRAME_SAMPLES as u64 <= end {
            self.emit_one();
            n += 1;
        }
        n
    }

    fn emit_one(&mut self) {
        let pos = self.next_pos;
        let mut mic = [0f32; FRAME_SAMPLES];
        let mut sys = [0f32; FRAME_SAMPLES];
        for (t, frame) in [(Track::Mic, &mut mic), (Track::System, &mut sys)] {
            for s in frame.iter_mut() {
                *s = self.buf[t.index()].pop_front().unwrap_or(0.0);
            }
        }
        let (mic_on, sys_on) = (self.enabled(Track::Mic), self.enabled(Track::System));

        let want = self.aec_user && self.route.wants_aec() && mic_on && sys_on;
        if want != self.aec_active {
            self.aec_active = want;
            self.aec.set_enabled(want);
            self.marker(
                pos,
                if want {
                    MarkerKind::AecOn
                } else {
                    MarkerKind::AecOff
                },
            );
        }

        if !self.writer_failed {
            let mut res = Ok(());
            if mic_on {
                res = self.writer.frame(Track::Mic, pos, &mic);
            }
            if res.is_ok() && sys_on {
                res = self.writer.frame(Track::System, pos, &sys);
            }
            if let Err(e) = res {
                self.fail(e);
            }
        }

        let mut clean = mic;
        if self.aec_active {
            self.aec.process(&sys, &mic, &mut clean);
        }
        let frame = AsrFrame {
            pos,
            mic: clean,
            system: sys,
            aec: self.aec_active,
        };
        if self.asr.push(frame).is_err() {
            self.asr_skipped.fetch_add(1, Ordering::Relaxed);
        }

        if !sys_on
            || !self.call_detected
            || pos < self.sys_pad_end
            || self.lost[Track::System.index()]
        {
            // Padded silence is not evidence of a broken tap.
            self.silent_frames = 0;
            self.silent_reported = false;
        } else {
            let rms = (sys.iter().map(|s| s * s).sum::<f32>() / FRAME_SAMPLES as f32).sqrt();
            if rms < SILENCE_RMS {
                self.silent_frames += 1;
                let limit = (self.cfg.silent_system_secs * 100.0) as u64;
                if !self.silent_reported && self.silent_frames >= limit {
                    self.silent_reported = true;
                    self.events.push(CaptureEvent::SilentSystemTrack {
                        silent_s: self.silent_frames as f32 / 100.0,
                    });
                }
            } else {
                self.silent_frames = 0;
                self.silent_reported = false;
            }
        }
        self.next_pos += FRAME_SAMPLES as u64;
    }

    /// Pads every enabled track to the next frame boundary past the leader and
    /// emits what is left, so nothing stays buffered across a pause or at the end.
    fn flush_partial(&mut self) {
        let fs = FRAME_SAMPLES as u64;
        let lead = Track::ALL
            .into_iter()
            .filter(|&t| self.enabled(t))
            .map(|t| self.end_of(t))
            .max();
        let Some(lead) = lead else { return };
        let target = lead.div_ceil(fs) * fs;
        for t in Track::ALL {
            let i = t.index();
            if self.enabled(t) {
                self.out.clear();
                self.rs[i].pad_to(target, &mut self.out);
                self.buf[i].extend(self.out.iter().copied());
            }
        }
        while self.next_pos + fs <= target {
            self.emit_one();
        }
    }

    /// Ends the session: drains, flushes and returns the writer.
    pub fn finish(mut self) -> W {
        if !self.inactive() {
            self.step();
            self.flush_partial();
        }
        self.writer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aec::testsig::{Rng, convolve, room_ir, speechlike};
    use crate::ring::{RingProducer, ring};

    const BASE: u64 = 7_000_000_000_000;

    #[derive(Default)]
    struct Capture {
        frames: [Vec<f32>; 2],
        last_pos: [Option<u64>; 2],
        markers: Vec<Marker>,
        /// Fail every write from this many mic frames on.
        fail_after: Option<usize>,
        calls_after_fail: usize,
    }

    impl FrameSink for Capture {
        fn frame(&mut self, t: Track, pos: u64, s: &[f32]) -> io::Result<()> {
            if self
                .fail_after
                .is_some_and(|n| self.frames[0].len() >= n * FRAME_SAMPLES)
            {
                self.calls_after_fail += 1;
                return Err(io::Error::from_raw_os_error(28));
            }
            let i = t.index();
            if let Some(p) = self.last_pos[i] {
                assert_eq!(
                    pos,
                    p + FRAME_SAMPLES as u64,
                    "writer frames are contiguous"
                );
            }
            self.last_pos[i] = Some(pos);
            self.frames[i].extend_from_slice(s);
            Ok(())
        }
        fn marker(&mut self, m: &Marker) -> io::Result<()> {
            self.markers.push(*m);
            Ok(())
        }
    }

    /// A fake device: turns a 16 kHz signal into blocks at `rate`.
    struct Source {
        prod: RingProducer,
        rate: f64,
        sig: Vec<f32>,
        n: u64,
    }

    impl Source {
        fn new(prod: RingProducer, rate: f64, sig: Vec<f32>) -> Self {
            Self {
                prod,
                rate,
                sig,
                n: 0,
            }
        }

        /// Pushes the next 10 ms of the signal, resampled by linear
        /// interpolation to the device rate. `false` when the signal is over.
        fn push_10ms(&mut self) -> bool {
            let frames = (self.rate / 100.0) as u64;
            let mut blk = Vec::with_capacity(frames as usize);
            for k in 0..frames {
                let t = (self.n + k) as f64 / self.rate * SAMPLE_RATE as f64;
                let i = t as usize;
                if i + 1 >= self.sig.len() {
                    return false;
                }
                let f = (t - i as f64) as f32;
                blk.push(self.sig[i] * (1.0 - f) + self.sig[i + 1] * f);
            }
            let host = BASE + (self.n as f64 / self.rate * 1e9) as u64;
            self.prod.push(&blk, self.rate, host);
            self.n += frames;
            true
        }

        /// Skips `secs` of the device stream without pushing (lost blocks).
        fn skip(&mut self, secs: f64) {
            self.n += (secs * self.rate) as u64;
        }
    }

    fn run_pipeline(
        p: &mut Pipeline<Capture>,
        mic: &mut Source,
        sys: Option<&mut Source>,
        secs: usize,
        asr: Option<&mut AsrConsumer>,
        mut on_asr: impl FnMut(AsrFrame),
    ) -> Vec<CaptureEvent> {
        let mut sys = sys;
        let mut asr = asr;
        let mut events = Vec::new();
        for _ in 0..secs * 100 {
            if !mic.push_10ms() {
                break;
            }
            if let Some(s) = sys.as_deref_mut() {
                s.push_10ms();
            }
            events.extend(p.step().events);
            if let Some(a) = asr.as_deref_mut() {
                while let Some(f) = a.pop() {
                    on_asr(f);
                }
            }
        }
        events
    }

    fn rms(x: &[f32]) -> f64 {
        (x.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
    }

    fn setup(
        cfg: PipelineConfig,
        both: bool,
    ) -> (
        Pipeline<Capture>,
        AsrConsumer,
        RingProducer,
        Option<RingProducer>,
    ) {
        let (mp, mc) = ring(1 << 17);
        let (sp, sc) = ring(1 << 17);
        let (p, a) = Pipeline::new(cfg, Some(mc), both.then_some(sc), Capture::default());
        (p, a, mp, both.then_some(sp))
    }

    fn fast_cfg() -> PipelineConfig {
        PipelineConfig {
            quality: Quality::Fast,
            ..Default::default()
        }
    }

    #[test]
    fn end_to_end_echo_is_cancelled_for_asr_but_raw_for_the_writer() {
        // 48 kHz mic hears a 44.1 kHz system track through a room.
        let secs = 9;
        let far = speechlike(7, secs as f32 + 1.0, 0.6);
        let echo = convolve(&far, &room_ir(3, 1000, 0.12, 60.0, 0.3));
        let mut rng = Rng(1);
        let mic_sig: Vec<f32> = echo.iter().map(|e| e + 0.0005 * rng.next_f32()).collect();

        let cfg = PipelineConfig {
            route: Route::Speakers,
            ..fast_cfg()
        };
        let (mut p, mut asr, mp, sp) = setup(cfg, true);
        let mut mic = Source::new(mp, 48_000.0, mic_sig.clone());
        let mut sys = Source::new(sp.unwrap(), 44_100.0, far.clone());
        let mut asr_mic = Vec::new();
        let mut asr_pos = Vec::new();
        run_pipeline(
            &mut p,
            &mut mic,
            Some(&mut sys),
            secs,
            Some(&mut asr),
            |f| {
                assert!(f.aec);
                asr_pos.push(f.pos);
                asr_mic.extend_from_slice(&f.mic);
            },
        );
        let erle = p.erle_db();
        let w = p.finish();

        // Writer: both raw tracks, same length, contiguous, ~9 s.
        assert_eq!(w.frames[0].len(), w.frames[1].len());
        let n = w.frames[0].len();
        assert!(n >= secs * 16_000 - 3200, "{n} samples written");
        // Raw mic is untouched echo: same level as the source echo.
        let sr = SAMPLE_RATE as usize;
        let raw = rms(&w.frames[0][6 * sr..8 * sr]);
        let src = rms(&mic_sig[6 * sr..8 * sr]);
        assert!((raw / src - 1.0).abs() < 0.15, "raw {raw} vs source {src}");
        // ASR mic: echo mostly gone after convergence.
        let cleaned = rms(&asr_mic[6 * sr..8 * sr]);
        let erle_meas = 20.0 * (raw / cleaned.max(1e-9)).log10();
        eprintln!("pipeline ERLE (6-8 s): {erle_meas:.1} dB, AEC3 estimate {erle:?}");
        assert!(erle_meas >= 15.0, "ERLE {erle_meas}");
        // ASR positions are consecutive; markers show AEC on from the start.
        assert!(asr_pos.windows(2).all(|w| w[1] == w[0] + 160));
        assert_eq!(
            w.markers[0],
            Marker {
                pos: 0,
                kind: MarkerKind::AecOn
            }
        );
        // Alignment: writer system equals the source (16 kHz) signal.
        let lag_ok = (0..200usize).any(|l| {
            let a = &w.frames[1][sr..2 * sr];
            let b = &far[sr + l..2 * sr + l];
            a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / (sr as f32) < 0.01
        });
        assert!(lag_ok, "system track does not match its source");
    }

    #[test]
    fn writer_never_blocked_by_a_stuck_asr_consumer_and_asr_skips_ahead() {
        let secs = 5;
        let cfg = PipelineConfig {
            asr_capacity_frames: 1000,
            ..fast_cfg()
        };
        let (mut p, mut asr, mp, sp) = setup(cfg, true);
        let mut mic = Source::new(mp, 48_000.0, speechlike(1, 6.0, 0.5));
        let mut sys = Source::new(sp.unwrap(), 48_000.0, speechlike(2, 6.0, 0.5));
        // Nobody reads the ASR ring for 5 s.
        run_pipeline(&mut p, &mut mic, Some(&mut sys), secs, None, |_| {});
        let w = p.writer();
        assert!(
            w.frames[0].len() >= 4 * 16_000,
            "writer got {} samples",
            w.frames[0].len()
        );
        assert_eq!(w.frames[0].len(), w.frames[1].len());
        let queued = asr.len();
        assert!(queued > 450, "{queued} frames queued");
        // The reader wakes up: it lands about 1 s behind real time.
        let first = asr.pop().expect("frame");
        let skipped = asr.skipped_frames();
        assert!(skipped > 300, "skipped {skipped}");
        assert!(first.pos >= skipped * 160, "first pos {}", first.pos);
        assert!(asr.len() <= 101, "{} left", asr.len());
        let mut last = first.pos;
        while let Some(f) = asr.pop() {
            assert_eq!(f.pos, last + 160, "no jumps after the skip");
            last = f.pos;
        }
        // The skip is reported as an event on the next step.
        mic.push_10ms();
        sys.push_10ms();
        let ev = p.step().events;
        assert!(
            ev.iter()
                .any(|e| matches!(e, CaptureEvent::AsrSkipped { frames } if *frames == skipped)),
            "{ev:?}"
        );
    }

    #[test]
    fn room_mode_has_only_the_mic_and_no_aec() {
        let (mut p, mut asr, mp, _) = setup(fast_cfg(), false);
        let mut mic = Source::new(mp, 48_000.0, speechlike(4, 3.0, 0.5));
        run_pipeline(&mut p, &mut mic, None, 2, Some(&mut asr), |f| {
            assert!(!f.aec)
        });
        let w = p.finish();
        assert!(w.frames[0].len() >= 30_000);
        assert!(w.frames[1].is_empty());
        assert!(w.markers.is_empty());
    }

    #[test]
    fn silent_system_track_is_reported_once_per_stretch() {
        let cfg = PipelineConfig {
            silent_system_secs: 3.0,
            call_detected: true,
            route: Route::Headphones,
            ..fast_cfg()
        };
        let (mut p, _asr, mp, sp) = setup(cfg, true);
        // 0-1 s audio, 1-6 s silence, 6-7 s audio, 7-12 s silence.
        let mut sig = speechlike(2, 1.0, 0.5);
        sig.extend(vec![0.0; 5 * 16_000]);
        sig.extend(speechlike(3, 1.0, 0.5));
        sig.extend(vec![0.0; 6 * 16_000]);
        let mut mic = Source::new(mp, 48_000.0, speechlike(9, 13.0, 0.3));
        let mut sys = Source::new(sp.unwrap(), 44_100.0, sig);
        let ev = run_pipeline(&mut p, &mut mic, Some(&mut sys), 12, None, |_| {});
        let silent: Vec<_> = ev
            .iter()
            .filter_map(|e| match e {
                CaptureEvent::SilentSystemTrack { silent_s } => Some(*silent_s),
                _ => None,
            })
            .collect();
        assert_eq!(silent.len(), 2, "{ev:?}");
        assert!(silent.iter().all(|&s| (3.0..3.2).contains(&s)));
        assert!(!p.aec_active(), "headphones: no AEC");
    }

    #[test]
    fn route_flips_toggle_aec_with_markers() {
        let (mut p, mut asr, mp, sp) = setup(
            PipelineConfig {
                route: Route::Speakers,
                ..fast_cfg()
            },
            true,
        );
        let mut mic = Source::new(mp, 48_000.0, speechlike(1, 10.0, 0.4));
        let mut sys = Source::new(sp.unwrap(), 48_000.0, speechlike(2, 10.0, 0.4));
        let mut flags = Vec::new();
        let mut go = |p: &mut Pipeline<Capture>, secs| {
            run_pipeline(p, &mut mic, Some(&mut sys), secs, Some(&mut asr), |f| {
                flags.push(f.aec)
            });
        };
        go(&mut p, 1);
        p.handle_event(CaptureEvent::RouteChanged {
            route: Route::Headphones,
            input_bluetooth_hfp: false,
        });
        go(&mut p, 1);
        p.handle_event(CaptureEvent::RouteChanged {
            route: Route::Speakers,
            input_bluetooth_hfp: false,
        });
        go(&mut p, 1);
        p.set_aec_enabled(false);
        go(&mut p, 1);
        let w = p.finish();
        let kinds: Vec<_> = w.markers.iter().map(|m| m.kind).collect();
        assert_eq!(
            kinds,
            vec![
                MarkerKind::AecOn,
                MarkerKind::AecOff,
                MarkerKind::AecOn,
                MarkerKind::AecOff
            ]
        );
        let pos: Vec<_> = w
            .markers
            .iter()
            .map(|m| (m.pos as f64 / 16_000.0).round() as u64)
            .collect();
        assert_eq!(pos, vec![0, 1, 2, 3]);
        let on = flags.iter().filter(|&&f| f).count();
        assert!((190..=205).contains(&on), "{on} AEC frames");
        // The stored mic is raw in every span.
    }

    #[test]
    fn pause_resume_discard_markers_and_a_continuous_timeline() {
        let (mut p, _asr, mp, sp) = setup(
            PipelineConfig {
                route: Route::Headphones,
                ..fast_cfg()
            },
            true,
        );
        let mut mic = Source::new(mp, 48_000.0, speechlike(1, 20.0, 0.4));
        let mut sys = Source::new(sp.unwrap(), 44_100.0, speechlike(2, 20.0, 0.4));
        run_pipeline(&mut p, &mut mic, Some(&mut sys), 2, None, |_| {});
        p.discard(30.0);
        p.pause();
        assert!(p.is_paused());
        let paused_at = p.position();
        assert_eq!(paused_at % 160, 0);
        // 5 s of the devices being stopped: nothing arrives; but a stray
        // block that was already queued is dropped.
        mic.skip(5.0);
        sys.skip(5.0);
        p.step();
        p.resume();
        run_pipeline(&mut p, &mut mic, Some(&mut sys), 2, None, |_| {});
        let end = p.position();
        let w = p.finish();
        // Pause time is not on the timeline: 2 s + 2 s.
        assert!((end as i64 - 64_000).abs() < 1_600, "end {end}");
        assert_eq!(w.frames[0].len(), w.frames[1].len());
        let kinds: Vec<_> = w.markers.iter().map(|m| (m.kind, m.pos)).collect();
        assert_eq!(kinds[0].0, MarkerKind::Discard { last_s: 30.0 });
        assert_eq!(kinds[1], (MarkerKind::Pause, paused_at));
        assert_eq!(kinds[2], (MarkerKind::Resume, paused_at));
        // No gap markers from the resume itself.
        assert!(
            kinds.iter().all(|k| !matches!(k.0, MarkerKind::Gap { .. })),
            "{kinds:?}"
        );
        // Both tracks stayed aligned across the pause: the audio right after
        // the resume is the source audio at 7 s, on both tracks.
        let a = paused_at as usize;
        let chk = |t: usize, rate_sig: &Vec<f32>| {
            let seg = &w.frames[t][a + 3200..a + 6400];
            (0..1600usize).any(|l| {
                let base = 7 * 16_000 + 3200 + l;
                let sig = &rate_sig[base..base + 3200];
                seg.iter().zip(sig).map(|(x, y)| (x - y).abs()).sum::<f32>() / 3200.0 < 0.02
            })
        };
        assert!(chk(0, &mic.sig) && chk(1, &sys.sig));
    }

    #[test]
    fn dropped_blocks_become_silence_and_are_reported() {
        // A tiny system ring that overflows while the pipeline thread is stalled.
        let (mp, mc) = ring(1 << 17);
        let (mut sp, sc) = ring(1200);
        let (mut p, _asr) = Pipeline::new(fast_cfg(), Some(mc), Some(sc), Capture::default());
        let mut mic = Source::new(mp, 48_000.0, speechlike(1, 8.0, 0.4));
        let sig = speechlike(2, 8.0, 0.4);
        let mut sys = Source {
            prod: RingProducer::clone_for_test(&mut sp),
            rate: 48_000.0,
            sig,
            n: 0,
        };
        let mut events = Vec::new();
        for i in 0..400 {
            mic.push_10ms();
            sys.push_10ms();
            // The pipeline thread stalls between 1 s and 1.5 s.
            if !(100..150).contains(&i) {
                events.extend(p.step().events);
            }
        }
        events.extend(p.step().events);
        let w = p.finish();
        assert!(events.iter().any(|e| matches!(e, CaptureEvent::Overrun { track: Track::System, dropped } if *dropped > 0)), "{events:?}");
        assert!(w.markers.iter().any(|m| matches!(
            m.kind,
            MarkerKind::Gap {
                track: Track::System,
                ..
            }
        )));
        // Timeline intact: both tracks have the full ~4 s.
        assert_eq!(w.frames[0].len(), w.frames[1].len());
        assert!(w.frames[1].len() >= 3 * 16_000 + 12_000);
    }

    #[test]
    fn sleep_stops_the_timeline_and_wake_rebases_without_padding() {
        let (mut p, _asr, mp, sp) = setup(
            PipelineConfig {
                route: Route::Headphones,
                ..fast_cfg()
            },
            true,
        );
        let mut mic = Source::new(mp, 48_000.0, speechlike(1, 40.0, 0.4));
        let mut sys = Source::new(sp.unwrap(), 48_000.0, speechlike(2, 40.0, 0.4));
        run_pipeline(&mut p, &mut mic, Some(&mut sys), 2, None, |_| {});
        p.handle_event(CaptureEvent::Sleep);
        let slept_at = p.position();
        // Blocks that straggle in during sleep are dropped.
        mic.push_10ms();
        p.step();
        assert_eq!(p.position(), slept_at);
        // The capture clock jumps (here by 3 s) and 20 s of wall time passed.
        mic.skip(3.0);
        sys.skip(3.0);
        p.wake(Some(Duration::from_secs(20)));
        run_pipeline(&mut p, &mut mic, Some(&mut sys), 2, None, |_| {});
        let end = p.position();
        let w = p.finish();
        assert!(
            w.markers
                .iter()
                .all(|m| !matches!(m.kind, MarkerKind::Gap { .. })),
            "no gap fill: {:?}",
            w.markers
        );
        let kinds: Vec<_> = w.markers.iter().map(|m| (m.kind, m.pos)).collect();
        assert_eq!(kinds[0], (MarkerKind::Sleep, slept_at));
        assert_eq!(kinds[1], (MarkerKind::Wake { slept_s: 20.0 }, slept_at));
        // 2 s + 2 s recorded, sleep is not on the timeline.
        assert!((end as f64 / 16_000.0 - 4.0).abs() < 0.2, "timeline {end}");
        assert_eq!(w.frames[0].len(), w.frames[1].len());
    }

    #[test]
    fn padded_silence_and_a_missing_call_do_not_trip_the_silent_detector() {
        // Not in a call: a silent system track is fine.
        let cfg = PipelineConfig {
            silent_system_secs: 2.0,
            route: Route::Headphones,
            ..fast_cfg()
        };
        let (mut p, _a, mp, sp) = setup(cfg, true);
        let mut mic = Source::new(mp, 48_000.0, speechlike(9, 8.0, 0.3));
        let mut sys = Source::new(sp.unwrap(), 48_000.0, vec![0.0; 8 * 16_000]);
        let ev = run_pipeline(&mut p, &mut mic, Some(&mut sys), 5, None, |_| {});
        assert!(
            !ev.iter()
                .any(|e| matches!(e, CaptureEvent::SilentSystemTrack { .. }))
        );
        // The gate opens: now real silence counts.
        p.set_call_detected(true);
        let ev = run_pipeline(&mut p, &mut mic, Some(&mut sys), 2, None, |_| {});
        assert!(
            ev.iter()
                .any(|e| matches!(e, CaptureEvent::SilentSystemTrack { .. }))
        );

        // A lost system track is padded with zeros, which is not "silent tap".
        let cfg = PipelineConfig {
            silent_system_secs: 2.0,
            call_detected: true,
            route: Route::Headphones,
            ..fast_cfg()
        };
        let (mut p, _a, mp, sp) = setup(cfg, true);
        let mut mic = Source::new(mp, 48_000.0, speechlike(9, 8.0, 0.3));
        let mut sys = Source::new(sp.unwrap(), 48_000.0, speechlike(2, 8.0, 0.3));
        run_pipeline(&mut p, &mut mic, Some(&mut sys), 1, None, |_| {});
        p.handle_event(CaptureEvent::TrackLost {
            track: Track::System,
        });
        let mut ev = Vec::new();
        for _ in 0..500 {
            mic.push_10ms();
            ev.extend(p.step().events);
        }
        assert!(
            !ev.iter()
                .any(|e| matches!(e, CaptureEvent::SilentSystemTrack { .. })),
            "{ev:?}"
        );
    }

    #[test]
    fn a_lost_track_is_padded_and_the_other_keeps_flowing() {
        let (mut p, _asr, mp, sp) = setup(
            PipelineConfig {
                route: Route::Headphones,
                ..fast_cfg()
            },
            true,
        );
        let mut mic = Source::new(mp, 48_000.0, speechlike(1, 10.0, 0.4));
        let mut sys = Source::new(sp.unwrap(), 48_000.0, speechlike(2, 10.0, 0.4));
        run_pipeline(&mut p, &mut mic, Some(&mut sys), 1, None, |_| {});
        p.handle_event(CaptureEvent::TrackLost {
            track: Track::System,
        });
        // Only the mic delivers for 3 s.
        for _ in 0..300 {
            mic.push_10ms();
            p.step();
        }
        let w = p.finish();
        assert!(w.frames[0].len() >= 4 * 16_000 - 1_600);
        assert_eq!(
            w.frames[0].len(),
            w.frames[1].len(),
            "system padded with silence"
        );
        assert!(w.markers.iter().any(|m| matches!(
            m.kind,
            MarkerKind::Gap {
                track: Track::System,
                ..
            }
        )));
        assert!(w.frames[1][2 * 16_000..].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn writer_failure_raises_disk_full_once_and_stops_writing() {
        let (mut p, mut asr, mp, _) = setup(fast_cfg(), false);
        p.writer_mut().fail_after = Some(100);
        let mut mic = Source::new(mp, 48_000.0, speechlike(1, 6.0, 0.4));
        let ev = run_pipeline(&mut p, &mut mic, None, 4, Some(&mut asr), |_| {});
        assert_eq!(
            ev.iter()
                .filter(|e| matches!(e, CaptureEvent::DiskFull))
                .count(),
            1
        );
        assert!(p.writer_failed());
        assert_eq!(
            p.writer().calls_after_fail,
            1,
            "the writer is not called again"
        );
        // ASR still gets the audio.
        assert!(asr.skipped_frames() == 0);
        assert!(p.position() >= 3 * 16_000);
    }

    impl RingProducer {
        /// Test helper: a second handle is impossible for SPSC, so move the
        /// producer out of `sp` by swapping in a dummy ring.
        fn clone_for_test(sp: &mut RingProducer) -> RingProducer {
            let (dummy, _c) = ring(64);
            std::mem::replace(sp, dummy)
        }
    }
}
