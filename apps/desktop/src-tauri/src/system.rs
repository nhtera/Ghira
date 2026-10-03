// SPDX-License-Identifier: Apache-2.0
//! App settings, OS permissions and meeting detection.
//!
//! - Settings live in the store (key `app`), typed here; unknown or missing
//!   fields take their defaults, so older stores keep working.
//! - Microphone permission comes from AVFoundation; system-audio access can't
//!   be queried on macOS (a denied tap just delivers silence, which the core
//!   reports as `silentSystemTrack`).
//! - The detection poller lists the processes using audio every 2 s while
//!   nothing is recording and emits `MeetingDetected` (the UI shows its own
//!   non-activating prompt). Never/snooze choices persist under `detect`.

use std::sync::{Arc, Mutex};
use std::time::Duration;
#[cfg(target_os = "macos")]
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::AppHandle;
#[cfg(target_os = "macos")]
use tauri::Manager;
use tauri_specta::Event;

use crate::core::Core;
use crate::{CoreState, blocking};

const SETTINGS_KEY: &str = "app";
const DETECT_KEY: &str = "detect";

/// App settings the UI reads (stored in the encrypted store). Every field is
/// required in the TypeScript type; what the store lacks takes its default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Onboarding finished (or skipped).
    pub onboarding_done: bool,
    /// Ask to record when a meeting app starts using the mic.
    pub detect_meetings: bool,
    /// ⌘M / Ctrl+M marks a moment from any app while recording.
    pub global_mark_shortcut: bool,
    /// ⌘⇧R / Ctrl+Shift+R starts or stops recording from any app.
    pub global_record_shortcut: bool,
    /// Voice profile of the user ("Me"): on while the speaker model is
    /// installed (the onboarding step is hidden while off).
    pub voice_profiles_me: bool,
    /// Saving other people's voices (consent dialog). Off for the alpha
    /// ([`THIRD_PARTY_APPROVED`]).
    pub voice_profiles_third_party: bool,
    /// No network at all, model downloads included (doc 02 §L).
    pub strict_offline: bool,
    /// Language of new meetings: `en`, `vi`, or `auto` (both, code-switching).
    pub meeting_language: MeetingLanguage,
    /// The cloud provider and model the send sheet starts with (empty: none
    /// chosen yet). Every send is still previewed and confirmed.
    pub cloud_provider: String,
    pub cloud_model: String,
    /// Hide names and personal data from cloud requests (restored locally).
    pub cloud_redact: bool,
    /// Days to keep meeting audio (0: until the meeting is deleted).
    pub audio_retention_days: u32,
    /// The consent message to share at the start of a meeting (empty: the
    /// built-in text in the app's language).
    pub consent_message_en: String,
    pub consent_message_vi: String,
    /// Check for app updates at launch and daily (never under strict offline).
    pub update_check: bool,
    /// App lock: open locked, lock after `lock_after_minutes` idle and after
    /// sleep; unlock with Touch ID or the Mac's password (`set_app_lock`).
    pub app_lock: bool,
    /// Idle minutes before the app locks (0: only at launch and after sleep).
    pub lock_after_minutes: u32,
    /// The live transcript's trade-off (doc 02 §B); applies from the next
    /// recording. 8 GB Macs always use Fast.
    pub live_mode: LiveMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum LiveMode {
    /// Chosen by the hardware tier.
    #[default]
    Auto,
    /// Lighter on the Mac; captions come a little later (1.1 s chunks).
    Fast,
    /// Captions sooner, more processing while recording (0.56 s chunks).
    Accurate,
}

