// SPDX-License-Identifier: Apache-2.0
//! Onboarding (M1) and the microphone permission.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum OnboardingStep {
    Languages,
    MicPriming,
    Voice,
    Consent,
    /// Phase 15; hidden while `sync_available` is false.
    Pair,
    Processing,
    Models,
    Done,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OnboardingState {
    pub completed: Vec<OnboardingStep>,
    /// Pairing exists (phase 15). False in v1.
    pub sync_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum MicPermission {
    NotDetermined,
    Granted,
    Denied,
}

#[tauri::command]
#[specta::specta]
pub async fn onboarding_state() -> Result<OnboardingState, String> {
    Err("not yet".into())
}

#[tauri::command]
#[specta::specta]
pub async fn onboarding_complete_step(_step: OnboardingStep) -> Result<OnboardingState, String> {
    Err("not yet".into())
}

#[tauri::command]
#[specta::specta]
pub fn mic_permission() -> MicPermission {
    platform::mic_permission()
}

/// Shows the system prompt (only the first time); resolves with the answer.
#[tauri::command]
#[specta::specta]
pub async fn request_mic_permission() -> MicPermission {
    tauri::async_runtime::spawn_blocking(|| {
        platform::request_mic_permission();
        // The prompt is asynchronous: wait for the answer (up to 2 minutes).
        for _ in 0..240 {
            let p = platform::mic_permission();
            if p != MicPermission::NotDetermined {
                return p;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        platform::mic_permission()
    })
    .await
    .unwrap_or(MicPermission::NotDetermined)
}
