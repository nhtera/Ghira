// SPDX-License-Identifier: Apache-2.0
//! Source presets for imports (phase 14d, D10): which app a recording came
//! from, a title and date from its file or folder name, and Zoom
//! per-participant grouping. Pure functions over names and numbers.
//!
//! Names are matched after Unicode normalisation (macOS hands out decomposed
//! Vietnamese). A name that is only a date, a counter or an id ("New Recording
//! 3", `audio123456`, `GMT20260703-...`) gives no title: the caller falls back
//! to the file name.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::{Local, NaiveDate, TimeZone};
use regex::Regex;

/// The values of `meetings.source_app` (ghi-store's list).
pub const SOURCE_APPS: [&str; 5] = ["zoom", "teams", "meet", "plaud", "voice_memos"];

/// Per-participant files whose durations differ from the group's by more than
/// this are not part of it (seconds).
const GROUP_TOLERANCE_S: f64 = 2.0;

/// A title and start time found in a file's name, folder or tags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TitleDate {
    /// `None` when the name carries no real title ("New Recording 3").
    pub title: Option<String>,
    /// Unix ms.
    pub started_at_ms: Option<i64>,
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("valid pattern")
}

/// `2026-07-03 14.05.02 <topic> 81234567890`, Zoom's meeting folder.
static ZOOM_FOLDER: LazyLock<Regex> = LazyLock::new(|| {
    re(r"^([0-9]{4})-([0-9]{2})-([0-9]{2}) ([0-9]{2})\.([0-9]{2})\.([0-9]{2}) (.+?) ([0-9]{8,12})$")
});
/// `20260703 140502-1A2B3C4D`, a Voice Memos export.
static VOICE_MEMOS: LazyLock<Regex> = LazyLock::new(|| {
    re(r"^([0-9]{4})([0-9]{2})([0-9]{2}) ([0-9]{2})([0-9]{2})([0-9]{2})-[0-9A-Fa-f]{6,}$")
});
/// `<title> (2026-07-03 14:05 GMT+7)` / `(2026-07-03 at 14:05 GMT-4)`, a Meet download.
static MEET: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(.*?)\s*\(([0-9]{4})-([0-9]{2})-([0-9]{2})(?: at)? ([0-9]{1,2}):([0-9]{2})\s*GMT(?:([+-])([0-9]{1,2})(?::?([0-9]{2}))?)?\)",
    )
});
/// `<title>-20260703_140502-Meeting Recording`, a Teams download.
static TEAMS: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(.*?)[-_ ]+([0-9]{4})([0-9]{2})([0-9]{2})_([0-9]{2})([0-9]{2})([0-9]{2})-Meeting Recording",
    )
});
/// A date in a name: ISO (`2026-07-03`, `2026_07_03`) or compact (`20260703`),
/// optionally followed by a time.
static STEM_DATE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"([0-9]{4})[-_.]?([0-9]{2})[-_.]?([0-9]{2})(?:[ T_-]+([0-9]{2})[.:_-]?([0-9]{2})(?:[.:_-]?([0-9]{2}))?)?",
    )
});

fn num(m: &regex::Captures, i: usize) -> Option<u32> {
    m.get(i)?.as_str().parse().ok()
}

/// Years a recording's name may claim (anything else is not a date).
const YEARS: std::ops::RangeInclusive<i32> = 1990..=2100;

