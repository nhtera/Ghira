// SPDX-License-Identifier: Apache-2.0
//! Meeting auto-detect policy.
//!
//! The platform layer only lists the processes Core Audio knows about
//! (`{pid, bundle_id, input, output}`). What counts as a meeting, when to ask
//! and when to keep quiet lives here, as pure logic driven by [`Detector::poll`].
//!
//! * A meeting app is recognised by bundle id: an exact match or a prefix that
//!   ends at a dot (`com.google.Chrome.helper.Renderer` belongs to Chrome,
//!   `com.google.ChromeX` does not). Only a *running mic input* triggers.
//! * Native apps (Zoom, Teams, Webex, Slack, Discord, Zalo) prompt with their
//!   name; browsers prompt generically ("Browser call detected").
//! * Apps that are not listed (Voice Memos, dictation, music players) never
//!   prompt. Our own process is ignored.
//! * After a prompt the app is quiet for 30 minutes, and while it keeps using
//!   the mic the same call is never prompted again. "Never for X" is permanent.
//!
//! [`DetectState`] is the part to persist between launches.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// One entry of `ghi_mac_audio_processes`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AudioProcess {
    pub pid: i32,
    #[serde(default)]
    pub bundle_id: String,
    #[serde(default)]
    pub input: bool,
    #[serde(default)]
    pub output: bool,
}

/// Parses the JSON array returned by `ghi_mac_audio_processes`.
pub fn parse_processes(json: &str) -> Result<Vec<AudioProcess>, serde_json::Error> {
    serde_json::from_str(json)
}

/// A recognised meeting app. The order is the prompt priority (native apps
/// first, then browsers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum App {
    Zoom,
    Teams,
    Webex,
    Slack,
    Discord,
    Zalo,
    FaceTime,
    Chrome,
    Edge,
    Safari,
    Arc,
    Firefox,
    Brave,
}

const TABLE: &[(App, &[&str])] = &[
    (App::Zoom, &["us.zoom.xos"]),
    (App::Teams, &["com.microsoft.teams2", "com.microsoft.teams"]),
    (
        App::Webex,
        &["Cisco-Systems.Spark", "com.webex.meetingmanager"],
    ),
    (App::Slack, &["com.tinyspeck.slackmacgap"]),
    (App::Discord, &["com.hnc.Discord"]),
    (App::Zalo, &["com.vng.zalo"]),
    (App::FaceTime, &["com.apple.FaceTime"]),
    (App::Chrome, &["com.google.Chrome"]),
    (App::Edge, &["com.microsoft.edgemac"]),
    (App::Safari, &["com.apple.Safari"]),
    (App::Arc, &["company.thebrowser.Browser"]),
    (App::Firefox, &["org.mozilla.firefox"]),
    (App::Brave, &["com.brave.Browser"]),
];

impl App {
    pub const fn is_browser(self) -> bool {
        matches!(
            self,
            App::Chrome | App::Edge | App::Safari | App::Arc | App::Firefox | App::Brave
        )
    }

