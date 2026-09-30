// SPDX-License-Identifier: Apache-2.0
//! Encrypted, crash-safe audio bundles: the only audio writer [RT-10].
//!
//! # File format (version 1)
//!
//! ```text
//! header   "GHIB" | version u8 | nonce prefix [19]
//! record*  len u32 LE | ciphertext (len bytes, includes the 16-byte tag)
//! ```
//!
//! Every record is one page (~1 s of audio) sealed with XChaCha20-Poly1305
//! under the meeting's [`Dek`]. This is the STREAM construction:
//! `nonce = prefix(19) ‖ page index u32 BE ‖ last flag u8` and
//! `aad = header ‖ caller aad ‖ page index u32 BE`, where `header` is
//! `magic ‖ version ‖ prefix`. Pages are sealed with the `AUDIO_INFO` subkey of
//! the DEK, and the caller's aad should be the track gid (so a mic page cannot
//! stand in for a system page). The index in the nonce
//! stops pages being swapped or reordered; [`BundleWriter::finish`] appends a
//! final record (empty plaintext, last flag = 1) so a truncated file is
//! detectable. Any page can be decrypted on its own (random access for
//! playback).
//!
//! # Never append to an existing file
//!
//! [`BundleWriter::create`] fails if the file exists and there is no resume
//! API: appending after a crash would reuse (prefix, index) nonces. After
//! [`recover`] truncates a crashed bundle, the next recording goes to a new
//! track file.
//!
//! # Index
//!
//! `<bundle>.ghi` lists every record's offset so [`BundleReader::page`] is O(1).
//! It is advisory: if it is missing or does not match the file, the reader
//! walks the length prefixes instead, and [`recover`] rebuilds it.
//!
//! # Durability
//!
//! [`BundleWriter::append`] only writes; it never syncs. The caller decides
//! when to call [`BundleWriter::sync`] (cheap barrier every few pages, a full
//! flush at checkpoints). After a crash, [`recover`] cuts the file back to
//! the last page that authenticates.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::stream::{NewStream, StreamBE32, StreamPrimitive};
use chacha20poly1305::aead::{KeyInit, Payload};
use rand_core::{OsRng, RngCore};

use crate::rowcrypt::{AUDIO_INFO, Dek, TAG_LEN};
use crate::{Result, StoreError};

const MAGIC: &[u8; 4] = b"GHIB";
const VERSION: u8 = 1;
const PREFIX_LEN: usize = 19;
const HEADER_LEN: u64 = (4 + 1 + PREFIX_LEN) as u64;
/// Largest page plaintext accepted (an hour of 48 kHz f32 mono would not fit;
/// pages are ~1 s, this only bounds corrupt length prefixes).
pub const MAX_PAGE_LEN: usize = 16 * 1024 * 1024;

const INDEX_MAGIC: &[u8; 4] = b"GHII";
const INDEX_VERSION: u8 = 1;

/// The `.ghi` index path that goes with a bundle file.
pub fn index_path(bundle: &Path) -> PathBuf {
    bundle.with_extension("ghi")
}

pub(crate) fn stream_cipher(key: &Dek) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(key.as_bytes().into())
}

/// STREAM (BE32) over XChaCha20-Poly1305: nonce = prefix(19) ‖ position u32 BE
/// ‖ last flag. Shared with [`crate::export`].
fn primitive(
    cipher: &XChaCha20Poly1305,
    prefix: &[u8; PREFIX_LEN],
) -> StreamBE32<XChaCha20Poly1305> {
    StreamBE32::from_aead(cipher.clone(), prefix.into())
}

