// SPDX-License-Identifier: Apache-2.0
//! Provider API keys (and other small per-account secrets) in the OS keystore.
//!
//! Unlike the master key ([`super::KeyStore`]) these are plain byte strings
//! identified by an account name (`openai`, `anthropic`, ...):
//! - macOS/iOS: a generic-password Keychain item per account under a
//!   caller-given service, device-only (non-synchronizable), in the login
//!   keychain like [`super::apple::KeychainStore::new`] ([`KeychainSecrets`]);
//! - Windows: one DPAPI-sealed file per account under a caller-given
//!   directory ([`DpapiSecrets`]);
//! - debug builds only: plain files, 0600 ([`FileSecrets`]).
//!
//! Values are returned as [`Zeroizing`] bytes and never appear in an error.

use zeroize::Zeroizing;

use crate::StoreError;

/// A device-only home for small named secrets.
pub trait SecretStore: Send + Sync {
    /// The secret for `account`, or `None` if none is stored.
    fn get(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StoreError>;
    /// Stores the secret, replacing any existing one.
    fn set(&self, account: &str, bytes: &[u8]) -> Result<(), StoreError>;
    /// Removes the secret (no error if there is none).
    fn delete(&self, account: &str) -> Result<(), StoreError>;
}

/// Account names are short identifiers: they become file names on some
/// backends, so nothing else is accepted.
fn check_account(account: &str) -> Result<(), StoreError> {
    let ok = !account.is_empty()
        && account.len() <= 64
        && account
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && !account.starts_with('.');
    if ok {
        Ok(())
    } else {
        Err(StoreError::Invalid(
            "a secret's account name is 1-64 letters, digits, '.', '_' or '-'".into(),
        ))
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use keychain::KeychainSecrets;

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod keychain {
    use security_framework::base::Error as SfError;
    use security_framework::passwords::{
        PasswordOptions, delete_generic_password_options, generic_password,
        set_generic_password_options,
    };
    use zeroize::Zeroizing;

    use super::{SecretStore, check_account};
    use crate::StoreError;

    const ERR_USER_CANCELED: i32 = -128;
    const ERR_AUTH_FAILED: i32 = -25293;
    const ERR_INTERACTION_NOT_ALLOWED: i32 = -25308;
    const ERR_ITEM_NOT_FOUND: i32 = -25300;

    /// One generic-password item per account under `service`. On macOS the
    /// login keychain (no entitlements needed, so the unsigned CLI can use it).
    pub struct KeychainSecrets {
        service: String,
    }

    impl KeychainSecrets {
        pub fn new(service: &str) -> KeychainSecrets {
            KeychainSecrets {
                service: service.to_owned(),
            }
        }

        fn query(&self, account: &str) -> PasswordOptions {
            let mut o = PasswordOptions::new_generic_password(&self.service, account);
            // Never leaves the device through iCloud Keychain.
            o.set_access_synchronized(Some(false));
            o
        }
    }

    fn map(e: SfError) -> StoreError {
        match e.code() {
            ERR_USER_CANCELED | ERR_AUTH_FAILED | ERR_INTERACTION_NOT_ALLOWED => {
                StoreError::KeyLocked
            }
            code => StoreError::Keystore {
                detail: format!(
                    "{} (status {code})",
                    e.message()
                        .unwrap_or_else(|| "Security.framework error".into())
                ),
            },
        }
    }

    impl SecretStore for KeychainSecrets {
        fn get(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StoreError> {
            check_account(account)?;
            match generic_password(self.query(account)) {
                Ok(v) => Ok(Some(Zeroizing::new(v))),
                Err(e) if e.code() == ERR_ITEM_NOT_FOUND => Ok(None),
                Err(e) => Err(map(e)),
            }
        }

        /// Adds the item, or updates it in place if it exists.
        fn set(&self, account: &str, bytes: &[u8]) -> Result<(), StoreError> {
            check_account(account)?;
            set_generic_password_options(bytes, self.query(account)).map_err(map)
        }

        fn delete(&self, account: &str) -> Result<(), StoreError> {
            check_account(account)?;
            match delete_generic_password_options(self.query(account)) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == ERR_ITEM_NOT_FOUND => Ok(()),
                Err(e) => Err(map(e)),
            }
        }
    }
}

#[cfg(windows)]
pub use dpapi::DpapiSecrets;

#[cfg(windows)]
mod dpapi {
    use std::path::PathBuf;
    use std::ptr;

    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    use zeroize::Zeroizing;

    use super::{SecretStore, atomic_write, check_account, read_file, remove_file};
    use crate::StoreError;

    /// Entropy label; the account name is appended so one account's file
    /// cannot be unsealed as another's.
    const ENTROPY: &[u8] = b"ghira/provider-secret/v1:";

    /// One DPAPI-sealed file per account in `dir` (the non-roaming app data
    /// dir, so the files are not synced to other machines). The blob is tied
    /// to the signed-in Windows user.
    pub struct DpapiSecrets {
        dir: PathBuf,
    }

    impl DpapiSecrets {
        pub fn new(dir: impl Into<PathBuf>) -> DpapiSecrets {
            DpapiSecrets { dir: dir.into() }
        }

        fn path(&self, account: &str) -> PathBuf {
            self.dir.join(format!("{account}.secret"))
        }
    }

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    fn last_error(what: &str) -> StoreError {
        StoreError::Keystore {
            detail: format!("{what} failed ({})", std::io::Error::last_os_error()),
        }
    }

    fn entropy(account: &str) -> Vec<u8> {
        [ENTROPY, account.as_bytes()].concat()
    }

    /// Copies the DPAPI output blob, wipes it and frees it.
    ///
    /// # Safety
    /// `out` must have been filled by a successful DPAPI call.
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: on success DPAPI returns `cbData` bytes at `pbData`.
        let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        // SAFETY: same buffer; it may hold the unprotected secret.
        unsafe { ptr::write_bytes(out.pbData, 0, out.cbData as usize) };
        // SAFETY: the buffer was allocated by DPAPI with LocalAlloc.
        unsafe { LocalFree(out.pbData as _) };
        v
    }

    fn protect(plain: &[u8], account: &str) -> Result<Vec<u8>, StoreError> {
        let input = blob(plain);
        let ent = entropy(account);
        let entropy = blob(&ent);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        // SAFETY: all pointers refer to live blobs; `out` is written by the call.
        let ok = unsafe {
            CryptProtectData(
                &input,
                ptr::null(),
                &entropy,
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(last_error("CryptProtectData"));
        }
        // SAFETY: the call succeeded.
        Ok(unsafe { take(out) })
    }

    fn unprotect(sealed: &[u8], account: &str) -> Result<Zeroizing<Vec<u8>>, StoreError> {
        let input = blob(sealed);
        let ent = entropy(account);
        let entropy = blob(&ent);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        // SAFETY: as in `protect`.
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                &entropy,
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(last_error("CryptUnprotectData"));
        }
        // SAFETY: the call succeeded.
        Ok(Zeroizing::new(unsafe { take(out) }))
    }

    impl SecretStore for DpapiSecrets {
        fn get(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StoreError> {
            check_account(account)?;
            match read_file(&self.path(account))? {
                Some(sealed) => unprotect(&sealed, account).map(Some),
                None => Ok(None),
            }
        }

        fn set(&self, account: &str, bytes: &[u8]) -> Result<(), StoreError> {
            check_account(account)?;
            atomic_write(&self.path(account), &protect(bytes, account)?)
        }

        fn delete(&self, account: &str) -> Result<(), StoreError> {
            check_account(account)?;
            remove_file(&self.path(account))
        }
    }
}

#[cfg(debug_assertions)]
pub use file::FileSecrets;

#[cfg(debug_assertions)]
mod file {
    use std::path::PathBuf;

    use zeroize::Zeroizing;

    use super::{SecretStore, atomic_write, check_account, read_file, remove_file};
    use crate::StoreError;

    /// Debug-only secrets in plain files (`<dir>/<account>.secret`, 0600). For
    /// tests and dev runs of `ghi`; compiled only with `debug_assertions`
    /// (like [`crate::keys::dev`], whose marker the release check looks for).
    pub struct FileSecrets {
        dir: PathBuf,
    }

    impl FileSecrets {
        pub fn new(dir: impl Into<PathBuf>) -> FileSecrets {
            FileSecrets { dir: dir.into() }
        }

        fn path(&self, account: &str) -> PathBuf {
            self.dir.join(format!("{account}.secret"))
        }
    }

    impl SecretStore for FileSecrets {
        fn get(&self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StoreError> {
            check_account(account)?;
            let marker = crate::keys::dev::MARKER.as_bytes();
            match read_file(&self.path(account))? {
                Some(bytes) => {
                    let body = bytes.strip_prefix(marker).ok_or(StoreError::Decrypt)?;
                    Ok(Some(Zeroizing::new(body.to_vec())))
                }
                None => Ok(None),
            }
        }

        fn set(&self, account: &str, bytes: &[u8]) -> Result<(), StoreError> {
            check_account(account)?;
            let mut data = Zeroizing::new(crate::keys::dev::MARKER.as_bytes().to_vec());
            data.extend_from_slice(bytes);
            atomic_write(&self.path(account), &data)
        }

        fn delete(&self, account: &str) -> Result<(), StoreError> {
            check_account(account)?;
            remove_file(&self.path(account))
        }
    }
}

/// Reads a whole file; `None` if it does not exist. Zeroed on drop.
#[cfg(any(windows, debug_assertions))]
fn read_file(path: &std::path::Path) -> Result<Option<Zeroizing<Vec<u8>>>, StoreError> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(Zeroizing::new(b))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Writes a temp file (0600 from the start), syncs, then renames over the
/// target: a crash leaves the old secret or the new one, never half.
#[cfg(any(windows, debug_assertions))]
fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<(), StoreError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let write = || -> std::io::Result<()> {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        let mut file = opts.open(&tmp)?;
        std::io::Write::write_all(&mut file, bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    };
    write().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.into()
    })
}

#[cfg(any(windows, debug_assertions))]
fn remove_file(path: &std::path::Path) -> Result<(), StoreError> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_names_are_restricted() {
        for ok in ["openai", "anthropic", "my-provider_2", "a.b"] {
            assert!(check_account(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            ".hidden",
            "a/b",
            "..",
            "a b",
            "x\0y",
            "é",
            &"x".repeat(65),
        ] {
            assert!(check_account(bad).is_err(), "{bad:?}");
        }
    }

    #[cfg(debug_assertions)]
    #[test]
    fn file_store_round_trip() {
        let dir = std::env::temp_dir().join(format!(
            "ghi-secrets-{}-{}",
            std::process::id(),
            crate::new_gid()
        ));
        let s = FileSecrets::new(&dir);
        assert!(s.get("openai").unwrap().is_none());
        s.set("openai", b"test-key-not-real").unwrap();
        assert_eq!(
            s.get("openai").unwrap().unwrap().as_slice(),
            b"test-key-not-real"
        );
        // Replace, and accounts are independent.
        s.set("openai", b"test-key-2-not-real").unwrap();
        s.set("anthropic", b"other").unwrap();
        assert_eq!(
            s.get("openai").unwrap().unwrap().as_slice(),
            b"test-key-2-not-real"
        );
        assert_eq!(s.get("anthropic").unwrap().unwrap().as_slice(), b"other");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("openai.secret"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        s.delete("openai").unwrap();
        s.delete("openai").unwrap();
        assert!(s.get("openai").unwrap().is_none());
        assert!(s.get("../etc/passwd").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn file_store_rejects_a_foreign_file() {
        let dir = std::env::temp_dir().join(format!(
            "ghi-secrets-{}-{}",
            std::process::id(),
            crate::new_gid()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.secret"), b"not ours").unwrap();
        assert!(matches!(
            FileSecrets::new(&dir).get("x"),
            Err(StoreError::Decrypt)
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
