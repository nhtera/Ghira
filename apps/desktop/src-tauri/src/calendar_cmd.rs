// SPDX-License-Identifier: Apache-2.0
//! Calendar (phase 14d): events from EventKit (macOS) or an ICS file, the "Up
//! next" strip, "ask to record when it starts", and naming a recording after
//! the event in progress. Events are never stored (D1); every command that
//! returns content refuses while the app is locked. Errors are codes.
//!
//! Sources: EventKit on macOS (`calendar_mac`) and one ICS file whose path
//! stays in Rust (setting `calendar`). Events are read on demand and cached
//! in memory for a minute; nothing here logs titles or names.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ghi_core::calendar::{self, CalEvent, MAX_ICS_BYTES};
use ghi_store::store::Store;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::core::Core;
use crate::{CoreState, blocking};

/// Calendar access and the connected ICS file.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CalendarStatus {
    /// EventKit: `unavailable` (not macOS), `notDetermined`, `denied`,
    /// `authorized`.
    pub eventkit: String,
    /// The connected ICS file (its name only; the path stays in Rust).
    pub ics: Option<IcsInfo>,
    /// Ask to record when a calendar meeting starts.
    pub ask_on_start: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IcsInfo {
    pub name: String,
    /// Events found in the next 14 days.
    pub events: u32,
}

/// Calendar settings the user can change.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CalendarPatch {
    #[serde(default)]
    #[specta(optional)]
    pub ask_on_start: Option<bool>,
}

/// One upcoming event, as the Up next strip and the popover show it.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EventView {
    /// Stable per occurrence; pass it to `set_event_ask`.
    pub key: String,
    pub title: String,
    pub start_ms: f64,
    pub end_ms: f64,
    /// Other attendees (a count; names only inside a recorded meeting).
    pub attendees: u32,
    /// `zoom`, `teams`, `meet` or `null`.
    pub join_app: Option<String>,
    /// Ask to record when it starts (armed by default for meeting-like events).
    pub ask: bool,
}

/// The `calendar` setting.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    /// The connected ICS file; the path never reaches the webview.
    ics_path: Option<String>,
    /// Off only when the user turned it off.
    ask_on_start: Option<bool>,
    /// Per event: ask (true) or not (false), pruned after the event.
    overrides: HashMap<String, bool>,
}

const SETTING: &str = "calendar";
const CACHE_FOR_MS: i64 = 60_000;
/// A parsed ICS file is read again after this long even if unchanged (the
/// window of recurring events moves).
const ICS_REPARSE_MS: i64 = 30 * 60_000;
const MAX_KEY_LEN: usize = 300;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
const FETCH_AHEAD_MS: i64 = 14 * DAY_MS;
const MAX_OVERRIDES: usize = 500;
const MAX_VIEWS: u32 = 50;
const TICK: Duration = Duration::from_secs(30);
/// The `app` of a prompt that came from the calendar alone.
pub(crate) const CALENDAR_APP: &str = "calendar";

impl Stored {
    fn asks(&self) -> bool {
        self.ask_on_start.unwrap_or(true)
    }

    fn armed(&self, e: &CalEvent) -> bool {
        self.overrides
            .get(&e.key)
            .copied()
            .unwrap_or_else(|| calendar::meeting_like(e))
    }
}

/// The parsed ICS file, kept while the file is unchanged.
struct IcsCache {
    path: String,
    mtime: Option<SystemTime>,
    len: u64,
    parsed_at: i64,
    events: Result<Vec<CalEvent>, String>,
}

#[derive(Default)]
struct Runtime {
    /// All sources' events and when they were read (wall clock, so a sleep
    /// counts).
    cache: Option<(i64, Vec<CalEvent>)>,
    ics: Option<IcsCache>,
    /// Events already asked about (by the ticker or the app detector).
    asked: HashSet<String>,
    /// The event last prompted, and when. Taken by the recording that follows
    /// "Start", cleared by any other answer.
    prompted: Option<(String, i64)>,
}

