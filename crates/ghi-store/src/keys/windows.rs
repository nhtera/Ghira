// SPDX-License-Identifier: Apache-2.0
//! Windows key store: the master key sealed with DPAPI for the current user
//! and kept in a file (in the non-roaming app data dir, so it is not synced
//! to other machines).
//!
//! `CryptProtectData` ties the blob to the signed-in Windows user; another
//! user or another machine cannot unprotect it. `CRYPTPROTECT_UI_FORBIDDEN`
//! keeps DPAPI from ever showing a prompt. `Protection::app_lock` has no
//! effect here; the app lock on Windows is enforced by the app UI (phase 12).

use std::path::PathBuf;
use std::ptr;

use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};

use zeroize::Zeroizing;

use super::{KeyRing, KeyStore, Protection};
use crate::StoreError;

/// Optional entropy so blobs from other DPAPI users of this account differ.
const ENTROPY: &[u8] = b"ghira/master-key/v1";

pub struct DpapiStore {
    path: PathBuf,
}

impl DpapiStore {
    pub fn new(path: impl Into<PathBuf>) -> DpapiStore {
        DpapiStore { path: path.into() }
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

/// Copies the DPAPI output blob and frees it.
///
/// # Safety
/// `out` must have been filled by a successful DPAPI call.
unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    // SAFETY: on success DPAPI returns `cbData` bytes at `pbData`.
    let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    // SAFETY: same buffer; wipe it (it may hold the unprotected key) before freeing.
    unsafe { ptr::write_bytes(out.pbData, 0, out.cbData as usize) };
    // SAFETY: the buffer was allocated by DPAPI with LocalAlloc.
    unsafe { LocalFree(out.pbData as _) };
    v
}

fn protect(plain: &[u8]) -> Result<Vec<u8>, StoreError> {
    let input = blob(plain);
    let entropy = blob(ENTROPY);
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

fn unprotect(sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, StoreError> {
    let input = blob(sealed);
    let entropy = blob(ENTROPY);
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

impl KeyStore for DpapiStore {
    fn load(&self) -> Result<Option<KeyRing>, StoreError> {
        let sealed = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let plain = unprotect(&sealed)?;
        KeyRing::from_bytes(&plain).map(Some)
    }

    fn save(&self, ring: &KeyRing, _: Protection) -> Result<(), StoreError> {
        let sealed = protect(&ring.to_bytes())?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Write a sibling, flush, then rename over the target: never a torn key file.
        let mut tmp = self.path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let write = || -> std::io::Result<()> {
            let f = std::fs::File::create(&tmp)?;
            std::io::Write::write_all(&mut &f, &sealed)?;
            f.sync_all()?;
            std::fs::rename(&tmp, &self.path)
        };
        write().inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        Ok(())
    }

    fn delete(&self) -> Result<(), StoreError> {
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}
