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
    /// Pairing exists (phase 15): the onboarding offers the pair step.
    pub sync_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum MicPermission {
    NotDetermined,
    Granted,
    Denied,
}

/// The settings-table key holding the completed steps.
const KEY: &str = "onboarding";

fn load(store: &ghi_store::store::Store) -> Result<OnboardingState, String> {
    let completed: Vec<OnboardingStep> = store
        .get_setting(KEY)
        .map_err(|e| e.to_string())?
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    Ok(OnboardingState {
        completed,
        sync_available: true,
    })
}

/// Records `step` as done (once). `Done` also marks onboarding finished in
/// the shared settings (`AppSettings.onboarding_done`), which the app uses to
/// decide between onboarding and the library.
pub fn complete(
    store: &ghi_store::store::Store,
    step: OnboardingStep,
) -> Result<OnboardingState, String> {
    let mut state = load(store)?;
    if !state.completed.contains(&step) {
        state.completed.push(step);
    }
    let v = serde_json::to_value(&state.completed).map_err(|e| e.to_string())?;
    store.set_setting(KEY, &v).map_err(|e| e.to_string())?;
    Ok(state)
}

#[tauri::command]
#[specta::specta]
pub async fn onboarding_state(core: ghi_app::CoreState<'_>) -> Result<OnboardingState, String> {
    ghi_app::blocking(&core, |c| {
        let store = c.store()?;
        load(&store)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn onboarding_complete_step(
    app: tauri::AppHandle,
    core: ghi_app::CoreState<'_>,
    step: OnboardingStep,
) -> Result<OnboardingState, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let store = core.store()?;
        let state = complete(&store, step)?;
        if step == OnboardingStep::Done {
            ghi_app::system::patch_settings(
                &app,
                &core,
                ghi_app::system::SettingsPatch {
                    onboarding_done: Some(true),
                    ..Default::default()
                },
            )?;
        }
        Ok(state)
    })
    .await
    .map_err(|e| e.to_string())?
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
