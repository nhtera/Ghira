// SPDX-License-Identifier: Apache-2.0
//! Which key store `ghi` uses for a data directory, and opening the store.
//!
//! - Debug builds: a plain key file next to the data directory
//!   (`<dir>.devkey`, never inside it) by default, so tests and dev runs never
//!   touch the Keychain; `GHI_KEYSTORE=keychain` switches to the platform store.
//! - Release builds: the platform store. The CLI has its own Keychain item
//!   (service `com.nhtera.ghira.cli`), never the app's.

use std::path::Path;
use std::sync::Arc;

use ghi_store::keys::{KeyStore, Protection};
use ghi_store::store::Store;

use crate::contract::{ErrorCode, ErrorDoc};

/// Keychain service of the CLI's master key (the app uses `com.nhtera.ghira`).
pub const KEYCHAIN_SERVICE: &str = "com.nhtera.ghira.cli";

pub fn keystore(dir: &Path) -> Result<Arc<dyn KeyStore>, ErrorDoc> {
    #[cfg(debug_assertions)]
    if std::env::var("GHI_KEYSTORE").as_deref() != Ok("keychain") {
        return Ok(Arc::new(ghi_store::keys::dev::FileKeyStore::new(
            dev_key_path(dir)?,
        )));
    }
    platform_keystore(dir)
}

/// `<dir>.devkey`, a sibling of the data directory (`data/` and `data` give
/// the same path, never one inside the directory).
#[cfg(debug_assertions)]
fn dev_key_path(dir: &Path) -> Result<std::path::PathBuf, ErrorDoc> {
    let abs = std::path::absolute(dir)
        .map_err(|e| ErrorDoc::new(ErrorCode::BadInput, format!("{}: {e}", dir.display())))?;
    let name = abs.file_name().ok_or_else(|| {
        ErrorDoc::new(
            ErrorCode::BadInput,
            format!("{}: name the data directory itself", dir.display()),
        )
    })?;
    let mut name = name.to_owned();
    name.push(".devkey");
    Ok(abs.with_file_name(name))
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    #[test]
    fn dev_key_is_a_sibling_even_with_a_trailing_slash() {
        let a = dev_key_path(Path::new("some/data")).unwrap();
        let b = dev_key_path(Path::new("some/data/")).unwrap();
        assert_eq!(a, b);
        assert!(a.ends_with("some/data.devkey"), "{}", a.display());
        assert!(dev_key_path(Path::new("some/data/.")).is_ok_and(|p| p == a));
    }
}

/// One Keychain item per data directory (account = its absolute path), so
/// separate stores never share or overwrite each other's keys.
#[cfg(target_os = "macos")]
fn platform_keystore(dir: &Path) -> Result<Arc<dyn KeyStore>, ErrorDoc> {
    let bad =
        |e: std::io::Error| ErrorDoc::new(ErrorCode::BadInput, format!("{}: {e}", dir.display()));
    std::fs::create_dir_all(dir).map_err(bad)?;
    let abs = dir.canonicalize().map_err(bad)?;
    Ok(Arc::new(ghi_store::keys::apple::KeychainStore::new(
        KEYCHAIN_SERVICE,
        &format!("store:{}", abs.display()),
    )))
}

#[cfg(windows)]
fn platform_keystore(dir: &Path) -> Result<Arc<dyn KeyStore>, ErrorDoc> {
    Ok(Arc::new(ghi_store::keys::windows::DpapiStore::new(
        dir.join("keys").join("master.dpapi"),
    )))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn platform_keystore(_: &Path) -> Result<Arc<dyn KeyStore>, ErrorDoc> {
    Err(ErrorDoc::new(
        ErrorCode::NotImplemented,
        "store: no OS key store on this platform yet",
    ))
}

/// Maps a store error to the CLI's error document. Messages carry no user content.
pub fn store_error(e: ghi_store::StoreError) -> ErrorDoc {
    use ghi_store::StoreError as E;
    let code = match e {
        E::NotFound { .. } | E::Invalid(_) | E::Decrypt => ErrorCode::BadInput,
        _ => ErrorCode::Internal,
    };
    ErrorDoc::new(code, format!("store: {e}"))
}

/// Opens (creating on first use) the store in `dir`.
pub fn open_store(dir: &Path) -> Result<Store, ErrorDoc> {
    let ks = keystore(dir)?;
    Store::open(dir, ks, Protection::default()).map_err(store_error)
}