impl LiveMode {
    /// The live ASR chunk for this mode on a machine of `tier`.
    pub fn chunk_ms(self, tier: ghi_models::Tier) -> u32 {
        const FAST: u32 = 1120;
        const ACCURATE: u32 = 560;
        match (tier, self) {
            (ghi_models::Tier::Light, _) => FAST,
            (_, LiveMode::Fast) => FAST,
            (_, LiveMode::Accurate) => ACCURATE,
            (t, LiveMode::Auto) => ghi_models::preset(t).asr_chunk_ms,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum MeetingLanguage {
    En,
    Vi,
    #[default]
    Auto,
}

impl MeetingLanguage {
    /// The session's language hint (`None` = detect per utterance).
    pub fn hint(self) -> Option<String> {
        match self {
            MeetingLanguage::En => Some("en".into()),
            MeetingLanguage::Vi => Some("vi".into()),
            MeetingLanguage::Auto => None,
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            onboarding_done: false,
            detect_meetings: true,
            global_mark_shortcut: true,
            global_record_shortcut: true,
            voice_profiles_me: false,
            voice_profiles_third_party: false,
            strict_offline: false,
            meeting_language: MeetingLanguage::Auto,
            cloud_provider: String::new(),
            cloud_model: String::new(),
            cloud_redact: true,
            audio_retention_days: 0,
            consent_message_en: String::new(),
            consent_message_vi: String::new(),
            update_check: true,
            app_lock: false,
            lock_after_minutes: 5,
            live_mode: LiveMode::Auto,
        }
    }
}

/// A change to some settings; fields left out keep their value.
#[derive(Debug, Clone, Default, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct SettingsPatch {
    pub onboarding_done: Option<bool>,
    pub detect_meetings: Option<bool>,
    pub global_mark_shortcut: Option<bool>,
    pub global_record_shortcut: Option<bool>,
    pub strict_offline: Option<bool>,
    pub meeting_language: Option<MeetingLanguage>,
    pub cloud_provider: Option<String>,
    pub cloud_model: Option<String>,
    pub cloud_redact: Option<bool>,
    pub audio_retention_days: Option<u32>,
    pub consent_message_en: Option<String>,
    pub consent_message_vi: Option<String>,
    pub update_check: Option<bool>,
    pub live_mode: Option<LiveMode>,
    /// Only through `set_app_lock` (which asks for Touch ID first).
    #[serde(skip)]
    pub app_lock: Option<bool>,
    #[serde(skip)]
    pub lock_after_minutes: Option<u32>,
}

/// Short text settings stay short.
fn short(v: Option<String>, cur: String, max: usize) -> String {
    v.map(|s| s.chars().take(max).collect()).unwrap_or(cur)
}

/// Other people's voice profiles wait for counsel's sign-off on the consent
/// evidence (phase 14c, D9): hard off in every build until this is changed in
/// source. Every command and the final pass check it server-side.
pub(crate) const THIRD_PARTY_APPROVED: bool = false;

/// The voice flags are not the user's to set: Me follows the speaker model,
/// third-party is the source constant AND what the user chose.
fn enforce(s: AppSettings, voice_model: bool) -> AppSettings {
    AppSettings {
        voice_profiles_me: voice_model,
        voice_profiles_third_party: THIRD_PARTY_APPROVED && s.voice_profiles_third_party,
        ..s
    }
}

/// The third-party proof for the store's voice calls: `Some` only when the
/// source constant allows it and the stored setting is on. The only place the
/// token is built.
pub(crate) fn third_party_token(
    store: &ghi_store::store::Store,
) -> Option<ghi_store::voice::ThirdPartyApproved> {
    if !THIRD_PARTY_APPROVED {
        return None;
    }
    let on = store
        .get_setting(SETTINGS_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.get("voiceProfilesThirdParty").and_then(|b| b.as_bool()))
        .unwrap_or(false);
    on.then(ghi_store::voice::ThirdPartyApproved::assert_flag_checked)
}

/// Stored fields over the defaults (an older store lacks newer fields).
fn from_stored(v: Option<serde_json::Value>) -> AppSettings {
    let mut base = serde_json::to_value(AppSettings::default()).unwrap_or_default();
    if let (Some(serde_json::Value::Object(stored)), serde_json::Value::Object(b)) = (v, &mut base)
    {
        for (k, v) in stored {
            if !b.contains_key(&k) {
                continue;
            }
            // Field by field: a value this build can't read (a newer
            // version's option) keeps its default instead of resetting
            // every setting, the app lock included.
            let old = b.insert(k.clone(), v);
            if serde_json::from_value::<AppSettings>(serde_json::Value::Object(b.clone())).is_err()
                && let Some(old) = old
            {
                b.insert(k, old);
            }
        }
    }
    enforce(serde_json::from_value(base).unwrap_or_default(), false)
}

/// The settings (cached: the detection poller reads them every 2 s).
pub fn load_settings(core: &Core) -> Result<AppSettings, String> {
    let mut cache = core
        .settings_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // The model can arrive while the app runs: Me's flag is read fresh.
    let voice_model = crate::core::voice_ready_cached(&core.models());
    if let Some(s) = cache.as_ref() {
        return Ok(enforce(s.clone(), voice_model));
    }
    let s = from_stored(
        core.store()?
            .get_setting(SETTINGS_KEY)
            .map_err(|e| e.to_string())?,
    );
    *cache = Some(s.clone());
    Ok(enforce(s, voice_model))
}

#[tauri::command]
#[specta::specta]
pub async fn get_settings(core: CoreState<'_>) -> Result<AppSettings, String> {
    blocking(&core, load_settings).await
}

/// Changes some settings and returns them all.
#[tauri::command]
#[specta::specta]
pub async fn update_settings(
    app: tauri::AppHandle,
    core: CoreState<'_>,
    patch: SettingsPatch,
) -> Result<AppSettings, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || patch_settings(&app, &core, patch))
        .await
        .map_err(|e| e.to_string())?
}