pub(crate) fn stream_seal(
    cipher: &XChaCha20Poly1305,
    prefix: &[u8; PREFIX_LEN],
    index: u32,
    last: bool,
    aad: &[u8],
    plaintext: &[u8],
) -> Vec<u8> {
    primitive(cipher, prefix)
        .encrypt(
            index,
            last,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("XChaCha20-Poly1305 encryption cannot fail for in-memory input")
}

pub(crate) fn stream_open(
    cipher: &XChaCha20Poly1305,
    prefix: &[u8; PREFIX_LEN],
    index: u32,
    last: bool,
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    primitive(cipher, prefix)
        .decrypt(
            index,
            last,
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| StoreError::Decrypt)
}

/// Length of the nonce prefix shared by the STREAM users in this crate.
pub(crate) const STREAM_PREFIX_LEN: usize = PREFIX_LEN;

/// AAD base: the authenticated header (magic ‖ version ‖ prefix) ‖ the
/// caller's aad (the track gid).
fn header_aad(prefix: &[u8; PREFIX_LEN], caller: &[u8]) -> Vec<u8> {
    let mut a = Vec::with_capacity(5 + PREFIX_LEN + caller.len());
    a.extend_from_slice(MAGIC);
    a.push(VERSION);
    a.extend_from_slice(prefix);
    a.extend_from_slice(caller);
    a
}

fn page_aad(base: &[u8], index: u32) -> Vec<u8> {
    let mut a = Vec::with_capacity(base.len() + 4);
    a.extend_from_slice(base);
    a.extend_from_slice(&index.to_be_bytes());
    a
}

/// Makes a file's data durable. `full`: `F_FULLFSYNC` (survives power loss);
/// otherwise `F_BARRIERFSYNC` (ordered, cheap) on macOS, `fsync` elsewhere
/// (`FlushFileBuffers` on Windows, which `sync_data` uses).
pub(crate) fn sync_file(file: &File, full: bool) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd;
        let cmd = if full {
            libc::F_FULLFSYNC
        } else {
            libc::F_BARRIERFSYNC
        };
        // SAFETY: valid open descriptor; the command takes no argument.
        if unsafe { libc::fcntl(file.as_raw_fd(), cmd) } == 0 {
            return Ok(());
        }
        // Some file systems (network, FAT) lack these commands.
    }
    let _ = full;
    file.sync_data()
}

/// Makes a new directory entry durable.
pub(crate) fn sync_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(buf, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < buf.len() {
            let n = file.seek_read(&mut buf[done..], offset + done as u64)?;
            if n == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            done += n;
        }
        Ok(())
    }
}

fn read_header(file: &File) -> Result<[u8; PREFIX_LEN]> {
    let mut h = [0u8; HEADER_LEN as usize];
    read_exact_at(file, &mut h, 0)
        .map_err(|_| StoreError::Invalid("bundle header missing".into()))?;
    if &h[..4] != MAGIC || h[4] != VERSION {
        return Err(StoreError::Invalid("not a Ghira audio bundle".into()));
    }
    let mut prefix = [0u8; PREFIX_LEN];
    prefix.copy_from_slice(&h[5..]);
    Ok(prefix)
}

/// Walks the length prefixes from the header. Returns each complete record's
/// `(offset, ciphertext length)`; stops at the first partial or absurd one.
fn scan_records(file: &File, file_len: u64) -> Vec<(u64, u32)> {
    let mut out = Vec::new();
    let mut pos = HEADER_LEN;
    while pos + 4 <= file_len {
        let mut l = [0u8; 4];
        if read_exact_at(file, &mut l, pos).is_err() {
            break;
        }
        let len = u32::from_le_bytes(l);
        if (len as usize) < TAG_LEN || len as usize > MAX_PAGE_LEN + TAG_LEN {
            break;
        }
        if pos + 4 + u64::from(len) > file_len {
            break;
        }
        out.push((pos, len));
        pos += 4 + u64::from(len);
    }
    out
}

fn write_index(bundle: &Path, offsets: &[u64], file_len: u64) -> std::io::Result<()> {
    let mut b = Vec::with_capacity(17 + offsets.len() * 8);
    b.extend_from_slice(INDEX_MAGIC);
    b.push(INDEX_VERSION);
    b.extend_from_slice(&(offsets.len() as u32).to_le_bytes());
    b.extend_from_slice(&file_len.to_le_bytes());
    for o in offsets {
        b.extend_from_slice(&o.to_le_bytes());
    }
    std::fs::write(index_path(bundle), b)
}

