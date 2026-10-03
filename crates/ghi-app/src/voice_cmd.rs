// SPDX-License-Identifier: Apache-2.0
//! Me's voice profile (phase 14c): "record your voice" in onboarding and in
//! People. Enrollment captures the mic only, buffers up to 25 s of 16 kHz
//! mono, and nothing is kept past finish or cancel (the buffer is wiped).
//! The user's agreement (a checkbox) is part of finishing; without it nothing
//! is stored. Errors are codes the UI turns into words.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ghi_audio::resample::TrackResampler;
use ghi_core::capture::CaptureError;
use ghi_core::voice_job::EnrollError;
use serde::Serialize;
use specta::Type;
use zeroize::{Zeroize, Zeroizing};

use crate::core::Core;
use crate::speakers_cmd::{BUSY_RECORDING, INVALID_CONSENT, storage};
use crate::{CoreState, blocking};

/// The speaker model is not installed.
pub const NO_MODEL: &str = "noModel";
/// Microphone access was refused.
pub const MIC_PERMISSION: &str = "micPermission";
/// No microphone could be opened.
pub const NO_MIC: &str = "noMic";
/// `enroll_voice_finish` or `_level` without a running enrollment.
pub const NOT_ENROLLING: &str = "notEnrolling";
/// Under 10 s of speech.
pub const TOO_SHORT: &str = "tooShort";
/// Nearly nothing but silence.
pub const TOO_QUIET: &str = "tooQuiet";

/// The longest passage kept (seconds of 16 kHz audio).
pub const MAX_SECONDS: f32 = 25.0;
const MAX_SAMPLES: usize = (MAX_SECONDS as usize) * 16_000;
/// The capture is released after this long even if nobody finishes.
const MAX_WALL: Duration = Duration::from_secs(90);
/// A 100 ms frame this loud (RMS, full scale 1.0) counts as speech.
const SPEECH_RMS: f32 = 0.005;
/// Less speech than this (seconds) is "too quiet", not "too short".
const MIN_ANY_SPEECH_S: f32 = 3.0;

/// The audio is wiped if nobody finishes this long after the mic closed.
const ABANDONED_AFTER: Duration = Duration::from_secs(60);

/// The consent texts onboarding and People show (`onboarding.voice.consent_*`).
const CONSENT_KEYS: [&str; 2] = [
    "onboarding.voice.consent_mac",
    "onboarding.voice.consent_win",
];