/// Applies a settings change (the app lock fields only come from
/// `set_app_lock`: the command's `SettingsPatch` can't carry them).
pub fn patch_settings(
    app: &tauri::AppHandle,
    c: &Core,
    patch: SettingsPatch,
) -> Result<AppSettings, String> {
    let r = (|| {
        // One change at a time: two patches can't lose each other's fields.
        static WRITING: Mutex<()> = Mutex::new(());
        let _one = WRITING.lock().unwrap_or_else(|e| e.into_inner());
        let cur = load_settings(c)?;
        let s = enforce(
            AppSettings {
                onboarding_done: patch.onboarding_done.unwrap_or(cur.onboarding_done),
                detect_meetings: patch.detect_meetings.unwrap_or(cur.detect_meetings),
                global_mark_shortcut: patch
                    .global_mark_shortcut
                    .unwrap_or(cur.global_mark_shortcut),
                global_record_shortcut: patch
                    .global_record_shortcut
                    .unwrap_or(cur.global_record_shortcut),
                strict_offline: patch.strict_offline.unwrap_or(cur.strict_offline),
                meeting_language: patch.meeting_language.unwrap_or(cur.meeting_language),
                cloud_redact: patch.cloud_redact.unwrap_or(cur.cloud_redact),
                update_check: patch.update_check.unwrap_or(cur.update_check),
                live_mode: patch.live_mode.unwrap_or(cur.live_mode),
                app_lock: patch.app_lock.unwrap_or(cur.app_lock),
                lock_after_minutes: patch.lock_after_minutes.unwrap_or(cur.lock_after_minutes),
                audio_retention_days: patch
                    .audio_retention_days
                    .map(|d| d.min(3650))
                    .unwrap_or(cur.audio_retention_days),
                cloud_provider: short(patch.cloud_provider, cur.cloud_provider.clone(), 40),
                cloud_model: short(patch.cloud_model, cur.cloud_model.clone(), 80),
                consent_message_en: short(
                    patch.consent_message_en,
                    cur.consent_message_en.clone(),
                    1000,
                ),
                consent_message_vi: short(
                    patch.consent_message_vi,
                    cur.consent_message_vi.clone(),
                    1000,
                ),
                ..cur.clone()
            },
            crate::core::voice_ready(&c.models()),
        );
        let retention_changed = s.audio_retention_days != cur.audio_retention_days;
        let v = serde_json::to_value(&s).map_err(|e| e.to_string())?;
        c.store()?
            .set_setting(SETTINGS_KEY, &v)
            .map_err(|e| e.to_string())?;
        *c.settings_cache().lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
        if retention_changed {
            apply_retention(c, s.audio_retention_days)?;
        }
        Ok(s)
    })();
    // Onboarding finished or the setting changed: (un)register ⌘⇧R.
    crate::tray::sync_record_shortcut(app);
    r
}

