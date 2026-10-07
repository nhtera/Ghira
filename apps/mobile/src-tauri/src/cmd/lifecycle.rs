// SPDX-License-Identifier: Apache-2.0
//! App lifecycle and device facts.

use serde::Serialize;
use specta::Type;

use crate::platform;

/// The app's scene. `lifecycle_state` is a stub for the scene until 16-E
/// reports it from Swift (`willResignActive` / `didEnterBackground`); it always
/// answers `active` now. Live phase changes arrive as `MobileEvent::Phase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Scene {
    Active,
    /// Resigned active (app switcher, a system sheet, locking).
    Inactive,
    Background,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LifecycleState {
    pub scene: Scene,
    /// Dynamic Type multiplier, capped at 2.0.
    pub text_scale: f32,
    /// `ProcessInfo.thermalState` raw value, 0 to 3.
    pub thermal: u8,
}

/// What the device can do (`tier.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TierClass {
    /// Live transcript and the on-phone final pass.
    Live,
    /// Records only; no on-phone transcript. The `Phone` target is disabled.
    RecordOnly,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTier {
    /// `iPhone16,1` style machine identifier (`SIMULATOR_MODEL_IDENTIFIER` on the simulator).
    pub model_id: String,
    pub ram_gb: f64,
    pub simulator: bool,
    pub tier: TierClass,
    /// It can write notes and action items itself (an 8 GB live phone; the
    /// notes model is an optional download).
    pub notes: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MobileAppVersion {
    pub app: String,
    pub core: String,
}

#[tauri::command]
#[specta::specta]
pub fn app_version() -> MobileAppVersion {
    MobileAppVersion {
        app: env!("CARGO_PKG_VERSION").to_owned(),
        core: ghi_core::version().to_owned(),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn lifecycle_state() -> Result<LifecycleState, String> {
    Ok(LifecycleState {
        scene: Scene::Active,
        text_scale: platform::text_scale(),
        thermal: u8::try_from(platform::device_stats().thermal.unwrap_or(0)).unwrap_or(0),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn device_tier() -> Result<DeviceTier, String> {
    Ok(crate::tier::detect())
}

/// The phone's thermal state now (0 nominal, 1 fair, 2 serious, 3 critical);
/// notes written on the phone wait at 2 or more. Changes arrive as
/// `MobileEvent::Thermal`.
#[tauri::command]
#[specta::specta]
pub fn thermal_level() -> u8 {
    platform::device_stats()
        .thermal
        .map_or(0, |t| t.clamp(0, 3) as u8)
}

/// Opens this app's page in the Settings app (microphone denied).
#[tauri::command]
#[specta::specta]
pub fn open_app_settings() {
    platform::open_settings();
}