static RT: LazyLock<Mutex<Runtime>> = LazyLock::new(|| Mutex::new(Runtime::default()));

fn rt() -> std::sync::MutexGuard<'static, Runtime> {
    RT.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn load(store: &Store) -> Stored {
    store
        .get_setting(SETTING)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn save(store: &Store, s: &Stored) -> Result<(), String> {
    let v = serde_json::to_value(s).map_err(|_| "storage".to_string())?;
    store
        .set_setting(SETTING, &v)
        .map_err(|_| "storage".to_string())
}

/// Forgets the cached events (the sources changed).
fn forget() {
    let mut r = rt();
    r.cache = None;
    r.ics = None;
}

/// The start time inside an event key (`<id>@<start_ms>`).
fn key_start(key: &str) -> Option<i64> {
    key.rsplit_once('@')?.1.parse().ok()
}

/// Drops the overrides of events that started more than a day ago, and keys
/// that are not an event's.
fn prune(overrides: &mut HashMap<String, bool>, now: i64) {
    overrides.retain(|k, _| key_start(k).is_some_and(|s| s > now - DAY_MS));
}

fn eventkit_ready() -> bool {
    #[cfg(target_os = "macos")]
    return crate::calendar_mac::access() == crate::calendar_mac::Access::Authorized;
    #[cfg(not(target_os = "macos"))]
    false
}

/// Events of the ICS file, or its error code. Only a regular file is read,
/// and never more than the limit.
fn read_ics(path: &str, window: (i64, i64)) -> Result<Vec<CalEvent>, String> {
    use std::io::Read;
    let meta = std::fs::metadata(path).map_err(|_| "icsInvalid".to_string())?;
    if !meta.is_file() {
        return Err("icsInvalid".into());
    }
    if meta.len() > MAX_ICS_BYTES as u64 {
        return Err("icsTooLarge".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_ICS_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(|_| "icsInvalid".to_string())?;
    calendar::parse_ics(&bytes, window)
}

/// The ICS file's events around `now`, parsed again only when the file changed
/// (path, modification time, size) or after half an hour.
fn ics_events(path: &str, now: i64) -> Result<Vec<CalEvent>, String> {
    let meta = std::fs::metadata(path).ok();
    let mtime = meta.as_ref().and_then(|m| m.modified().ok());
    let len = meta.as_ref().map_or(0, |m| m.len());
    // Copied out, so the lock is not held past this statement.
    let hit = rt()
        .ics
        .as_ref()
        .filter(|c| {
            c.path == path
                && c.mtime == mtime
                && c.len == len
                && (0..ICS_REPARSE_MS).contains(&(now - c.parsed_at))
        })
        .map(|c| c.events.clone());
    if let Some(events) = hit {
        return events;
    }
    let events = read_ics(path, window(now));
    rt().ics = Some(IcsCache {
        path: path.to_string(),
        mtime,
        len,
        parsed_at: now,
        events: events.clone(),
    });
    events
}

fn window(now: i64) -> (i64, i64) {
    (now - DAY_MS, now + FETCH_AHEAD_MS)
}

/// The Calendar app's events and an ICS file's: a calendar that is both
/// subscribed there and imported as a file counts once (the app's first).
fn merge_sources(mut app: Vec<CalEvent>, ics: Vec<CalEvent>) -> Vec<CalEvent> {
    let seen: HashSet<(String, i64)> = app
        .iter()
        .map(|e| (ghi_text::fold(&e.title), e.start_ms))
        .collect();
    app.extend(
        ics.into_iter()
            .filter(|e| !seen.contains(&(ghi_text::fold(&e.title), e.start_ms))),
    );
    app
}

/// Every source's events, soonest first (cached for a minute).
fn fetch(store: &Store) -> Vec<CalEvent> {
    let now = now_ms();
    let hit = rt()
        .cache
        .as_ref()
        .filter(|(at, _)| (0..CACHE_FOR_MS).contains(&(now - at)))
        .map(|(_, events)| events.clone());
    if let Some(events) = hit {
        return events;
    }
    let mut events: Vec<CalEvent> = Vec::new();
    #[cfg(target_os = "macos")]
    if eventkit_ready()
        && let Ok(e) = crate::calendar_mac::events(window(now))
    {
        events.extend(e);
    }
    if let Some(path) = load(store).ics_path
        && let Ok(e) = ics_events(&path, now)
    {
        events = merge_sources(events, e);
    }
    events.sort_by(|a, b| a.start_ms.cmp(&b.start_ms).then_with(|| a.key.cmp(&b.key)));
    events.dedup_by(|a, b| a.key == b.key);
    rt().cache = Some((now, events.clone()));
    events
}

fn status(store: &Store) -> CalendarStatus {
    let stored = load(store);
    #[cfg(target_os = "macos")]
    let eventkit = crate::calendar_mac::access().as_str().to_string();
    #[cfg(not(target_os = "macos"))]
    let eventkit = "unavailable".to_string();
    let now = now_ms();
    let ics = stored.ics_path.as_deref().map(|p| IcsInfo {
        name: file_name(p),
        events: ics_events(p, now).map_or(0, |e| {
            e.iter()
                .filter(|e| e.start_ms >= now && e.start_ms <= now + FETCH_AHEAD_MS)
                .count() as u32
        }),
    });
    CalendarStatus {
        eventkit,
        ics,
        ask_on_start: stored.asks(),
    }
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

/// The strip's rows: not yet over, soonest first.
fn views(events: &[CalEvent], stored: &Stored, now: i64, limit: u32) -> Vec<EventView> {
    events
        .iter()
        .filter(|e| !e.all_day && e.end_ms.max(e.start_ms + 60_000) > now)
        .take(limit.min(MAX_VIEWS) as usize)
        .map(|e| EventView {
            key: e.key.clone(),
            title: e.title.clone(),
            start_ms: e.start_ms as f64,
            end_ms: e.end_ms as f64,
            attendees: e.attendees.len() as u32,
            join_app: e.join_app.clone(),
            ask: stored.armed(e),
        })
        .collect()
}

/// Where calendar access stands. Errors: `storage`.
#[tauri::command]
#[specta::specta]
pub async fn calendar_status(core: CoreState<'_>) -> Result<CalendarStatus, String> {
    blocking(&core, |c| {
        let store = c.store()?;
        Ok(status(&store))
    })
    .await
}

/// Asks macOS for calendar access (the OS prompt). Errors: `notSupported`.
#[tauri::command]
#[specta::specta]
pub async fn request_calendar_access(core: CoreState<'_>) -> Result<CalendarStatus, String> {
    blocking(&core, |c| {
        let store = c.store()?;
        #[cfg(target_os = "macos")]
        {
            crate::calendar_mac::request_access();
            forget();
            Ok(status(&store))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = store;
            Err("notSupported".into())
        }
    })
    .await
}

/// Changes calendar settings; returns the new status. Errors: `storage`.
#[tauri::command]
#[specta::specta]
pub async fn set_calendar(
    core: CoreState<'_>,
    patch: CalendarPatch,
) -> Result<CalendarStatus, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        let mut s = load(&store);
        if let Some(ask) = patch.ask_on_start {
            s.ask_on_start = Some(ask);
        }
        save(&store, &s)?;
        Ok(status(&store))
    })
    .await
}

/// One native file dialog at a time.
static PICKING: AtomicBool = AtomicBool::new(false);

/// Lets the user pick an .ics file (a native dialog in Rust); returns its
/// name, or `null` if cancelled. Errors: `icsInvalid`, `icsTooLarge`,
/// `storage`.
#[tauri::command]
#[specta::specta]
pub async fn pick_ics_file(core: CoreState<'_>) -> Result<Option<String>, String> {
    // Locked: no dialog either.
    blocking(&core, |c| c.store().map(|_| ())).await?;
    if PICKING.swap(true, Ordering::AcqRel) {
        return Err("busy".into());
    }
    let picked = rfd::AsyncFileDialog::new()
        .add_filter("iCalendar", &["ics"])
        .pick_file()
        .await;
    PICKING.store(false, Ordering::Release);
    let Some(file) = picked else { return Ok(None) };
    let path = file.path().to_string_lossy().into_owned();
    blocking(&core, move |c| {
        let store = c.store()?;
        // Read it once now so a bad file is refused before it is kept.
        let now = now_ms();
        read_ics(&path, (now, now + FETCH_AHEAD_MS))?;
        let name = file_name(&path);
        let mut s = load(&store);
        s.ics_path = Some(path);
        save(&store, &s)?;
        forget();
        Ok(Some(name))
    })
    .await
}

/// Forgets the ICS file. Errors: `storage`.
#[tauri::command]
#[specta::specta]
pub async fn remove_ics_file(core: CoreState<'_>) -> Result<(), String> {
    blocking(&core, |c| {
        let store = c.store()?;
        let mut s = load(&store);
        s.ics_path = None;
        save(&store, &s)?;
        forget();
        Ok(())
    })
    .await
}

/// The next events (at most `limit`, soonest first), meeting-like ones
/// included whether armed or not. Empty without a calendar. Errors: `storage`.
#[tauri::command]
#[specta::specta]
pub async fn upcoming_events(core: CoreState<'_>, limit: u32) -> Result<Vec<EventView>, String> {
    blocking(&core, move |c| {
        let store = c.store()?;
        Ok(views(&fetch(&store), &load(&store), now_ms(), limit))
    })
    .await
}

/// Turns "ask to record when it starts" on or off for one event. Errors:
/// `storage`.
#[tauri::command]
#[specta::specta]
pub async fn set_event_ask(core: CoreState<'_>, key: String, ask: bool) -> Result<(), String> {
    blocking(&core, move |c| {
        if key.len() > MAX_KEY_LEN || key_start(&key).is_none() {
            return Err("invalid".into());
        }
        let store = c.store()?;
        let mut s = load(&store);
        prune(&mut s.overrides, now_ms());
        if s.overrides.len() < MAX_OVERRIDES || s.overrides.contains_key(&key) {
            s.overrides.insert(key, ask);
        }
        save(&store, &s)
    })
    .await
}

/// The user answered a calendar prompt (of the ticker or the app detector). Any
/// answer but Start forgets which event it was about, so a dismissed prompt
/// never names a recording started later.
pub(crate) fn prompt_answered(start: bool) {
    if !start {
        rt().prompted = None;
    }
}

/// Called after a recording started: a meeting-like event in progress (from
/// 10 minutes before its start to its end), or the one the user was just
/// prompted for, gives the meeting its title (when it has none) and its
/// sealed calendar info (D4). The recording does not wait for this: it runs on
/// its own thread and never fails or panics into the recording.
pub(crate) fn on_recording_started(core: &Core, meeting: &str) {
    let Ok(store) = core.store() else { return };
    // Taken now: a later answer must not change which event this was.
    let prompted = rt().prompted.take();
    let meeting = meeting.to_string();
    let _ = std::thread::Builder::new()
        .name("ghi-calendar-name".into())
        .spawn(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                name_recording(&store, &meeting, prompted)
            }));
        });
}