/// A wall-clock time in this machine's time zone as unix ms (`None` for a
/// year outside 1990-2100 or an impossible date).
pub fn local_ms(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Option<i64> {
    if !YEARS.contains(&y) {
        return None;
    }
    let date = NaiveDate::from_ymd_opt(y, mo, d)?.and_hms_opt(h, mi, s)?;
    Local
        .from_local_datetime(&date)
        .earliest()
        .map(|t| t.timestamp_millis())
}

/// A wall-clock time at a fixed UTC offset (in minutes) as unix ms.
fn offset_ms(y: i32, mo: u32, d: u32, h: u32, mi: u32, offset_min: i32) -> Option<i64> {
    if !YEARS.contains(&y) {
        return None;
    }
    let date = NaiveDate::from_ymd_opt(y, mo, d)?.and_hms_opt(h, mi, 0)?;
    Some(date.and_utc().timestamp_millis() - i64::from(offset_min) * 60_000)
}

/// A time a Teams name carries without saying its zone: this machine's local
/// time or UTC, whichever is closer to the file's modification time (a
/// recording is saved soon after it ends); local when the file can't be
/// looked at.
fn local_or_utc(path: &Path, y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Option<i64> {
    let local = local_ms(y, mo, d, h, mi, s);
    let utc = YEARS
        .contains(&y)
        .then(|| NaiveDate::from_ymd_opt(y, mo, d)?.and_hms_opt(h, mi, s))
        .flatten()
        .map(|t| t.and_utc().timestamp_millis());
    let mtime = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64);
    match (local, utc, mtime) {
        (Some(l), Some(u), Some(m)) => Some(if (u - m).abs() < (l - m).abs() { u } else { l }),
        (l, _, _) => l,
    }
}

fn dir_name(p: &Path) -> Option<String> {
    p.file_name().map(|n| ghi_text::nfc(&n.to_string_lossy()))
}

/// The meeting folder of a Zoom file: its folder, or the one above
/// `Audio Record`.
fn zoom_meeting_folder(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    let name = dir_name(parent)?;
    if name.eq_ignore_ascii_case("audio record") {
        dir_name(parent.parent()?)
    } else {
        Some(name)
    }
}

/// Whether `dir` is a Zoom meeting folder by its name
/// (`2026-07-03 14.05.02 <topic> 81234567890`).
pub fn is_zoom_folder(dir: &Path) -> bool {
    dir_name(dir).is_some_and(|n| ZOOM_FOLDER.is_match(&n))
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| ghi_text::nfc(&s.to_string_lossy()))
        .unwrap_or_default()
}

/// The app a file came from, by its path and name (one of [`SOURCE_APPS`]).
pub fn detect_source(path: &Path) -> Option<&'static str> {
    let full = ghi_text::nfc(&path.to_string_lossy()).to_lowercase();
    let stem = file_stem(path);
    let name = stem.to_lowercase();
    if full.contains("voicememos")
        || full.contains("voice memos")
        || name.starts_with("new recording")
        || VOICE_MEMOS.is_match(&stem)
    {
        Some("voice_memos")
    } else if MEET.is_match(&stem) || full.contains("/meet recordings/") {
        Some("meet")
    } else if full.contains("/zoom/")
        || name.starts_with("gmt")
        || name.starts_with("audio_only")
        || (name.starts_with("audio")
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("m4a"))
            && name[5..].starts_with(|c: char| c.is_ascii_digit()))
        || zoom_meeting_folder(path).is_some_and(|f| ZOOM_FOLDER.is_match(&f))
        || path
            .parent()
            .and_then(dir_name)
            .is_some_and(|f| f.eq_ignore_ascii_case("audio record"))
    {
        Some("zoom")
    } else if full.contains("teams") || name.contains("meeting recording") || TEAMS.is_match(&stem)
    {
        Some("teams")
    } else if full.contains("plaud") {
        Some("plaud")
    } else {
        None
    }
}

/// A stem that says nothing about the meeting: only digits and separators, a
/// counter ("New Recording 3"), or an app's id name (`audio123…`, `GMT2026…`).
fn is_placeholder(s: &str) -> bool {
    let t = s.trim().to_lowercase();
    if t.is_empty() || t.chars().all(|c| !c.is_alphabetic()) {
        return true;
    }
    let rest = |p: &str| t.strip_prefix(p).map(str::to_string);
    if rest("new recording").is_some()
        || rest("audio_only").is_some()
        || rest("audio_recording").is_some()
        || rest("recording").is_some_and(|r| r.chars().all(|c| !c.is_alphabetic()))
    {
        return true;
    }
    // Nothing but the apps' own words ("GMT_Recording", "Audio Only").
    const BOILERPLATE: [&str; 11] = [
        "gmt",
        "recording",
        "audio",
        "only",
        "new",
        "voice",
        "memo",
        "memos",
        "plaud",
        "zoom",
        "meeting",
    ];
    if t.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().any(char::is_alphabetic))
        .all(|w| BOILERPLATE.contains(&w))
    {
        return true;
    }
    for p in ["audio", "gmt", "plaud", "voice"] {
        if let Some(r) = rest(p)
            && r.chars()
                .all(|c| c.is_ascii_digit() || "_- .:t".contains(c) || c.is_ascii_hexdigit())
        {
            return true;
        }
    }
    false
}

