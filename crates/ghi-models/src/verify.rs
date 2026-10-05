// SPDX-License-Identifier: Apache-2.0
//! Hash checks, install status and offline import.
//!
//! A model file is trusted only by its pinned SHA-256 (RT-12): the engines call
//! [`verify_file`] at every load, and [`import_file`] accepts a file only if its
//! hash is a registry entry's.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{Model, path_in, registry};

const BUF: usize = 1024 * 1024;

#[derive(Debug)]
pub enum VerifyError {
    Missing,
    Size { expected: u64, got: u64 },
    Hash,
    Io(String),
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::Missing => f.write_str("file is missing"),
            VerifyError::Size { expected, got } => {
                write!(f, "size is {got} bytes, expected {expected}")
            }
            VerifyError::Hash => f.write_str("SHA-256 does not match the registry"),
            VerifyError::Io(d) => write!(f, "read error: {d}"),
        }
    }
}

impl std::error::Error for VerifyError {}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Streaming SHA-256 of a file (1 MiB reads); also returns its size.
fn hash_file(path: &Path) -> io::Result<(String, u64)> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; BUF];
    let mut total = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok((hex(&hasher.finalize()), total));
        }
        total += n as u64;
        hasher.update(&buf[..n]);
    }
}

/// Checks `path` against the pinned size, then the pinned SHA-256.
pub fn verify_file(path: &Path, m: &Model) -> Result<(), VerifyError> {
    let meta = fs::metadata(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => VerifyError::Missing,
        _ => VerifyError::Io(e.to_string()),
    })?;
    if meta.len() != m.size {
        return Err(VerifyError::Size {
            expected: m.size,
            got: meta.len(),
        });
    }
    let (digest, _) = hash_file(path).map_err(|e| VerifyError::Io(e.to_string()))?;
    if digest.eq_ignore_ascii_case(&m.sha256) {
        Ok(())
    } else {
        Err(VerifyError::Hash)
    }
}

#[derive(Debug)]
pub enum ImportError {
    Io(String),
    /// The file's SHA-256 is not any registry entry's.
    UnknownHash(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Io(d) => write!(f, "file error: {d}"),
            ImportError::UnknownHash(h) => {
                write!(f, "SHA-256 {h} is not a known model; refused")
            }
        }
    }
}

impl std::error::Error for ImportError {}

fn io_err(e: io::Error) -> ImportError {
    ImportError::Io(e.to_string())
}

/// Installs a model from a local file (no network). The file's hash must be a
/// registry entry's; it is copied to `<dir>/<file>.import` and renamed into
/// place only after the copy hashes the same.
pub fn import_file(src: &Path, dir: &Path) -> Result<Model, ImportError> {
    import_from(src, dir, &registry())
}

/// [`import_file`] against an explicit list of known models.
pub fn import_from(src: &Path, dir: &Path, known: &[Model]) -> Result<Model, ImportError> {
    let (digest, size) = hash_file(src).map_err(io_err)?;
    let model = known
        .iter()
        .find(|m| m.size == size && m.sha256.eq_ignore_ascii_case(&digest))
        .ok_or(ImportError::UnknownHash(digest))?
        .clone();
    fs::create_dir_all(dir).map_err(io_err)?;
    let dest = path_in(dir, &model);
    let mut tmp_name = dest.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".import");
    let tmp: PathBuf = dest.with_file_name(tmp_name);
    let copy_digest = copy_hashed(src, &tmp).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io_err(e)
    })?;
    if !copy_digest.eq_ignore_ascii_case(&model.sha256) {
        // The source changed between the two passes.
        let _ = fs::remove_file(&tmp);
        return Err(ImportError::UnknownHash(copy_digest));
    }
    fs::rename(&tmp, &dest).map_err(io_err)?;
    Ok(model)
}

