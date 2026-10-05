// SPDX-License-Identifier: Apache-2.0
//! Recording metrics for the owner's device checklists: one JSON line every
//! few seconds, numbers and phase names only (never text, names or paths).
//! The file rotates at [`MAX_BYTES`] and keeps one older generation, so it
//! stays small however long the phone records.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;

/// Rotate when the current file passes this size.
pub const MAX_BYTES: u64 = 512 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub t_s: f64,
    pub phase: String,
    pub recorded_s: f64,
    pub processed_s: f64,
    pub backlog_s: f64,
    /// Audio seconds per wall second during the last catch-up.
    pub catch_up_x: Option<f64>,
    pub rtf: Option<f64>,
    /// Engine steps that ended while the app was not active (their results
    /// were dropped and redone).
    pub steps_while_inactive: u32,
    /// Steps or loads that overlapped the move to the background.
    pub gpu_overlaps: u32,
    pub engine_resets: u32,
    pub thermal: Option<i32>,
    /// Process footprint, MB.
    pub footprint_mb: Option<f64>,
    pub battery: Option<f64>,
    /// What reached the screen's event bus in this window (numbers only).
    pub ui: Option<crate::engine::UiWindow>,
}

pub struct Metrics {
    path: PathBuf,
    lock: Mutex<()>,
}

impl Metrics {
    /// Logs to `dir/live.jsonl` (the directory is created).
    pub fn new(dir: &Path) -> Metrics {
        let _ = fs::create_dir_all(dir);
        Metrics {
            path: dir.join("live.jsonl"),
            lock: Mutex::new(()),
        }
    }

    pub fn log(&self, sample: &Sample) {
        let _ = self.append(sample);
    }

    fn append(&self, sample: &Sample) -> io::Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if fs::metadata(&self.path).is_ok_and(|m| m.len() >= MAX_BYTES) {
            fs::rename(&self.path, self.path.with_extension("jsonl.1"))?;
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let mut line = serde_json::to_vec(sample)?;
        line.push(b'\n');
        f.write_all(&line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_numbers_and_the_file_rotates() {
        let d = std::env::temp_dir().join(format!("ghi-metrics-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let m = Metrics::new(&d);
        let s = Sample {
            phase: "live".into(),
            steps_while_inactive: 2,
            catch_up_x: Some(3.5),
            ..Sample::default()
        };
        m.log(&s);
        let text = fs::read_to_string(d.join("live.jsonl")).unwrap();
        let v: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(v["stepsWhileInactive"], 2);
        assert_eq!(v["catchUpX"], 3.5);
        assert!(
            v.as_object()
                .unwrap()
                .values()
                .all(|x| !x.is_string() || x == "live"),
            "no strings but the phase"
        );
        while fs::metadata(d.join("live.jsonl")).unwrap().len() < MAX_BYTES {
            m.log(&s);
        }
        m.log(&s);
        assert!(d.join("live.jsonl.1").exists(), "rotated");
        let now = fs::read_to_string(d.join("live.jsonl")).unwrap();
        assert_eq!(now.lines().count(), 1, "a fresh file");
        fs::remove_dir_all(d).unwrap();
    }
}
