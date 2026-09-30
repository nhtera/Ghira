// SPDX-License-Identifier: Apache-2.0
//! Password-encrypted archives for "export everything" and backups.
//!
//! # Format (version 1)
//!
//! ```text
//! header  "GHIX" | version u8 | m_kib u32 LE | t u32 LE | p u32 LE
//!         | salt [16] | nonce prefix [19]                     (52 bytes)
//! chunk*  len u32 LE | ciphertext                              (STREAM)
//! ```
//!
//! The key is Argon2id(password, salt) with the parameters from the header.
//! The plaintext stream is cut into 64 KiB chunks, each sealed with
//! XChaCha20-Poly1305 using the STREAM construction shared with
//! [`crate::bundle`] (nonce = prefix ‖ chunk index ‖ last flag,
//! aad = label ‖ header ‖ chunk index), so reordering, dropping or truncating
//! chunks fails to authenticate. The plaintext is a simple container:
//!
//! ```text
//! entry*  name_len u16 LE (>0) | name (UTF-8, '/' separated) | kind u8 (0 file, 1 memory)
//!         | data_len u64 LE | data
//! end     name_len = 0
//! ```
//!
//! [`import_archive`] verifies the whole archive before anything appears in
//! the destination directory.

use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::XChaCha20Poly1305;
use rand_core::{OsRng, RngCore};
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

use crate::bundle::{
    STREAM_PREFIX_LEN, stream_cipher, stream_open, stream_seal, sync_dir, sync_file,
};
use crate::rowcrypt::{Dek, TAG_LEN};
use crate::{Result, StoreError};

const MAGIC: &[u8; 4] = b"GHIX";
const VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const HEADER_LEN: usize = 4 + 1 + 12 + SALT_LEN + STREAM_PREFIX_LEN;
const CHUNK: usize = 64 * 1024;
const AAD_LABEL: &[u8] = b"ghix1";
const MAX_NAME: usize = 1024;
const KIND_FILE: u8 = 0;
const KIND_MEMORY: u8 = 1;

/// Import refuses KDF parameters beyond these (a hostile archive must not be
/// able to make us allocate gigabytes).
const MAX_M_KIB: u32 = 256 * 1024;
const MAX_T: u32 = 8;
const MAX_P: u32 = 8;
/// Largest in-memory entry an import will hold.
const MAX_CAPTURE: u64 = 1024 * 1024;

/// Argon2id cost parameters, stored in the archive header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    /// Memory in KiB.
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl Default for KdfParams {
    /// 64 MiB, 3 passes, 1 lane.
    fn default() -> Self {
        KdfParams {
            m_kib: 64 * 1024,
            t: 3,
            p: 1,
        }
    }
}

fn random_tag() -> String {
    let mut tag = [0u8; 8];
    OsRng.fill_bytes(&mut tag);
    tag.iter().map(|b| format!("{b:02x}")).collect()
}

fn derive_key(password: &str, salt: &[u8], kdf: &KdfParams) -> Result<Dek> {
    let params = Params::new(kdf.m_kib, kdf.t, kdf.p, Some(32))
        .map_err(|_| StoreError::Invalid("bad key-derivation parameters".into()))?;
    let pw: Zeroizing<String> = Zeroizing::new(password.nfkc().collect());
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(pw.as_bytes(), salt, out.as_mut())
        .map_err(|_| StoreError::Invalid("key derivation failed".into()))?;
    Ok(Dek::from_bytes(*out))
}

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Archive entry names: relative, '/'-separated, no `.`/`..`/empty parts,
/// no backslash, colon or NUL (blocks traversal and Windows drive tricks),
/// no Windows device names (`CON`, `NUL.txt`, ...) and no part ending in a
/// dot or space (Windows strips those).
fn validate_name(name: &str) -> Result<()> {
    let bad_part = |c: &str| {
        let stem = c
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end()
            .to_ascii_uppercase();
        c.is_empty()
            || c == "."
            || c == ".."
            || c.ends_with(['.', ' '])
            || WINDOWS_RESERVED.contains(&stem.as_str())
    };
    let bad = name.is_empty()
        || name.len() > MAX_NAME
        || name.starts_with('/')
        || name.contains(['\\', ':', '\0'])
        || name.split('/').any(bad_part);
    if bad {
        return Err(StoreError::Invalid("unsafe file name in archive".into()));
    }
    Ok(())
}

