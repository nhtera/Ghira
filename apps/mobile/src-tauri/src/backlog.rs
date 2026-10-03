// SPDX-License-Identifier: Apache-2.0
//! The audio the live engine has not processed yet: an append-only sealed file
//! of 16 kHz mono 16-bit samples, written by the capture pump and read by the
//! engine at its own pace.
//!
//! Every sample goes through it, so "live" is just a backlog that stays near
//! zero; while the phone is locked (no GPU work allowed) or too hot, it grows,
//! and after unlock the engine drains it as fast as it can (catch-up). On
//! disk rather than in memory (an hour is 115 MB), but sealed: each published
//! batch is one ChaCha20-Poly1305 record under a key that exists only in this
//! process's memory, so a stray file is noise (the encrypted bundle in the
//! store is the audio's source of truth). The file is deleted when the
//! recording ends; [`sweep`] removes the ones a crash left behind.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

/// File extension of a backlog, so [`sweep`] never touches anything else.
pub const EXTENSION: &str = "backlog";
/// Publish batches of at least this many samples at a time (100 ms): fewer,
/// bigger records. `publish` with fewer pending samples still works.
pub const MIN_BATCH: usize = 1_600;

/// One sealed batch.
#[derive(Debug, Clone, Copy)]
struct Record {
    /// Index of its first sample in the backlog.
    first: u64,
    samples: u32,
    offset: u64,
    /// Bytes on disk (samples * 2 + the tag).
    len: u32,
}

/// What the writer publishes and the reader follows.
#[derive(Default)]
struct Shared {
    written: AtomicU64,
    records: Mutex<Vec<Record>>,
}

fn nonce(n: u64) -> Nonce {
    let mut b = [0u8; 12];
    b[4..].copy_from_slice(&n.to_le_bytes());
    *Nonce::from_slice(&b)
}

fn cipher_from(key: &Zeroizing<[u8; 32]>) -> ChaCha20Poly1305 {
    ChaCha20Poly1305::new(Key::from_slice(&key[..]))
}

pub struct BacklogWriter {
    file: File,
    cipher: ChaCha20Poly1305,
    pending: Vec<u8>,
    offset: u64,
    /// A write failed: stop (partial writes would misalign the samples).
    failed: bool,
    shared: Arc<Shared>,
}

pub struct BacklogReader {
    file: File,
    cipher: ChaCha20Poly1305,
    read: u64,
    shared: Arc<Shared>,
    /// The record decrypted last (reads go in small steps within one).
    cached: Option<(usize, Vec<i16>)>,
    bytes: Vec<u8>,
}

/// Creates the sealed backlog file at `path` (replacing an old one), under a
/// fresh random key that is dropped with the two ends.
pub fn create(path: &Path) -> io::Result<(BacklogWriter, BacklogReader)> {
    let mut key = Zeroizing::new([0u8; 32]);
    OsRng.fill_bytes(&mut key[..]);
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    let shared = Arc::new(Shared::default());
    Ok((
        BacklogWriter {
            file,
            cipher: cipher_from(&key),
            pending: Vec::new(),
            offset: 0,
            failed: false,
            shared: shared.clone(),
        },
        BacklogReader {
            file: File::open(path)?,
            cipher: cipher_from(&key),
            read: 0,
            shared,
            cached: None,
            bytes: Vec::new(),
        },
    ))
}

/// Deletes the backlogs a crash left in `dir`; returns how many.
pub fn sweep(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == EXTENSION))
        .filter(|p| fs::remove_file(p).is_ok())
        .count()
}

impl BacklogWriter {
    /// Appends samples (not yet visible to the reader). After a write error
    /// the backlog stays as it was (the engine falls behind; the audio in the
    /// store is unaffected).
    pub fn push(&mut self, samples: &[f32]) -> io::Result<()> {
        if self.failed {
            return Ok(());
        }
        self.pending.reserve(samples.len() * 2);
        for &s in samples {
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
            self.pending.extend_from_slice(&v.to_le_bytes());
        }
        Ok(())
    }

    /// Samples pushed and not yet published.
    pub fn pending(&self) -> usize {
        self.pending.len() / 2
    }

