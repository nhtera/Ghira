// SPDX-License-Identifier: Apache-2.0
//! Calendar (phase 17, P1): the phone's EventKit calendar names a recording
//! after the event in progress and keeps that event's sealed facts with it
//! (title, attendees: they feed the vocabulary, cloud redaction, rename
//! suggestions and the notes template, as on the desktop). Events are read
//! on demand (Swift, `GhiCalendar.swift`), cached in memory for a minute and
//! never stored; every command that returns content refuses while the app
//! is locked. There is no ticker on the phone. Errors are codes.
//!
//! Off by default: the `calendar_phone` setting turns it on after the user answers
//! the system prompt in Settings → Calendar, and off again (iOS keeps the
//! permission; only Settings → Ghira revokes it).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use ghi_core::calendar::{self, CalEvent, RawEvent};
use ghi_store::store::Store;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::platform;

/// Not `calendar`: the desktop keeps a different shape under that name.
const SETTING: &str = "calendar_phone";
const CACHE_FOR_MS: i64 = 60_000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// The prompt can take as long as the user does.
const PROMPT_WAIT_MS: u64 = 5 * 60 * 1000;

/// What iOS says about calendar access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum CalendarAccess {
    /// Not on an iPhone (a browser, host tests).
    Unavailable,
    NotDetermined,
    /// Denied, restricted, or write-only (which cannot read events).
    Denied,
    Authorized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CalendarStatus {
    pub access: CalendarAccess,
    /// The user turned it on and iOS lets the app read events.
    pub connected: bool,
}

/// The calendar event in progress (or about to start), as the record screen
/// shows it. Names only inside a recorded meeting: here a count.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub title: String,
    pub start_ms: f64,
    pub end_ms: f64,
    /// Other attendees.
    pub attendees: u32,
    /// `zoom`, `teams`, `meet` or `null`.
    pub join_app: Option<String>,
}

/// The answer of `calendar_current_event` (a wrapper: a bare nullable struct
/// is inlined into the bindings' command line, which the grant check reads).
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CurrentEvent {
    pub event: Option<CalendarEvent>,
}

/// The `calendar_phone` setting. Off unless the user turned it on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    enabled: bool,
}

fn load(store: &Store) -> Stored {
    store
        .get_setting(SETTING)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn save(store: &Store, s: Stored) -> Result<(), String> {
    let v = serde_json::to_value(s).map_err(|_| "storage".to_string())?;
    store
        .set_setting(SETTING, &v)
        .map_err(|_| "storage".to_string())
}

fn status_of(stored: Stored, access: CalendarAccess) -> CalendarStatus {
    CalendarStatus {
        access,
        connected: stored.enabled && access == CalendarAccess::Authorized,
    }
}

fn status(store: &Store) -> CalendarStatus {
    status_of(load(store), platform::calendar_access())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Events and when they were read (wall clock, so a sleep counts).
type Cached = Option<(i64, Vec<CalEvent>)>;

static CACHE: LazyLock<Mutex<Cached>> = LazyLock::new(|| Mutex::new(None));
/// Bumped by every `forget`: a read that began before it must not refill the cache.
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn cache() -> std::sync::MutexGuard<'static, Cached> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Drops the cached events (the setting or access changed, or the app locked).
pub fn forget() {
    let mut c = cache();
    GENERATION.fetch_add(1, Ordering::AcqRel);
    *c = None;
}

/// The events Swift handed over (a JSON array of raw events), canceled ones
/// and junk left out, soonest first.
fn parse_events(json: &str) -> Vec<CalEvent> {
    let raw: Vec<RawEvent> = serde_json::from_str(json).unwrap_or_default();
    let mut events: Vec<CalEvent> = raw.into_iter().filter_map(calendar::from_raw).collect();
    events.sort_by(|a, b| a.start_ms.cmp(&b.start_ms).then_with(|| a.key.cmp(&b.key)));
    events.dedup_by(|a, b| a.key == b.key);
    events
}

/// The events around `now` (the last day to the next), cached for a minute.
/// Empty without access.
fn fetch() -> Vec<CalEvent> {
    let now = now_ms();
    if let Some((at, events)) = cache().as_ref()
        && (0..CACHE_FOR_MS).contains(&(now - at))
    {
        return events.clone();
    }
    let generation = GENERATION.load(Ordering::Acquire);
    let events = platform::calendar_events_json(now - DAY_MS, now + DAY_MS)
        .map(|j| parse_events(&j))
        .unwrap_or_default();
    let mut c = cache();
    if GENERATION.load(Ordering::Acquire) == generation {
        *c = Some((now, events.clone()));
    }
    events
}

fn view(e: &CalEvent) -> CalendarEvent {
    CalendarEvent {
        title: e.title.clone(),
        start_ms: e.start_ms as f64,
        end_ms: e.end_ms as f64,
        attendees: e.attendees.len() as u32,
        join_app: e.join_app.clone(),
    }
}

/// The meeting-like event in progress, else the one starting within ten
/// minutes (`calendar::current_event`).
fn current(events: &[CalEvent], now: i64) -> Option<CalendarEvent> {
    calendar::current_event(events, now).map(view)
}

/// Gives a recording that started at `now` the title and sealed facts of the
/// event in progress, if any.
fn name_recording(store: &Store, meeting: &str, events: &[CalEvent], now: i64) {
    if let Some(e) = calendar::event_for_recording(events, None, now) {
        calendar::name_meeting(store, meeting, e);
    }
}

/// Called after a recording started: with the calendar on, the event in
/// progress names the meeting (when it has no title) and its sealed facts are
/// kept. The recording does not wait for this: it runs on its own thread and
/// never fails or panics into the recording.
pub fn on_recording_started(store: std::sync::Arc<Store>, meeting: &str) {
    if !status(&store).connected {
        return;
    }
    let meeting = meeting.to_string();
    let _ = std::thread::Builder::new()
        .name("ghi-calendar-name".into())
        .spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                name_recording(&store, &meeting, &fetch(), now_ms())
            }));
        });
}