    /// Stable id used in persisted state.
    pub const fn key(self) -> &'static str {
        match self {
            App::Zoom => "zoom",
            App::Teams => "teams",
            App::Webex => "webex",
            App::Slack => "slack",
            App::Discord => "discord",
            App::Zalo => "zalo",
            App::FaceTime => "facetime",
            App::Chrome => "chrome",
            App::Edge => "edge",
            App::Safari => "safari",
            App::Arc => "arc",
            App::Firefox => "firefox",
            App::Brave => "brave",
        }
    }

    /// The id of this app in the settings' "detect in these apps" list
    /// (`zoom`, `teams`, `meet` for every browser, …); `None` for apps the
    /// list does not offer (always detected).
    pub const fn setting_key(self) -> Option<&'static str> {
        match self {
            App::Zoom => Some("zoom"),
            App::Teams => Some("teams"),
            App::Webex => Some("webex"),
            App::Slack => Some("slack"),
            App::Zalo => Some("zalo"),
            App::FaceTime => Some("facetime"),
            App::Discord => None,
            _ => Some("meet"),
        }
    }

    pub fn from_key(key: &str) -> Option<App> {
        TABLE.iter().map(|t| t.0).find(|a| a.key() == key)
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            App::Zoom => "Zoom",
            App::Teams => "Microsoft Teams",
            App::Webex => "Webex",
            App::Slack => "Slack",
            App::Discord => "Discord",
            App::Zalo => "Zalo",
            App::FaceTime => "FaceTime",
            App::Chrome => "Google Chrome",
            App::Edge => "Microsoft Edge",
            App::Safari => "Safari",
            App::Arc => "Arc",
            App::Firefox => "Firefox",
            App::Brave => "Brave",
        }
    }

    /// The text of the prompt: the app's name, or the generic browser text.
    pub const fn prompt_title(self) -> &'static str {
        match self {
            App::Zoom => "Zoom call detected",
            App::Teams => "Teams call detected",
            App::Webex => "Webex call detected",
            App::Slack => "Slack call detected",
            App::Discord => "Discord call detected",
            App::Zalo => "Zalo call detected",
            App::FaceTime => "FaceTime call detected",
            _ => "Browser call detected",
        }
    }

    /// Classifies a bundle id.
    pub fn from_bundle_id(bundle_id: &str) -> Option<App> {
        TABLE
            .iter()
            .find(|(_, prefixes)| prefixes.iter().any(|p| has_bundle_prefix(bundle_id, p)))
            .map(|t| t.0)
    }
}

/// `id == prefix` or `id` starts with `prefix` followed by a dot (ASCII case
/// insensitive, as bundle ids are).
fn has_bundle_prefix(id: &str, prefix: &str) -> bool {
    let (id, prefix) = (id.as_bytes(), prefix.as_bytes());
    id.len() >= prefix.len()
        && id[..prefix.len()].eq_ignore_ascii_case(prefix)
        && (id.len() == prefix.len() || id[prefix.len()] == b'.')
}

/// Every process of the meeting app that has the mic running right now (the
/// first by prompt priority; `only` as in [`Detector::set_only`]), sorted:
/// the include list for a per-app system-audio tap. Empty when no meeting app
/// is in a call (the caller then captures the whole system).
pub fn meeting_app_pids(
    processes: &[AudioProcess],
    only: Option<&[String]>,
    own_pid: i32,
) -> Vec<i32> {
    let mut by_app: BTreeMap<App, (bool, Vec<i32>)> = BTreeMap::new();
    for p in processes.iter().filter(|p| p.pid != own_pid) {
        let Some(app) = App::from_bundle_id(&p.bundle_id) else {
            continue;
        };
        if let (Some(only), Some(k)) = (only, app.setting_key())
            && !only.iter().any(|o| o == k)
        {
            continue;
        }
        let e = by_app.entry(app).or_default();
        e.0 |= p.input;
        e.1.push(p.pid);
    }
    let mut pids = by_app
        .into_values()
        .find(|(mic, _)| *mic)
        .map(|(_, pids)| pids)
        .unwrap_or_default();
    pids.sort_unstable();
    pids
}

/// What to show the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub app: App,
    /// "Zoom call detected", or "Browser call detected" for browsers.
    pub title: &'static str,
    /// Every process of this app (mic running or not, e.g. output-only
    /// helpers): the include list for a per-app tap.
    pub pids: Vec<i32>,
}

/// The part of the detector to persist (JSON via serde).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DetectState {
    /// App keys the user chose "Never" for.
    #[serde(default)]
    pub never: BTreeSet<String>,
    /// App key to unix seconds until which it stays quiet (cooldown or snooze).
    #[serde(default)]
    pub quiet_until: BTreeMap<String, u64>,
}

#[derive(Debug, Clone)]
pub struct DetectConfig {
    /// Quiet time after a prompt.
    pub cooldown: Duration,
    /// Consecutive polls with the mic running before an app prompts (filters
    /// blips such as a browser tab probing the mic).
    pub stable_polls: u32,
    /// Our own pid, never considered.
    pub own_pid: i32,
}

