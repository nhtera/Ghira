// SPDX-License-Identifier: Apache-2.0
//! `running.lock`: present while the app runs, removed on a clean exit. A
//! lock found at launch means the previous run did not end cleanly.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

const NAME: &str = "running.lock";

/// Creates the marker; removed by [`RunningMarker::release`] or on drop.
/// Call [`previous_run_crashed`] first.
pub fn running_marker(dir: &Path) -> Option<RunningMarker> {
    std::fs::create_dir_all(dir).ok()?;
    let path = dir.join(NAME);
    std::fs::write(&path, b"").ok()?;
    Some(RunningMarker { path: Some(path) })
}

/// True when a marker from an earlier run is still there.
pub fn previous_run_crashed(dir: &Path) -> bool {
    dir.join(NAME).exists()
}

/// When the stale marker was written (the crashed run's start).
pub fn stale_marker_time(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir.join(NAME)).ok()?.modified().ok()
}

/// Guard for the marker file.
pub struct RunningMarker {
    path: Option<PathBuf>,
}

impl RunningMarker {
    /// Removes the marker now (the app calls this on exit, where drops do
    /// not run because the process ends through `exit`).
    pub fn release(&mut self) {
        if let Some(p) = self.path.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

impl Drop for RunningMarker {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_drop_removes_and_leftover_means_crash() {
        let d = tempfile::tempdir().unwrap();
        assert!(!previous_run_crashed(d.path()));
        let m = running_marker(d.path()).unwrap();
        assert!(previous_run_crashed(d.path()));
        drop(m);
        assert!(!previous_run_crashed(d.path()));
        // Simulated crash: forgotten guard leaves the file.
        std::mem::forget(running_marker(d.path()).unwrap());
        assert!(previous_run_crashed(d.path()));
        assert!(stale_marker_time(d.path()).is_some());
    }
}