/// Loads the sidecar index if it matches the file; `None` means "scan".
fn load_index(bundle: &Path, file: &File, file_len: u64) -> Option<Vec<u64>> {
    let b = std::fs::read(index_path(bundle)).ok()?;
    if b.len() < 17 || &b[..4] != INDEX_MAGIC || b[4] != INDEX_VERSION {
        return None;
    }
    let count = u32::from_le_bytes(b[5..9].try_into().ok()?) as usize;
    let indexed_len = u64::from_le_bytes(b[9..17].try_into().ok()?);
    if indexed_len != file_len || b.len() != 17 + count * 8 {
        return None;
    }
    let offsets: Vec<u64> = b[17..]
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().expect("8-byte chunk")))
        .collect();
    // Cheap sanity check: the records must tile the file exactly.
    match offsets.last() {
        None => (file_len == HEADER_LEN).then_some(offsets),
        Some(&last) => {
            let mut l = [0u8; 4];
            read_exact_at(file, &mut l, last).ok()?;
            let end = last + 4 + u64::from(u32::from_le_bytes(l));
            (offsets[0] == HEADER_LEN && end == file_len && offsets.windows(2).all(|w| w[0] < w[1]))
                .then_some(offsets)
        }
    }
}

/// Appends encrypted pages to a new bundle file.
pub struct BundleWriter {
    file: File,
    path: PathBuf,
    cipher: XChaCha20Poly1305,
    aad: Vec<u8>,
    prefix: [u8; PREFIX_LEN],
    offsets: Vec<u64>,
    pos: u64,
    finished: bool,
    /// Set after any I/O failure: the file state is unknown, so no further
    /// page may be written (a page index must never be reused).
    poisoned: AtomicBool,
}

impl BundleWriter {
    /// Creates `path` (fails if it exists) and makes the header and the
    /// directory entry durable. `aad` binds the pages to their owner
    /// (e.g. meeting gid + track kind); [`BundleReader`] needs the same value.
    pub fn create(path: &Path, key: &Dek, aad: &[u8]) -> Result<Self> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let mut prefix = [0u8; PREFIX_LEN];
        OsRng.fill_bytes(&mut prefix);
        let mut header = Vec::with_capacity(HEADER_LEN as usize);
        header.extend_from_slice(MAGIC);
        header.push(VERSION);
        header.extend_from_slice(&prefix);
        file.write_all(&header)?;
        sync_file(&file, true)?;
        if let Some(dir) = path.parent() {
            sync_dir(if dir.as_os_str().is_empty() {
                Path::new(".")
            } else {
                dir
            })?;
        }
        Ok(BundleWriter {
            file,
            path: path.to_path_buf(),
            cipher: stream_cipher(&key.subkey(AUDIO_INFO)),
            aad: header_aad(&prefix, aad),
            prefix,
            offsets: Vec::new(),
            pos: HEADER_LEN,
            finished: false,
            poisoned: AtomicBool::new(false),
        })
    }

    fn poison(&self) {
        self.poisoned.store(true, Ordering::Relaxed);
    }

    fn check(&self) -> Result<()> {
        if self.poisoned.load(Ordering::Relaxed) {
            return Err(StoreError::Io(std::io::Error::other(
                "bundle writer failed earlier and is closed; start a new track file",
            )));
        }
        Ok(())
    }

    fn write_record(&mut self, index: u32, last: bool, plaintext: &[u8]) -> Result<()> {
        let ct = stream_seal(
            &self.cipher,
            &self.prefix,
            index,
            last,
            &page_aad(&self.aad, index),
            plaintext,
        );
        let mut rec = Vec::with_capacity(4 + ct.len());
        rec.extend_from_slice(&(ct.len() as u32).to_le_bytes());
        rec.extend_from_slice(&ct);
        if let Err(e) = self.file.write_all(&rec) {
            // Half a record may be on disk. Cut it off if we can, but never
            // write again: the index could be reused with different data.
            self.poison();
            let _ = self.file.set_len(self.pos);
            return Err(e.into());
        }
        self.offsets.push(self.pos);
        self.pos += rec.len() as u64;
        Ok(())
    }

    /// Seals and writes one page. Does not sync. Returns the page's index.
    pub fn append(&mut self, page: &[u8]) -> Result<u32> {
        self.check()?;
        if self.finished {
            return Err(StoreError::Invalid("bundle already finished".into()));
        }
        if page.len() > MAX_PAGE_LEN {
            return Err(StoreError::Invalid("audio page too large".into()));
        }
        let index = u32::try_from(self.offsets.len())
            .map_err(|_| StoreError::Invalid("too many pages".into()))?;
        if index == u32::MAX {
            return Err(StoreError::Invalid("too many pages".into()));
        }
        self.write_record(index, false, page)?;
        Ok(index)
    }

    /// Flushes to disk: `durable = false` is an ordering barrier
    /// (`F_BARRIERFSYNC` on macOS), `true` survives power loss
    /// (`F_FULLFSYNC`).
    pub fn sync(&self, durable: bool) -> Result<()> {
        self.check()?;
        // After a failed fsync the kernel may have dropped the dirty pages:
        // treat the file as unknown.
        sync_file(&self.file, durable).inspect_err(|_| self.poison())?;
        Ok(())
    }

    /// Writes the final record, the index, and a durable sync. Returns the
    /// number of audio pages. Calling it again is a no-op.
    pub fn finish(&mut self) -> Result<u32> {
        if self.finished {
            return Ok(self.offsets.len() as u32 - 1);
        }
        self.check()?;
        let pages = self.offsets.len() as u32;
        self.write_record(pages, true, &[])?;
        sync_file(&self.file, true).inspect_err(|_| self.poison())?;
        self.finished = true;
        // The index is advisory; failing to write it is not an error.
        let _ = write_index(&self.path, &self.offsets, self.pos);
        Ok(pages)
    }

    /// Pages appended so far (excluding the final record).
    pub fn page_count(&self) -> u32 {
        self.offsets.len() as u32 - u32::from(self.finished)
    }
}

