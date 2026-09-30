// SPDX-License-Identifier: Apache-2.0
//! The audio the live engine has not processed yet: an append-only file of
//! 16 kHz mono 16-bit samples, written by the capture pump and read by the
//! engine at its own pace.
//!
//! Every sample goes through it, so "live" is just a backlog that stays near
//! zero; while the phone is locked (no GPU work allowed) or too hot, it grows,
//! and after unlock the engine drains it as fast as it can (catch-up). On
//! disk rather than in memory: an hour is 115 MB.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Samples written so far (shared by the writer and the reader).
#[derive(Debug, Default)]
struct Shared {
    written: AtomicU64,
}

pub struct BacklogWriter {
    out: BufWriter<File>,
    pending: u64,
    /// A write failed: stop (partial writes would misalign the samples).
    failed: bool,
    shared: Arc<Shared>,
}

pub struct BacklogReader {
    file: File,
    read: u64,
    shared: Arc<Shared>,
    bytes: Vec<u8>,
}

/// Creates the backlog file at `path` (replacing an old one).
pub fn create(path: &Path) -> io::Result<(BacklogWriter, BacklogReader, PathBuf)> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    let file_r = File::open(path)?;
    let shared = Arc::new(Shared::default());
    Ok((
        BacklogWriter {
            out: BufWriter::new(file),
            pending: 0,
            failed: false,
            shared: shared.clone(),
        },
        BacklogReader {
            file: file_r,
            read: 0,
            shared,
            bytes: Vec::new(),
        },
        path.to_path_buf(),
    ))
}

impl BacklogWriter {
    /// Appends samples. After a write error the backlog stays as it was
    /// (the engine falls behind; the audio file is unaffected).
    pub fn push(&mut self, samples: &[f32]) -> io::Result<()> {
        if self.failed {
            return Ok(());
        }
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for &s in samples {
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        if let Err(e) = self.out.write_all(&bytes) {
            self.failed = true;
            return Err(e);
        }
        self.pending += samples.len() as u64;
        Ok(())
    }

    /// Makes the pushed samples visible to the reader.
    pub fn publish(&mut self) -> io::Result<()> {
        if self.pending == 0 || self.failed {
            return Ok(());
        }
        if let Err(e) = self.out.flush() {
            self.failed = true;
            return Err(e);
        }
        self.shared
            .written
            .fetch_add(self.pending, Ordering::Release);
        self.pending = 0;
        Ok(())
    }

    /// Samples written and published.
    pub fn written(&self) -> u64 {
        self.shared.written.load(Ordering::Acquire)
    }
}

impl BacklogReader {
    /// Samples waiting to be read.
    pub fn available(&self) -> u64 {
        self.shared.written.load(Ordering::Acquire) - self.read
    }

    /// Samples read so far.
    #[cfg_attr(not(any(test, feature = "nemo")), allow(dead_code))]
    pub fn position(&self) -> u64 {
        self.read
    }

    /// Moves the read position back to `position` (≤ the current one), to
    /// redo audio whose results were dropped.
    #[cfg_attr(not(any(test, feature = "nemo")), allow(dead_code))]
    pub fn rewind(&mut self, position: u64) {
        self.read = position.min(self.read);
    }

    /// Reads up to `max` samples into `out` (cleared first); returns how many.
    pub fn read(&mut self, max: usize, out: &mut Vec<f32>) -> io::Result<usize> {
        out.clear();
        let n = (self.available() as usize).min(max);
        if n == 0 {
            return Ok(0);
        }
        self.bytes.resize(n * 2, 0);
        self.file.seek(SeekFrom::Start(self.read * 2))?;
        self.file.read_exact(&mut self.bytes)?;
        out.extend(
            self.bytes
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / i16::MAX as f32),
        );
        self.read += n as u64;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_sees_only_published_samples_in_order() {
        let dir = std::env::temp_dir().join(format!("ghi-backlog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (mut w, mut r, path) = create(&dir.join("backlog.pcm")).unwrap();
        let mut out = Vec::new();
        w.push(&[0.5, -0.5, 0.25]).unwrap();
        assert_eq!(r.available(), 0, "not published yet");
        w.publish().unwrap();
        assert_eq!(r.read(2, &mut out).unwrap(), 2);
        assert!((out[0] - 0.5).abs() < 1e-3 && (out[1] + 0.5).abs() < 1e-3);
        w.push(&[1.5]).unwrap(); // clipped
        w.publish().unwrap();
        assert_eq!(r.read(10, &mut out).unwrap(), 2);
        assert!((out[0] - 0.25).abs() < 1e-3 && (out[1] - 1.0).abs() < 1e-3);
        assert_eq!((r.position(), r.available(), w.written()), (4, 0, 4));
        r.rewind(1);
        assert_eq!(r.read(10, &mut out).unwrap(), 3);
        assert!((out[0] + 0.5).abs() < 1e-3, "redone from sample 1");
        r.rewind(99);
        assert_eq!(r.position(), 4, "never forward");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
