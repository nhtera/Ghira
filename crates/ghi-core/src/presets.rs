// SPDX-License-Identifier: Apache-2.0
//! Source presets for imports (phase 14d, D10): which app a recording came
//! from, a title and date from its file or folder name, and Zoom
//! per-participant grouping. Pure functions over names and numbers.
//!
//! W0-B stub: the signatures are final, the bodies are inert (slice S2).

use std::path::{Path, PathBuf};

/// The values of `meetings.source_app` (ghi-store's list).
pub const SOURCE_APPS: [&str; 5] = ["zoom", "teams", "meet", "plaud", "voice_memos"];

/// A title and start time found in a file's name, folder or tags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TitleDate {
    /// `None` when the name carries no real title ("New Recording 3").
    pub title: Option<String>,
    /// Unix ms.
    pub started_at_ms: Option<i64>,
}

/// The app a file came from, by its path and name (one of [`SOURCE_APPS`]).
pub fn detect_source(_path: &Path) -> Option<&'static str> {
    None
}

/// Title and date: folder/file name patterns first, then the container tags
/// (`tag_title`, `tag_date_ms`), then nothing.
pub fn title_date(_path: &Path, _tag_title: Option<&str>, _tag_date_ms: Option<i64>) -> TitleDate {
    TitleDate::default()
}

/// The participant's name in a Zoom per-participant file name
/// (`audioNguyễnVănAn1123…m4a`), or `None` for `audio_recording_N` and the
/// like.
pub fn zoom_participant(_file_name: &str) -> Option<String> {
    None
}

/// Groups files that are one Zoom recording's participant tracks (same
/// `Audio Record` folder or Zoom name pattern, durations within 2 s). Each
/// group is a list of indexes into `files`; files in no group are left out.
pub fn zoom_group(_files: &[(PathBuf, f64)]) -> Vec<Vec<usize>> {
    Vec::new()
}