enum Source {
    File(PathBuf),
    Memory(Zeroizing<Vec<u8>>),
}

/// One archive entry: a file on disk or bytes in memory.
///
/// In-memory entries never touch the disk in plaintext, on export or on
/// import (see [`Imported::entry`]); the store uses one for the key ring.
pub struct Entry {
    name: String,
    source: Source,
}

impl Entry {
    /// A file to read from `path`, stored as `name`.
    pub fn file(name: impl Into<String>, path: impl Into<PathBuf>) -> Entry {
        Entry {
            name: name.into(),
            source: Source::File(path.into()),
        }
    }

    /// Bytes held in memory (zeroed on drop), stored as `name`.
    pub fn bytes(name: impl Into<String>, data: Vec<u8>) -> Entry {
        Entry {
            name: name.into(),
            source: Source::Memory(Zeroizing::new(data)),
        }
    }
}

struct ChunkWriter<W: Write> {
    out: W,
    cipher: XChaCha20Poly1305,
    prefix: [u8; STREAM_PREFIX_LEN],
    aad: Vec<u8>,
    index: u32,
    buf: Zeroizing<Vec<u8>>,
}

impl<W: Write> ChunkWriter<W> {
    fn emit(&mut self, len: usize, last: bool) -> Result<()> {
        let mut aad = self.aad.clone();
        aad.extend_from_slice(&self.index.to_be_bytes());
        let ct = stream_seal(
            &self.cipher,
            &self.prefix,
            self.index,
            last,
            &aad,
            &self.buf[..len],
        );
        self.out.write_all(&(ct.len() as u32).to_le_bytes())?;
        self.out.write_all(&ct)?;
        self.buf.drain(..len);
        self.index = self
            .index
            .checked_add(1)
            .ok_or_else(|| StoreError::Invalid("archive too large".into()))?;
        Ok(())
    }