/// Applies the audio retention setting and deletes the audio past it (the
/// text stays). Also run at startup and hourly (`spawn_retention`).
pub fn apply_retention(core: &Core, days: u32) -> Result<(), String> {
    let store = core.store()?;
    store
        .apply_retention_days((days > 0).then_some(days))
        .map_err(|e| e.to_string())?;
    store.retention_sweep_now().map_err(|e| e.to_string())?;
    Ok(())
}

/// Hourly retention sweep in the background.
pub fn spawn_retention(core: Arc<Core>) {
    let _ = std::thread::Builder::new()
        .name("ghi-retention".into())
        .spawn(move || {
            // Not in the way of startup.
            std::thread::sleep(Duration::from_secs(30));
            loop {
                if let Ok(s) = load_settings(&core) {
                    let _ = apply_retention(&core, s.audio_retention_days);
                }
                std::thread::sleep(Duration::from_secs(3600));
            }
        });
}

/// Microphone access as macOS reports it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    Granted,
    Denied,
    /// Not asked yet.
    Undetermined,
    /// Blocked by a policy (MDM, parental controls).
    Restricted,
    /// Not applicable on this platform (no macOS permission model).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Unsupported,
}

#[cfg(target_os = "macos")]
fn from_mac(p: ghi_audio::macos::MicPermission) -> Permission {
    use ghi_audio::macos::MicPermission as M;
    match p {
        M::Authorized => Permission::Granted,
        M::Denied => Permission::Denied,
        M::Undetermined => Permission::Undetermined,
        M::Restricted => Permission::Restricted,
    }
}

#[tauri::command]
#[specta::specta]
pub async fn mic_permission() -> Permission {
    #[cfg(target_os = "macos")]
    return from_mac(ghi_audio::macos::mic_permission());
    #[cfg(not(target_os = "macos"))]
    Permission::Unsupported
}

/// Shows the OS prompt when undetermined (blocks until answered).
#[tauri::command]
#[specta::specta]
pub async fn request_mic_permission() -> Result<Permission, String> {
    #[cfg(target_os = "macos")]
    return tauri::async_runtime::spawn_blocking(|| {
        from_mac(ghi_audio::macos::request_mic_permission())
    })
    .await
    .map_err(|e| e.to_string());
    #[cfg(not(target_os = "macos"))]
    Ok(Permission::Unsupported)
}

/// A pane of the OS privacy settings, for a denied permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum PrivacyPane {
    Microphone,
    /// "Screen & System Audio Recording" on macOS.
    SystemAudio,
    Notifications,
    /// "Calendars" on macOS.
    Calendars,
}

/// Opens System Settings on the pane where the user can turn access on.
#[tauri::command]
#[specta::specta]
pub fn open_privacy_settings(pane: PrivacyPane) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Fixed URLs only: nothing from the webview reaches the command line.
        let url = match pane {
            PrivacyPane::Microphone => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
            }
            PrivacyPane::SystemAudio => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
            }
            PrivacyPane::Notifications => {
                "x-apple.systempreferences:com.apple.Notifications-Settings.extension"
            }
            PrivacyPane::Calendars => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars"
            }
        };
        // Started, not awaited: a command must not block on another process.
        std::process::Command::new("/usr/bin/open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pane;
        Err("not available on this platform yet".into())
    }
}

