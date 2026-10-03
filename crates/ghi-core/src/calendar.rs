// SPDX-License-Identifier: Apache-2.0
//! Calendar events (phase 14d): the shape of an event, ICS parsing, and the
//! rules that turn events into prompts, titles and suggestions. Events are not
//! stored (D1): they come from EventKit or an ICS file on demand; only what a
//! recorded meeting needs is kept, sealed, in [`CalendarInfo`].

use std::collections::{HashMap, HashSet};

use chrono::{Local, NaiveDateTime, TimeZone};
use icalendar::{
    Calendar, CalendarDateTime, Component, DatePerhapsTime, Event, EventLike, EventStatus,
};
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
    /// From an ICS file, which may list the user among the attendees.
    #[serde(default)]
    pub from_ics: bool,
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

/// The calendar facts stored with a recorded meeting, if any (and readable).
pub fn info(store: &ghi_store::store::Store, meeting: &str) -> Option<CalendarInfo> {
    store
        .calendar_info(meeting)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value(v).ok())
}

/// Largest ICS file read.
pub const MAX_ICS_BYTES: usize = 20 * 1024 * 1024;

/// Most events looked at in one file, and most occurrences returned.
const MAX_EVENTS: usize = 20_000;
const MAX_OCCURRENCES: usize = 2_000;
/// Most occurrences taken from one recurring event.
const MAX_PER_EVENT: usize = 200;
/// Steps a recurrence rule may walk: per event, and per file.
const EVENT_STEPS: u64 = 50_000;
const FILE_STEPS: u64 = 500_000;
const MAX_TITLE_CHARS: usize = 200;
const MAX_ATTENDEES: usize = 100;

const MIN_MS: i64 = 60_000;
const DAY_MS: i64 = 24 * 60 * MIN_MS;
/// How early before its start an event counts as "in progress".
const LEAD_MS: i64 = 10 * MIN_MS;

