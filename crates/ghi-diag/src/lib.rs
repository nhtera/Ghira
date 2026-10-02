// SPDX-License-Identifier: Apache-2.0
//! Local diagnostics for the alpha: crash reports, a rolling event log and a
//! "previous run crashed" marker. Everything stays in one folder on this
//! machine; nothing is sent anywhere and there is no third-party SDK.
//!
//! What may reach a file: static strings, numbers and enum names written by
//! our own code, plus the panic message **after** [`scrub`]. Never transcript
//! text, titles, speaker names, paths with user content or keys.

mod log_file;
mod marker;
mod panic_hook;
mod scrub;
mod time;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub use log_file::init_log;
pub use marker::{RunningMarker, previous_run_crashed, running_marker, stale_marker_time};
pub use panic_hook::install_panic_hook;
pub use scrub::{scrub, scrub_line};

/// Env var through which the app tells its sidecars where reports go.
pub const DIR_ENV: &str = "GHI_DIAG_DIR";

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Remembers the diagnostics folder for this process (first call wins).
pub(crate) fn set_dir(dir: &Path) {
    let _ = DIR.set(dir.to_path_buf());
}

/// The folder given to [`install_panic_hook`] / [`init_log`], if any.
pub fn dir() -> Option<&'static Path> {
    DIR.get().map(PathBuf::as_path)
}

/// The folder from [`DIR_ENV`], for a process spawned by the app.
pub fn dir_from_env() -> Option<PathBuf> {
    std::env::var_os(DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Writes `body` as `<dir>/<utc>-<app>.txt` (never overwrites).
pub fn write_report(dir: &Path, app: &str, body: &str) -> std::io::Result<PathBuf> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let stamp = time::utc_stamp();
    for n in 0..100u32 {
        let name = if n == 0 {
            format!("{stamp}-{app}.txt")
        } else {
            format!("{stamp}-{app}-{n}.txt")
        };
        let path = dir.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                f.write_all(body.as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("no free report name"))
}

/// Report for a worker process that died: its exit description (written by
/// our code) and the last stderr lines, each scrubbed.
pub fn write_worker_exit(dir: &Path, status: &str, stderr_lines: &[String]) {
    let mut body = format!(
        "ghira worker report\nversion: {}\nstatus: {}\n\nstderr (last {} lines, scrubbed):\n",
        env!("CARGO_PKG_VERSION"),
        scrub_line(status),
        stderr_lines.len()
    );
    for l in stderr_lines {
        body.push_str(&scrub_line(l));
        body.push('\n');
    }
    let _ = write_report(dir, "worker", &body);
}