/// A meeting app started using the microphone.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetected {
    /// Stable app key (`zoom`, `teams`, `chrome`, …), for the reply.
    pub app: String,
    /// Shown in the prompt ("Zoom"); browsers prompt generically.
    pub app_name: String,
    pub browser: bool,
    /// The calendar event this meeting is (its title), when there is one.
    #[serde(default)]
    #[specta(optional)]
    pub title: Option<String>,
    /// That event's key (for the reply and the dedupe with the calendar ticker).
    #[serde(default)]
    #[specta(optional)]
    pub event: Option<String>,
}

/// The user's answer to a detection prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DetectReply {
    /// Recording starts (the UI calls start_recording itself).
    Start,
    /// Quiet for this call (the 30 min cooldown already applies).
    NotNow,
    /// Never ask for this app again.
    Never,
}

/// The detector, shared by the poller and the reply command.
#[derive(Default)]
pub struct Detection(Mutex<Option<ghi_audio::detect::Detector>>);

fn persist(core: &Core, d: &ghi_audio::detect::Detector) {
    if let (Ok(store), Ok(v)) = (core.store(), serde_json::to_value(d.state())) {
        let _ = store.set_setting(DETECT_KEY, &v);
    }
}

#[tauri::command]
#[specta::specta]
pub async fn reply_meeting_detected(
    core: CoreState<'_>,
    detection: tauri::State<'_, Arc<Detection>>,
    app: String,
    reply: DetectReply,
) -> Result<(), String> {
    let Some(app) = ghi_audio::detect::App::from_key(&app) else {
        return Err("unknown app".into());
    };
    let detection = detection.inner().clone();
    blocking(&core, move |c| {
        let mut slot = detection.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(d) = slot.as_mut() {
            match reply {
                DetectReply::Never => d.never(app),
                DetectReply::NotNow | DetectReply::Start => {}
            }
            persist(c, d);
        }
        Ok(())
    })
    .await
}

/// Starts the detection poller (macOS; elsewhere a no-op until phase 13).
pub fn spawn_detection(app: AppHandle, core: Arc<Core>, detection: Arc<Detection>) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::thread::Builder::new()
            .name("ghi-detect".into())
            .spawn(move || detection_loop(app, core, detection));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, core, detection);
}

