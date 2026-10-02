// SPDX-License-Identifier: Apache-2.0
//! The panic hook that writes a crash report. Release builds use
//! `panic = "abort"`; the hook still runs first.

use std::fmt::Write as _;
use std::panic::PanicHookInfo;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use crate::{scrub, scrub_line, time};

static STARTED: OnceLock<Instant> = OnceLock::new();
/// OS version and machine model, read once at install (not in the hook).
static SYSTEM: OnceLock<(Option<String>, Option<String>)> = OnceLock::new();

/// Installs the hook. `context` is called inside the panic (keep it to an
/// atomic read, e.g. the session state NAME); its result is scrubbed.
/// The previous hook (the default one, printing to stderr) still runs after.
pub fn install_panic_hook(app: &str, dir: PathBuf, context: fn() -> String) {
    crate::set_dir(&dir);
    STARTED.get_or_init(Instant::now);
    SYSTEM.get_or_init(|| (sysctl("kern.osproductversion"), sysctl("hw.model")));
    let app = app.to_owned();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // A failing report must never mask the panic itself.
        let _ = crate::write_report(&dir, &app, &report(&app, info, context));
        previous(info);
    }));
}

fn report(app: &str, info: &PanicHookInfo<'_>, context: fn() -> String) -> String {
    let payload = info.payload();
    let msg = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    let (os, model) = SYSTEM.get().cloned().unwrap_or_default();
    let thread = std::thread::current();
    let mut out = String::new();
    let _ = writeln!(out, "ghira crash report");
    let _ = writeln!(out, "time: {}", time::utc_iso());
    let _ = writeln!(out, "app: {app} {}", env!("CARGO_PKG_VERSION"));
    if let Some(sha) = option_env!("GHI_GIT_SHA") {
        let _ = writeln!(out, "git: {sha}");
    }
    let _ = writeln!(out, "os: macOS {}", os.as_deref().unwrap_or("unknown"));
    let _ = writeln!(out, "model: {}", model.as_deref().unwrap_or("unknown"));
    let _ = writeln!(out, "thread: {}", scrub_line(thread.name().unwrap_or("?")));
    if let Some(l) = info.location() {
        let _ = writeln!(out, "location: {}:{}", l.file(), l.line());
    }
    if let Some(s) = STARTED.get() {
        let _ = writeln!(out, "uptime_s: {}", s.elapsed().as_secs());
    }
    let _ = writeln!(out, "state: {}", scrub_line(&context()));
    let _ = writeln!(out, "message: {}", scrub(&msg));
    let _ = writeln!(
        out,
        "\nbacktrace:\n{}",
        std::backtrace::Backtrace::force_capture()
    );
    out
}

#[cfg(target_os = "macos")]
fn sysctl(name: &str) -> Option<String> {
    use std::ffi::CString;
    let name = CString::new(name).ok()?;
    let mut buf = [0u8; 128];
    let mut len = buf.len();
    // SAFETY: `buf`/`len` describe a valid writable buffer and `name` is a
    // NUL-terminated string; no new value is set.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    let bytes = &buf[..len];
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

#[cfg(not(target_os = "macos"))]
fn sysctl(_name: &str) -> Option<String> {
    None
}
