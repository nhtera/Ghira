// SPDX-License-Identifier: Apache-2.0
//! Phone-only settings, beside the shared `AppSettings` (`get_settings` /
//! `update_settings`: meeting language, cloud provider, audio retention,
//! consent message). Stored in the store's settings table under a separate
//! `mobile` key, so the desktop settings type stays unchanged.

use ghi_store::store::Store;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::types::ProcessingTarget;

/// The settings-table key.
pub const KEY: &str = "mobile";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MobileSettings {
    /// What new recordings and imports use (Settings → Processing). `Desktop`
    /// needs a paired computer (`pairingNotAvailable` otherwise).
    pub default_target: ProcessingTarget,
    /// Download models over Wi-Fi only (default true).
    pub models_wifi_only: bool,
    /// Hours the paired computer may stay away from a meeting handed to it
    /// before this phone processes it itself (1..=168; default 12). A phone
    /// below the live tier keeps waiting.
    pub desktop_offline_hours: u32,
}

impl Default for MobileSettings {
    fn default() -> Self {
        MobileSettings {
            default_target: ProcessingTarget::Phone,
            models_wifi_only: true,
            desktop_offline_hours: DEFAULT_OFFLINE_HOURS,
        }
    }
}

/// The default of [`MobileSettings::desktop_offline_hours`].
pub const DEFAULT_OFFLINE_HOURS: u32 = ghi_app::sync_service::DEFAULT_OFFLINE_HOURS;
/// The range of [`MobileSettings::desktop_offline_hours`].
pub const OFFLINE_HOURS: std::ops::RangeInclusive<u32> = 1..=168;

/// Stored fields over the defaults: an older store lacks newer fields and a
/// value this build cannot read keeps its default, field by field.
pub fn from_stored(v: Option<serde_json::Value>) -> MobileSettings {
    let mut base = serde_json::to_value(MobileSettings::default()).unwrap_or_default();
    if let (Some(serde_json::Value::Object(stored)), serde_json::Value::Object(b)) = (v, &mut base)
    {
        for (k, v) in stored {
            if !b.contains_key(&k) {
                continue;
            }
            let old = b.insert(k.clone(), v);
            if serde_json::from_value::<MobileSettings>(serde_json::Value::Object(b.clone()))
                .is_err()
                && let Some(old) = old
            {
                b.insert(k, old);
            }
        }
    }
    serde_json::from_value(base).unwrap_or_default()
}

pub fn load(store: &Store) -> Result<MobileSettings, String> {
    Ok(from_stored(
        store.get_setting(KEY).map_err(|e| e.to_string())?,
    ))
}

/// Saves `s` and returns what is stored. The desktop target needs a paired
/// computer (`pairingNotAvailable`); the offline hours stay within
/// [`OFFLINE_HOURS`].
pub fn save(store: &Store, mut s: MobileSettings) -> Result<MobileSettings, String> {
    if s.default_target == ProcessingTarget::Desktop
        && !ghi_app::sync_service::spoke::has_hub(store)
    {
        return Err("pairingNotAvailable".into());
    }
    s.desktop_offline_hours = s
        .desktop_offline_hours
        .clamp(*OFFLINE_HOURS.start(), *OFFLINE_HOURS.end());
    let v = serde_json::to_value(s).map_err(|e| e.to_string())?;
    store.set_setting(KEY, &v).map_err(|e| e.to_string())?;
    Ok(s)
}

#[tauri::command]
#[specta::specta]
pub async fn mobile_settings(core: ghi_app::CoreState<'_>) -> Result<MobileSettings, String> {
    ghi_app::blocking(&core, |c| {
        let store = c.store()?;
        load(&store)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn set_mobile_settings(
    core: ghi_app::CoreState<'_>,
    settings: MobileSettings,
) -> Result<MobileSettings, String> {
    ghi_app::blocking(&core, move |c| {
        let store = c.store()?;
        save(&store, settings)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_store::keys::MemoryKeyStore;
    use std::sync::Arc;

    fn store() -> (tempfile::TempDir, Store) {
        let t = tempfile::tempdir().unwrap();
        let s = Store::open(
            t.path(),
            Arc::new(MemoryKeyStore::default()),
            Default::default(),
        )
        .unwrap();
        (t, s)
    }

    #[test]
    fn defaults_then_a_round_trip() {
        let (_t, store) = store();
        let d = load(&store).unwrap();
        assert_eq!(d.default_target, ProcessingTarget::Phone);
        assert!(d.models_wifi_only);
        let s = MobileSettings {
            default_target: ProcessingTarget::Cloud,
            models_wifi_only: false,
            desktop_offline_hours: 24,
        };
        assert_eq!(save(&store, s).unwrap(), s);
        assert_eq!(load(&store).unwrap(), s);
    }

    #[test]
    fn the_desktop_target_needs_a_paired_computer_and_nothing_is_stored_without() {
        let (_t, store) = store();
        let want = MobileSettings {
            default_target: ProcessingTarget::Desktop,
            models_wifi_only: false,
            desktop_offline_hours: 12,
        };
        assert_eq!(save(&store, want), Err("pairingNotAvailable".into()));
        assert_eq!(load(&store).unwrap(), MobileSettings::default());
        store
            .pin_device(
                &ghi_store::sync::devices::NewDevice {
                    gid: "01a10db5-956d-729d-91b2-76be7b439cdd".into(),
                    name: "Mac".into(),
                    platform: "desktop".into(),
                    role: ghi_store::sync::devices::DeviceRole::Hub,
                    static_pub: [7; 32],
                },
                &[9; 32],
            )
            .unwrap();
        assert_eq!(save(&store, want).unwrap(), want);
    }

    #[test]
    fn the_offline_hours_stay_in_range() {
        let (_t, store) = store();
        let with = |hours| MobileSettings {
            desktop_offline_hours: hours,
            ..MobileSettings::default()
        };
        assert_eq!(save(&store, with(0)).unwrap().desktop_offline_hours, 1);
        assert_eq!(
            save(&store, with(100_000)).unwrap().desktop_offline_hours,
            168
        );
    }

    #[test]
    fn an_older_or_odd_store_keeps_defaults_field_by_field() {
        let s = from_stored(Some(serde_json::json!({"modelsWifiOnly": false})));
        assert_eq!(s.default_target, ProcessingTarget::Phone);
        assert!(!s.models_wifi_only);
        let s = from_stored(Some(serde_json::json!({
            "defaultTarget": "teleport", "modelsWifiOnly": false, "future": 1
        })));
        assert_eq!(s.default_target, ProcessingTarget::Phone);
        assert!(!s.models_wifi_only);
        assert_eq!(
            from_stored(Some(serde_json::json!("junk"))),
            MobileSettings::default()
        );
    }
}
