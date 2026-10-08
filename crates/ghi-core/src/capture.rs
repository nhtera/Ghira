// SPDX-License-Identifier: Apache-2.0
//! Where a session's audio comes from: live capture (macOS for now; Windows
//! in phase 13) or a replay of PCM into the same rings, for tests, soak runs
//! and the eval kit. Either way the session sees rings + capture events.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ghi_audio::ring::{RingConsumer, RingProducer, ring};
use ghi_audio::{CaptureEvent, Route, Track};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureError {
    /// Microphone or system-audio permission is missing.
    Permission(String),
    /// Not available here (platform, device).
    Unavailable(String),
    Internal(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::Permission(m)
            | CaptureError::Unavailable(m)
            | CaptureError::Internal(m) => f.write_str(m),
        }
    }
}

enum Handle {
    /// Shared so a rebuild can run off the pump thread (see
    /// [`Capture::restarter`]).
    #[cfg(target_os = "macos")]
    Mac(Arc<std::sync::Mutex<ghi_audio::macos::MacCapture>>),
    Replay {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    },
}

/// Rings and events of a running capture. Dropping it stops the capture.
pub struct Capture {
    pub mic: Option<RingConsumer>,
    pub system: Option<RingConsumer>,
    pub events: mpsc::Receiver<CaptureEvent>,
    pub route: Route,
    /// "capture" or "replay".
    pub kind: &'static str,
    handle: Option<Handle>,
}

impl Capture {
    /// A replay has played everything (live capture never ends by itself).
    pub fn ended(&self) -> bool {
        match &self.handle {
            #[cfg(target_os = "macos")]
            Some(Handle::Mac(_)) => false,
            Some(Handle::Replay { thread, .. }) => thread.as_ref().is_none_or(|t| t.is_finished()),
            None => true,
        }
    }

    /// Gives the caller a way to send capture events into the session too
    /// (the desktop's own sleep/wake notifications, tests). Events already
    /// queued move over; events a platform layer sends later are not
    /// forwarded, so use it on replays.
    pub fn event_sender(&mut self) -> mpsc::Sender<CaptureEvent> {
        let (tx, rx) = mpsc::channel();
        let old = std::mem::replace(&mut self.events, rx);
        while let Ok(ev) = old.try_recv() {
            let _ = tx.send(ev);
        }
        tx
    }

    /// A job that rebuilds the devices after a track was lost (the mic taken
    /// by another app, a device that went away); the same rings keep feeding
    /// the session. It blocks while the devices come back, so run it off the
    /// pump thread. `None` when this is not live capture (nothing to rebuild).
    #[allow(clippy::type_complexity)]
    pub fn restarter(&self) -> Option<Box<dyn FnOnce() -> Result<(), CaptureError> + Send>> {
        match &self.handle {
            #[cfg(target_os = "macos")]
            Some(Handle::Mac(c)) => {
                let c = c.clone();
                Some(Box::new(move || {
                    c.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .resume()
                        .map_err(|e| CaptureError::Unavailable(e.to_string()))
                }))
            }
            _ => None,
        }
    }

