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
    /// is rejected until phase 15.
    pub default_target: ProcessingTarget,
    /// Download models over Wi-Fi only (default true).
    pub models_wifi_only: bool,
}

impl Default for MobileSettings {
    fn default() -> Self {
        MobileSettings {
            default_target: ProcessingTarget::Phone,
            models_wifi_only: true,
        }
    }
}

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

/// Saves `s` (the desktop target is refused) and returns what is stored.
pub fn save(store: &Store, s: MobileSettings) -> Result<MobileSettings, String> {
    if s.default_target == ProcessingTarget::Desktop {
        return Err("desktopUnavailable".into());
    }
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
        };
        assert_eq!(save(&store, s).unwrap(), s);
        assert_eq!(load(&store).unwrap(), s);
    }

    #[test]
    fn the_desktop_target_is_refused_and_nothing_is_stored() {
        let (_t, store) = store();
        let r = save(
            &store,
            MobileSettings {
                default_target: ProcessingTarget::Desktop,
                models_wifi_only: false,
            },
        );
        assert_eq!(r, Err("desktopUnavailable".into()));
        assert_eq!(load(&store).unwrap(), MobileSettings::default());
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
