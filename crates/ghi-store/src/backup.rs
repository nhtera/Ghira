// SPDX-License-Identifier: Apache-2.0
//! Keeps the data directory out of OS backups (Time Machine, iCloud/iTunes
//! device backups). The database, audio and keys are only useful on this
//! device (the keys never leave it), and encrypted user content should not
//! land in cloud backups by accident.
//!
//! macOS / iOS: sets `NSURLIsExcludedFromBackupKey` on the path (a directory
//! excludes its contents). Windows data lives in the non-roaming
//! `%LOCALAPPDATA%`; Linux and Android need nothing here (Android's
//! `allowBackup=false` is a manifest setting, phase 17).

use std::path::Path;

use crate::Result;

/// Excludes (`exclude = true`) or re-includes `path` in OS backups.
/// A no-op that returns `Ok` on platforms without a per-path switch.
pub fn exclude_from_backup(path: &Path, exclude: bool) -> Result<()> {
    imp::set(path, exclude)
}

/// Whether `path` is currently excluded from backups (`false` where the
/// platform has no such flag).
pub fn is_excluded_from_backup(path: &Path) -> Result<bool> {
    imp::get(path)
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod imp {
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr;

    use core_foundation_sys::base::{Boolean, CFRelease, CFTypeRef, kCFAllocatorDefault};
    use core_foundation_sys::error::CFErrorRef;
    use core_foundation_sys::number::{
        CFBooleanGetValue, CFBooleanRef, kCFBooleanFalse, kCFBooleanTrue,
    };
    use core_foundation_sys::string::CFStringRef;
    use core_foundation_sys::url::{
        CFURLCreateFromFileSystemRepresentation, CFURLRef, CFURLSetResourcePropertyForKey,
    };

    use crate::{Result, StoreError};

    // core-foundation-sys hides the key behind a feature and omits the getter.
    unsafe extern "C" {
        static kCFURLIsExcludedFromBackupKey: CFStringRef;
        fn CFURLCopyResourcePropertyForKey(
            url: CFURLRef,
            key: CFStringRef,
            value_out: *mut CFTypeRef,
            error: *mut CFErrorRef,
        ) -> Boolean;
        fn CFURLClearResourcePropertyCacheForKey(url: CFURLRef, key: CFStringRef);
    }

    fn fail(what: &str) -> StoreError {
        StoreError::Io(std::io::Error::other(format!("backup exclusion: {what}")))
    }

    struct Url(CFURLRef);

    impl Url {
        fn new(path: &Path) -> Result<Url> {
            let meta = std::fs::metadata(path)?;
            let bytes = path.as_os_str().as_bytes();
            // SAFETY: `bytes` is valid for its length; the URL copies it.
            let u = unsafe {
                CFURLCreateFromFileSystemRepresentation(
                    kCFAllocatorDefault,
                    bytes.as_ptr(),
                    bytes.len() as isize,
                    u8::from(meta.is_dir()),
                )
            };
            if u.is_null() {
                Err(fail("could not make a URL"))
            } else {
                Ok(Url(u))
            }
        }
    }

    impl Drop for Url {
        fn drop(&mut self) {
            // SAFETY: created by a Create function; released once.
            unsafe { CFRelease(self.0 as CFTypeRef) };
        }
    }

    pub fn set(path: &Path, exclude: bool) -> Result<()> {
        let url = Url::new(path)?;
        // SAFETY: valid URL and constants; the error out-pointer may be null.
        let ok = unsafe {
            let value = if exclude {
                kCFBooleanTrue
            } else {
                kCFBooleanFalse
            };
            CFURLSetResourcePropertyForKey(
                url.0,
                kCFURLIsExcludedFromBackupKey,
                value as CFTypeRef,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            Err(fail("the system refused to set the flag"))
        } else {
            Ok(())
        }
    }

    pub fn get(path: &Path) -> Result<bool> {
        let url = Url::new(path)?;
        // Resource values may come from a cache shared by URLs of the same
        // file: read the flag from the file system, not a stale copy.
        // SAFETY: valid URL and key.
        unsafe { CFURLClearResourcePropertyCacheForKey(url.0, kCFURLIsExcludedFromBackupKey) };
        let mut value: CFTypeRef = ptr::null();
        // SAFETY: valid URL and key; `value` receives a +1 reference or stays null.
        let ok = unsafe {
            CFURLCopyResourcePropertyForKey(
                url.0,
                kCFURLIsExcludedFromBackupKey,
                &mut value,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(fail("could not read the flag"));
        }
        if value.is_null() {
            return Ok(false);
        }
        // SAFETY: the value for this key is a CFBoolean we own (+1).
        let excluded = unsafe { CFBooleanGetValue(value as CFBooleanRef) };
        // SAFETY: release the reference obtained from the Copy call.
        unsafe { CFRelease(value) };
        Ok(excluded)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
mod imp {
    use std::path::Path;

    use crate::Result;

    pub fn set(_: &Path, _: bool) -> Result<()> {
        Ok(())
    }

    pub fn get(_: &Path) -> Result<bool> {
        Ok(false)
    }
}
