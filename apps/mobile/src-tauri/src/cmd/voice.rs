// SPDX-License-Identifier: Apache-2.0
//! "Me" voice enrollment on the phone (M1 optional step, Settings → Voice
//! profile). `voice_status` is `ghi_app::voice_cmd`; the enrollment itself is
//! the shared one (`Core::start_enrollment` / `finish_enrollment`: up to 25 s
//! of 16 kHz audio held only in memory and wiped at finish or cancel), with the
//! phone's microphone: the Swift audio tap into a ring ([`open_mic`]).
//!
//! Consent must be given before the passage is read; without it nothing is
//! recorded or stored. The consent text key stored with the profile is
//! `mobile.voice.consent`. Errors are the `ghi_app::voice_cmd` codes
//! (`noModel`, `micPermission`, `noMic`, `busyRecording`, `invalidConsent`,
//! `tooShort`, `tooQuiet`, `notEnrolling`, `storage`).

use std::sync::atomic::{AtomicBool, Ordering};

use ghi_app::speakers_cmd::{BUSY_RECORDING, INVALID_CONSENT};
use ghi_app::voice_cmd::{MIC_PERMISSION, NO_MIC};

use super::onboarding::MicPermission;
use crate::platform;

/// The locale key of the agreement the user ticked.
pub const CONSENT_KEY: &str = "mobile.voice.consent";

/// The user ticked the agreement for the passage they are about to read.
static CONSENT: AtomicBool = AtomicBool::new(false);

/// Releases the microphone when the enrollment ends.
struct MicGuard;

impl Drop for MicGuard {
    fn drop(&mut self) {
        platform::audio_stop();
        crate::session::detach_tap();
    }
}

/// Opens the phone's microphone for an enrollment: the audio tap's blocks go
/// into a ring the enrollment reads. Refused while a recording holds the tap.
pub fn open_mic() -> Result<(ghi_audio::ring::RingConsumer, Box<dyn std::any::Any + Send>), String>
{
    if platform::mic_permission() != MicPermission::Granted {
        return Err(MIC_PERMISSION.into());
    }
    // A few seconds of 48 kHz audio: the enrollment drains it as it comes.
    let (producer, consumer) = ghi_audio::ring::ring(48_000 * 4);
    crate::session::attach_tap(producer).map_err(|_| BUSY_RECORDING.to_owned())?;
    if let Err(e) = platform::audio_start() {
        log::warn!("enrollment mic: {e}");
        crate::session::detach_tap();
        return Err(NO_MIC.into());
    }
    Ok((consumer, Box::new(MicGuard)))
}

/// The agreement for the next passage (checked before the mic opens). Taking
/// it back ends an enrollment in progress and wipes its audio.
#[tauri::command]
#[specta::specta]
pub async fn voice_set_consent(core: ghi_app::CoreState<'_>, given: bool) -> Result<(), String> {
    CONSENT.store(given, Ordering::Release);
    if !given {
        ghi_app::blocking(&core, |c| {
            drop(c.take_enrollment());
            Ok(())
        })
        .await?;
    }
    Ok(())
}

/// Starts capturing the passage. Needs consent and the voice model.
#[tauri::command]
#[specta::specta]
pub async fn voice_enroll_start(
    core: ghi_app::CoreState<'_>,
    recorder: tauri::State<'_, std::sync::Arc<crate::session::Recorder>>,
) -> Result<(), String> {
    if !CONSENT.load(Ordering::Acquire) {
        return Err(INVALID_CONSENT.into());
    }
    let recording = recorder.latest().is_some();
    ghi_app::blocking(&core, move |c| {
        if recording {
            return Err(BUSY_RECORDING.into());
        }
        c.store()?;
        c.start_enrollment()
    })
    .await
}

/// Finishes: stores the profile. Errors are the `ghi_app::voice_cmd` codes.
#[tauri::command]
#[specta::specta]
pub async fn voice_enroll_stop(core: ghi_app::CoreState<'_>) -> Result<(), String> {
    let consent = CONSENT.load(Ordering::Acquire);
    ghi_app::blocking(&core, move |c| {
        // The audio is wiped by `finish_enrollment` whatever happens next.
        if !consent {
            drop(c.take_enrollment());
            return Err(INVALID_CONSENT.into());
        }
        ghi_app::voice_cmd::finish_enrollment(c, CONSENT_KEY)
    })
    .await?;
    CONSENT.store(false, Ordering::Release);
    Ok(())
}

/// Cancels and wipes the buffered audio.
#[tauri::command]
#[specta::specta]
pub async fn voice_enroll_cancel(core: ghi_app::CoreState<'_>) -> Result<(), String> {
    CONSENT.store(false, Ordering::Release);
    ghi_app::blocking(&core, |c| {
        drop(c.take_enrollment());
        Ok(())
    })
    .await
}