impl Default for DetectConfig {
    fn default() -> Self {
        Self {
            cooldown: Duration::from_secs(30 * 60),
            stable_polls: 2,
            own_pid: std::process::id() as i32,
        }
    }
}

pub struct Detector {
    cfg: DetectConfig,
    state: DetectState,
    /// Apps prompted whose mic is still running: one call, one prompt.
    in_call: BTreeSet<App>,
    /// Consecutive polls each app has had the mic running.
    streak: BTreeMap<App, u32>,
    /// Settings keys of the apps to detect (see [`App::setting_key`]); `None`
    /// detects every app.
    only: Option<BTreeSet<String>>,
}

fn unix(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Detector {
    pub fn new(cfg: DetectConfig, state: DetectState) -> Self {
        Self {
            cfg,
            state,
            in_call: BTreeSet::new(),
            streak: BTreeMap::new(),
            only: None,
        }
    }

    /// Detects only the apps whose settings key is listed (`None`: all).
    pub fn set_only(&mut self, keys: Option<&[String]>) {
        self.only = keys.map(|k| k.iter().cloned().collect());
    }

    fn wanted(&self, app: App) -> bool {
        match (&self.only, app.setting_key()) {
            (Some(only), Some(k)) => only.contains(k),
            _ => true,
        }
    }

    /// State to save. Call after `poll`, `never`, `allow` and `snooze`.
    pub fn state(&self) -> &DetectState {
        &self.state
    }

    /// Never prompt for `app` again (until [`allow`](Self::allow)).
    pub fn never(&mut self, app: App) {
        self.state.never.insert(app.key().to_string());
    }

    pub fn allow(&mut self, app: App) {
        self.state.never.remove(app.key());
        self.state.quiet_until.remove(app.key());
    }

    pub fn is_never(&self, app: App) -> bool {
        self.state.never.contains(app.key())
    }

    /// Stay quiet about `app` for `dur` from `now` (replaces any cooldown).
    pub fn snooze(&mut self, app: App, now: SystemTime, dur: Duration) {
        self.state
            .quiet_until
            .insert(app.key().to_string(), unix(now) + dur.as_secs());
    }

    /// Feeds one snapshot of the process list (about once a second). Returns a
    /// prompt when a meeting app just started using the microphone and nothing
    /// says to stay quiet. Starting the cooldown is part of returning it.
    pub fn poll(&mut self, processes: &[AudioProcess], now: SystemTime) -> Option<Prompt> {
        // Every process of an app is a tap target; only a running mic input
        // makes the app a candidate.
        let mut active: BTreeMap<App, Vec<i32>> = BTreeMap::new();
        let mut mic_on: BTreeSet<App> = BTreeSet::new();
        for p in processes {
            if p.pid == self.cfg.own_pid {
                continue;
            }
            if let Some(app) = App::from_bundle_id(&p.bundle_id)
                && self.wanted(app)
            {
                active.entry(app).or_default().push(p.pid);
                if p.input {
                    mic_on.insert(app);
                }
            }
        }
        active.retain(|a, _| mic_on.contains(a));
        self.in_call.retain(|a| active.contains_key(a));
        self.streak.retain(|a, _| active.contains_key(a));
        for app in active.keys() {
            *self.streak.entry(*app).or_insert(0) += 1;
        }
        if !self.in_call.is_empty() {
            return None;
        }
        let now_s = unix(now);
        for (app, pids) in active {
            if self.streak.get(&app).copied().unwrap_or(0) < self.cfg.stable_polls
                || self.is_never(app)
                || self
                    .state
                    .quiet_until
                    .get(app.key())
                    .is_some_and(|&t| now_s < t)
            {
                continue;
            }
            self.state
                .quiet_until
                .insert(app.key().to_string(), now_s + self.cfg.cooldown.as_secs());
            self.in_call.insert(app);
            let mut pids = pids;
            pids.sort_unstable();
            return Some(Prompt {
                app,
                title: app.prompt_title(),
                pids,
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: i32, id: &str, input: bool) -> AudioProcess {
        AudioProcess {
            pid,
            bundle_id: id.into(),
            input,
            output: true,
        }
    }

    fn t(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_800_000_000 + secs)
    }

    fn det() -> Detector {
        Detector::new(
            DetectConfig {
                own_pid: 999,
                ..Default::default()
            },
            DetectState::default(),
        )
    }

    /// Polls twice (the stable-poll requirement) and returns the second result.
    fn settle(d: &mut Detector, ps: &[AudioProcess], at: u64) -> Option<Prompt> {
        assert_eq!(d.poll(ps, t(at)), None, "first poll only starts the streak");
        d.poll(ps, t(at + 1))
    }

    #[test]
    fn bundle_id_matching_respects_dot_boundaries() {
        assert_eq!(App::from_bundle_id("us.zoom.xos"), Some(App::Zoom));
        assert_eq!(App::from_bundle_id("us.zoom.xos.helper"), Some(App::Zoom));
        assert_eq!(App::from_bundle_id("us.zoom.xosx"), None);
        assert_eq!(
            App::from_bundle_id("com.microsoft.teams2"),
            Some(App::Teams)
        );
        assert_eq!(App::from_bundle_id("com.microsoft.teams"), Some(App::Teams));
        assert_eq!(
            App::from_bundle_id("com.microsoft.teams2.helper.gpu"),
            Some(App::Teams)
        );
        assert_eq!(App::from_bundle_id("com.microsoft.teamsX"), None);
        assert_eq!(App::from_bundle_id("Cisco-Systems.Spark"), Some(App::Webex));
        assert_eq!(
            App::from_bundle_id("com.webex.meetingmanager"),
            Some(App::Webex)
        );
        assert_eq!(
            App::from_bundle_id("com.tinyspeck.slackmacgap"),
            Some(App::Slack)
        );
        assert_eq!(
            App::from_bundle_id("com.hnc.Discord.helper"),
            Some(App::Discord)
        );
        assert_eq!(App::from_bundle_id("com.vng.zalo"), Some(App::Zalo));
        assert_eq!(
            App::from_bundle_id("com.google.Chrome.helper.Renderer"),
            Some(App::Chrome)
        );
        assert_eq!(App::from_bundle_id("com.google.ChromeX"), None);
        assert_eq!(
            App::from_bundle_id("com.google.Chrome.canary"),
            Some(App::Chrome)
        );
        assert_eq!(App::from_bundle_id("COM.GOOGLE.CHROME"), Some(App::Chrome));
        assert_eq!(App::from_bundle_id(""), None);
        for a in [App::Edge, App::Safari, App::Arc, App::Firefox, App::Brave] {
            assert!(a.is_browser());
            assert_eq!(App::from_key(a.key()), Some(a));
        }
        assert!(!App::Zoom.is_browser());
    }

    #[test]
    fn zoom_mic_prompts_by_name_with_pids() {
        let mut d = det();
        let ps = [
            proc(10, "us.zoom.xos", true),
            proc(11, "us.zoom.xos.helper", true),
            proc(12, "us.zoom.xos", false),
        ];
        let p = settle(&mut d, &ps, 0).expect("prompt");
        assert_eq!(p.app, App::Zoom);
        assert_eq!(p.title, "Zoom call detected");
        assert_eq!(
            p.pids,
            vec![10, 11, 12],
            "output-only processes are tap targets too"
        );
    }

    #[test]
    fn browsers_get_the_generic_prompt_and_helpers_map_to_the_parent() {
        let mut d = det();
        let ps = [
            proc(5, "com.google.Chrome.helper", true),
            proc(6, "com.google.Chrome", false),
        ];
        let p = settle(&mut d, &ps, 0).unwrap();
        assert_eq!(p.app, App::Chrome);
        assert_eq!(p.title, "Browser call detected");
        assert_eq!(p.pids, vec![5, 6]);
    }

    #[test]
    fn output_only_and_unlisted_apps_never_prompt() {
        let mut d = det();
        let ps = [
            proc(1, "us.zoom.xos", false),                // playing, no mic
            proc(2, "com.apple.VoiceMemos", true),        // Voice Memos
            proc(3, "com.apple.assistant_service", true), // dictation / Siri
            proc(4, "com.apple.speech.recognitiond", true),
            proc(5, "com.spotify.client", true),
            proc(6, "", true),
        ];
        for i in 0..10 {
            assert_eq!(d.poll(&ps, t(i)), None);
        }
        assert!(d.state().quiet_until.is_empty());
    }

    #[test]
    fn own_process_is_ignored() {
        let mut d = det();
        let ps = [proc(999, "us.zoom.xos", true)];
        for i in 0..5 {
            assert_eq!(d.poll(&ps, t(i)), None);
        }
    }

    #[test]
    fn a_one_poll_blip_does_not_prompt() {
        let mut d = det();
        let on = [proc(1, "us.zoom.xos", true)];
        assert_eq!(d.poll(&on, t(0)), None);
        assert_eq!(d.poll(&[], t(1)), None);
        assert_eq!(d.poll(&on, t(2)), None, "streak restarted");
        assert!(d.poll(&on, t(3)).is_some());
    }

    #[test]
    fn one_prompt_per_call_even_past_the_cooldown() {
        let mut d = det();
        let on = [proc(1, "us.zoom.xos", true)];
        assert!(settle(&mut d, &on, 0).is_some());
        // The same call keeps the mic for over an hour: no second prompt.
        for s in (2..4000).step_by(100) {
            assert_eq!(d.poll(&on, t(s)), None);
        }
        // After it ends and the cooldown has passed, a new call prompts.
        assert_eq!(d.poll(&[], t(4000)), None);
        assert!(settle(&mut d, &on, 4001).is_some());
        // A call right after that prompt is inside the new cooldown.
        assert_eq!(d.poll(&[], t(4100)), None);
        assert_eq!(settle(&mut d, &on, 4200), None);
    }

    #[test]
    fn cooldown_expires_after_thirty_minutes() {
        let mut d = det();
        let on = [proc(1, "us.zoom.xos", true)];
        assert!(settle(&mut d, &on, 0).is_some()); // prompted at t=1
        assert_eq!(d.poll(&[], t(2)), None);
        // New call at 29 min: quiet. At 31 min: prompt.
        assert_eq!(settle(&mut d, &on, 29 * 60), None);
        assert_eq!(d.poll(&[], t(29 * 60 + 5)), None);
        assert!(settle(&mut d, &on, 31 * 60).is_some());
    }

    #[test]
    fn cooldown_is_per_app() {
        let mut d = det();
        let zoom = [proc(1, "us.zoom.xos", true)];
        let slack = [proc(2, "com.tinyspeck.slackmacgap", true)];
        assert!(settle(&mut d, &zoom, 0).is_some());
        d.poll(&[], t(10));
        assert!(settle(&mut d, &slack, 20).is_some());
    }

    #[test]
    fn one_call_at_a_time_and_native_apps_first() {
        let mut d = det();
        let both = [
            proc(1, "com.google.Chrome.helper", true),
            proc(2, "us.zoom.xos", true),
        ];
        let p = settle(&mut d, &both, 0).unwrap();
        assert_eq!(p.app, App::Zoom, "native app outranks the browser");
        // The browser is part of the same call: no second prompt.
        for i in 2..10 {
            assert_eq!(d.poll(&both, t(i)), None);
        }
    }

    #[test]
    fn never_for_an_app_sticks_and_can_be_undone() {
        let mut d = det();
        d.never(App::Discord);
        let on = [proc(1, "com.hnc.Discord", true)];
        for i in 0..5 {
            assert_eq!(d.poll(&on, t(i)), None);
        }
        // It applies to that app only.
        let zoom = [proc(2, "us.zoom.xos", true)];
        assert!(settle(&mut d, &zoom, 10).is_some());
        d.allow(App::Discord);
        assert!(!d.is_never(App::Discord));
        d.poll(&[], t(20));
        assert!(settle(&mut d, &on, 30).is_some());
    }

    #[test]
    fn snooze_defers_the_prompt() {
        let mut d = det();
        d.snooze(App::Teams, t(0), Duration::from_secs(3600));
        let on = [proc(1, "com.microsoft.teams2", true)];
        assert_eq!(settle(&mut d, &on, 100), None);
        d.poll(&[], t(200));
        assert!(settle(&mut d, &on, 3700).is_some());
    }

    #[test]
    fn state_survives_a_round_trip_through_json() {
        let mut d = det();
        d.never(App::Slack);
        let on = [proc(1, "us.zoom.xos", true)];
        assert!(settle(&mut d, &on, 0).is_some());
        let json = serde_json::to_string(d.state()).unwrap();
        let restored: DetectState = serde_json::from_str(&json).unwrap();
        assert_eq!(&restored, d.state());
        let mut d2 = Detector::new(
            DetectConfig {
                own_pid: 999,
                ..Default::default()
            },
            restored,
        );
        assert!(d2.is_never(App::Slack));
        // A relaunch during the cooldown stays quiet.
        assert_eq!(settle(&mut d2, &on, 60), None);
        // Old or partial state files still load.
        let empty: DetectState = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, DetectState::default());
    }

    #[test]
    fn only_the_chosen_apps_are_detected() {
        let mut d = det();
        d.set_only(Some(&["teams".to_string(), "meet".to_string()]));
        let zoom = [proc(1, "us.zoom.xos", true)];
        for i in 0..4 {
            assert_eq!(d.poll(&zoom, t(i)), None, "zoom is not chosen");
        }
        let chrome = [proc(2, "com.google.Chrome", true)];
        d.poll(&chrome, t(10));
        assert_eq!(d.poll(&chrome, t(11)).map(|p| p.app), Some(App::Chrome));
        // Not offered in the list: always detected. FaceTime is offered.
        assert_eq!(
            App::from_bundle_id("com.apple.FaceTime"),
            Some(App::FaceTime)
        );
        assert_eq!(App::Discord.setting_key(), None);
        d.set_only(None);
        d.poll(&zoom, t(2000));
        assert_eq!(d.poll(&zoom, t(2001)).map(|p| p.app), Some(App::Zoom));
    }

    #[test]
    fn the_tap_targets_the_app_in_a_call() {
        let ps = [
            proc(10, "us.zoom.xos", true),
            proc(11, "us.zoom.xos.helper", false),
            proc(20, "com.spotify.client", false),
            proc(30, "com.google.Chrome", false),
        ];
        assert_eq!(meeting_app_pids(&ps, None, 999), vec![10, 11]);
        // A browser that only plays audio is not a call.
        assert!(meeting_app_pids(&ps[2..], None, 999).is_empty());
        // Zoom not chosen: nothing to tap.
        assert!(meeting_app_pids(&ps, Some(&["meet".to_string()]), 999).is_empty());
        // Our own process is never a target.
        assert!(meeting_app_pids(&ps, None, 10).is_empty());
    }

    #[test]
    fn parses_the_swift_process_list() {
        let json = r#"[{"pid":123,"bundle_id":"us.zoom.xos","input":true,"output":false},
                       {"pid":7,"bundle_id":"","input":false,"output":true},
                       {"pid":8,"input":true}]"#;
        let ps = parse_processes(json).unwrap();
        assert_eq!(ps.len(), 3);
        assert_eq!(
            ps[0],
            AudioProcess {
                pid: 123,
                bundle_id: "us.zoom.xos".into(),
                input: true,
                output: false
            }
        );
        assert_eq!(ps[2].bundle_id, "");
        assert!(parse_processes("nope").is_err());
        assert!(parse_processes("[]").unwrap().is_empty());
    }
}