fn copy_hashed(src: &Path, dst: &Path) -> io::Result<String> {
    let mut input = File::open(src)?;
    let mut out = File::create(dst)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; BUF];
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])?;
    }
    out.flush()?;
    Ok(hex(&hasher.finalize()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelStatus {
    pub id: String,
    /// The file is in the directory with the pinned size.
    pub installed: bool,
    /// Full hash check result; `None` when it was not asked for or the file is
    /// not installed.
    pub verified: Option<bool>,
    /// Pinned size in bytes.
    pub size: u64,
}

/// Cheap status of every registry model: presence and size only.
pub fn status(dir: &Path) -> Vec<ModelStatus> {
    status_impl(dir, false)
}

/// Every model in `ids` is in `dir` at its pinned size (an unknown id is
/// not). Cheap, for "can this job run yet"; the hash is checked at load.
pub fn installed(dir: &Path, ids: &[&str]) -> bool {
    ids.iter().all(|id| {
        crate::find(id)
            .is_some_and(|m| fs::metadata(path_in(dir, &m)).is_ok_and(|md| md.len() == m.size))
    })
}

/// Like [`status`], plus a full SHA-256 check of each installed file.
pub fn status_verified(dir: &Path) -> Vec<ModelStatus> {
    status_impl(dir, true)
}

fn status_impl(dir: &Path, verify: bool) -> Vec<ModelStatus> {
    registry()
        .into_iter()
        .map(|m| {
            let path = path_in(dir, &m);
            let installed = fs::metadata(&path).is_ok_and(|md| md.len() == m.size);
            let verified = (verify && installed).then(|| verify_file(&path, &m).is_ok());
            ModelStatus {
                id: m.id,
                installed,
                verified,
                size: m.size,
            }
        })
        .collect()
}

/// Files already verified by this process: (path, size, mtime).
static VERIFIED: std::sync::Mutex<Vec<(std::path::PathBuf, u64, std::time::SystemTime)>> =
    std::sync::Mutex::new(Vec::new());

/// [`verify_file`] before a model is handed to native code (RT-12: a swapped
/// GGUF is parsed by C++), once per process for an unchanged file (same
/// path, size and modification time).
pub fn verify_for_load(path: &Path, m: &Model) -> Result<(), VerifyError> {
    let meta = fs::metadata(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => VerifyError::Missing,
        _ => VerifyError::Io(e.to_string()),
    })?;
    let key = (
        path.to_path_buf(),
        meta.len(),
        meta.modified().unwrap_or(std::time::UNIX_EPOCH),
    );
    if VERIFIED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&key)
    {
        return Ok(());
    }
    verify_file(path, m)?;
    VERIFIED.lock().unwrap_or_else(|e| e.into_inner()).push(key);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DATA: &[u8] = b"pretend this is a gguf";

    fn model() -> Model {
        Model {
            id: "test".into(),
            role: "asr".into(),
            repo: "o/r".into(),
            revision: "0".repeat(40),
            file: "test.gguf".into(),
            sha256: hex(&Sha256::digest(DATA)),
            size: DATA.len() as u64,
            license: "MIT".into(),
            chat_format: None,
            mirrors: vec![],
            optional: false,
        }
    }

    #[test]
    fn verify_accepts_good_and_rejects_bad() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.gguf");
        assert!(matches!(
            verify_file(&p, &model()),
            Err(VerifyError::Missing)
        ));
        fs::write(&p, DATA).unwrap();
        verify_file(&p, &model()).unwrap();
        let mut bad = DATA.to_vec();
        bad[0] ^= 1;
        fs::write(&p, &bad).unwrap();
        assert!(matches!(verify_file(&p, &model()), Err(VerifyError::Hash)));
        fs::write(&p, &DATA[..5]).unwrap();
        assert!(matches!(
            verify_file(&p, &model()),
            Err(VerifyError::Size {
                expected: 22,
                got: 5
            })
        ));
    }

    #[test]
    fn verify_throughput() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.bin");
        let chunk = vec![7u8; BUF];
        let mut f = File::create(&p).unwrap();
        let mut hasher = Sha256::new();
        for _ in 0..64 {
            f.write_all(&chunk).unwrap();
            hasher.update(&chunk);
        }
        drop(f);
        let mut m = model();
        m.size = 64 * BUF as u64;
        m.sha256 = hex(&hasher.finalize());
        let t = std::time::Instant::now();
        verify_file(&p, &m).unwrap();
        let s = t.elapsed().as_secs_f64();
        println!("verify_file: {:.0} MB/s", 64.0 * 1.048576 / s);
    }

    #[test]
    fn import_known_installs_and_unknown_is_refused() {
        let work = tempfile::tempdir().unwrap();
        let src = work.path().join("whatever.bin");
        fs::write(&src, DATA).unwrap();
        let dir = work.path().join("models");
        let m = import_from(&src, &dir, &[model()]).unwrap();
        assert_eq!(m.id, "test");
        verify_file(&dir.join("test.gguf"), &m).unwrap();
        assert!(!dir.join("test.gguf.import").exists());

        let other = work.path().join("other.bin");
        fs::write(&other, b"not a pinned model").unwrap();
        let dir2 = work.path().join("models2");
        let e = import_from(&other, &dir2, &[model()]).unwrap_err();
        assert!(matches!(e, ImportError::UnknownHash(_)));
        assert!(!dir2.exists() || fs::read_dir(&dir2).unwrap().next().is_none());
        // The real registry refuses it too.
        assert!(matches!(
            import_file(&other, &dir2),
            Err(ImportError::UnknownHash(_))
        ));
    }

    #[test]
    fn status_reports_presence_size_and_verification() {
        let dir = tempfile::tempdir().unwrap();
        let all = status(dir.path());
        assert_eq!(all.len(), registry().len());
        assert!(all.iter().all(|s| !s.installed && s.verified.is_none()));
        let m = registry().remove(0);
        // Right name, wrong size: not installed.
        fs::write(path_in(dir.path(), &m), DATA).unwrap();
        let s = status_verified(dir.path());
        assert!(!s[0].installed && s[0].verified.is_none());
    }

    #[test]
    fn installed_needs_every_model_at_its_pinned_size() {
        let dir = tempfile::tempdir().unwrap();
        let [a, b] = [registry().remove(0), registry().remove(1)];
        let ids = [a.id.as_str(), b.id.as_str()];
        assert!(!installed(dir.path(), &ids));
        // Sparse files of the pinned size (the hash is checked at load).
        for m in [&a, &b] {
            File::create(path_in(dir.path(), m))
                .unwrap()
                .set_len(m.size)
                .unwrap();
        }
        assert!(installed(dir.path(), &ids));
        assert!(!installed(dir.path(), &[a.id.as_str(), "no-such-model"]));
        File::create(path_in(dir.path(), &b)).unwrap(); // truncated: a download in progress
        assert!(!installed(dir.path(), &ids));
    }
}
