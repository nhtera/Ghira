// SPDX-License-Identifier: Apache-2.0
//! Calendar events (phase 14d): the shape of an event, ICS parsing, and the
//! rules that turn events into prompts, titles and suggestions. Events are not
//! stored (D1): they come from EventKit or an ICS file on demand; only what a
//! recorded meeting needs is kept, sealed, in [`CalendarInfo`].
//!
//! W0-B stub: the signatures are final, the bodies are inert (slice S4).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// One calendar event, in memory only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalEvent {
    /// Stable per occurrence (UID + start for recurring events).
    pub key: String,
    pub title: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub all_day: bool,
    /// Display names of the other attendees (not the user).
    pub attendees: Vec<String>,
    /// `zoom`, `teams` or `meet` when the invite has such a link.
    pub join_app: Option<String>,
}

/// What `meetings.calendar_ct` holds for a recorded meeting.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarInfo {
    pub event: String,
    pub title: String,
    pub attendees: Vec<String>,
    /// The calendar the event came from.
    pub calendar: Option<String>,
}

/// Largest ICS file read.
pub const MAX_ICS_BYTES: usize = 20 * 1024 * 1024;

/// Events of an ICS file starting inside `window` (unix ms, from..to), with
/// recurrences expanded and cancelled events left out.
pub fn parse_ics(_bytes: &[u8], _window: (i64, i64)) -> Result<Vec<CalEvent>, String> {
    Err("notImplemented".into())
}

/// `zoom`, `teams` or `meet` for a text with such a join link.
pub fn join_app(_text: &str) -> Option<&'static str> {
    None
}

/// Worth asking to record: another attendee or a call link.
pub fn meeting_like(e: &CalEvent) -> bool {
    !e.all_day && (!e.attendees.is_empty() || e.join_app.is_some())
}

/// The next event starting after `now_ms`.
pub fn next_event(_events: &[CalEvent], _now_ms: i64) -> Option<&CalEvent> {
    None
}

/// The meeting-like event in progress (or starting within 10 minutes).
pub fn current_event(_events: &[CalEvent], _now_ms: i64) -> Option<&CalEvent> {
    None
}

/// Events to ask about now: starting within [-1 min, +5 min] of `now_ms`,
/// armed (`overrides` else [`meeting_like`]) and not in `asked`.
pub fn due_prompts<'a>(
    _events: &'a [CalEvent],
    _asked: &HashSet<String>,
    _overrides: &HashMap<String, bool>,
    _now_ms: i64,
) -> Vec<&'a CalEvent> {
    Vec::new()
}

/// A notes template id for a title and attendees (`1:1` for exactly two
/// people, keyword lists in EN and VN), when nothing was chosen.
pub fn suggest_template(_title: &str, _attendees: &[String]) -> Option<&'static str> {
    None
}