fn clean_title(s: &str) -> Option<String> {
    let t = s
        .trim()
        .trim_end_matches(" - Recording")
        .trim_matches(|c: char| c.is_whitespace() || "-_.".contains(c))
        .to_string();
    (!is_placeholder(&t)).then_some(t)
}

/// Title and date: folder/file name patterns first, then the container tags
/// (`tag_title`, `tag_date_ms`), then nothing.
pub fn title_date(path: &Path, tag_title: Option<&str>, tag_date_ms: Option<i64>) -> TitleDate {
    let mut out = from_names(path);
    if out.title.is_none() {
        out.title = tag_title.and_then(clean_title);
    }
    if out.started_at_ms.is_none() {
        out.started_at_ms = tag_date_ms;
    }
    out
}

fn from_names(path: &Path) -> TitleDate {
    let stem = file_stem(path);
    // A Zoom meeting folder carries topic and start time.
    if let Some(folder) = zoom_meeting_folder(path)
        && let Some(m) = ZOOM_FOLDER.captures(&folder)
    {
        return TitleDate {
            title: clean_title(&m[7]),
            started_at_ms: local_ms(
                m[1].parse().ok().unwrap_or(0),
                num(&m, 2).unwrap_or(0),
                num(&m, 3).unwrap_or(0),
                num(&m, 4).unwrap_or(0),
                num(&m, 5).unwrap_or(0),
                num(&m, 6).unwrap_or(0),
            ),
        };
    }
    if let Some(m) = VOICE_MEMOS.captures(&stem) {
        return TitleDate {
            title: None,
            started_at_ms: local_ms(
                m[1].parse().ok().unwrap_or(0),
                num(&m, 2).unwrap_or(0),
                num(&m, 3).unwrap_or(0),
                num(&m, 4).unwrap_or(0),
                num(&m, 5).unwrap_or(0),
                num(&m, 6).unwrap_or(0),
            ),
        };
    }
    if let Some(m) = MEET.captures(&stem) {
        let sign = if m.get(7).is_some_and(|s| s.as_str() == "-") {
            -1
        } else {
            1
        };
        let off = sign * (num(&m, 8).unwrap_or(0) as i32 * 60 + num(&m, 9).unwrap_or(0) as i32);
        return TitleDate {
            title: clean_title(&m[1]),
            started_at_ms: offset_ms(
                m[2].parse().ok().unwrap_or(0),
                num(&m, 3).unwrap_or(0),
                num(&m, 4).unwrap_or(0),
                num(&m, 5).unwrap_or(0),
                num(&m, 6).unwrap_or(0),
                off,
            ),
        };
    }
    if let Some(m) = TEAMS.captures(&stem) {
        let (y, mo, d) = (
            m[2].parse().unwrap_or(0),
            num(&m, 3).unwrap_or(0),
            num(&m, 4).unwrap_or(0),
        );
        let (h, mi, sec) = (
            num(&m, 5).unwrap_or(0),
            num(&m, 6).unwrap_or(0),
            num(&m, 7).unwrap_or(0),
        );
        return TitleDate {
            title: clean_title(&m[1]),
            started_at_ms: local_or_utc(path, y, mo, d, h, mi, sec),
        };
    }
    // Any other name: a date inside it, and what is left as the title.
    if let Some(m) = STEM_DATE.captures(&stem) {
        let (y, mo, d) = (
            m[1].parse().unwrap_or(0),
            num(&m, 2).unwrap_or(0),
            num(&m, 3).unwrap_or(0),
        );
        if (1990..=2100).contains(&y) && (1..=12).contains(&mo) && (1..=31).contains(&d) {
            let ms = local_ms(
                y,
                mo,
                d,
                num(&m, 4).unwrap_or(0),
                num(&m, 5).unwrap_or(0),
                num(&m, 6).unwrap_or(0),
            );
            if ms.is_some() {
                let whole = m.get(0).expect("whole match");
                let left = format!("{}{}", &stem[..whole.start()], &stem[whole.end()..]);
                return TitleDate {
                    title: clean_title(&left),
                    started_at_ms: ms,
                };
            }
        }
    }
    TitleDate {
        title: clean_title(&stem),
        started_at_ms: None,
    }
}