/// A running enrollment: the capture lives in its thread, the audio in `pcm`.
pub struct Enrollment {
    stop: Arc<AtomicBool>,
    pcm: Arc<Mutex<Zeroizing<Vec<f32>>>>,
    /// Loudness of the latest block, 0..1 (`f32` bits).
    level: Arc<AtomicU32>,
    /// The mic is released (the buffer is full or it ran too long).
    closed: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Enrollment {
    /// Opens the mic and starts buffering.
    fn start() -> Result<Enrollment, String> {
        let mut cap = ghi_core::capture::live(false, &[]).map_err(|e| match e {
            CaptureError::Permission(_) => MIC_PERMISSION.to_string(),
            CaptureError::Unavailable(_) | CaptureError::Internal(_) => NO_MIC.to_string(),
        })?;
        let mut mic = cap.mic.take().ok_or_else(|| NO_MIC.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        // Allocated once, so the audio never moves (and is never left behind
        // by a reallocation); wiped when dropped.
        let pcm: Arc<Mutex<Zeroizing<Vec<f32>>>> =
            Arc::new(Mutex::new(Zeroizing::new(Vec::with_capacity(MAX_SAMPLES))));
        let level = Arc::new(AtomicU32::new(0));
        let closed = Arc::new(AtomicBool::new(false));
        let thread = {
            let (stop, pcm, level, closed) =
                (stop.clone(), pcm.clone(), level.clone(), closed.clone());
            std::thread::Builder::new()
                .name("ghi-enroll".into())
                .spawn(move || {
                    let started = Instant::now();
                    let mut rs = TrackResampler::default();
                    let mut block = Zeroizing::new(Vec::<f32>::with_capacity(1 << 16));
                    let mut out = Zeroizing::new(Vec::<f32>::with_capacity(1 << 16));
                    while !stop.load(Ordering::Acquire)
                        && started.elapsed() < MAX_WALL
                        && pcm.lock().unwrap_or_else(|e| e.into_inner()).len() < MAX_SAMPLES
                    {
                        let Some((rate, host)) = mic.pop_into(&mut block) else {
                            std::thread::sleep(Duration::from_millis(10));
                            continue;
                        };
                        out.clear();
                        rs.push(&block, rate, host, &mut out);
                        if out.is_empty() {
                            continue;
                        }
                        let rms =
                            (out.iter().map(|x| x * x).sum::<f32>() / out.len() as f32).sqrt();
                        level.store((rms * 4.0).min(1.0).to_bits(), Ordering::Relaxed);
                        let mut p = pcm.lock().unwrap_or_else(|e| e.into_inner());
                        let room = MAX_SAMPLES.saturating_sub(p.len());
                        p.extend_from_slice(&out[..out.len().min(room)]);
                    }
                    // The mic is released here, finished or not.
                    drop(cap);
                    level.store(0, Ordering::Relaxed);
                    closed.store(true, Ordering::Release);
                    // Nobody came to finish: the audio goes.
                    let since = Instant::now();
                    while !stop.load(Ordering::Acquire) && since.elapsed() < ABANDONED_AFTER {
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    if !stop.load(Ordering::Acquire) {
                        pcm.lock().unwrap_or_else(|e| e.into_inner()).zeroize();
                    }
                })
                .map_err(|_| NO_MIC.to_string())?
        };
        Ok(Enrollment {
            stop,
            pcm,
            level,
            closed,
            thread: Some(thread),
        })
    }

    fn seconds(&self) -> f32 {
        self.pcm.lock().unwrap_or_else(|e| e.into_inner()).len() as f32 / 16_000.0
    }

    fn running(&self) -> bool {
        !self.closed.load(Ordering::Acquire)
    }

    /// Stops the capture and hands over the audio (wiped when dropped).
    fn finish(mut self) -> Zeroizing<Vec<f32>> {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        std::mem::take(&mut *self.pcm.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

impl Drop for Enrollment {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        // `pcm` is `Zeroizing`: wiped here if nobody took it.
    }
}

/// Seconds of 16 kHz audio with a loud enough 100 ms frame.
fn speech_seconds(pcm: &[f32]) -> f32 {
    let frames = pcm
        .chunks_exact(1_600)
        .filter(|f| (f.iter().map(|x| x * x).sum::<f32>() / f.len() as f32).sqrt() >= SPEECH_RMS)
        .count();
    frames as f32 / 10.0
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MeProfile {
    /// When they agreed (unix ms).
    pub at_ms: f64,
    /// Voice samples kept (windows of their speech).
    pub samples: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStatus {
    /// The speaker model is installed.
    pub model_ready: bool,
    pub me_profile: Option<MeProfile>,
    pub enrolling: bool,
}

/// Where the model and Me's profile stand. Errors: `storage`.
#[tauri::command]
#[specta::specta]
pub async fn voice_status(core: CoreState<'_>) -> Result<VoiceStatus, String> {
    blocking(&core, |c| {
        let store = c.store()?;
        let me = store
            .me_voice_profile(ghi_core::profiles::VOICE_MODEL)
            .map_err(storage)?
            .map(|p| MeProfile {
                at_ms: p.consent.at_ms as f64,
                samples: p.sets.iter().map(|s| s.exemplars.len()).sum::<usize>() as u32,
            });
        if c.locked() {
            return Err("the app is locked".into());
        }
        Ok(VoiceStatus {
            model_ready: crate::core::voice_ready(&c.models()),
            me_profile: me,
            enrolling: c.enrolling(),
        })
    })
    .await
}

/// Opens the mic for the passage (any earlier enrollment is dropped). Errors:
/// `busyRecording`, `noModel`, `micPermission`, `noMic`.
#[tauri::command]
#[specta::specta]
pub async fn enroll_voice_start(core: CoreState<'_>) -> Result<(), String> {
    blocking(&core, |c| {
        c.store()?;
        c.start_enrollment()
    })
    .await
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnrollLevel {
    /// Loudness of the latest moment, 0..1.
    pub level: f32,
    /// Audio buffered so far.
    pub seconds: f32,
    pub max_seconds: f32,
    /// The mic is closed (the buffer is full or it ran too long): finish.
    pub done: bool,
}

/// For the level meter and the timer; poll a few times a second. Errors:
/// `notEnrolling`.
#[tauri::command]
#[specta::specta]
pub async fn enroll_voice_level(core: CoreState<'_>) -> Result<EnrollLevel, String> {
    let core = core.inner().clone();
    core.with_enrollment(|e| EnrollLevel {
        level: f32::from_bits(e.level.load(Ordering::Relaxed)),
        seconds: e.seconds(),
        max_seconds: MAX_SECONDS,
        done: !e.running(),
    })
    .ok_or_else(|| NOT_ENROLLING.to_string())
}

/// Ends the recording, learns the voice and stores Me's profile, replacing the
/// old one. `consent_text_key` is the locale key of the agreement the user
/// ticked (`onboarding.voice.consent_mac` or `_win`). The audio is wiped. Errors: `notEnrolling`, `busyRecording`,
/// `invalidConsent`, `tooShort`, `tooQuiet`, `noModel`, `storage`.
#[tauri::command]
#[specta::specta]
pub async fn enroll_voice_finish(
    core: CoreState<'_>,
    consent_text_key: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        // First, so the audio is wiped whatever happens next (a locked app
        // included).
        let enrollment = c.take_enrollment().ok_or(NOT_ENROLLING)?;
        let pcm = enrollment.finish();
        let store = c.store()?;
        if c.busy() {
            return Err(BUSY_RECORDING.into());
        }
        if !CONSENT_KEYS.contains(&consent_text_key.as_str()) {
            return Err(INVALID_CONSENT.into());
        }
        if speech_seconds(&pcm) < MIN_ANY_SPEECH_S {
            return Err(TOO_QUIET.into());
        }
        let mut embedder = (crate::core::voice_factory(&c.models()))().map_err(|_| NO_MODEL)?;
        let consent = ghi_core::voice_job::self_consent(&consent_text_key);
        ghi_core::voice_job::enroll_from_pcm(&store, embedder.as_mut(), &pcm, &consent)
            .map(|_| ())
            .map_err(|e| match e {
                EnrollError::NoConsent => INVALID_CONSENT.to_string(),
                EnrollError::TooLittleSpeech => TOO_SHORT.to_string(),
                EnrollError::Model(_) => NO_MODEL.to_string(),
                EnrollError::Store(e) => storage(e),
            })
    })
    .await
}

/// Stops the recording and wipes the audio. Never an error.
#[tauri::command]
#[specta::specta]
pub async fn enroll_voice_cancel(core: CoreState<'_>) -> Result<(), String> {
    blocking(&core, |c| {
        drop(c.take_enrollment());
        Ok(())
    })
    .await
}

/// Why an enrollment may not start now (pure, so it is testable).
fn enrollment_gate(busy: bool, model_ready: bool) -> Result<(), String> {
    if busy {
        return Err(BUSY_RECORDING.into());
    }
    if !model_ready {
        return Err(NO_MODEL.into());
    }
    Ok(())
}

/// Stores a started enrollment, unless a recording began meanwhile: the
/// caller holds the lifecycle lock (so a start can't slip in between the
/// check and the store); a refused enrollment is dropped, which wipes it and
/// releases the mic.
fn place_enrollment(
    slot: &Mutex<Option<Enrollment>>,
    recording: bool,
    e: Enrollment,
) -> Result<(), String> {
    if recording {
        drop(e);
        return Err(BUSY_RECORDING.into());
    }
    *slot.lock().unwrap_or_else(|x| x.into_inner()) = Some(e);
    Ok(())
}

/// Ends the enrollment in `slot` (lock, quit, a recording starting).
pub fn drop_enrollment(slot: &Mutex<Option<Enrollment>>) {
    drop(slot.lock().unwrap_or_else(|x| x.into_inner()).take());
}

impl Core {
    /// Starts an enrollment (refused while recording or without the model).
    pub fn start_enrollment(&self) -> Result<(), String> {
        enrollment_gate(self.busy(), crate::core::voice_ready(&self.models()))?;
        // One at a time: an earlier one (a reopened dialog) is dropped first.
        drop(self.take_enrollment());
        // The mic opens (and may ask permission) without the lifecycle lock.
        let e = Enrollment::start()?;
        // Then, under it, a recording that began meanwhile wins.
        let _lifecycle = self.lifecycle_guard();
        place_enrollment(self.enrollment_mutex(), self.recording(), e)
    }

    pub fn enrolling(&self) -> bool {
        self.enrollment_slot().is_some()
    }

    pub fn take_enrollment(&self) -> Option<Enrollment> {
        self.enrollment_slot().take()
    }

    fn with_enrollment<T>(&self, f: impl FnOnce(&Enrollment) -> T) -> Option<T> {
        self.enrollment_slot().as_ref().map(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_is_measured_in_loud_100ms_frames() {
        let mut pcm = vec![0.0f32; 16_000]; // 1 s of silence
        pcm.extend((0..16_000 * 4).map(|i| (i as f32 * 0.05).sin() * 0.1)); // 4 s
        assert!((speech_seconds(&pcm) - 4.0).abs() < 0.11);
        assert_eq!(speech_seconds(&vec![0.001; 16_000 * 10]), 0.0);
        assert_eq!(speech_seconds(&[]), 0.0);
    }

    #[test]
    fn a_finished_enrollment_hands_over_audio_and_wipes_on_drop() {
        let pcm: Arc<Mutex<Zeroizing<Vec<f32>>>> = Arc::default();
        pcm.lock().unwrap().extend_from_slice(&[0.5; 1_000]);
        let e = Enrollment {
            stop: Arc::default(),
            pcm,
            level: Arc::default(),
            closed: Arc::default(),
            thread: None,
        };
        assert!((e.seconds() - 1_000.0 / 16_000.0).abs() < 1e-6);
        assert!(e.running(), "the mic is open until it closes");
        e.closed.store(true, Ordering::Release);
        assert!(!e.running());
        let audio = e.finish();
        assert_eq!(audio.len(), 1_000);
    }

    fn idle() -> Enrollment {
        let pcm: Arc<Mutex<Zeroizing<Vec<f32>>>> = Arc::default();
        pcm.lock().unwrap().extend_from_slice(&[0.5; 100]);
        Enrollment {
            stop: Arc::default(),
            pcm,
            level: Arc::default(),
            closed: Arc::default(),
            thread: None,
        }
    }

    #[test]
    fn an_enrollment_is_refused_while_recording_or_without_the_model() {
        assert_eq!(enrollment_gate(true, true), Err(BUSY_RECORDING.into()));
        assert_eq!(enrollment_gate(false, false), Err(NO_MODEL.into()));
        assert_eq!(enrollment_gate(true, false), Err(BUSY_RECORDING.into()));
        assert!(enrollment_gate(false, true).is_ok());
    }

    #[test]
    fn a_recording_that_began_meanwhile_wins_and_wipes_the_audio() {
        let slot = Mutex::new(None);
        let e = idle();
        let pcm = e.pcm.clone();
        assert_eq!(place_enrollment(&slot, true, e), Err(BUSY_RECORDING.into()));
        assert!(slot.lock().unwrap().is_none());
        // Dropped: nothing else holds the audio but this test's handle, and
        // the enrollment stopped.
        assert_eq!(Arc::strong_count(&pcm), 1);
        place_enrollment(&slot, false, idle()).unwrap();
        assert!(slot.lock().unwrap().is_some());
    }

    #[test]
    fn locking_or_quitting_drops_the_enrollment() {
        let slot = Mutex::new(None);
        place_enrollment(&slot, false, idle()).unwrap();
        let stop = slot.lock().unwrap().as_ref().unwrap().stop.clone();
        drop_enrollment(&slot);
        assert!(slot.lock().unwrap().is_none());
        assert!(stop.load(Ordering::Acquire), "its capture was told to stop");
        drop_enrollment(&slot); // nothing there: fine
    }

    #[test]
    fn only_the_shown_consent_texts_are_accepted() {
        assert!(CONSENT_KEYS.contains(&"onboarding.voice.consent_mac"));
        assert!(!CONSENT_KEYS.contains(&"anything.else"));
    }

    #[test]
    fn the_error_codes_are_the_documented_ones() {
        let codes = [
            NO_MODEL,
            MIC_PERMISSION,
            NO_MIC,
            NOT_ENROLLING,
            TOO_SHORT,
            TOO_QUIET,
        ];
        assert_eq!(
            codes,
            [
                "noModel",
                "micPermission",
                "noMic",
                "notEnrolling",
                "tooShort",
                "tooQuiet"
            ]
        );
    }
}