#[cfg(target_os = "macos")]
fn detection_loop(app: AppHandle, core: Arc<Core>, detection: Arc<Detection>) {
    use ghi_audio::detect::{DetectConfig, DetectState, Detector};
    // The store opens in the background at launch; detection starts after
    // (and keeps retrying if it can't open yet, e.g. a locked keychain).
    let store = loop {
        match core.store() {
            Ok(s) => break s,
            Err(_) => std::thread::sleep(Duration::from_secs(30)),
        }
    };
    let state: DetectState = store
        .get_setting(DETECT_KEY)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    *detection.0.lock().unwrap_or_else(|e| e.into_inner()) =
        Some(Detector::new(DetectConfig::default(), state));
    loop {
        std::thread::sleep(Duration::from_secs(2));
        // Starting or recording: no prompt (and no store reads meanwhile).
        if core.busy()
            || !load_settings(&core)
                .map(|s| s.detect_meetings)
                .unwrap_or(false)
        {
            continue;
        }
        let Ok(processes) = ghi_audio::macos::audio_processes() else {
            continue;
        };
        let prompt = {
            let mut slot = detection.0.lock().unwrap_or_else(|e| e.into_inner());
            let Some(d) = slot.as_mut() else { continue };
            let p = d.poll(&processes, SystemTime::now());
            if p.is_some() {
                persist(&core, d);
            }
            p
        };
        if let Some(p) = prompt {
            let detected = MeetingDetected {
                app: p.app.key().into(),
                app_name: p.app.display_name().into(),
                browser: p.app.is_browser(),
                title: None,
                event: None,
            };
            // In the main window only when the user is looking at it; else
            // (hidden, behind a full-screen call, another Space) the panel.
            let main_focused = app
                .get_webview_window("main")
                .and_then(|w| w.is_focused().ok())
                .unwrap_or(false);
            if main_focused {
                let _ = detected.emit_to(&app, "main");
            } else {
                crate::panels::open_detect(&app, detected);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_settings_merge_over_defaults_and_flags_stay_off() {
        assert_eq!(from_stored(None), AppSettings::default());
        let s = from_stored(Some(serde_json::json!({
            "strictOffline": true,
            "voiceProfilesMe": true,
            "voiceProfilesThirdParty": true,
            "unknownOldField": 1
        })));
        assert!(s.strict_offline && s.detect_meetings);
        // Me follows the speaker model, never the stored value.
        assert!(!s.voice_profiles_me);
        assert!(enforce(s.clone(), true).voice_profiles_me);
        assert!(!enforce(AppSettings::default(), false).voice_profiles_me);
        // An older store without newer fields keeps their defaults.
        let s = from_stored(Some(serde_json::json!({ "onboardingDone": true })));
        assert!(s.onboarding_done && !s.strict_offline);
        // One unreadable value keeps its default; the others stay.
        let s = from_stored(Some(serde_json::json!({
            "appLock": true,
            "liveMode": "turbo",
            "strictOffline": true
        })));
        assert!(s.app_lock && s.strict_offline);
        assert_eq!(s.live_mode, LiveMode::Auto);
    }

    #[test]
    fn third_party_voice_profiles_are_never_enabled_by_settings() {
        // The source constant is off: whatever is stored or patched.
        const { assert!(!THIRD_PARTY_APPROVED, "release builds ship it off (D9)") };
        for model in [false, true] {
            let on = AppSettings {
                voice_profiles_third_party: true,
                ..AppSettings::default()
            };
            assert!(!enforce(on, model).voice_profiles_third_party);
        }
        let s = from_stored(Some(serde_json::json!({ "voiceProfilesThirdParty": true })));
        assert!(!s.voice_profiles_third_party);
    }

    #[test]
    fn the_third_party_token_needs_the_constant() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ghi_store::store::Store::open(
            tmp.path(),
            std::sync::Arc::new(ghi_store::keys::MemoryKeyStore::default()),
            ghi_store::keys::Protection::default(),
        )
        .unwrap();
        store
            .set_setting(
                SETTINGS_KEY,
                &serde_json::json!({"voiceProfilesThirdParty": true}),
            )
            .unwrap();
        assert!(third_party_token(&store).is_none());
    }

    #[test]
    fn live_modes_pick_the_chunk_and_light_macs_stay_fast() {
        use ghi_models::Tier;
        assert_eq!(LiveMode::Auto.chunk_ms(Tier::Balanced), 560);
        assert_eq!(LiveMode::Fast.chunk_ms(Tier::Balanced), 1120);
        assert_eq!(LiveMode::Accurate.chunk_ms(Tier::Max), 560);
        for m in [LiveMode::Auto, LiveMode::Fast, LiveMode::Accurate] {
            assert_eq!(m.chunk_ms(Tier::Light), 1120, "8 GB: Fast is forced");
        }
    }
}

/// A system notification (notes ready, recovered); clicking it brings the
/// app forward. The text comes localized from the UI.
#[tauri::command]
#[specta::specta]
pub fn show_notification(app: tauri::AppHandle, title: String, body: String) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    let clip = |s: String, n: usize| s.chars().take(n).collect::<String>();
    app.notification()
        .builder()
        .title(clip(title, 120))
        .body(clip(body, 400))
        .show()
        .map_err(|e| e.to_string())
}
