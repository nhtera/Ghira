// SPDX-License-Identifier: Apache-2.0
//! Backup exclusion flag.

use ghi_store::backup::{exclude_from_backup, is_excluded_from_backup};

#[cfg(target_os = "macos")]
fn tmutil_says_excluded(path: &std::path::Path) -> Option<bool> {
    let out = std::process::Command::new("tmutil")
        .arg("isexcluded")
        .arg(path)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    Some(text.contains("[Excluded]"))
}

#[test]
fn set_read_back_and_unset() {
    // Not the system temp dir or target/ (tmutil and cargo exclude those by
    // path), so `tmutil isexcluded` starts at "included". Removed on drop.
    let t = tempfile::Builder::new()
        .prefix(".tmp-backup-test-")
        .tempdir_in(env!("CARGO_MANIFEST_DIR"))
        .unwrap();
    for dir in [true, false] {
        let path = if dir {
            let d = t.path().join("data");
            std::fs::create_dir(&d).unwrap();
            d
        } else {
            let f = t.path().join("file.bin");
            std::fs::write(&f, b"x").unwrap();
            f
        };
        assert!(!is_excluded_from_backup(&path).unwrap());
        #[cfg(target_os = "macos")]
        assert_eq!(
            tmutil_says_excluded(&path),
            Some(false),
            "tmutil already excludes the test dir"
        );
        exclude_from_backup(&path, true).unwrap();
        if cfg!(any(target_os = "macos", target_os = "ios")) {
            assert!(is_excluded_from_backup(&path).unwrap());
            #[cfg(target_os = "macos")]
            if let Some(v) = tmutil_says_excluded(&path) {
                assert!(v, "tmutil does not see the exclusion");
            }
        }
        exclude_from_backup(&path, false).unwrap();
        assert!(!is_excluded_from_backup(&path).unwrap());
        #[cfg(target_os = "macos")]
        if let Some(v) = tmutil_says_excluded(&path) {
            assert!(!v);
        }
    }
    assert!(
        exclude_from_backup(&t.path().join("missing"), true).is_err() || !cfg!(target_os = "macos")
    );
}