/// Random-access reader over a bundle.
pub struct BundleReader {
    file: File,
    cipher: XChaCha20Poly1305,
    aad: Vec<u8>,
    prefix: [u8; PREFIX_LEN],
    offsets: Vec<u64>,
    pages: u32,
    complete: bool,
}

impl BundleReader {
    /// Opens a bundle and checks whether its final record is present and
    /// authentic. Pages are authenticated when read.
    pub fn open(path: &Path, key: &Dek, aad: &[u8]) -> Result<Self> {
        let file = File::open(path)?;
        let file_len = file.metadata()?.len();
        let prefix = read_header(&file)?;
        let offsets = match load_index(path, &file, file_len) {
            Some(o) => o,
            None => scan_records(&file, file_len)
                .into_iter()
                .map(|(o, _)| o)
                .collect(),
        };
        let mut r = BundleReader {
            file,
            cipher: stream_cipher(&key.subkey(AUDIO_INFO)),
            aad: header_aad(&prefix, aad),
            prefix,
            pages: offsets.len() as u32,
            offsets,
            complete: false,
        };
        if let Some(last) = r.offsets.len().checked_sub(1)
            && r.read_record(last, true).is_ok()
        {
            r.complete = true;
            r.pages -= 1;
        }
        Ok(r)
    }

    fn read_record(&self, i: usize, last: bool) -> Result<Vec<u8>> {
        let off = self.offsets[i];
        let mut l = [0u8; 4];
        read_exact_at(&self.file, &mut l, off)?;
        let len = u32::from_le_bytes(l) as usize;
        if !(TAG_LEN..=MAX_PAGE_LEN + TAG_LEN).contains(&len) {
            return Err(StoreError::Decrypt);
        }
        let mut ct = vec![0u8; len];
        read_exact_at(&self.file, &mut ct, off + 4).map_err(|_| StoreError::Decrypt)?;
        let index = i as u32;
        stream_open(
            &self.cipher,
            &self.prefix,
            index,
            last,
            &page_aad(&self.aad, index),
            &ct,
        )
    }

