// SPDX-License-Identifier: Apache-2.0
//! Event-only rolling log through the `log` facade. Call sites pass static
//! strings, numbers and enum names; never transcript text, titles, names,
//! user paths or keys.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{LevelFilter, Log, Metadata, Record};

/// Roll when the file would grow past this.
const MAX_BYTES: u64 = 2 * 1024 * 1024;
/// Rolled files kept (`ghira.1.log` ..).
const KEEP: usize = 3;

struct Appender {
    dir: PathBuf,
    file: File,
    size: u64,
}

impl Appender {
    fn open(dir: &Path) -> std::io::Result<Appender> {
        std::fs::create_dir_all(dir)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("ghira.log"))?;
        let size = file.metadata()?.len();
        Ok(Appender {
            dir: dir.to_path_buf(),
            file,
            size,
        })
    }

    fn roll(&mut self) -> std::io::Result<()> {
        for n in (1..KEEP).rev() {
            let from = self.dir.join(format!("ghira.{n}.log"));
            if from.exists() {
                std::fs::rename(&from, self.dir.join(format!("ghira.{}.log", n + 1)))?;
            }
        }
        std::fs::rename(self.dir.join("ghira.log"), self.dir.join("ghira.1.log"))?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join("ghira.log"))?;
        self.size = 0;
        Ok(())
    }

    fn write_line(&mut self, line: &str) {
        if self.size + line.len() as u64 > MAX_BYTES {
            let _ = self.roll();
        }
        if self.file.write_all(line.as_bytes()).is_ok() {
            self.size += line.len() as u64;
        }
    }
}

struct Logger(Mutex<Appender>);

impl Log for Logger {
    fn enabled(&self, m: &Metadata<'_>) -> bool {
        m.level() <= log::Level::Info
    }

    fn log(&self, r: &Record<'_>) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!(
            "{} {:<5} {} {}\n",
            crate::time::utc_iso(),
            r.level(),
            r.target(),
            r.args()
        );
        if let Ok(mut a) = self.0.lock() {
            a.write_line(&line);
        }
    }

    fn flush(&self) {
        if let Ok(mut a) = self.0.lock() {
            let _ = a.file.flush();
        }
    }
}

/// Starts logging to `<dir>/ghira.log` (info and above). Errors when a logger
/// is already installed or the folder is not writable.
pub fn init_log(dir: &Path) -> std::io::Result<()> {
    crate::set_dir(dir);
    let logger = Logger(Mutex::new(Appender::open(dir)?));
    log::set_boxed_logger(Box::new(logger)).map_err(std::io::Error::other)?;
    log::set_max_level(LevelFilter::Info);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolls_at_cap_and_keeps_three() {
        let d = tempfile::tempdir().unwrap();
        let mut a = Appender::open(d.path()).unwrap();
        let line = format!("{}\n", "x".repeat(1023));
        for _ in 0..(2 * 1024 * 6) {
            a.write_line(&line);
        }
        let names: Vec<String> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        for n in ["ghira.log", "ghira.1.log", "ghira.2.log", "ghira.3.log"] {
            assert!(names.iter().any(|x| x == n), "{n} missing in {names:?}");
        }
        assert_eq!(names.len(), 4, "{names:?}");
        for n in &names {
            assert!(std::fs::metadata(d.path().join(n)).unwrap().len() <= MAX_BYTES);
        }
    }
}