    fn write(&mut self, data: &[u8]) -> Result<()> {
        self.buf.extend_from_slice(data);
        // Keep the tail back: the last chunk needs the last-flag nonce.
        while self.buf.len() > CHUNK {
            self.emit(CHUNK, false)?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<W> {
        let n = self.buf.len();
        self.emit(n, true)?;
        Ok(self.out)
    }
}

/// Writes an encrypted archive of `files` (`(name in archive, path on disk)`)
/// to `out` using [`KdfParams::default`]. `out` is replaced atomically.
pub fn export_archive(files: &[(String, PathBuf)], out: &Path, password: &str) -> Result<()> {
    export_archive_with(files, out, password, &KdfParams::default())
}

/// [`export_archive`] with explicit Argon2id parameters.
pub fn export_archive_with(
    files: &[(String, PathBuf)],
    out: &Path,
    password: &str,
    kdf: &KdfParams,
) -> Result<()> {
    let entries: Vec<Entry> = files
        .iter()
        .map(|(n, p)| Entry::file(n.clone(), p.clone()))
        .collect();
    export_entries_with(&entries, out, password, kdf)
}

/// Writes an archive of file and in-memory entries with the default cost.
pub fn export_entries(entries: &[Entry], out: &Path, password: &str) -> Result<()> {
    export_entries_with(entries, out, password, &KdfParams::default())
}

/// [`export_entries`] with explicit Argon2id parameters.
pub fn export_entries_with(
    entries: &[Entry],
    out: &Path,
    password: &str,
    kdf: &KdfParams,
) -> Result<()> {
    for e in entries {
        validate_name(&e.name)?;
    }
    let part = out.with_file_name(format!(".export-{}", random_tag()));
    let result = write_archive(entries, &part, password, kdf).and_then(|()| {
        fs::rename(&part, out)?;
        if let Some(dir) = out.parent() {
            sync_dir(if dir.as_os_str().is_empty() {
                Path::new(".")
            } else {
                dir
            })?;
        }
        Ok(())
    });
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

fn write_archive(files: &[Entry], part: &Path, password: &str, kdf: &KdfParams) -> Result<()> {
    let mut salt = [0u8; SALT_LEN];
    let mut prefix = [0u8; STREAM_PREFIX_LEN];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut prefix);
    let key = derive_key(password, &salt, kdf)?;

    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.push(VERSION);
    for v in [kdf.m_kib, kdf.t, kdf.p] {
        header.extend_from_slice(&v.to_le_bytes());
    }
    header.extend_from_slice(&salt);
    header.extend_from_slice(&prefix);

    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(part)?;
    let mut out = BufWriter::new(file);
    out.write_all(&header)?;
    let mut aad = AAD_LABEL.to_vec();
    aad.extend_from_slice(&header);
    let mut w = ChunkWriter {
        out,
        cipher: stream_cipher(&key),
        prefix,
        aad,
        index: 0,
        buf: Zeroizing::new(Vec::new()),
    };

    let mut block = Zeroizing::new(vec![0u8; CHUNK]);
    for Entry { name, source } in files {
        w.write(&(name.len() as u16).to_le_bytes())?;
        w.write(name.as_bytes())?;
        match source {
            Source::File(path) => {
                let mut f = File::open(path)?;
                let len = f.metadata()?.len();
                w.write(&[KIND_FILE])?;
                w.write(&len.to_le_bytes())?;
                let mut left = len;
                while left > 0 {
                    let n = f.read(&mut block[..left.min(CHUNK as u64) as usize])?;
                    if n == 0 {
                        return Err(StoreError::Invalid("file shrank while exporting".into()));
                    }
                    w.write(&block[..n])?;
                    left -= n as u64;
                }
            }
            Source::Memory(data) => {
                w.write(&[KIND_MEMORY])?;
                w.write(&(data.len() as u64).to_le_bytes())?;
                w.write(data)?;
            }
        }
    }
    w.write(&0u16.to_le_bytes())?;
    let out = w.finish()?;
    let file = out
        .into_inner()
        .map_err(|e| StoreError::Io(e.into_error()))?;
    sync_file(&file, true)?;
    Ok(())
}

struct ChunkReader<R: Read> {
    src: R,
    cipher: XChaCha20Poly1305,
    prefix: [u8; STREAM_PREFIX_LEN],
    aad: Vec<u8>,
    index: u32,
    cur: Zeroizing<Vec<u8>>,
    pos: usize,
    finished: bool,
}

impl<R: Read> ChunkReader<R> {
    /// Loads the next chunk. Reaching end of file before the last-flag chunk
    /// means the archive was truncated.
    fn next_chunk(&mut self) -> Result<()> {
        let mut l = [0u8; 4];
        self.src
            .read_exact(&mut l)
            .map_err(|_| StoreError::Decrypt)?;
        let len = u32::from_le_bytes(l) as usize;
        if !(TAG_LEN..=CHUNK + TAG_LEN).contains(&len) {
            return Err(StoreError::Decrypt);
        }
        let mut ct = vec![0u8; len];
        self.src
            .read_exact(&mut ct)
            .map_err(|_| StoreError::Decrypt)?;
        let mut aad = self.aad.clone();
        aad.extend_from_slice(&self.index.to_be_bytes());
        match stream_open(&self.cipher, &self.prefix, self.index, false, &aad, &ct) {
            Ok(p) => self.cur = Zeroizing::new(p),
            Err(_) => {
                self.cur = Zeroizing::new(stream_open(
                    &self.cipher,
                    &self.prefix,
                    self.index,
                    true,
                    &aad,
                    &ct,
                )?);
                self.finished = true;
                // Nothing may follow the last chunk.
                let mut probe = [0u8; 1];
                if self.src.read(&mut probe)? != 0 {
                    return Err(StoreError::Decrypt);
                }
            }
        }
        self.pos = 0;
        self.index += 1;
        Ok(())
    }

    /// Fills `buf` from the plaintext stream.
    fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<()> {
        while !buf.is_empty() {
            if self.pos == self.cur.len() {
                if self.finished {
                    return Err(StoreError::Invalid("archive is malformed".into()));
                }
                self.next_chunk()?;
                continue;
            }
            let n = buf.len().min(self.cur.len() - self.pos);
            buf[..n].copy_from_slice(&self.cur[self.pos..self.pos + n]);
            self.pos += n;
            buf = &mut buf[n..];
        }
        Ok(())
    }

    /// True once every byte, including the last-flag chunk, was consumed.
    fn at_end(&self) -> bool {
        self.finished && self.pos == self.cur.len()
    }
}

/// Removes the staging directory however the import ends.
struct Staging(PathBuf);

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// What [`import_archive_entries`] produced.
pub struct Imported {
    /// Every entry name in archive order (file and in-memory entries).
    pub names: Vec<String>,
    memory: Vec<(String, Zeroizing<Vec<u8>>)>,
}

impl Imported {
    /// An in-memory entry (see [`Entry::bytes`]) by name. Such entries are
    /// never written to disk.
    pub fn entry(&self, name: &str) -> Option<&[u8]> {
        self.memory
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, d)| d.as_slice())
    }
}