fn name_recording(store: &Store, meeting: &str, prompted: Option<(String, i64)>) {
    let now = now_ms();
    let events = fetch(store);
    let Some(e) = calendar::event_for_recording(&events, prompted.as_ref(), now) else {
        return;
    };
    // Recording it already: no prompt for this event when it is stopped.
    rt().asked.insert(e.key.clone());
    calendar::name_meeting(store, meeting, e);
}

/// What the app detector does with a calendar event in progress.
#[cfg(target_os = "macos")]
pub(crate) enum Attach {
    /// Carry the event's title and key on the prompt.
    Event { title: String, key: String },
    /// The event was already asked about: no second prompt (D3).
    Skip,
}

/// Called when the app detector is about to prompt: ties the prompt to the
/// meeting-like event in progress, once per event.
#[cfg(target_os = "macos")]
pub(crate) fn for_detection(core: &Core) -> Option<Attach> {
    let store = core.store().ok()?;
    let now = now_ms();
    let events = fetch(&store);
    let stored = load(&store);
    let e = calendar::current_event(&events, now).filter(|e| stored.armed(e))?;
    let mut r = rt();
    if !r.asked.insert(e.key.clone()) {
        return Some(Attach::Skip);
    }
    r.prompted = Some((e.key.clone(), now));
    Some(Attach::Event {
        title: e.title.clone(),
        key: e.key.clone(),
    })
}