/// Where calendar access and the setting stand. Errors: `locked`, `storage`.
#[tauri::command]
#[specta::specta]
pub async fn calendar_status(core: ghi_app::CoreState<'_>) -> Result<CalendarStatus, String> {
    ghi_app::blocking(&core, |c| {
        let store = c.store()?;
        Ok(status(&store))
    })
    .await
}

/// Turns the calendar on: shows the system prompt the first time (resolves
/// with the answer), then keeps the setting if iOS allows reading events. A
/// denied answer leaves it off (`access` says `denied`: the UI points to
/// Settings). Errors: `locked`, `storage`.
#[tauri::command]
#[specta::specta]
pub async fn calendar_connect(core: ghi_app::CoreState<'_>) -> Result<CalendarStatus, String> {
    ghi_app::blocking(&core, |c| {
        let store = c.store()?;
        if platform::calendar_access() == CalendarAccess::NotDetermined {
            platform::request_calendar_access();
            // The prompt is asynchronous: wait for the answer.
            let until =
                std::time::Instant::now() + std::time::Duration::from_millis(PROMPT_WAIT_MS);
            while platform::calendar_access() == CalendarAccess::NotDetermined
                && std::time::Instant::now() < until
            {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
        forget();
        let on = platform::calendar_access() == CalendarAccess::Authorized;
        save(&store, Stored { enabled: on })?;
        Ok(status(&store))
    })
    .await
}

/// Turns the calendar off. iOS keeps the permission (only Settings → Ghira
/// takes it back); nothing is read until it is turned on again. Errors:
/// `locked`, `storage`.
#[tauri::command]
#[specta::specta]
pub async fn calendar_disconnect(core: ghi_app::CoreState<'_>) -> Result<CalendarStatus, String> {
    ghi_app::blocking(&core, |c| {
        let store = c.store()?;
        save(&store, Stored { enabled: false })?;
        forget();
        Ok(status(&store))
    })
    .await
}

/// The event in progress (or starting within ten minutes) when the calendar
/// is connected; `event` is `null` otherwise. Errors: `locked`, `storage`.
#[tauri::command]
#[specta::specta]
pub async fn calendar_current_event(core: ghi_app::CoreState<'_>) -> Result<CurrentEvent, String> {
    ghi_app::blocking(&core, |c| {
        let store = c.store()?;
        let event = if status(&store).connected {
            current(&fetch(), now_ms())
        } else {
            None
        };
        Ok(CurrentEvent { event })
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

    const NOW: i64 = 1_800_000_000_000;

    fn json(events: &str) -> String {
        format!("[{events}]")
    }

    fn standup(start: i64, end: i64) -> String {
        format!(
            r#"{{"id":"s1","title":"Standup","startMs":{start},"endMs":{end},
            "people":[{{"name":"Lan Nguyen","address":"mailto:lan@x.vn"}}],
            "location":"https://meet.google.com/abc-defg-hij"}}"#
        )
    }

    #[test]
    fn it_is_off_until_the_user_turns_it_on_and_ios_allows_it() {
        let (_t, store) = store();
        assert_eq!(load(&store), Stored::default());
        assert!(!status_of(load(&store), CalendarAccess::Authorized).connected);
        save(&store, Stored { enabled: true }).unwrap();
        assert!(status_of(load(&store), CalendarAccess::Authorized).connected);
        // Turned on, then access taken away in Settings: not connected.
        for a in [
            CalendarAccess::Denied,
            CalendarAccess::NotDetermined,
            CalendarAccess::Unavailable,
        ] {
            assert!(!status_of(load(&store), a).connected);
        }
        // A junk value reads as off.
        store.set_setting(SETTING, &serde_json::json!(3)).unwrap();
        assert_eq!(load(&store), Stored::default());
    }

    #[test]
    fn swift_events_parse_sorted_without_canceled_or_junk() {
        let later = standup(NOW + 3_600_000, NOW + 7_200_000).replace("s1", "s2");
        let canceled = r#"{"id":"c","title":"Gone","startMs":5,"canceled":true}"#;
        let events = parse_events(&json(&format!(
            "{later},{},{canceled}",
            standup(NOW - 60_000, NOW + 1_800_000)
        )));
        assert_eq!(
            events.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
            [
                format!("s1@{}", NOW - 60_000),
                format!("s2@{}", NOW + 3_600_000)
            ]
        );
        assert_eq!(events[0].join_app.as_deref(), Some("meet"));
        assert_eq!(events[0].attendees, ["Lan Nguyen"]);
        assert!(parse_events("not json").is_empty());
        assert!(parse_events("[]").is_empty());
    }

    #[test]
    fn the_record_screen_shows_the_meeting_in_progress_or_about_to_start() {
        let events = parse_events(&json(&standup(NOW - 60_000, NOW + 1_800_000)));
        let now = current(&events, NOW).unwrap();
        assert_eq!((now.title.as_str(), now.attendees), ("Standup", 1));
        assert_eq!(now.join_app.as_deref(), Some("meet"));
        // Ten minutes ahead counts, an hour ahead and after the end do not.
        assert!(current(&events, NOW - 60_000 - 9 * 60_000).is_some());
        assert!(current(&events, NOW - 60_000 - 3_600_000).is_none());
        assert!(current(&events, NOW + 2_000_000).is_none());
        // A lunch with nobody else is not meeting-like.
        let solo = parse_events(&json(
            r#"{"id":"l","title":"Lunch","startMs":1799999000000,"endMs":1800001000000}"#,
        ));
        assert!(current(&solo, NOW).is_none());
    }

    #[test]
    fn a_recording_is_named_after_the_event_and_keeps_its_sealed_facts() {
        let (_t, store) = store();
        let new = |title: &str| {
            store
                .create_meeting(ghi_store::store::NewMeeting {
                    title: title.into(),
                    ..Default::default()
                })
                .unwrap()
                .gid
        };
        let events = parse_events(&json(&standup(NOW - 60_000, NOW + 1_800_000)));
        let (blank, named, none) = (new(""), new("Mine"), new(""));
        name_recording(&store, &blank, &events, NOW);
        name_recording(&store, &named, &events, NOW);
        assert_eq!(store.get_meeting(&blank).unwrap().title, "Standup");
        assert_eq!(store.get_meeting(&named).unwrap().title, "Mine");
        let info = calendar::info(&store, &blank).unwrap();
        assert_eq!(info.attendees, ["Lan Nguyen"]);
        assert_eq!(info.emails, ["lan@x.vn"]);
        assert_eq!(calendar::info(&store, &named).unwrap().title, "Standup");
        // No event in progress: the meeting is left alone.
        name_recording(&store, &none, &events, NOW + 10_000_000);
        assert_eq!(store.get_meeting(&none).unwrap().title, "");
        assert!(calendar::info(&store, &none).is_none());
    }

    #[test]
    fn a_read_that_finishes_after_forget_does_not_refill_the_cache() {
        let g = GENERATION.load(Ordering::Acquire);
        forget();
        assert_ne!(GENERATION.load(Ordering::Acquire), g);
        assert!(cache().is_none());
    }

    /// Off an iPhone nothing is readable and nothing happens.
    #[test]
    fn off_ios_there_is_no_calendar() {
        assert_eq!(platform::calendar_access(), CalendarAccess::Unavailable);
        assert!(fetch().is_empty());
        let (_t, store) = store();
        let m = store
            .create_meeting(ghi_store::store::NewMeeting::default())
            .unwrap()
            .gid;
        save(&store, Stored { enabled: true }).unwrap();
        on_recording_started(Arc::new(store), &m);
    }
}