    /// Number of audio pages (excluding the final record).
    pub fn page_count(&self) -> u32 {
        self.pages
    }

    /// True if the final record is present and authentic: the recording
    /// ended cleanly and nothing was cut off.
    pub fn complete(&self) -> bool {
        self.complete
    }

    /// Decrypts page `i`. Errors with [`StoreError::Decrypt`] if the key,
    /// AAD, position or bytes are wrong.
    pub fn page(&self, i: u32) -> Result<Vec<u8>> {
        if i >= self.pages {
            return Err(StoreError::Invalid(format!("page {i} out of range")));
        }
        self.read_record(i as usize, false)
    }

    /// All pages, concatenated.
    pub fn read_all(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        for i in 0..self.pages {
            out.extend_from_slice(&self.page(i)?);
        }
        Ok(out)
    }
}

/// Outcome of [`recover`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovered {
    /// Authentic audio pages kept.
    pub pages: u32,
    /// The final record was present: the bundle was closed cleanly.
    pub complete: bool,
    /// Bytes cut off the end (a partial or unauthenticated tail).
    pub truncated_bytes: u64,
}

/// Repairs a bundle after a crash: keeps the longest prefix of pages that
/// authenticate, truncates the rest and rebuilds the index. Never keeps a
/// page that fails authentication.
///
/// If the very first record fails to authenticate the key or AAD is most
/// likely wrong, so this returns [`StoreError::Decrypt`] and leaves the file
/// untouched instead of truncating it to nothing.
pub fn recover(path: &Path, key: &Dek, aad: &[u8]) -> Result<Recovered> {
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    let file_len = file.metadata()?.len();
    let prefix = read_header(&file)?;
    let cipher = stream_cipher(&key.subkey(AUDIO_INFO));
    let base_aad = header_aad(&prefix, aad);

    let records = scan_records(&file, file_len);
    let mut offsets = Vec::new();
    let mut good_end = HEADER_LEN;
    let mut pages = 0u32;
    let mut complete = false;
    for (i, &(off, len)) in records.iter().enumerate() {
        let index = i as u32;
        let mut ct = vec![0u8; len as usize];
        read_exact_at(&file, &mut ct, off + 4)?;
        let page_aad = page_aad(&base_aad, index);
        let ok_data = stream_open(&cipher, &prefix, index, false, &page_aad, &ct).is_ok();
        let ok_last =
            !ok_data && stream_open(&cipher, &prefix, index, true, &page_aad, &ct).is_ok();
        if !ok_data && !ok_last {
            if i == 0 {
                return Err(StoreError::Decrypt);
            }
            break;
        }
        offsets.push(off);
        good_end = off + 4 + u64::from(len);
        if ok_last {
            complete = true;
            break;
        }
        pages += 1;
    }

    let truncated_bytes = file_len - good_end;
    if truncated_bytes > 0 {
        file.set_len(good_end)?;
    }
    sync_file(&file, true)?;
    // Best effort: without an index the reader scans.
    if write_index(path, &offsets, good_end).is_err() {
        let _ = std::fs::remove_file(index_path(path));
    }
    Ok(Recovered {
        pages,
        complete,
        truncated_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_write_poisons_the_writer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.ghb");
        let key = Dek::from_bytes([3; 32]);
        let mut w = BundleWriter::create(&path, &key, b"t").unwrap();
        w.append(b"one").unwrap();
        // Inject a failure: a read-only handle makes the next write fail.
        w.file = File::open(&path).unwrap();
        assert!(matches!(w.append(b"two"), Err(StoreError::Io(_))));
        // Nothing more may be written, synced or finished: the index of the
        // failed page must never be reused with different plaintext.
        assert!(w.append(b"three").is_err());
        assert!(w.sync(true).is_err());
        assert!(w.finish().is_err());
        assert!(!w.finished);
        assert_eq!(w.page_count(), 1);
    }
}