/// Splits `NguyễnVănAn` into `Nguyễn Văn An` (a capital after a lowercase
/// letter starts a new word); names that already have spaces are kept.
fn split_camel(s: &str) -> String {
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in s.chars() {
        if c.is_uppercase() && prev.is_some_and(char::is_lowercase) {
            out.push(' ');
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

/// The participant's name in a Zoom per-participant file name
/// (`audioNguyễnVănAn1123…m4a`), or `None` for `audio_recording_N` and the
/// like.
pub fn zoom_participant(file_name: &str) -> Option<String> {
    let name = ghi_text::nfc(file_name);
    let stem = Path::new(&name).file_stem()?.to_string_lossy().into_owned();
    let rest = stem.get(..5).filter(|p| p.eq_ignore_ascii_case("audio"))?;
    let rest = &stem[rest.len()..];
    let name = rest.trim_end_matches(|c: char| c.is_ascii_digit());
    if name.starts_with('_') || name.is_empty() {
        // `audio_only`, `audio_recording_3`, `audio1234567`.
        return None;
    }
    let name = split_camel(name.trim()).trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// Zoom's per-participant file name outside its `Audio Record` folder:
/// `audio<Name><8 or more digits>`.
static PARTICIPANT_NAME: LazyLock<Regex> = LazyLock::new(|| re(r"^audio.+[0-9]{8,}$"));

/// A per-participant file: anything but the mixed `audio_only` inside an
/// `Audio Record` folder; elsewhere only Zoom's own pattern
/// `audio<Name><8+ digits>` (`audio-en.m4a` and `audiobook3.m4a` are not).
fn is_participant_file(path: &Path) -> bool {
    let Some(name) = path
        .file_name()
        .map(|n| ghi_text::nfc(&n.to_string_lossy()))
    else {
        return false;
    };
    let lower = name.to_lowercase();
    if !lower.starts_with("audio") || lower.starts_with("audio_only") {
        return false;
    }
    let in_record = path
        .parent()
        .and_then(dir_name)
        .is_some_and(|f| f.eq_ignore_ascii_case("audio record"));
    let stem = Path::new(&name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    in_record || PARTICIPANT_NAME.is_match(&stem)
}

/// Groups files that are one Zoom recording's participant tracks (same
/// `Audio Record` folder or Zoom name pattern, durations within 2 s). Each
/// group is a list of indexes into `files` (sorted by file name); files in no
/// group are left out. `files` are `(path, duration in seconds)`.
pub fn zoom_group(files: &[(PathBuf, f64)]) -> Vec<Vec<usize>> {
    let mut by_dir: Vec<(PathBuf, Vec<usize>)> = Vec::new();
    for (i, (p, _)) in files.iter().enumerate() {
        if !is_participant_file(p) {
            continue;
        }
        let dir = p.parent().map(Path::to_path_buf).unwrap_or_default();
        match by_dir.iter_mut().find(|(d, _)| *d == dir) {
            Some((_, v)) => v.push(i),
            None => by_dir.push((dir, vec![i])),
        }
    }
    let mut groups = Vec::new();
    for (_, mut idx) in by_dir {
        // The reference length is the median: a stray short file is the odd one.
        let mut lens: Vec<f64> = idx.iter().map(|&i| files[i].1).collect();
        lens.sort_by(f64::total_cmp);
        let median = lens[lens.len() / 2];
        idx.retain(|&i| (files[i].1 - median).abs() <= GROUP_TOLERANCE_S);
        if idx.len() >= 2 {
            idx.sort_by_key(|&i| files[i].0.file_name().map(|n| n.to_os_string()));
            groups.push(idx);
        }
    }
    groups.sort_by_key(|g| g[0]);
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn sources_by_path_and_name() {
        let cases: &[(&str, Option<&str>)] = &[
            (
                "/Users/a/Library/Group Containers/x/Recordings/20260703 140502-1A2B3C4D.m4a",
                Some("voice_memos"),
            ),
            ("/tmp/New Recording 3.m4a", Some("voice_memos")),
            (
                "/Users/a/Documents/Zoom/2026-07-03 14.05.02 Sprint 81234567890/audio_only.m4a",
                Some("zoom"),
            ),
            ("/x/GMT20260703-070502_Recording.mp4", Some("zoom")),
            ("/x/audio1234567890.m4a", Some("zoom")),
            (
                "/x/2026-07-03 14.05.02 Sprint 81234567890/Audio Record/audioLinh1234.m4a",
                Some("zoom"),
            ),
            (
                "/x/Weekly sync (2026-07-03 14:05 GMT+7) - Recording.mp4",
                Some("meet"),
            ),
            ("/x/Standup (2026-07-03 at 09:00 GMT-4).mp4", Some("meet")),
            (
                "/x/Weekly sync-20260703_140502-Meeting Recording.mp4",
                Some("teams"),
            ),
            ("/Users/a/Downloads/Teams/whatever.mp4", Some("teams")),
            ("/Users/a/Plaud/2026-07-03 14:05:02.mp3", Some("plaud")),
            ("/x/random-notes.mp3", None),
        ];
        for (path, want) in cases {
            assert_eq!(detect_source(Path::new(path)), *want, "{path}");
        }
    }

    #[test]
    fn titles_and_dates_from_names() {
        // Zoom: the meeting folder, also from inside Audio Record.
        for f in [
            "/z/2026-07-03 14.05.02 Họp kế hoạch quý 4 81234567890/audio_only.m4a",
            "/z/2026-07-03 14.05.02 Họp kế hoạch quý 4 81234567890/Audio Record/audioLinh1234.m4a",
        ] {
            let t = title_date(Path::new(f), None, None);
            assert_eq!(t.title.as_deref(), Some("Họp kế hoạch quý 4"), "{f}");
            assert_eq!(t.started_at_ms, local_ms(2026, 7, 3, 14, 5, 2));
        }
        // Voice Memos: date only, no title.
        let t = title_date(Path::new("/m/20260703 140502-1A2B3C4D.m4a"), None, None);
        assert_eq!(t.title, None);
        assert_eq!(t.started_at_ms, local_ms(2026, 7, 3, 14, 5, 2));
        // A renamed memo keeps its name.
        let t = title_date(Path::new("/m/Ý tưởng sản phẩm.m4a"), None, None);
        assert_eq!(t.title.as_deref(), Some("Ý tưởng sản phẩm"));
        assert_eq!(t.started_at_ms, None);
        // Meet: title, date at the given offset.
        let t = title_date(
            Path::new("/d/Weekly sync (2026-07-03 14:05 GMT+7) - Recording.mp4"),
            None,
            None,
        );
        assert_eq!(t.title.as_deref(), Some("Weekly sync"));
        assert_eq!(t.started_at_ms, Some(1_783_062_300_000));
        let t = title_date(
            Path::new("/d/Standup (2026-07-03 at 09:00 GMT-4).mp4"),
            None,
            None,
        );
        assert_eq!(t.started_at_ms, Some(1_783_083_600_000));
        // Teams.
        let t = title_date(
            Path::new("/d/Họp khách hàng-20260703_140502-Meeting Recording.mp4"),
            None,
            None,
        );
        assert_eq!(t.title.as_deref(), Some("Họp khách hàng"));
        assert_eq!(t.started_at_ms, local_ms(2026, 7, 3, 14, 5, 2));
        // Any name with a date: the date is taken out of the title.
        let t = title_date(Path::new("/d/Client call 2026-07-03.mp3"), None, None);
        assert_eq!(t.title.as_deref(), Some("Client call"));
        assert_eq!(t.started_at_ms, local_ms(2026, 7, 3, 0, 0, 0));
        let t = title_date(Path::new("/d/2026-07-03 14:05:02.mp3"), None, None);
        assert_eq!(
            (t.title, t.started_at_ms),
            (None, local_ms(2026, 7, 3, 14, 5, 2))
        );
    }

    #[test]
    fn id_like_names_give_no_title() {
        for f in [
            "/x/New Recording 3.m4a",
            "/x/audio1234567890.m4a",
            "/x/GMT20260703-070502_Recording.mp4",
            "/x/20260703.wav",
            "/x/1234.wav",
            "/x/audio_only.m4a",
            "/x/Recording.m4a",
        ] {
            assert_eq!(title_date(Path::new(f), None, None).title, None, "{f}");
        }
    }

    #[test]
    fn tags_fill_what_the_name_lacks() {
        let t = title_date(
            Path::new("/x/New Recording 3.m4a"),
            Some("Họp nhóm"),
            Some(5),
        );
        assert_eq!(t.title.as_deref(), Some("Họp nhóm"));
        assert_eq!(t.started_at_ms, Some(5));
        // The name wins over the tags.
        let t = title_date(
            Path::new("/x/Client call 2026-07-03.mp3"),
            Some("Other"),
            Some(5),
        );
        assert_eq!(t.title.as_deref(), Some("Client call"));
        assert_eq!(t.started_at_ms, local_ms(2026, 7, 3, 0, 0, 0));
        // A placeholder tag is no title.
        assert_eq!(
            title_date(Path::new("/x/audio123.m4a"), Some("New Recording"), None).title,
            None
        );
    }

    #[test]
    fn zoom_participant_names() {
        assert_eq!(
            zoom_participant("audioNguyễnVănAn1123456789.m4a").as_deref(),
            Some("Nguyễn Văn An")
        );
        // macOS hands out decomposed characters.
        assert_eq!(
            zoom_participant("audioNguye\u{302}\u{303}n1123.m4a").as_deref(),
            Some("Nguyễn")
        );
        assert_eq!(
            zoom_participant("audioJohn Smith4567.m4a").as_deref(),
            Some("John Smith")
        );
        assert_eq!(
            zoom_participant("audioLinh1234.m4a").as_deref(),
            Some("Linh")
        );
        for none in [
            "audio_recording_3.m4a",
            "audio_only.m4a",
            "audio1234567.m4a",
            "notes.m4a",
            "audio.m4a",
        ] {
            assert_eq!(zoom_participant(none), None, "{none}");
        }
    }

    #[test]
    fn zoom_groups_by_folder_and_duration() {
        let dir = "/z/2026-07-03 14.05.02 Sprint 81234567890/Audio Record";
        let files = vec![
            (p(&format!("{dir}/audioLinh1234.m4a")), 3600.0),
            (p(&format!("{dir}/audioMinh5678.m4a")), 3600.5),
            (p(&format!("{dir}/audio_recording_3.m4a")), 3599.0),
            (p(&format!("{dir}/audioStray9.m4a")), 600.0),
            (
                p("/z/2026-07-03 14.05.02 Sprint 81234567890/audio_only.m4a"),
                3600.0,
            ),
            (p("/other/audioSolo1.m4a"), 100.0),
            (p("/e/Audio Record/audioA1.m4a"), 50.0),
            (p("/e/Audio Record/audioB2.m4a"), 50.1),
        ];
        let g = zoom_group(&files);
        // Sorted by name inside; the stray (600 s) and the mixed file are out;
        // a lone file is no group.
        assert_eq!(g, vec![vec![0, 1, 2], vec![6, 7]]);
        assert!(zoom_group(&[]).is_empty());
    }

    #[test]
    fn other_audio_files_are_not_zoom_tracks() {
        // Outside an `Audio Record` folder only Zoom's own pattern counts:
        // audio<Name><8+ digits>.
        let dur = 600.0;
        let files: Vec<(PathBuf, f64)> = [
            "/m/audio-en.m4a",
            "/m/audio-vi.m4a",
            "/m/audiobook1.m4a",
            "/m/audiobook2.m4a",
            "/m/audio_only.m4a",
            "/m/audioLinh1111.m4a",
            "/m/audioMinh2222.m4a",
        ]
        .iter()
        .map(|p| (p.into(), dur))
        .collect();
        assert!(zoom_group(&files).is_empty(), "{:?}", zoom_group(&files));
        // The real pattern, with a long id, is a group even without the folder.
        let real: Vec<(PathBuf, f64)> =
            ["/m/audioLinh1234567890.m4a", "/m/audioMinh2345678901.m4a"]
                .iter()
                .map(|p| (p.into(), dur))
                .collect();
        assert_eq!(zoom_group(&real), vec![vec![0, 1]]);
        // A Zoom meeting folder is recognised by its name only.
        assert!(is_zoom_folder(Path::new(
            "/z/2026-07-03 14.05.02 Sprint 81234567890"
        )));
        assert!(!is_zoom_folder(Path::new("/z/Sprint notes")));
        assert!(!is_zoom_folder(Path::new("/z/2026-07-03 Sprint")));
    }

    #[test]
    fn only_ascii_digits_make_a_date_and_years_are_checked() {
        // Full-width digits are not digits here.
        let t = title_date(
            Path::new("/z/２０２６-０７-０３ １４.０５.０２ Sprint 81234567890/a.m4a"),
            None,
            None,
        );
        assert_eq!(t.started_at_ms, None);
        assert_eq!(
            detect_source(Path::new("/x/audio１２３４５６７８９０.m4a")),
            None
        );
        assert_eq!(
            zoom_participant("audioLinh１２３４.m4a").as_deref(),
            Some("Linh１２３４")
        );
        // Years outside 1990-2100, and impossible dates, are no dates anywhere.
        for f in [
            "/x/1850-01-01 notes.mp3",
            "/x/21990101 140502-1A2B3C4D.m4a",
            "/z/1850-07-03 14.05.02 Sprint 81234567890/a.m4a",
            "/x/Weekly sync (1850-07-03 14:05 GMT+7).mp4",
            "/x/Weekly-18500703_140502-Meeting Recording.mp4",
            "/x/2026-13-45 notes.mp3",
        ] {
            assert_eq!(
                title_date(Path::new(f), None, None).started_at_ms,
                None,
                "{f}"
            );
        }
        assert_eq!(local_ms(1850, 1, 1, 0, 0, 0), None);
        assert!(local_ms(2026, 7, 3, 0, 0, 0).is_some());
    }

    /// A Teams name has no zone: it is read as local time or UTC, whichever
    /// lands nearer the file's modification time.
    #[test]
    fn teams_times_are_local_or_utc_whichever_is_nearer_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir
            .path()
            .join("Weekly-20260703_140502-Meeting Recording.mp4");
        std::fs::write(&p, b"x").unwrap();
        let local = local_ms(2026, 7, 3, 14, 5, 2).unwrap();
        let utc = chrono::NaiveDate::from_ymd_opt(2026, 7, 3)
            .unwrap()
            .and_hms_opt(14, 5, 2)
            .unwrap()
            .and_utc()
            .timestamp_millis();
        let set = |t: i64| {
            std::fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_millis(t as u64))
                .unwrap();
        };
        set(utc + 600_000);
        assert_eq!(title_date(&p, None, None).started_at_ms, Some(utc));
        set(local + 600_000);
        assert_eq!(title_date(&p, None, None).started_at_ms, Some(local));
        // A file we can't look at: local.
        let gone = dir
            .path()
            .join("Gone-20260703_140502-Meeting Recording.mp4");
        assert_eq!(title_date(&gone, None, None).started_at_ms, Some(local));
    }
}