/// The next event to ask about (marks it asked), or none.
fn claim(
    events: &[CalEvent],
    stored: &Stored,
    asked: &mut HashSet<String>,
    now: i64,
) -> Option<CalEvent> {
    let e = calendar::due_prompts(events, asked, &stored.overrides, now)
        .into_iter()
        .next()?
        .clone();
    asked.insert(e.key.clone());
    Some(e)
}

/// One pass of the ticker.
fn tick(app: &tauri::AppHandle, core: &Core) {
    // Recording or starting, or locked: no prompt, no calendar reads.
    if core.busy() {
        return;
    }
    let Ok(store) = core.store() else { return };
    let stored = load(&store);
    if !stored.asks() || !(eventkit_ready() || stored.ics_path.is_some()) {
        return;
    }
    let now = now_ms();
    let events = fetch(&store);
    let e = {
        let mut r = rt();
        // Forget asked events long over.
        r.asked
            .retain(|k| key_start(k).is_none_or(|s| s > now - DAY_MS));
        let Some(e) = claim(&events, &stored, &mut r.asked, now) else {
            return;
        };
        r.prompted = Some((e.key.clone(), now));
        e
    };
    crate::system::show_detect(
        app,
        crate::system::MeetingDetected {
            app: CALENDAR_APP.into(),
            app_name: String::new(),
            browser: false,
            title: Some(e.title),
            event: Some(e.key),
        },
    );
}

