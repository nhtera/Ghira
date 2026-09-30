// SPDX-License-Identifier: Apache-2.0
//! Debug-only key store: the master key in a plain file. For tests and dev
//! runs of `ghi`. Compiled only with `debug_assertions`; the release check
//! (`tools/scripts/check-no-dev-key.sh`) fails if its marker is in a binary.

use std::path::PathBuf;

use super::{KeyRing, KeyStore, Protection};
use crate::StoreError;

/// Present in the binary only if this module is compiled in.
pub const MARKER: &str = "GHIRA-DEV-FILE-KEYSTORE-DO-NOT-SHIP";

pub struct FileKeyStore {
    path: PathBuf,
}

impl FileKeyStore {
    pub fn new(path: impl Into<PathBuf>) -> FileKeyStore {
        FileKeyStore { path: path.into() }
    }
}

impl KeyStore for FileKeyStore {
    fn load(&self) -> Result<Option<KeyRing>, StoreError> {
        match std::fs::read(&self.path) {
            Ok(bytes) => {
                let bytes = zeroize::Zeroizing::new(bytes);
                let body = bytes
                    .strip_prefix(MARKER.as_bytes())
                    .ok_or(StoreError::Decrypt)?;
                Ok(Some(KeyRing::from_bytes(body)?))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn save(&self, ring: &KeyRing, _: Protection) -> Result<(), StoreError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut bytes = zeroize::Zeroizing::new(MARKER.as_bytes().to_vec());
        bytes.extend_from_slice(&ring.to_bytes());
        // Write a temp file (0600 from the start), sync, then rename over the
        // old key: a crash leaves the old key or the new one, never half.
        let mut tmp = self.path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = std::path::PathBuf::from(tmp);
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut file = opts.open(&tmp)?;
        std::io::Write::write_all(&mut file, &bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    fn delete(&self) -> Result<(), StoreError> {
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let path = std::env::temp_dir().join(format!("ghi-devkey-{}", std::process::id()));
        let s = FileKeyStore::new(&path);
        assert!(s.load().unwrap().is_none());
        let k = super::super::load_or_create(&s, Protection::default()).unwrap();
        assert_eq!(s.load().unwrap(), Some(k));
        s.delete().unwrap();
        assert!(s.load().unwrap().is_none());
    }
}
