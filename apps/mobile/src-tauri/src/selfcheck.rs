// SPDX-License-Identifier: Apache-2.0
//! `GHI_SELFTEST=storage` (test-hooks builds only): what the host tests cannot
//! see. It checks, inside the real app, that
//!
//! - the store's files use the data-protection class
//!   `CompleteUntilFirstUserAuthentication` (never `Complete`, which would make
//!   a write fail while the phone is locked: a recording keeps running behind
//!   the lock screen), and
//! - the Keychain accepts the master-key item and a small secret (store, read
//!   back, delete) under the test service.
//!
//! The result is written to `<data>/selftest-storage.json`
//! (`{"ok": bool, "checks": [{"name", "ok", "detail"}]}`); the XCUITest that
//! launches the app (16-K) reads it. The simulator reports no protection class,
//! so that check is `skipped` there.

use std::path::Path;

use ghi_store::keys::secrets::SecretStore;
use serde_json::{Value, json};

/// Service for the throw-away Keychain items (never the app's real one).
const SERVICE: &str = "com.nhtera.ghira.test.selftest";

/// `fcntl` command returning a file's protection class.
const F_GETPROTECTIONCLASS: i32 = 63;
/// `PROTECTION_CLASS_C`: `CompleteUntilFirstUserAuthentication`.
const CLASS_C: i32 = 3;

/// The class of every file and folder under `dir`: `(path relative, class)`.
#[cfg(unix)]
fn classes(dir: &Path) -> Vec<(String, i32)> {
    use std::os::fd::AsRawFd;
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.filter_map(Result::ok) {
            let p = e.path();
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                stack.push(p.clone());
            }
            if !(t.is_dir() || t.is_file()) {
                continue;
            }
            let Ok(f) = std::fs::File::open(&p) else {
                continue;
            };
            // SAFETY: a read-only fcntl query on a descriptor we own.
            let class = unsafe { libc::fcntl(f.as_raw_fd(), F_GETPROTECTIONCLASS) };
            let rel = p.strip_prefix(dir).unwrap_or(&p).display().to_string();
            out.push((rel, class));
        }
    }
    out
}

#[cfg(not(unix))]
fn classes(_dir: &Path) -> Vec<(String, i32)> {
    Vec::new()
}

fn check(name: &str, ok: bool, detail: impl Into<String>) -> Value {
    json!({"name": name, "ok": ok, "detail": detail.into()})
}

fn protection(store_dir: &Path) -> Value {
    if cfg!(target_abi = "sim") || !cfg!(target_os = "ios") {
        return json!({"name": "protectionClass", "ok": true, "skipped": true,
                      "detail": "the simulator has no data protection"});
    }
    let found = classes(store_dir);
    let bad: Vec<String> = found
        .iter()
        .filter(|(_, c)| *c != CLASS_C)
        .map(|(p, c)| format!("{p}:{c}"))
        .collect();
    check(
        "protectionClass",
        !found.is_empty() && bad.is_empty(),
        if found.is_empty() {
            "no store files".to_owned()
        } else {
            format!("{} files, not class C: {bad:?}", found.len())
        },
    )
}

fn keychain() -> Vec<Value> {
    let mut out = Vec::new();
    let secrets = ghi_store::keys::secrets::KeychainSecrets::new(SERVICE);
    let r = (|| -> Result<bool, String> {
        secrets
            .set("selftest", b"round-trip")
            .map_err(|e| e.to_string())?;
        let got = secrets.get("selftest").map_err(|e| e.to_string())?;
        secrets.delete("selftest").map_err(|e| e.to_string())?;
        let gone = secrets
            .get("selftest")
            .map_err(|e| e.to_string())?
            .is_none();
        Ok(got.as_deref().map(|v| v.as_slice()) == Some(b"round-trip".as_slice()) && gone)
    })();
    out.push(check(
        "keychainSecret",
        r == Ok(true),
        r.err().unwrap_or_default(),
    ));

    use ghi_store::keys::{KeyRing, KeyStore, Protection};
    let ks = ghi_store::keys::apple::KeychainStore::new(SERVICE, "store");
    let r = (|| -> Result<bool, String> {
        let ring = KeyRing::generate();
        ks.save(&ring, Protection::default())
            .map_err(|e| e.to_string())?;
        let back = ks.load().map_err(|e| e.to_string())?;
        ks.delete().map_err(|e| e.to_string())?;
        let same = back.is_some_and(|b| b.to_bytes().as_slice() == ring.to_bytes().as_slice());
        Ok(same && ks.load().map_err(|e| e.to_string())?.is_none())
    })();
    out.push(check(
        "keychainKey",
        r == Ok(true),
        r.err().unwrap_or_default(),
    ));
    out
}

/// Runs the checks and writes the result file.
pub fn run(data: &Path) {
    let mut checks = vec![protection(&data.join("store"))];
    checks.extend(keychain());
    let ok = checks.iter().all(|c| c["ok"] == true);
    let result = json!({"ok": ok, "checks": checks});
    if let Err(e) = std::fs::write(data.join("selftest-storage.json"), result.to_string()) {
        eprintln!("ghira: storage self-test result: {e}");
    }
}

/// Starts the self-test when the app was launched with `GHI_SELFTEST=storage`
/// (or `keychain`), once the store is open.
pub fn spawn_if_asked(core: std::sync::Arc<ghi_app::core::Core>) {
    let asked = std::env::var("GHI_SELFTEST").is_ok_and(|v| v == "storage" || v == "keychain");
    if !asked {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("ghi-selfcheck".into())
        .spawn(move || {
            // Files exist once the store is open and has written.
            for _ in 0..100 {
                if core.store_even_locked().is_ok() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            run(core.data_dir());
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_run_writes_a_result_and_skips_the_class_check() {
        let t = tempfile::tempdir().unwrap();
        let checks = vec![protection(&t.path().join("store"))];
        assert_eq!(checks[0]["skipped"], true);
        // On the host the keychain checks may legitimately fail (no entitlement
        // in a test binary); the file is written either way.
        run(t.path());
        let v: Value =
            serde_json::from_slice(&std::fs::read(t.path().join("selftest-storage.json")).unwrap())
                .unwrap();
        assert_eq!(v["checks"].as_array().unwrap().len(), 3);
    }
}
