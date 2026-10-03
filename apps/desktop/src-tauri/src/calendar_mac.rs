// SPDX-License-Identifier: Apache-2.0
//! EventKit on macOS (phase 14d): full access to events, read on demand. The
//! `EKEventStore` is created inside each blocking call (objc2 objects are not
//! `Send`). Notes are read only to find a call link and are never stored or
//! logged.

use std::sync::Mutex;
use std::time::Duration;

use block2::RcBlock;
use ghi_core::calendar::{CalEvent, email_of, join_app, person_name};
use objc2::msg_send;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::Bool;
use objc2_event_kit::{
    EKAuthorizationStatus, EKEntityType, EKEvent, EKEventStatus, EKEventStore, EKParticipant,
};
use objc2_foundation::{NSDate, NSError, NSString, NSURL};

/// Most events read in one call.
const MAX_EVENTS: usize = 2_000;
/// Most of an event's notes looked at for a call link.
const NOTES_CHARS: usize = 20_000;

/// The one event store, made on first use. objc2 objects are not `Send`;
/// every use is under the mutex, one thread at a time.
struct SharedStore(Retained<EKEventStore>);
// SAFETY: the store is only touched while STORE's lock is held.
unsafe impl Send for SharedStore {}
static STORE: Mutex<Option<SharedStore>> = Mutex::new(None);

fn with_store<R>(f: impl FnOnce(&EKEventStore) -> R) -> R {
    let mut slot = STORE.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: a fresh store.
    let s = slot.get_or_insert_with(|| SharedStore(unsafe { EKEventStore::new() }));
    f(&s.0)
}

/// Drops the shared store (after access changed, the next call makes a new one).
fn forget_store() {
    *STORE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// What macOS says about calendar access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    NotDetermined,
    Denied,
    Authorized,
}

impl Access {
    /// The word `CalendarStatus.eventkit` carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Access::NotDetermined => "notDetermined",
            Access::Denied => "denied",
            Access::Authorized => "authorized",
        }
    }
}

/// The current authorization (never prompts).
pub fn access() -> Access {
    // SAFETY: a class method taking a plain enum.
    let s = unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) };
    if s == EKAuthorizationStatus::NotDetermined {
        Access::NotDetermined
    } else if s == EKAuthorizationStatus::FullAccess {
        Access::Authorized
    } else {
        // Restricted, denied, or write-only (which cannot read events).
        Access::Denied
    }
}

/// Shows the OS prompt (blocks until answered; call off the UI thread).
pub fn request_access() -> Access {
    if access() != Access::NotDetermined {
        return access();
    }
    // SAFETY: a fresh store.
    let store = unsafe { EKEventStore::new() };
    let (tx, rx) = std::sync::mpsc::channel();
    let reply = RcBlock::new(move |granted: Bool, _err: *mut NSError| {
        let _ = tx.send(granted.as_bool());
    });
    // SAFETY: the store and the block outlive the call; the reply runs once
    // on a system queue and only sends on the channel.
    unsafe { store.requestFullAccessToEventsWithCompletion(&*reply as *const _ as *mut _) };
    let _ = rx.recv_timeout(Duration::from_secs(300));
    forget_store();
    access()
}

fn date(ms: i64) -> Retained<NSDate> {
    NSDate::dateWithTimeIntervalSince1970(ms as f64 / 1000.0)
}

fn ms(d: &NSDate) -> i64 {
    (d.timeIntervalSince1970() * 1000.0).round() as i64
}

/// A participant's display name; `None` for the user themself.
fn participant(p: &EKParticipant) -> Option<(String, String)> {
    // SAFETY: plain getters on a live participant; nil is handled.
    unsafe {
        if p.isCurrentUser() {
            return None;
        }
        let name = p.name().map(|n| n.to_string());
        let url: Option<Retained<NSURL>> = msg_send![p, URL];
        let addr = url
            .and_then(|u| u.absoluteString())
            .map(|s| s.to_string())
            .unwrap_or_default();
        Some(person_name(name.as_deref(), &addr))
            .filter(|n| n.trim().chars().count() >= 2)
            .map(|n| (n, email_of(&addr).unwrap_or_default()))
    }
}

fn convert(e: &EKEvent) -> Option<CalEvent> {
    // SAFETY: plain getters on a live event; the ones that could be nil are
    // read as options.
    unsafe {
        if e.status() == EKEventStatus::Canceled {
            return None;
        }
        let start: Option<Retained<NSDate>> = msg_send![e, startDate];
        let end: Option<Retained<NSDate>> = msg_send![e, endDate];
        let start = start?;
        let start_ms = ms(&start);
        let end_ms = end.map_or(start_ms, |d| ms(&d)).max(start_ms);
        let title: Option<Retained<NSString>> = msg_send![e, title];
        let title = title.map(|t| t.to_string()).unwrap_or_default();
        // The invite's own id, so a calendar both subscribed and imported
        // gives the same key; else the store's.
        let id = e
            .calendarItemExternalIdentifier()
            .or_else(|| e.eventIdentifier())
            .map(|s| s.to_string())
            .unwrap_or_else(|| e.calendarItemIdentifier().to_string());
        let mut attendees: Vec<String> = Vec::new();
        let mut emails: Vec<String> = Vec::new();
        let people = e
            .organizer()
            .into_iter()
            .chain(e.attendees().into_iter().flat_map(|a| a.to_vec()));
        for (name, email) in people.filter_map(|p| participant(&p)) {
            let folded = ghi_text::fold(&name);
            if !attendees.iter().any(|a| ghi_text::fold(a) == folded) {
                attendees.push(name);
                emails.push(email);
            }
        }
        let url: Option<Retained<NSURL>> = msg_send![e, URL];
        let text = [
            Some(title.clone()),
            e.location().map(|s| s.to_string()),
            url.and_then(|u| u.absoluteString()).map(|s| s.to_string()),
            e.notes()
                .map(|s| s.to_string().chars().take(NOTES_CHARS).collect()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<String>>()
        .join("\n");
        Some(CalEvent {
            key: format!("{id}@{start_ms}"),
            title: title.trim().chars().take(200).collect(),
            start_ms,
            end_ms,
            all_day: e.isAllDay(),
            attendees,
            emails,
            join_app: join_app(&text).map(str::to_string),
            from_ics: false,
        })
    }
}

/// Events starting inside `window` (unix ms, from..to), soonest first.
/// Errors: `calendarDenied`.
pub fn events(window: (i64, i64)) -> Result<Vec<CalEvent>, String> {
    if access() != Access::Authorized {
        return Err("calendarDenied".into());
    }
    let mut out: Vec<CalEvent> = autoreleasepool(|_| {
        // SAFETY: the predicate is built from, and used with, the same store.
        let found = with_store(|store| unsafe {
            let predicate = store.predicateForEventsWithStartDate_endDate_calendars(
                &date(window.0),
                &date(window.1),
                None,
            );
            store.eventsMatchingPredicate(&predicate)
        });
        found
            .iter()
            .take(MAX_EVENTS)
            .filter_map(|e| convert(&e))
            .collect()
    });
    out.sort_by(|a, b| a.start_ms.cmp(&b.start_ms).then_with(|| a.key.cmp(&b.key)));
    Ok(out)
}