/// Decrypts and verifies `archive`, then writes its files under `dest_dir`.
/// Returns the entry names in archive order (in-memory entries are dropped).
///
/// Nothing is written into `dest_dir` unless the whole archive verifies.
/// Existing files are never overwritten (the import fails instead). A wrong
/// password or damaged archive gives [`StoreError::Decrypt`]; unsafe names
/// give [`StoreError::Invalid`].
pub fn import_archive(archive: &Path, password: &str, dest_dir: &Path) -> Result<Vec<String>> {
    Ok(import_archive_entries(archive, password, dest_dir)?.names)
}

/// [`import_archive`] that also returns the in-memory entries (at most 1 MiB
/// each). They never touch the disk, not even in staging.
pub fn import_archive_entries(archive: &Path, password: &str, dest_dir: &Path) -> Result<Imported> {
    let mut src = BufReader::new(File::open(archive)?);
    let mut header = [0u8; HEADER_LEN];
    src.read_exact(&mut header)
        .map_err(|_| StoreError::Invalid("not a Ghira archive".into()))?;
    if &header[..4] != MAGIC || header[4] != VERSION {
        return Err(StoreError::Invalid("not a Ghira archive".into()));
    }
    let u32_at = |o: usize| u32::from_le_bytes(header[o..o + 4].try_into().expect("4 bytes"));
    let kdf = KdfParams {
        m_kib: u32_at(5),
        t: u32_at(9),
        p: u32_at(13),
    };
    if kdf.m_kib > MAX_M_KIB || kdf.t > MAX_T || kdf.p > MAX_P {
        return Err(StoreError::Invalid(
            "archive key-derivation cost is out of range".into(),
        ));
    }
    let salt = &header[17..17 + SALT_LEN];
    let mut prefix = [0u8; STREAM_PREFIX_LEN];
    prefix.copy_from_slice(&header[17 + SALT_LEN..]);
    let key = derive_key(password, salt, &kdf)?;

    let mut aad = AAD_LABEL.to_vec();
    aad.extend_from_slice(&header);
    let mut r = ChunkReader {
        src,
        cipher: stream_cipher(&key),
        prefix,
        aad,
        index: 0,
        cur: Zeroizing::new(Vec::new()),
        pos: 0,
        finished: false,
    };

    fs::create_dir_all(dest_dir)?;
    let staging = Staging(dest_dir.join(format!(".ghi-import-{}", random_tag())));
    fs::create_dir(&staging.0)?;

    let mut names = Vec::new();
    let mut written = Vec::new();
    let mut memory: Vec<(String, Zeroizing<Vec<u8>>)> = Vec::new();
    let mut block = Zeroizing::new(vec![0u8; CHUNK]);
    loop {
        let mut nl = [0u8; 2];
        r.read_exact(&mut nl)?;
        let name_len = u16::from_le_bytes(nl) as usize;
        if name_len == 0 {
            break;
        }
        let mut name = vec![0u8; name_len];
        r.read_exact(&mut name)?;
        let name = String::from_utf8(name)
            .map_err(|_| StoreError::Invalid("unsafe file name in archive".into()))?;
        validate_name(&name)?;
        let mut kind = [0u8; 1];
        r.read_exact(&mut kind)?;
        let mut dl = [0u8; 8];
        r.read_exact(&mut dl)?;
        let mut left = u64::from_le_bytes(dl);

        match kind[0] {
            KIND_FILE => {}
            KIND_MEMORY => {
                if left > MAX_CAPTURE || memory.iter().any(|(n, _)| *n == name) {
                    return Err(StoreError::Invalid("archive is malformed".into()));
                }
                let mut data = Zeroizing::new(vec![0u8; left as usize]);
                r.read_exact(&mut data)?;
                memory.push((name.clone(), data));
                names.push(name);
                continue;
            }
            _ => return Err(StoreError::Invalid("archive is malformed".into())),
        }
        let target = staging.0.join(&name);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|_| StoreError::Invalid("duplicate file name in archive".into()))?;
        while left > 0 {
            let n = left.min(CHUNK as u64) as usize;
            r.read_exact(&mut block[..n])?;
            f.write_all(&block[..n])?;
            left -= n as u64;
        }
        f.sync_all()?;
        written.push(name.clone());
        names.push(name);
    }
    // Only a clean end (last chunk consumed, nothing after) counts.
    if !r.at_end() {
        return Err(StoreError::Invalid("archive is malformed".into()));
    }

    for name in &written {
        if dest_dir.join(name).exists() {
            return Err(StoreError::Invalid(format!("{name} already exists")));
        }
    }
    let mut moved: Vec<PathBuf> = Vec::new();
    for name in &written {
        let to = dest_dir.join(name);
        let step = (|| -> std::io::Result<()> {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(staging.0.join(name), &to)
        })();
        if let Err(e) = step {
            for p in moved {
                let _ = fs::remove_file(p);
            }
            return Err(e.into());
        }
        moved.push(to);
    }
    Ok(Imported { names, memory })
}