    /// Stops the source (rings get no more audio).
    pub fn stop(&mut self) {
        match self.handle.take() {
            #[cfg(target_os = "macos")]
            Some(Handle::Mac(c)) => drop(c),
            Some(Handle::Replay { stop, thread }) => {
                stop.store(true, Ordering::Relaxed);
                if let Some(t) = thread {
                    let _ = t.join();
                }
            }
            None => {}
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// One track's PCM for [`replay`].
pub struct ReplayTrack {
    pub track: Track,
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

/// Plays PCM into the rings in 10 ms blocks stamped with an ideal clock.
/// `speed`: `Some(1.0)` = real time, `Some(x)` = x times faster, `None` = as
/// fast as the session reads (waits for ring space instead of dropping).
/// Shorter tracks are padded with silence to the longest.
pub fn replay(tracks: Vec<ReplayTrack>, speed: Option<f64>) -> Result<Capture, CaptureError> {
    if tracks.is_empty() {
        return Err(CaptureError::Unavailable("nothing to replay".into()));
    }
    let longest = tracks
        .iter()
        .map(|t| t.samples.len() as f64 / f64::from(t.sample_rate))
        .fold(0.0, f64::max);
    let mut consumers: [Option<RingConsumer>; 2] = [None, None];
    let mut feeds: Vec<(RingProducer, ReplayTrack)> = Vec::new();
    let (events_tx, events) = mpsc::channel();
    for mut t in tracks {
        let n = (longest * f64::from(t.sample_rate)).ceil() as usize;
        t.samples.resize(n.max(t.samples.len()), 0.0);
        let (tx, rx) = ring(4 * t.sample_rate as usize);
        consumers[t.track.index()] = Some(rx);
        let _ = events_tx.send(CaptureEvent::TrackStarted {
            track: t.track,
            device: "replay".into(),
        });
        feeds.push((tx, t));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let stop = stop.clone();
        std::thread::Builder::new()
            .name("ghi-replay".into())
            .spawn(move || play(feeds, speed, &stop))
            .map_err(|e| CaptureError::Internal(format!("replay thread: {e}")))?
    };
    let [mic, system] = consumers;
    Ok(Capture {
        mic,
        system,
        events,
        route: Route::Headphones,
        kind: "replay",
        handle: Some(Handle::Replay {
            stop,
            thread: Some(thread),
        }),
    })
}

fn play(mut feeds: Vec<(RingProducer, ReplayTrack)>, speed: Option<f64>, stop: &AtomicBool) {
    let base = Instant::now();
    // An arbitrary but realistic host clock origin.
    let start_ns = 1_000_000_000u64;
    let mut block = 0u64;
    loop {
        let mut more = false;
        for (tx, t) in &mut feeds {
            let n = (t.sample_rate / 100) as usize;
            let from = block as usize * n;
            if from >= t.samples.len() {
                continue;
            }
            more = true;
            let to = (from + n).min(t.samples.len());
            let at = start_ns + from as u64 * 1_000_000_000 / u64::from(t.sample_rate);
            // Full ring: in real time that is a drop (counted by `push`); as
            // fast as possible, wait for the session first, so a slow reader
            // is not reported as lost audio.
            if speed.is_none() {
                while !tx.has_room(to - from) && !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            tx.push(&t.samples[from..to], f64::from(t.sample_rate), at);
        }
        if !more || stop.load(Ordering::Relaxed) {
            return;
        }
        block += 1;
        if let Some(x) = speed {
            let due = base + Duration::from_secs_f64(0.01 * block as f64 / x.max(0.01));
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
        }
    }
}

/// Live capture on macOS: mic, plus system audio (`system`, the far end of a
/// call) from a Core Audio process tap. Asks for microphone permission.
#[cfg(target_os = "macos")]
pub fn live(system: bool, tap_pids: &[i32]) -> Result<Capture, CaptureError> {
    use ghi_audio::macos::{self, CaptureConfig, MacCapture, MacError, MicPermission};
    // The prompt blocks until answered; keep it off the caller's thread.
    let perm = std::thread::spawn(|| match macos::mic_permission() {
        MicPermission::Undetermined => macos::request_mic_permission(),
        p => p,
    })
    .join()
    .map_err(|_| CaptureError::Internal("mic permission: thread panicked".into()))?;
    if perm != MicPermission::Authorized {
        return Err(CaptureError::Permission(format!(
            "microphone access is {perm:?}: allow it in System Settings > Privacy & Security > Microphone"
        )));
    }
    let (route, _) = macos::route().unwrap_or((Route::Unknown, false));
    let started = MacCapture::start(&CaptureConfig {
        mic: true,
        system,
        tap_pids: tap_pids.to_vec(),
    })
    .map_err(|e| match e.code {
        MacError::MIC_PERMISSION | MacError::SYSTEM_PERMISSION => {
            CaptureError::Permission(e.to_string())
        }
        MacError::UNSUPPORTED => CaptureError::Unavailable(e.to_string()),
        _ => CaptureError::Internal(e.to_string()),
    })?;
    Ok(Capture {
        mic: started.mic,
        system: started.system,
        events: started.events,
        route,
        kind: "capture",
        handle: Some(Handle::Mac(Arc::new(std::sync::Mutex::new(
            started.capture,
        )))),
    })
}

#[cfg(not(target_os = "macos"))]
pub fn live(_system: bool, _tap_pids: &[i32]) -> Result<Capture, CaptureError> {
    Err(CaptureError::Unavailable(
        "live capture is macOS-only for now (Windows in phase 13)".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_replay_delivers_every_sample_with_backpressure() {
        let samples: Vec<f32> = (0..48_000 * 6).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut cap = replay(
            vec![ReplayTrack {
                track: Track::Mic,
                samples,
                sample_rate: 48_000,
            }],
            None,
        )
        .unwrap();
        let mut rx = cap.mic.take().unwrap();
        let mut got = 0usize;
        let mut buf = Vec::new();
        let t = Instant::now();
        while got < 48_000 * 6 {
            if rx.pop_into(&mut buf).is_some() {
                got += buf.len();
            } else {
                assert!(t.elapsed() < Duration::from_secs(10), "stalled at {got}");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        assert_eq!(got, 48_000 * 6, "no drops: the ring is 4 s, the audio 6 s");
        assert_eq!(rx.dropped_samples(), 0);
        while !cap.ended() {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// A reader that falls behind (a loaded machine) only makes a fast replay
    /// wait: the ring fills, nothing is counted as dropped (CI saw 960).
    #[test]
    fn fast_replay_waiting_for_a_slow_reader_is_not_a_drop() {
        let total = 48_000 * 6;
        let samples: Vec<f32> = (0..total).map(|i| (i as f32 * 0.01).sin()).collect();
        let mut cap = replay(
            vec![ReplayTrack {
                track: Track::Mic,
                samples,
                sample_rate: 48_000,
            }],
            None,
        )
        .unwrap();
        let mut rx = cap.mic.take().unwrap();
        // The 4 s ring fills long before this, so the replay has to wait.
        std::thread::sleep(Duration::from_millis(300));
        let (mut got, mut buf, t) = (0usize, Vec::new(), Instant::now());
        while got < total {
            if rx.pop_into(&mut buf).is_some() {
                got += buf.len();
            } else {
                assert!(t.elapsed() < Duration::from_secs(10), "stalled at {got}");
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        assert_eq!(got, total);
        assert_eq!(rx.dropped_samples(), 0);
        while !cap.ended() {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