/// Windows time zone names (Outlook exports) and the IANA zone they mean.
const WINDOWS_ZONES: &[(&str, &str)] = &[
    ("SE Asia Standard Time", "Asia/Bangkok"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("China Standard Time", "Asia/Shanghai"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("Korea Standard Time", "Asia/Seoul"),
    ("India Standard Time", "Asia/Kolkata"),
    ("Taipei Standard Time", "Asia/Taipei"),
    ("W. Australia Standard Time", "Australia/Perth"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
    ("GMT Standard Time", "Europe/London"),
    ("W. Europe Standard Time", "Europe/Berlin"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("Romance Standard Time", "Europe/Paris"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("FLE Standard Time", "Europe/Kiev"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("Turkey Standard Time", "Europe/Istanbul"),
    ("Arab Standard Time", "Asia/Riyadh"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("Eastern Standard Time", "America/New_York"),
    ("Central Standard Time", "America/Chicago"),
    ("Mountain Standard Time", "America/Denver"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("Central Standard Time (Mexico)", "America/Mexico_City"),
    ("Mountain Standard Time (Mexico)", "America/Chihuahua"),
    ("Pacific Standard Time (Mexico)", "America/Tijuana"),
    ("US Mountain Standard Time", "America/Phoenix"),
    ("Alaskan Standard Time", "America/Anchorage"),
    ("Hawaiian Standard Time", "Pacific/Honolulu"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("UTC", "UTC"),
];

/// Mailbox names that are not a person (without a display name).
const ROLE_NAMES: &[&str] = &[
    "sales",
    "team",
    "info",
    "support",
    "noreply",
    "no-reply",
    "donotreply",
    "do-not-reply",
    "admin",
    "hello",
    "contact",
    "office",
    "billing",
    "help",
    "hr",
    "mail",
    "calendar",
];

fn err(code: &str) -> String {
    code.to_string()
}

/// Events of an ICS file starting inside `window` (unix ms, from..to), with
/// recurrences expanded and cancelled events left out. Soonest first.
/// Errors: `icsTooLarge`, `icsInvalid`.
pub fn parse_ics(bytes: &[u8], window: (i64, i64)) -> Result<Vec<CalEvent>, String> {
    if bytes.len() > MAX_ICS_BYTES {
        return Err(err("icsTooLarge"));
    }
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}');
    if !text.contains("BEGIN:VCALENDAR") {
        return Err(err("icsInvalid"));
    }
    let text = map_windows_zones(text);
    let cal: Calendar = text.parse().map_err(|_| err("icsInvalid"))?;

    let events: Vec<icalendar::CalendarEvent<'_>> =
        cal.calendar_events().take(MAX_EVENTS).collect();
    let user = infer_user(&events);

    // Moved or edited occurrences of a recurring event replace the rule's own.
    let mut replaced: HashSet<(String, i64)> = HashSet::new();
    for e in events.iter().map(|c| c.event()) {
        if let (Some(uid), Some(id)) = (e.get_uid(), e.get_recurrence_id())
            && let Some(ms) = to_ms(&id)
        {
            replaced.insert((uid.to_string(), ms));
        }
    }

    let mut budget = FILE_STEPS;
    let mut out: Vec<CalEvent> = Vec::new();
    for ev in &events {
        let e = ev.event();
        if e.get_status() == Some(EventStatus::Cancelled) {
            continue;
        }
        let Some(start) = e.get_start() else { continue };
        let Some(start_ms) = to_ms(&start) else {
            continue;
        };
        let all_day = matches!(start, DatePerhapsTime::Date(_));
        let dur_ms = duration_ms(e, start_ms, all_day);
        let uid = e.get_uid().unwrap_or("").to_string();
        let moved = e.get_recurrence_id().is_some();
        let proto = proto_event(e, all_day, user.as_deref());

        let starts: Vec<i64> = match e.property_value("RRULE") {
            Some(rule) if !moved => {
                if heavy_rule(rule) || rule_steps(rule, start_ms, window.1) > EVENT_STEPS {
                    continue;
                }
                expand(ev, start_ms, window, &mut budget)
            }
            _ => vec![start_ms],
        };
        for s in starts {
            if s < window.0 || s > window.1 || (!moved && replaced.contains(&(uid.clone(), s))) {
                continue;
            }
            let key = if uid.is_empty() {
                format!("t:{}@{s}", proto.title)
            } else {
                format!("{uid}@{s}")
            };
            out.push(CalEvent {
                key,
                start_ms: s,
                end_ms: s + dur_ms,
                ..proto.clone()
            });
        }
    }
    // The window and the order first, then the caps.
    out.sort_by(|a, b| a.start_ms.cmp(&b.start_ms).then_with(|| a.key.cmp(&b.key)));
    out.dedup_by(|a, b| a.key == b.key);
    out.truncate(MAX_OCCURRENCES);
    Ok(out)
}

/// Lowercase addresses of an event's organizer and attendees.
fn addresses(e: &Event) -> HashSet<String> {
    let strip = |a: &str| {
        let a = a.trim();
        a.strip_prefix("mailto:")
            .or_else(|| a.strip_prefix("MAILTO:"))
            .unwrap_or(a)
            .to_lowercase()
    };
    let mut set: HashSet<String> = e
        .get_attendees()
        .iter()
        .map(|a| strip(&a.cal_address))
        .collect();
    if let Some(p) = e.properties().get("ORGANIZER") {
        set.insert(strip(p.value()));
    }
    set.retain(|a| a.contains('@'));
    set
}

/// The user's own address: the one address in most events' invites, when
/// that is clear (an ICS file does not say who owns it).
fn infer_user(events: &[icalendar::CalendarEvent<'_>]) -> Option<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut with_people = 0usize;
    for e in events {
        let set = addresses(e.event());
        if set.is_empty() {
            continue;
        }
        with_people += 1;
        for a in set {
            *counts.entry(a).or_default() += 1;
        }
    }
    let top = counts.values().copied().max()?;
    let mut best = counts.iter().filter(|(_, n)| **n == top);
    let (addr, _) = best.next()?;
    // Unique, in at least two events and in at least half of them.
    (best.next().is_none() && top >= 2 && top * 2 >= with_people).then(|| addr.clone())
}

/// The event's fields that do not depend on which occurrence it is.
fn proto_event(e: &Event, all_day: bool, user: Option<&str>) -> CalEvent {
    let title: String = e
        .get_summary()
        .unwrap_or("")
        .trim()
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect();
    let mut attendees: Vec<String> = Vec::new();
    let mut add = |name: String, address: &str| {
        let n = name.trim().to_string();
        let folded = ghi_text::fold(&n);
        let is_user = user.is_some_and(|u| address.trim().to_lowercase().ends_with(u));
        if !is_user
            && n.chars().count() >= 2
            && attendees.len() < MAX_ATTENDEES
            && !attendees.iter().any(|a| ghi_text::fold(a) == folded)
        {
            attendees.push(n);
        }
    };
    if let Some(p) = e.properties().get("ORGANIZER") {
        add(
            person_name(p.params().get("CN").map(|c| c.value()), p.value()),
            p.value(),
        );
    }
    for a in e.get_attendees() {
        add(person_name(a.cn.as_deref(), &a.cal_address), &a.cal_address);
    }
    let text = [
        e.get_summary(),
        e.get_location(),
        e.get_description(),
        e.property_value("URL"),
        e.property_value("X-GOOGLE-CONFERENCE"),
        e.property_value("X-MICROSOFT-SKYPETEAMSMEETINGURL"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n");
    CalEvent {
        key: String::new(),
        title,
        start_ms: 0,
        end_ms: 0,
        all_day,
        attendees,
        join_app: join_app(&text).map(str::to_string),
        from_ics: true,
    }
}

/// A display name: the invite's common name, else the address before `@`
/// (a "name" that is itself an address counts as the address). A mailbox that
/// is not a person (sales@, noreply@, …) without a display name has no name.
pub fn person_name(cn: Option<&str>, address: &str) -> String {
    if let Some(cn) = cn
        .map(|c| c.trim().trim_matches('"'))
        .filter(|c| !c.is_empty() && !c.contains('@'))
    {
        return cn.to_string();
    }
    let addr = cn
        .filter(|c| c.contains('@'))
        .map_or(address.trim(), str::trim);
    let addr = addr
        .strip_prefix("mailto:")
        .or_else(|| addr.strip_prefix("MAILTO:"))
        .unwrap_or(addr);
    let local = addr.split('@').next().unwrap_or("");
    if ROLE_NAMES.contains(&local.to_lowercase().as_str()) {
        return String::new();
    }
    local.replace(['.', '_'], " ")
}

/// Sub-daily rules over a long history would cost too much to expand.
fn heavy_rule(rrule: &str) -> bool {
    let r = rrule.to_ascii_uppercase();
    ["FREQ=SECONDLY", "FREQ=MINUTELY", "FREQ=HOURLY"]
        .iter()
        .any(|f| r.contains(f))
}

/// A rough count of the occurrences a rule would produce between its start
/// and `until_ms`: the periods in between times the sizes of its BY… lists
/// (capped by COUNT). Over the budget, the rule is not walked at all.
fn rule_steps(rrule: &str, start_ms: i64, until_ms: i64) -> u64 {
    let (mut period, mut interval, mut count, mut per) = (0i64, 1i64, None::<u64>, 1u64);
    for part in rrule.split(';') {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        match k.to_ascii_uppercase().as_str() {
            "FREQ" => {
                period = match v.to_ascii_uppercase().as_str() {
                    "DAILY" => DAY_MS,
                    "WEEKLY" => 7 * DAY_MS,
                    "MONTHLY" => 28 * DAY_MS,
                    "YEARLY" => 365 * DAY_MS,
                    _ => return u64::MAX,
                }
            }
            "INTERVAL" => interval = v.parse::<i64>().unwrap_or(1).max(1),
            "COUNT" => count = v.parse().ok(),
            "BYHOUR" | "BYMINUTE" | "BYSECOND" | "BYDAY" | "BYMONTHDAY" | "BYMONTH"
            | "BYYEARDAY" | "BYWEEKNO" => {
                per = per.saturating_mul(v.split(',').count().max(1) as u64);
            }
            _ => {}
        }
    }
    if period == 0 {
        return u64::MAX;
    }
    let periods = ((until_ms - start_ms).max(0) / period.saturating_mul(interval)) as u64 + 1;
    let n = periods.saturating_mul(per);
    count.map_or(n, |c| n.min(c))
}

/// Start times of a recurring event inside `window`. The rule is walked by
/// hand with a step budget (per event, and `budget` left for the file) and
/// stops once past the window.
fn expand(
    ev: &icalendar::CalendarEvent<'_>,
    start_ms: i64,
    window: (i64, i64),
    budget: &mut u64,
) -> Vec<i64> {
    let Ok(set) = ev.get_recurrence() else {
        // A rule or zone that cannot be read still has its first occurrence.
        return vec![start_ms];
    };
    let mut out = Vec::new();
    let mut steps = 0u64;
    for d in set.into_iter() {
        steps += 1;
        if steps > EVENT_STEPS || *budget == 0 {
            break;
        }
        *budget -= 1;
        let ms = d.timestamp_millis();
        if ms > window.1 {
            break;
        }
        if ms >= window.0 {
            out.push(ms);
            if out.len() >= MAX_PER_EVENT {
                break;
            }
        }
    }
    out
}

fn duration_ms(e: &Event, start_ms: i64, all_day: bool) -> i64 {
    match e.get_end().and_then(|d| to_ms(&d)) {
        Some(end) if end >= start_ms => end - start_ms,
        _ if all_day => DAY_MS,
        _ => 0,
    }
}

/// A moment as unix ms. A date is local midnight; a floating time and a zone
/// that is not known are local. A time that falls in a daylight-saving gap
/// moves an hour on; one that happens twice is the first.
fn to_ms(d: &DatePerhapsTime) -> Option<i64> {
    match d {
        DatePerhapsTime::Date(date) => resolve(&Local, date.and_hms_opt(0, 0, 0)?),
        DatePerhapsTime::DateTime(CalendarDateTime::Utc(u)) => Some(u.timestamp_millis()),
        DatePerhapsTime::DateTime(CalendarDateTime::Floating(n)) => resolve(&Local, *n),
        DatePerhapsTime::DateTime(CalendarDateTime::WithTimezone { date_time, tzid }) => {
            match tzid.parse::<chrono_tz::Tz>() {
                Ok(tz) => resolve(&tz, *date_time),
                Err(_) => resolve(&Local, *date_time),
            }
        }
    }
}

fn resolve<Z: TimeZone>(tz: &Z, n: NaiveDateTime) -> Option<i64> {
    tz.from_local_datetime(&n)
        .earliest()
        .or_else(|| {
            tz.from_local_datetime(&(n + chrono::Duration::hours(1)))
                .earliest()
        })
        .map(|d| d.timestamp_millis())
}

/// The IANA zone for a Windows zone name; a suffix like " (Mexico)" that has
/// no entry of its own falls back to the name without it.
fn windows_zone(name: &str) -> Option<&'static str> {
    let find = |n: &str| WINDOWS_ZONES.iter().find(|(w, _)| *w == n).map(|(_, i)| *i);
    find(name).or_else(|| name.find(" (").and_then(|i| find(&name[..i])))
}

/// Rewrites Windows zone names in `TZID` parameters to IANA names.
fn map_windows_zones(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("TZID=") {
        let (head, tail) = rest.split_at(i + 5);
        out.push_str(head);
        // The value, quoted or up to the next separator, and the text after it.
        let (orig, value, after) = match tail.strip_prefix('"').and_then(|q| q.find('"')) {
            Some(j) => (&tail[..j + 2], &tail[1..j + 1], &tail[j + 2..]),
            None => {
                let j = tail.find([';', ':', '\r', '\n']).unwrap_or(tail.len());
                (&tail[..j], &tail[..j], &tail[j..])
            }
        };
        out.push_str(windows_zone(value).unwrap_or(orig));
        rest = after;
    }
    out.push_str(rest);
    out
}

/// `zoom`, `teams` or `meet` for a text with such a join link.
pub fn join_app(text: &str) -> Option<&'static str> {
    let t = text.to_ascii_lowercase();
    [
        ("zoom", &["zoom.us/", "zoom.com/", "zoomgov.com/"][..]),
        ("teams", &["teams.microsoft.com/", "teams.live.com/"][..]),
        ("meet", &["meet.google.com/"][..]),
    ]
    .into_iter()
    .find(|(_, hosts)| hosts.iter().any(|h| t.contains(h)))
    .map(|(app, _)| app)
}

/// Worth asking to record: another attendee or a call link. From an ICS file
/// (which may list the user too) one name is not enough without a call link.
pub fn meeting_like(e: &CalEvent) -> bool {
    let people = if e.from_ics { 2 } else { 1 };
    !e.all_day && (e.attendees.len() >= people || e.join_app.is_some())
}

/// The next event starting after `now_ms` (all-day events are not "next").
pub fn next_event(events: &[CalEvent], now_ms: i64) -> Option<&CalEvent> {
    events
        .iter()
        .filter(|e| !e.all_day && e.start_ms > now_ms)
        .min_by_key(|e| e.start_ms)
}

/// The meeting-like event in progress: the one that started last when
/// several overlap; else the soonest one starting within 10 minutes.
pub fn current_event(events: &[CalEvent], now_ms: i64) -> Option<&CalEvent> {
    let like = || events.iter().filter(|e| meeting_like(e));
    like()
        .filter(|e| e.start_ms <= now_ms && now_ms < e.end_ms.max(e.start_ms + MIN_MS))
        .max_by_key(|e| e.start_ms)
        .or_else(|| {
            like()
                .filter(|e| e.start_ms > now_ms && e.start_ms - LEAD_MS <= now_ms)
                .min_by_key(|e| e.start_ms)
        })
}

/// Events to ask about now: starting within [-1 min, +5 min] of `now_ms`,
/// armed (`overrides` else [`meeting_like`]) and not in `asked`.
pub fn due_prompts<'a>(
    events: &'a [CalEvent],
    asked: &HashSet<String>,
    overrides: &HashMap<String, bool>,
    now_ms: i64,
) -> Vec<&'a CalEvent> {
    let mut due: Vec<&CalEvent> = events
        .iter()
        .filter(|e| {
            !e.all_day
                && !asked.contains(&e.key)
                && overrides
                    .get(&e.key)
                    .copied()
                    .unwrap_or_else(|| meeting_like(e))
                && now_ms >= e.start_ms - MIN_MS
                && now_ms <= e.start_ms + 5 * MIN_MS
        })
        .collect();
    due.sort_by_key(|e| e.start_ms);
    due
}

const ONE_ON_ONE: &[&str] = &[
    "1:1",
    "1-1",
    "1 on 1",
    "one on one",
    "one-on-one",
    "1on1",
    "gặp riêng",
];
const INTERVIEW: &[&str] = &["interview", "phỏng vấn"];
const STANDUP: &[&str] = &[
    "standup",
    "stand-up",
    "stand up",
    "daily",
    "scrum",
    "họp giao ban",
];
const LECTURE: &[&str] = &[
    "lecture",
    "workshop",
    "training",
    "class",
    "webinar",
    "seminar",
    "bài giảng",
    "tập huấn",
    "đào tạo",
    "hội thảo",
    "buổi học",
    "lớp học",
];
const SALES: &[&str] = &["sales", "demo", "discovery", "pitch", "bán hàng", "tư vấn"];
const CLIENT: &[&str] = &["client", "customer", "khách hàng"];

/// A notes template id for a title and the other attendees, when nothing was
/// chosen: `one_on_one` for exactly one other person, keyword lists in EN and
/// VN for the rest.
pub fn suggest_template(title: &str, attendees: &[String]) -> Option<&'static str> {
    let t = title.to_lowercase();
    let has = |kws: &[&str]| kws.iter().any(|k| has_phrase(&t, k));
    let by_title = [
        (ONE_ON_ONE, "one_on_one"),
        (INTERVIEW, "interview"),
        (STANDUP, "standup"),
        (LECTURE, "lecture"),
        (SALES, "sales"),
        (CLIENT, "client"),
    ]
    .into_iter()
    .find(|(kws, _)| has(kws))
    .map(|(_, id)| id);
    by_title.or((attendees.len() == 1).then_some("one_on_one"))
}

/// `phrase` in `text` as whole words (a letter or digit on either side
/// means it is part of a longer word).
fn has_phrase(text: &str, phrase: &str) -> bool {
    let word = |c: char| c.is_alphanumeric();
    text.match_indices(phrase).any(|(i, m)| {
        let before = text[..i].chars().next_back();
        let after = text[i + m.len()..].chars().next();
        !before.is_some_and(word) && !after.is_some_and(word)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0)
            .unwrap()
            .timestamp_millis()
    }

    fn window() -> (i64, i64) {
        (utc(2026, 10, 1, 0, 0), utc(2026, 10, 31, 0, 0))
    }

    fn ics(events: &str) -> Vec<u8> {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//t//EN\r\n{events}END:VCALENDAR\r\n")
            .into_bytes()
    }

    fn ev(title: &str, start: i64, attendees: &[&str], join: Option<&str>) -> CalEvent {
        CalEvent {
            key: format!("{title}@{start}"),
            title: title.into(),
            start_ms: start,
            end_ms: start + 30 * MIN_MS,
            all_day: false,
            attendees: attendees.iter().map(|s| s.to_string()).collect(),
            join_app: join.map(str::to_string),
            from_ics: false,
        }
    }

    #[test]
    fn utc_event_with_people_and_link() {
        let f = ics(
            "BEGIN:VEVENT\r\nUID:a1\r\nDTSTART:20261005T030000Z\r\nDTEND:20261005T040000Z\r\n\
SUMMARY:Sprint planning\r\nDESCRIPTION:Join https://acme.zoom.us/j/123\r\n\
ORGANIZER;CN=Linh Tran:mailto:linh@acme.com\r\n\
ATTENDEE;CN=Minh:mailto:minh@acme.com\r\nATTENDEE:mailto:sarah.chen@acme.com\r\n\
ATTENDEE;CN=\"Linh Tran\":mailto:linh@acme.com\r\nEND:VEVENT\r\n",
        );
        let got = parse_ics(&f, window()).unwrap();
        assert_eq!(got.len(), 1);
        let e = &got[0];
        assert_eq!(e.title, "Sprint planning");
        assert_eq!(e.start_ms, utc(2026, 10, 5, 3, 0));
        assert_eq!(e.end_ms - e.start_ms, 60 * MIN_MS);
        assert_eq!(e.join_app.as_deref(), Some("zoom"));
        assert_eq!(e.attendees, ["Linh Tran", "Minh", "sarah chen"]);
        assert!(meeting_like(e));
        assert!(e.key.starts_with("a1@"));
    }

    #[test]
    fn time_zones_iana_windows_and_offsets() {
        // 09:00 in Ho Chi Minh City (UTC+7) is 02:00 UTC.
        let want = utc(2026, 10, 5, 2, 0);
        for tz in ["Asia/Ho_Chi_Minh", "SE Asia Standard Time"] {
            let f = ics(&format!(
                "BEGIN:VEVENT\r\nUID:z\r\nDTSTART;TZID={tz}:20261005T090000\r\nDTEND;TZID={tz}:20261005T100000\r\nSUMMARY:Z\r\nEND:VEVENT\r\n"
            ));
            let got = parse_ics(&f, window()).unwrap();
            assert_eq!(got.len(), 1, "{tz}");
            assert_eq!(got[0].start_ms, want, "{tz}");
        }
        // A quoted Windows name too.
        let f = ics(
            "BEGIN:VEVENT\r\nUID:q\r\nDTSTART;TZID=\"SE Asia Standard Time\":20261005T090000\r\nSUMMARY:Q\r\nEND:VEVENT\r\n",
        );
        assert_eq!(parse_ics(&f, window()).unwrap()[0].start_ms, want);
        // Daylight saving: New York in October is UTC-4.
        let f = ics(
            "BEGIN:VEVENT\r\nUID:ny\r\nDTSTART;TZID=America/New_York:20261005T090000\r\nSUMMARY:NY\r\nEND:VEVENT\r\n",
        );
        assert_eq!(
            parse_ics(&f, window()).unwrap()[0].start_ms,
            utc(2026, 10, 5, 13, 0)
        );
    }

    #[test]
    fn weekly_recurrence_with_exdate_and_moved_occurrence() {
        let f = ics(
            "BEGIN:VEVENT\r\nUID:w\r\nDTSTART;TZID=Asia/Ho_Chi_Minh:20260914T090000\r\n\
DTEND;TZID=Asia/Ho_Chi_Minh:20260914T093000\r\nRRULE:FREQ=WEEKLY;BYDAY=MO\r\n\
EXDATE;TZID=Asia/Ho_Chi_Minh:20261012T090000\r\nSUMMARY:Standup\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:w\r\nRECURRENCE-ID;TZID=Asia/Ho_Chi_Minh:20261019T090000\r\n\
DTSTART;TZID=Asia/Ho_Chi_Minh:20261019T140000\r\nDTEND;TZID=Asia/Ho_Chi_Minh:20261019T143000\r\n\
SUMMARY:Standup (moved)\r\nEND:VEVENT\r\n",
        );
        let got = parse_ics(&f, window()).unwrap();
        let starts: Vec<i64> = got.iter().map(|e| e.start_ms).collect();
        // Oct 5 and 26 by rule; Oct 12 excluded; Oct 19 moved to 14:00.
        assert_eq!(
            starts,
            [
                utc(2026, 10, 5, 2, 0),
                utc(2026, 10, 19, 7, 0),
                utc(2026, 10, 26, 2, 0)
            ]
        );
        assert_eq!(got[1].title, "Standup (moved)");
        let keys: HashSet<_> = got.iter().map(|e| &e.key).collect();
        assert_eq!(keys.len(), 3);
        assert_eq!(got[0].end_ms - got[0].start_ms, 30 * MIN_MS);
    }

    #[test]
    fn recurrence_stops_at_until_and_count() {
        let f = ics(
            "BEGIN:VEVENT\r\nUID:c\r\nDTSTART:20261001T010000Z\r\nRRULE:FREQ=DAILY;COUNT=3\r\nSUMMARY:C\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:u\r\nDTSTART:20260901T010000Z\r\nRRULE:FREQ=DAILY;UNTIL=20261002T010000Z\r\nSUMMARY:U\r\nEND:VEVENT\r\n",
        );
        let got = parse_ics(&f, window()).unwrap();
        assert_eq!(got.iter().filter(|e| e.title == "C").count(), 3);
        assert_eq!(got.iter().filter(|e| e.title == "U").count(), 2);
    }

    #[test]
    fn all_day_cancelled_and_out_of_window() {
        let f = ics(
            "BEGIN:VEVENT\r\nUID:d\r\nDTSTART;VALUE=DATE:20261006\r\nDTEND;VALUE=DATE:20261007\r\nSUMMARY:Holiday\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:x\r\nDTSTART:20261007T030000Z\r\nSTATUS:CANCELLED\r\nSUMMARY:Gone\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:o\r\nDTSTART:20270101T030000Z\r\nSUMMARY:Later\r\nEND:VEVENT\r\n",
        );
        let got = parse_ics(&f, window()).unwrap();
        assert_eq!(got.len(), 1);
        assert!(got[0].all_day);
        assert_eq!(got[0].end_ms - got[0].start_ms, 24 * 60 * MIN_MS);
        assert!(!meeting_like(&got[0]));
        assert!(next_event(&got, utc(2026, 10, 1, 0, 0)).is_none());
    }

    #[test]
    fn sub_daily_rules_are_skipped() {
        let f = ics(
            "BEGIN:VEVENT\r\nUID:s\r\nDTSTART:20000101T010000Z\r\nRRULE:FREQ=SECONDLY\r\nSUMMARY:Spam\r\nEND:VEVENT\r\n",
        );
        assert!(parse_ics(&f, window()).unwrap().is_empty());
    }

    #[test]
    fn malformed_and_huge_files() {
        assert_eq!(parse_ics(b"", window()), Err("icsInvalid".into()));
        assert_eq!(
            parse_ics(b"hello world", window()),
            Err("icsInvalid".into())
        );
        assert_eq!(
            parse_ics(&[0xff, 0xfe, 0x00, 0x01], window()),
            Err("icsInvalid".into())
        );
        let big = vec![b'a'; MAX_ICS_BYTES + 1];
        assert_eq!(parse_ics(&big, window()), Err("icsTooLarge".into()));
        // An event without a start, or with a broken date, is skipped; the rest read.
        let f = ics(
            "BEGIN:VEVENT\r\nUID:n\r\nSUMMARY:No start\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:b\r\nDTSTART:garbage\r\nSUMMARY:Bad\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:g\r\nDTSTART:20261008T030000Z\r\nSUMMARY:Good\r\nEND:VEVENT\r\n",
        );
        let got = parse_ics(&f, window()).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].title, "Good");
    }

    #[test]
    fn many_events_are_bounded() {
        let mut body = String::new();
        for i in 0..3000 {
            body.push_str(&format!(
                "BEGIN:VEVENT\r\nUID:e{i}\r\nDTSTART:20261008T030000Z\r\nSUMMARY:E{i}\r\nEND:VEVENT\r\n"
            ));
        }
        let got = parse_ics(&ics(&body), window()).unwrap();
        assert_eq!(got.len(), MAX_OCCURRENCES);
    }

    #[test]
    fn join_links() {
        assert_eq!(join_app("https://us02web.zoom.us/j/9"), Some("zoom"));
        assert_eq!(
            join_app("https://teams.microsoft.com/l/meetup-join/x"),
            Some("teams")
        );
        assert_eq!(
            join_app("see HTTPS://MEET.GOOGLE.COM/abc-defg"),
            Some("meet")
        );
        assert_eq!(join_app("lunch at the zoo"), None);
        assert_eq!(join_app("we use zoom.us for calls"), None);
        assert_eq!(join_app(""), None);
    }

    #[test]
    fn meeting_like_rules() {
        assert!(meeting_like(&ev("A", 0, &["Linh"], None)));
        assert!(meeting_like(&ev("A", 0, &[], Some("meet"))));
        assert!(!meeting_like(&ev("Lunch", 0, &[], None)));
        let mut all_day = ev("Trip", 0, &["Linh"], None);
        all_day.all_day = true;
        assert!(!meeting_like(&all_day));
    }

    #[test]
    fn next_and_current() {
        let now = 1_000 * MIN_MS;
        let events = vec![
            ev("past", now - 120 * MIN_MS, &["a"], None),
            ev("later", now + 90 * MIN_MS, &["a"], None),
            ev("soon", now + 30 * MIN_MS, &[], None),
        ];
        assert_eq!(next_event(&events, now).unwrap().title, "soon");
        assert!(next_event(&events, now + 100 * MIN_MS).is_none());
        // "soon" is not meeting-like, "later" is 90 min away, "past" ended.
        assert!(current_event(&events, now).is_none());
        // 5 minutes before "later", and inside it.
        assert_eq!(
            current_event(&events, now + 85 * MIN_MS).unwrap().title,
            "later"
        );
        assert_eq!(
            current_event(&events, now + 100 * MIN_MS).unwrap().title,
            "later"
        );
        assert!(current_event(&events, now + 130 * MIN_MS).is_none());
        // The one that started last wins when two overlap.
        let two = vec![
            ev("a", now, &["x"], None),
            ev("b", now + 10 * MIN_MS, &["x"], None),
        ];
        assert_eq!(current_event(&two, now + 12 * MIN_MS).unwrap().title, "b");
    }

    #[test]
    fn prompts_are_asked_once_and_overrides_win() {
        let t = 5_000 * MIN_MS;
        let mut events = vec![
            ev("sync", t, &["Linh"], None),
            ev("lunch", t, &[], None),
            ev("muted", t, &["Linh"], None),
        ];
        let asked = HashSet::new();
        let mut over = HashMap::new();
        over.insert(events[2].key.clone(), false);
        over.insert(events[1].key.clone(), true);
        let keys = |now| -> Vec<String> {
            due_prompts(&events, &asked, &over, now)
                .iter()
                .map(|e| e.title.clone())
                .collect()
        };
        assert_eq!(keys(t), ["sync", "lunch"]);
        // The window is [-1 min, +5 min].
        assert!(keys(t - 2 * MIN_MS).is_empty());
        assert_eq!(keys(t - MIN_MS).len(), 2);
        assert_eq!(keys(t + 5 * MIN_MS).len(), 2);
        assert!(keys(t + 6 * MIN_MS).is_empty());
        // Asked once.
        let mut asked = HashSet::new();
        asked.insert(events[0].key.clone());
        let left: Vec<_> = due_prompts(&events, &asked, &over, t);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].title, "lunch");
        // All-day events are never asked.
        events[0].all_day = true;
        assert!(due_prompts(&events[..1], &HashSet::new(), &HashMap::new(), t).is_empty());
    }

    #[test]
    fn template_suggestions() {
        let one = vec!["Linh".to_string()];
        let none: Vec<String> = Vec::new();
        let three: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        assert_eq!(
            suggest_template("Weekly 1:1 with Linh", &three),
            Some("one_on_one")
        );
        assert_eq!(suggest_template("Catch up", &one), Some("one_on_one"));
        assert_eq!(suggest_template("Catch up", &three), None);
        assert_eq!(suggest_template("Daily standup", &three), Some("standup"));
        assert_eq!(suggest_template("Họp daily team", &three), Some("standup"));
        assert_eq!(
            suggest_template("Phỏng vấn ứng viên", &one),
            Some("interview")
        );
        assert_eq!(
            suggest_template("Interview: backend", &none),
            Some("interview")
        );
        assert_eq!(
            suggest_template("Sales demo for Acme", &three),
            Some("sales")
        );
        assert_eq!(suggest_template("Workshop: Rust", &three), Some("lecture"));
        assert_eq!(
            suggest_template("Đào tạo nhân viên mới", &three),
            Some("lecture")
        );
        assert_eq!(suggest_template("Client review", &three), Some("client"));
        assert_eq!(
            suggest_template("Họp khách hàng ABC", &three),
            Some("client")
        );
        // Whole words only: "classic" is not a class, "dailyplanet" is not daily.
        assert_eq!(suggest_template("Classic rock night", &three), None);
        assert_eq!(suggest_template("Planning", &none), None);
        assert_eq!(suggest_template("", &none), None);
        // Every id exists as a built-in template.
        for id in [
            "one_on_one",
            "interview",
            "standup",
            "lecture",
            "sales",
            "client",
        ] {
            assert!(ghi_llm::template::builtin(id).is_ok(), "{id}");
        }
    }

    #[test]
    fn a_rule_over_the_step_budget_finishes_fast_and_is_skipped() {
        let t = std::time::Instant::now();
        // Every minute of every day since 2000: tens of millions of steps.
        let hours = (0..24).map(|h| h.to_string()).collect::<Vec<_>>().join(",");
        let mins = (0..60).map(|m| m.to_string()).collect::<Vec<_>>().join(",");
        let f = ics(&format!(
            "BEGIN:VEVENT\r\nUID:b\r\nDTSTART:20000101T000000Z\r\nRRULE:FREQ=DAILY;BYHOUR={hours};BYMINUTE={mins}\r\nSUMMARY:Heavy\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:ok\r\nDTSTART:20261008T030000Z\r\nSUMMARY:Fine\r\nEND:VEVENT\r\n"
        ));
        let got = parse_ics(&f, window()).unwrap();
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        assert_eq!(
            got.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            ["Fine"]
        );
        assert!(rule_steps("FREQ=DAILY;COUNT=3", 0, i64::MAX / 4) <= 3);
        assert_eq!(rule_steps("FREQ=SECONDLY", 0, 1), u64::MAX);
    }

    #[test]
    fn the_file_budget_stops_many_long_rules() {
        // 100 daily rules since 2000: each fits its own budget, all together do not.
        let mut body = String::new();
        for i in 0..100 {
            body.push_str(&format!(
                "BEGIN:VEVENT\r\nUID:r{i}\r\nDTSTART:20000101T010000Z\r\nRRULE:FREQ=DAILY\r\nSUMMARY:R{i}\r\nEND:VEVENT\r\n"
            ));
        }
        let t = std::time::Instant::now();
        let got = parse_ics(&ics(&body), window()).unwrap();
        assert!(t.elapsed() < std::time::Duration::from_secs(10));
        assert!(!got.is_empty() && got.len() < 100 * 31);
    }

    #[test]
    fn a_moved_occurrence_with_floating_times_replaces_the_rule_s() {
        let f = ics(
            "BEGIN:VEVENT\r\nUID:fl\r\nDTSTART:20261005T090000\r\nDTEND:20261005T093000\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\nSUMMARY:Floating\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:fl\r\nRECURRENCE-ID:20261012T090000\r\nDTSTART:20261012T150000\r\nDTEND:20261012T153000\r\nSUMMARY:Floating (moved)\r\nEND:VEVENT\r\n",
        );
        let got = parse_ics(&f, window()).unwrap();
        let titles: Vec<&str> = got.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["Floating", "Floating (moved)", "Floating"]);
        // The moved one is 6 hours later than the rule's own would have been.
        let rule_own = got[0].start_ms + 7 * DAY_MS;
        assert_eq!(got[1].start_ms - rule_own, 6 * 60 * MIN_MS);
    }

    #[test]
    fn a_time_in_a_daylight_saving_gap_is_kept() {
        // 02:30 on 2026-03-08 does not exist in New York (clocks jump 02:00 -> 03:00).
        let f = ics(
            "BEGIN:VEVENT\r\nUID:g\r\nDTSTART;TZID=America/New_York:20260308T023000\r\nSUMMARY:Gap\r\nEND:VEVENT\r\n",
        );
        let got = parse_ics(&f, (utc(2026, 3, 1, 0, 0), utc(2026, 3, 31, 0, 0))).unwrap();
        assert_eq!(got.len(), 1);
        // Moved an hour on: 03:30 EDT, which is 07:30 UTC.
        assert_eq!(got[0].start_ms, utc(2026, 3, 8, 7, 30));
        // A time that happens twice (01:30 on 2026-11-01) is the first one, EDT.
        let f = ics(
            "BEGIN:VEVENT\r\nUID:d\r\nDTSTART;TZID=America/New_York:20261101T013000\r\nSUMMARY:Twice\r\nEND:VEVENT\r\n",
        );
        let got = parse_ics(&f, (utc(2026, 11, 1, 0, 0), utc(2026, 11, 30, 0, 0))).unwrap();
        assert_eq!(got[0].start_ms, utc(2026, 11, 1, 5, 30));
    }

    #[test]
    fn windows_zone_names_with_a_suffix() {
        assert_eq!(
            windows_zone("Central Standard Time (Mexico)"),
            Some("America/Mexico_City")
        );
        assert_eq!(
            windows_zone("Central Standard Time"),
            Some("America/Chicago")
        );
        // No entry of its own: the name without the suffix.
        assert_eq!(
            windows_zone("Eastern Standard Time (Foo)"),
            Some("America/New_York")
        );
        assert_eq!(windows_zone("Nowhere Time"), None);
        let f = ics(
            "BEGIN:VEVENT\r\nUID:m\r\nDTSTART;TZID=Central Standard Time (Mexico):20261005T090000\r\nSUMMARY:MX\r\nEND:VEVENT\r\n",
        );
        // Mexico City has been UTC-6 all year since 2022.
        assert_eq!(
            parse_ics(&f, window()).unwrap()[0].start_ms,
            utc(2026, 10, 5, 15, 0)
        );
        // Quoted and unknown values are left as they were.
        let t =
            map_windows_zones("DTSTART;TZID=\"Pacific Standard Time\":1\nX;TZID=\"Nowhere\":2\n");
        assert_eq!(
            t,
            "DTSTART;TZID=America/Los_Angeles:1\nX;TZID=\"Nowhere\":2\n"
        );
    }

    #[test]
    fn the_user_is_inferred_and_role_mailboxes_are_not_people() {
        let one = |uid: &str, day: u32, other: &str| {
            format!(
                "BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:202610{day:02}T030000Z\r\nSUMMARY:{uid}\r\n\
ATTENDEE;CN=Me Myself:mailto:me@acme.com\r\nATTENDEE;CN={other}:mailto:{other}@acme.com\r\n\
ATTENDEE:mailto:sales@acme.com\r\nEND:VEVENT\r\n"
            )
        };
        let f = ics(&format!(
            "{}{}{}",
            one("a", 5, "Linh"),
            one("b", 6, "Minh"),
            one("c", 7, "Sarah")
        ));
        let got = parse_ics(&f, window()).unwrap();
        // "Me Myself" appears in every event, so is the user; sales@ is a mailbox.
        // Sales@ appears everywhere too: tied with the user, so nobody is inferred.
        assert_eq!(got[0].attendees, ["Me Myself", "Linh"]);
        // Without the tie the user goes.
        let one = |uid: &str, day: u32, other: &str| {
            format!(
                "BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:202610{day:02}T030000Z\r\nSUMMARY:{uid}\r\n\
ATTENDEE;CN=Me Myself:mailto:me@acme.com\r\nATTENDEE;CN={other}:mailto:{other}@acme.com\r\nEND:VEVENT\r\n"
            )
        };
        let f = ics(&format!(
            "{}{}{}",
            one("a", 5, "Linh"),
            one("b", 6, "Minh"),
            one("c", 7, "Linh")
        ));
        let got = parse_ics(&f, window()).unwrap();
        assert_eq!(got[1].attendees, ["Minh"]);
        // Linh is in two of three events, me in all: me is the user.
        assert!(
            got.iter()
                .all(|e| !e.attendees.contains(&"Me Myself".to_string()))
        );
        // From a file, one other person without a call link is not a meeting.
        assert!(!meeting_like(&got[0]));
        let mut two = got[0].clone();
        two.attendees.push("Zed".into());
        assert!(meeting_like(&two));
        let mut link = got[0].clone();
        link.join_app = Some("zoom".into());
        assert!(meeting_like(&link));
        assert_eq!(person_name(None, "mailto:Support@acme.com"), "");
        assert_eq!(person_name(None, "mailto:ann.lee@acme.com"), "ann lee");
        assert_eq!(
            person_name(Some("Support Team"), "mailto:support@acme.com"),
            "Support Team"
        );
    }

    #[test]
    fn the_window_and_order_apply_before_the_cap() {
        // 2500 events in file order latest-first: the cap keeps the soonest 2000.
        let mut body = String::new();
        for i in (0..2500).rev() {
            let start = utc(2026, 10, 2, 0, 0) + i as i64 * MIN_MS;
            let d = chrono::DateTime::from_timestamp_millis(start).unwrap();
            body.push_str(&format!(
                "BEGIN:VEVENT\r\nUID:w{i}\r\nDTSTART:{}\r\nSUMMARY:W\r\nEND:VEVENT\r\n",
                d.format("%Y%m%dT%H%M%SZ")
            ));
        }
        let got = parse_ics(&ics(&body), window()).unwrap();
        assert_eq!(got.len(), MAX_OCCURRENCES);
        assert_eq!(got[0].start_ms, utc(2026, 10, 2, 0, 0));
        assert!(got.windows(2).all(|w| w[0].start_ms <= w[1].start_ms));
    }

    #[test]
    fn the_event_in_progress_beats_one_about_to_start() {
        let now = 1_000 * MIN_MS;
        let events = vec![
            ev("early", now - 20 * MIN_MS, &["a"], None),
            ev("soon", now + 5 * MIN_MS, &["a"], None),
        ];
        // "early" is in progress (30 min long), "soon" is only about to start.
        assert_eq!(current_event(&events, now).unwrap().title, "early");
        // Once "early" is over, the one about to start.
        assert_eq!(
            current_event(&events, now + 11 * MIN_MS).unwrap().title,
            "soon"
        );
        let both = vec![
            ev("a", now - 10 * MIN_MS, &["x"], None),
            ev("b", now - 2 * MIN_MS, &["x"], None),
        ];
        assert_eq!(current_event(&both, now).unwrap().title, "b");
    }

    fn store() -> (tempfile::TempDir, ghi_store::store::Store) {
        use ghi_store::keys::{MemoryKeyStore, Protection};
        let tmp = tempfile::tempdir().unwrap();
        let s = ghi_store::store::Store::open(
            tmp.path(),
            std::sync::Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        (tmp, s)
    }

    #[test]
    fn info_round_trips_and_feeds_the_vocabulary() {
        let (_tmp, store) = store();
        let m = store
            .create_meeting(ghi_store::store::NewMeeting::default())
            .unwrap()
            .gid;
        assert_eq!(info(&store, &m), None);
        assert_eq!(
            crate::vocab::meeting_terms(&store, &m).unwrap(),
            crate::vocab::effective_terms(&store).unwrap()
        );
        let i = CalendarInfo {
            event: "a1@5".into(),
            title: "Sprint planning".into(),
            attendees: vec!["Lê Minh Anh".into(), "Sarah".into()],
            calendar: Some("Work".into()),
        };
        store
            .set_calendar_info(&m, Some(&serde_json::to_value(&i).unwrap()))
            .unwrap();
        assert_eq!(info(&store, &m), Some(i));
        store
            .set_setting(
                crate::vocab::TERMS_SETTING,
                &serde_json::json!(["Kubernetes", "sarah"]),
            )
            .unwrap();
        let terms = crate::vocab::meeting_terms(&store, &m).unwrap();
        // Attendees first; a term the attendee already covers is not repeated.
        assert_eq!(terms, ["Lê Minh Anh", "Sarah", "Kubernetes"]);
        // Many attendees: capped, and the user's own terms still come after.
        let crowd = CalendarInfo {
            attendees: (0..60).map(|i| format!("Person {i}")).collect(),
            ..info(&store, &m).unwrap()
        };
        store
            .set_calendar_info(&m, Some(&serde_json::to_value(&crowd).unwrap()))
            .unwrap();
        let terms = crate::vocab::meeting_terms(&store, &m).unwrap();
        assert_eq!(terms.len(), 32);
        assert_eq!(&terms[30..], ["Kubernetes", "sarah"]);
        // A shredded meeting has no readable info.
        store.delete_meeting(&m).unwrap();
        assert_eq!(info(&store, &m), None);
    }
}