/// Removes staging leftovers of crashed exports/imports in `dir`
/// (`.ghi-import-*` and `.export-*`, files or directories) older than
/// `max_age`. The store calls this on open with a zero age (single process
/// per store). A missing `dir` is fine.
pub fn clean_stale_staging(dir: &Path, max_age: std::time::Duration) -> Result<()> {
    clean_stale(dir, &[".ghi-import-", ".export-"], max_age)
}

/// Removes `.ghi-import-*` leftovers only: for the parent of an import
/// destination, which may hold other things.
pub fn clean_stale_imports(dir: &Path, max_age: std::time::Duration) -> Result<()> {
    clean_stale(dir, &[".ghi-import-"], max_age)
}

fn clean_stale(dir: &Path, prefixes: &[&str], max_age: std::time::Duration) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !prefixes.iter().any(|p| name.starts_with(p)) {
            continue;
        }
        // symlink_metadata: never follow a link out of `dir`.
        let Ok(meta) = entry.path().symlink_metadata() else {
            continue;
        };
        let old = meta
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_none_or(|age| age >= max_age);
        if old {
            let _ = if meta.is_dir() {
                fs::remove_dir_all(entry.path())
            } else {
                fs::remove_file(entry.path())
            };
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An authentic archive that carries a hostile name is refused on import
    /// and nothing lands outside (or inside) the destination.
    #[test]
    fn import_rejects_hostile_names_in_an_authentic_archive() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("f.txt");
        fs::write(&f, b"x").unwrap();
        let fast = KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
        };
        for bad in ["../evil.txt", "/abs.txt", "a/../../evil.txt"] {
            let arch = t.path().join("h.ghx");
            // Bypasses the export-side name check on purpose.
            write_archive(&[Entry::file(bad, f.clone())], &arch, "pw", &fast).unwrap();
            let dest = t.path().join("dest");
            let r = import_archive(&arch, "pw", &dest);
            assert!(matches!(r, Err(StoreError::Invalid(_))), "{bad}");
            assert!(!t.path().join("evil.txt").exists());
            assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
            fs::remove_file(&arch).unwrap();
        }
    }

    #[test]
    fn names() {
        for ok in ["a", "a/b.txt", "meetings/x/audio.ghb", "tiếng-việt.txt"] {
            validate_name(ok).unwrap();
        }
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../b",
            "a//b",
            "./a",
            "a\\b",
            "C:x",
            "a/",
            "a\0b",
        ] {
            assert!(validate_name(bad).is_err(), "{bad:?}");
        }
    }
}