/// Starts the calendar ticker (every 30 s; also where only an ICS file works).
pub fn spawn_ticker(app: tauri::AppHandle, core: std::sync::Arc<Core>) {
    let _ = std::thread::Builder::new()
        .name("ghi-calendar".into())
        .spawn(move || {
            loop {
                std::thread::sleep(TICK);
                // A failure of one pass must not end the ticker.
                let _ =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tick(&app, &core)));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(key: &str, start: i64, people: usize) -> CalEvent {
        CalEvent {
            key: key.into(),
            title: key.into(),
            start_ms: start,
            end_ms: start + 30 * 60_000,
            all_day: false,
            attendees: (0..people).map(|i| format!("P{i}")).collect(),
            emails: Vec::new(),
            join_app: None,
            from_ics: false,
        }
    }

    #[test]
    fn views_skip_ended_and_all_day_and_arm_meeting_like_events() {
        let now = 10_000_000;
        let mut all_day = ev("trip@1", now + 1000, 3);
        all_day.all_day = true;
        let events = vec![
            ev("over@1", now - 3_600_000, 2),
            all_day,
            ev("sync@2", now + 600_000, 2),
            ev("lunch@3", now + 900_000, 0),
        ];
        let mut stored = Stored::default();
        stored.overrides.insert("lunch@3".into(), true);
        stored.overrides.insert("sync@2".into(), false);
        let v = views(&events, &stored, now, 10);
        assert_eq!(
            v.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            ["sync@2", "lunch@3"]
        );
        assert!(!v[0].ask, "override off wins over meeting-like");
        assert!(v[1].ask, "override on wins over no attendees");
        assert_eq!(v[0].attendees, 2);
        assert_eq!(views(&events, &Stored::default(), now, 1).len(), 1);
        assert!(views(&events, &Stored::default(), now, 0).is_empty());
        // Not yet armed by default for an event with nobody else.
        assert!(!views(&events, &Stored::default(), now, 10)[1].ask);
    }

    #[test]
    fn overrides_are_pruned_a_day_after_the_event() {
        let now = 10 * DAY_MS;
        let mut o = HashMap::new();
        o.insert(format!("old@{}", now - 2 * DAY_MS), true);
        o.insert(format!("new@{}", now + 1000), false);
        o.insert("odd-key".into(), true);
        prune(&mut o, now);
        assert_eq!(o.len(), 1, "old ones and keys that are not an event's go");
        assert!(o.contains_key(&format!("new@{}", now + 1000)));
        assert_eq!(key_start("a@b@123"), Some(123));
        assert_eq!(key_start("nokey"), None);
    }

    #[test]
    fn a_prompt_is_claimed_once_and_the_off_switch_holds() {
        let t = 5_000_000;
        let events = vec![ev("a@1", t, 1), ev("b@2", t, 1)];
        let stored = Stored::default();
        let mut asked = HashSet::new();
        assert_eq!(claim(&events, &stored, &mut asked, t).unwrap().key, "a@1");
        assert_eq!(claim(&events, &stored, &mut asked, t).unwrap().key, "b@2");
        assert!(claim(&events, &stored, &mut asked, t).is_none());
        let mut off = Stored::default();
        off.overrides.insert("a@1".into(), false);
        assert_eq!(
            claim(&events, &off, &mut HashSet::new(), t).unwrap().key,
            "b@2"
        );
    }

    #[test]
    fn stored_settings_tolerate_old_and_junk_values() {
        let s: Stored = serde_json::from_value(serde_json::json!({ "askOnStart": false })).unwrap();
        assert!(!s.asks() && s.ics_path.is_none());
        assert!(Stored::default().asks());
        assert!(serde_json::from_value::<Stored>(serde_json::json!({ "overrides": 3 })).is_err());
    }

    #[test]
    fn ics_errors_are_codes() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_ics("/no/such/file.ics", (0, 1)),
            Err("icsInvalid".into())
        );
        let bad = dir.path().join("bad.ics");
        std::fs::write(&bad, "not a calendar").unwrap();
        assert_eq!(
            read_ics(bad.to_str().unwrap(), (0, 1)),
            Err("icsInvalid".into())
        );
        let good = dir.path().join("good.ics");
        std::fs::write(
            &good,
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART:20261005T030000Z\r\nSUMMARY:Hi\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        let got = read_ics(good.to_str().unwrap(), (0, i64::MAX / 2)).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(file_name(good.to_str().unwrap()), "good.ics");
    }

    /// Runs a test body on a thread and fails, rather than hangs, if it does not
    /// finish (a lock held across a call that locks again).
    fn bounded(body: impl FnOnce() + Send + 'static) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            body();
            let _ = tx.send(());
        });
        rx.recv_timeout(Duration::from_secs(20))
            .expect("the test did not finish: a runtime lock is probably held twice");
    }

    // The runtime state is global: tests that touch it take turns.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn store() -> (tempfile::TempDir, std::sync::Arc<Store>) {
        use ghi_store::keys::{MemoryKeyStore, Protection};
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::open(
            tmp.path(),
            std::sync::Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        (tmp, std::sync::Arc::new(s))
    }

    fn seed(events: Vec<CalEvent>) {
        let mut r = rt();
        r.cache = Some((now_ms(), events));
        r.asked.clear();
        r.prompted = None;
    }

    fn new_meeting(store: &Store) -> String {
        store
            .create_meeting(ghi_store::store::NewMeeting::default())
            .unwrap()
            .gid
    }

    #[test]
    fn a_recording_started_by_hand_in_an_event_is_named_and_marks_it_asked() {
        bounded(|| {
            let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
            let (_tmp, store) = store();
            let now = now_ms();
            let mut e = ev("hand@1", now - 5 * 60_000, 2);
            e.key = format!("hand@{}", e.start_ms);
            seed(vec![e.clone()]);
            let m = new_meeting(&store);
            name_recording(&store, &m, None);
            assert_eq!(store.get_meeting(&m).unwrap().title, "hand@1");
            let info = calendar::info(&store, &m).unwrap();
            assert_eq!(info.attendees, ["P0", "P1"]);
            // After stopping, the ticker does not ask about the same event.
            assert!(rt().asked.contains(&e.key));
            let stored = Stored::default();
            let mut asked = rt().asked.clone();
            assert!(claim(&[e], &stored, &mut asked, now).is_none());
        });
    }

    #[test]
    fn a_dismissed_prompt_names_no_later_recording() {
        bounded(|| {
            let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
            let (_tmp, store) = store();
            let now = now_ms();
            // The event is over; its prompt was shown and then answered "Not now".
            let mut e = ev("over@1", now - 2 * 3_600_000, 3);
            e.key = format!("over@{}", e.start_ms);
            seed(vec![e.clone()]);
            rt().prompted = Some((e.key.clone(), now - 1000));
            prompt_answered(false);
            assert!(rt().prompted.is_none());
            let m = new_meeting(&store);
            let none = rt().prompted.take();
            name_recording(&store, &m, none);
            assert_eq!(store.get_meeting(&m).unwrap().title, "");
            assert!(calendar::info(&store, &m).is_none());
            // Start keeps it for the recording that follows, which takes it once.
            let mut soon = ev("soon@1", now + 2 * 60_000, 1);
            soon.key = format!("soon@{}", soon.start_ms);
            seed(vec![soon.clone()]);
            rt().prompted = Some((soon.key.clone(), now));
            prompt_answered(true);
            let taken = rt().prompted.take();
            let m2 = new_meeting(&store);
            name_recording(&store, &m2, taken);
            assert_eq!(store.get_meeting(&m2).unwrap().title, "soon@1");
            assert!(rt().prompted.is_none());
        });
    }

    #[test]
    fn a_calendar_in_the_app_and_as_a_file_counts_once() {
        let mut a = ev("Sprint planning@1", 5_000, 2);
        a.key = "uid-a@5000".into();
        let mut b = ev("sprint   planning@1", 5_000, 2);
        b.title = "Sprint Planning@1".into();
        b.key = "uid-b@5000".into();
        let c = ev("Other@2", 6_000, 1);
        let merged = merge_sources(vec![a.clone()], vec![b, c.clone()]);
        assert_eq!(
            merged.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
            [a.key.as_str(), c.key.as_str()]
        );
    }

    #[test]
    fn the_ics_cache_follows_the_file_and_a_directory_is_not_a_file() {
        let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_ics(dir.path().to_str().unwrap(), (0, 1)),
            Err("icsInvalid".into())
        );
        let f = dir.path().join("a.ics");
        let body = |title: &str| {
            format!(
                "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART:20261005T030000Z\r\nSUMMARY:{title}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            )
        };
        std::fs::write(&f, body("One")).unwrap();
        let p = f.to_str().unwrap();
        let t = 1_790_000_000_000;
        forget();
        assert_eq!(ics_events(p, t).unwrap()[0].title, "One");
        // Unchanged: the cached parse (the file is not read again).
        assert_eq!(rt().ics.as_ref().unwrap().parsed_at, t);
        assert_eq!(ics_events(p, t + 1000).unwrap()[0].title, "One");
        assert_eq!(rt().ics.as_ref().unwrap().parsed_at, t);
        // A change of size is a new parse.
        std::fs::write(&f, body("Longer title")).unwrap();
        assert_eq!(ics_events(p, t + 2000).unwrap()[0].title, "Longer title");
        // Half an hour on, it is read again.
        assert_eq!(ics_events(p, t + 2000 + ICS_REPARSE_MS).unwrap().len(), 1);
        assert_eq!(
            rt().ics.as_ref().unwrap().parsed_at,
            t + 2000 + ICS_REPARSE_MS
        );
        forget();
    }

    #[test]
    fn event_keys_must_look_like_an_event_s() {
        assert!(key_start("uid@123").is_some());
        assert!(key_start(&"a".repeat(MAX_KEY_LEN + 1)).is_none());
    }
}