    /// Seals the pushed samples into one record and makes them visible.
    pub fn publish(&mut self) -> io::Result<()> {
        if self.pending.is_empty() || self.failed {
            return Ok(());
        }
        let samples = (self.pending.len() / 2) as u32;
        let mut records = self
            .shared
            .records
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let n = records.len() as u64;
        let sealed = self
            .cipher
            .encrypt(&nonce(n), self.pending.as_slice())
            .map_err(|_| io::Error::other("sealing the backlog"))?;
        if let Err(e) = self.file.write_all(&sealed) {
            self.failed = true;
            return Err(e);
        }
        let first = self.shared.written.load(Ordering::Acquire);
        records.push(Record {
            first,
            samples,
            offset: self.offset,
            len: sealed.len() as u32,
        });
        drop(records);
        self.offset += sealed.len() as u64;
        self.pending.clear();
        self.shared
            .written
            .fetch_add(u64::from(samples), Ordering::Release);
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

    /// Moves the read position back to `position` (<= the current one), to
    /// redo audio whose results were dropped.
    pub fn rewind(&mut self, position: u64) {
        self.read = position.min(self.read);
    }

    fn record(&self, idx: usize) -> Record {
        self.shared
            .records
            .lock()
            .unwrap_or_else(|e| e.into_inner())[idx]
    }

    /// The index of the record holding sample `at`.
    fn locate(&self, at: u64) -> usize {
        let records = self
            .shared
            .records
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        records.partition_point(|r| r.first + u64::from(r.samples) <= at)
    }

    fn open(&mut self, idx: usize) -> io::Result<&[i16]> {
        if self.cached.as_ref().is_none_or(|(i, _)| *i != idx) {
            let rec = self.record(idx);
            self.bytes.resize(rec.len as usize, 0);
            self.file.seek(SeekFrom::Start(rec.offset))?;
            self.file.read_exact(&mut self.bytes)?;
            let plain = self
                .cipher
                .decrypt(&nonce(idx as u64), self.bytes.as_slice())
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "the backlog is damaged")
                })?;
            let pcm = plain
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();
            self.cached = Some((idx, pcm));
        }
        Ok(&self.cached.as_ref().expect("just filled").1)
    }

    /// Reads up to `max` samples into `out` (cleared first); returns how many.
    pub fn read(&mut self, max: usize, out: &mut Vec<f32>) -> io::Result<usize> {
        out.clear();
        let n = (self.available() as usize).min(max);
        let mut idx = self.locate(self.read);
        while out.len() < n {
            let rec = self.record(idx);
            let from = (self.read - rec.first) as usize;
            let take = (n - out.len()).min(rec.samples as usize - from);
            let pcm = self.open(idx)?;
            out.extend(
                pcm[from..from + take]
                    .iter()
                    .map(|&v| v as f32 / i16::MAX as f32),
            );
            self.read += take as u64;
            idx += 1;
        }
        Ok(n)
    }
}

/// The path of a recording's backlog in `dir`.
pub fn path_for(dir: &Path, meeting: &str) -> PathBuf {
    dir.join(format!("{meeting}.{EXTENSION}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ghi-backlog-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reader_sees_only_published_samples_in_order() {
        let d = dir("order");
        let (mut w, mut r) = create(&d.join("a.backlog")).unwrap();
        let mut out = Vec::new();
        w.push(&[0.5, -0.5, 0.25]).unwrap();
        assert_eq!(r.available(), 0, "not published yet");
        w.publish().unwrap();
        assert_eq!(r.read(2, &mut out).unwrap(), 2);
        assert!((out[0] - 0.5).abs() < 1e-3 && (out[1] + 0.5).abs() < 1e-3);
        w.push(&[1.5]).unwrap(); // clipped
        w.publish().unwrap();
        assert_eq!(r.read(10, &mut out).unwrap(), 2, "across two records");
        assert!((out[0] - 0.25).abs() < 1e-3 && (out[1] - 1.0).abs() < 1e-3);
        assert_eq!((r.position(), r.available(), w.written()), (4, 0, 4));
        r.rewind(1);
        assert_eq!(r.read(10, &mut out).unwrap(), 3);
        assert!((out[0] + 0.5).abs() < 1e-3, "redone from sample 1");
        r.rewind(99);
        assert_eq!(r.position(), 4, "never forward");
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn rewind_across_a_publish_redoes_the_audio_of_both_records() {
        let d = dir("rewind");
        let (mut w, mut r) = create(&d.join("a.backlog")).unwrap();
        let mut out = Vec::new();
        let first: Vec<f32> = (0..1000).map(|i| i as f32 / 2000.0).collect();
        let second: Vec<f32> = (0..1000).map(|i| -(i as f32) / 2000.0).collect();
        w.push(&first).unwrap();
        w.publish().unwrap();
        w.push(&second).unwrap();
        w.publish().unwrap();
        assert_eq!(r.read(1500, &mut out).unwrap(), 1500);
        r.rewind(900);
        assert_eq!(r.read(2000, &mut out).unwrap(), 1100);
        assert!((out[0] - 900.0 / 2000.0).abs() < 1e-3, "tail of the first");
        assert!((out[100] - 0.0).abs() < 1e-3, "start of the second");
        assert!(
            (out[1099] + 999.0 / 2000.0).abs() < 1e-3,
            "end of the second"
        );
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn the_file_is_sealed_and_damage_is_detected() {
        let d = dir("sealed");
        let path = d.join("a.backlog");
        let (mut w, mut r) = create(&path).unwrap();
        // A loud, recognisable pattern: plaintext would repeat these bytes.
        let pcm = vec![0.5f32; 4000];
        w.push(&pcm).unwrap();
        w.publish().unwrap();
        let on_disk = fs::read(&path).unwrap();
        assert_eq!(on_disk.len(), 8000 + 16, "samples * 2 + the tag");
        let plain = i16::MAX / 2;
        let needle: Vec<u8> = (0..8).flat_map(|_| (plain + 1).to_le_bytes()).collect();
        assert!(
            !on_disk
                .windows(needle.len())
                .any(|w| w == needle.as_slice()),
            "no plaintext run"
        );
        // Another backlog has another key: the same bytes would not open.
        let mut flipped = on_disk.clone();
        flipped[100] ^= 1;
        fs::write(&path, &flipped).unwrap();
        let mut out = Vec::new();
        let err = r.read(100, &mut out).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn sweep_removes_only_backlogs() {
        let d = dir("sweep");
        fs::write(d.join("x.backlog"), b"old").unwrap();
        fs::write(d.join("y.backlog"), b"old").unwrap();
        fs::write(d.join("keep.txt"), b"keep").unwrap();
        assert_eq!(sweep(&d), 2);
        assert!(d.join("keep.txt").exists() && !d.join("x.backlog").exists());
        assert_eq!(sweep(&d.join("missing")), 0);
        fs::remove_dir_all(d).unwrap();
    }
}
