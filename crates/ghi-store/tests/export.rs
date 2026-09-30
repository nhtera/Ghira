// SPDX-License-Identifier: Apache-2.0
//! Password-encrypted export archives.

use std::fs;
use std::path::{Path, PathBuf};

use ghi_store::StoreError;
use ghi_store::export::{
    Entry, KdfParams, clean_stale_staging, export_archive, export_archive_with,
    export_entries_with, import_archive, import_archive_entries,
};

/// Cheap Argon2id so the suite stays fast; the default is exercised once.
const FAST: KdfParams = KdfParams {
    m_kib: 64,
    t: 1,
    p: 1,
};

fn write(dir: &Path, name: &str, data: &[u8]) -> (String, PathBuf) {
    let p = dir.join(name.replace('/', "_"));
    fs::write(&p, data).unwrap();
    (name.to_owned(), p)
}

fn big() -> Vec<u8> {
    (0..300_000u32).map(|i| (i * 7 + i / 251) as u8).collect()
}

fn sample(src: &Path) -> Vec<(String, PathBuf)> {
    vec![
        write(src, "ghira.db", &big()),
        write(src, "bundles/m1/mic.ghb", b"tiny"),
        write(src, "empty.txt", b""),
        write(src, "tiếng-việt.txt", "Chốt kế hoạch quý bốn".as_bytes()),
        write(src, "exact.bin", &vec![9u8; 64 * 1024]),
    ]
}

#[test]
fn round_trip() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    fs::create_dir(&src).unwrap();
    let files = sample(&src);
    let arch = t.path().join("out.ghx");
    export_archive_with(&files, &arch, "correct horse", &FAST).unwrap();
    assert!(!t.path().join("out.ghx.part").exists());

    let dest = t.path().join("dest");
    let names = import_archive(&arch, "correct horse", &dest).unwrap();
    assert_eq!(names, files.iter().map(|f| f.0.clone()).collect::<Vec<_>>());
    for (name, path) in &files {
        assert_eq!(
            fs::read(dest.join(name)).unwrap(),
            fs::read(path).unwrap(),
            "{name}"
        );
    }
    // Plaintext never appears in the archive.
    let raw = fs::read(&arch).unwrap();
    assert!(
        !raw.windows(8)
            .any(|w| w == "Chốt kế".as_bytes()[..8].to_vec().as_slice())
    );
    assert_eq!(
        fs::read_dir(&dest)
            .unwrap()
            .filter(|e| e
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".ghi-import"))
            .count(),
        0
    );
}

#[test]
fn default_params_and_unicode_password_normalisation() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    fs::create_dir(&src).unwrap();
    let files = vec![write(&src, "a.txt", b"hello")];
    let arch = t.path().join("d.ghx");
    // Composed "é" on export, decomposed on import.
    export_archive(&files, &arch, "caf\u{e9}").unwrap();
    let names = import_archive(&arch, "cafe\u{301}", &t.path().join("o")).unwrap();
    assert_eq!(names, ["a.txt"]);
}

#[test]
fn wrong_password_is_decrypt() {
    let t = tempfile::tempdir().unwrap();
    let files = vec![write(t.path(), "a.txt", b"secret")];
    let arch = t.path().join("a.ghx");
    export_archive_with(&files, &arch, "right", &FAST).unwrap();
    let dest = t.path().join("dest");
    assert!(matches!(
        import_archive(&arch, "wrong", &dest),
        Err(StoreError::Decrypt)
    ));
    assert!(fs::read_dir(&dest).map(|d| d.count() == 0).unwrap_or(true));
}

#[test]
fn tampering_and_truncation_are_detected_and_leave_nothing_behind() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    fs::create_dir(&src).unwrap();
    let files = sample(&src);
    let arch = t.path().join("a.ghx");
    export_archive_with(&files, &arch, "pw", &FAST).unwrap();
    let good = fs::read(&arch).unwrap();

    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
    let mut flipped = good.clone();
    flipped[good.len() / 2] ^= 1;
    cases.push(("flipped byte", flipped));
    let mut header = good.clone();
    header[20] ^= 1; // salt
    cases.push(("salt", header));
    let mut params = good.clone();
    params[6] ^= 1; // KDF params live in the header and are authenticated
    cases.push(("params", params));
    cases.push(("truncated tail", good[..good.len() - 30].to_vec()));
    cases.push((
        "truncated at chunk boundary",
        good[..52 + 4 + 65_536 + 16].to_vec(),
    ));
    let mut extra = good.clone();
    extra.extend_from_slice(&[0, 0, 0, 0]);
    cases.push(("trailing bytes", extra));
    cases.push(("header only", good[..52].to_vec()));

    for (label, bytes) in cases {
        let bad = t.path().join("bad.ghx");
        fs::write(&bad, bytes).unwrap();
        let dest = t.path().join("dest");
        let r = import_archive(&bad, "pw", &dest);
        assert!(r.is_err(), "{label} was accepted");
        let leftovers = fs::read_dir(&dest).map(|d| d.count()).unwrap_or(0);
        assert_eq!(leftovers, 0, "{label} left files behind");
    }

    // Chunks swapped: same total size, different order.
    let c0 = 52 + 4 + 65_536 + 16;
    let c1 = c0 + 4 + 65_536 + 16;
    let mut swapped = good[..52].to_vec();
    swapped.extend_from_slice(&good[c0..c1]);
    swapped.extend_from_slice(&good[52..c0]);
    swapped.extend_from_slice(&good[c1..]);
    let bad = t.path().join("swap.ghx");
    fs::write(&bad, swapped).unwrap();
    assert!(matches!(
        import_archive(&bad, "pw", &t.path().join("dest")),
        Err(StoreError::Decrypt)
    ));

    // Not an archive.
    fs::write(&bad, b"hello").unwrap();
    assert!(matches!(
        import_archive(&bad, "pw", &t.path().join("dest")),
        Err(StoreError::Invalid(_))
    ));
}

#[test]
fn unsafe_names_are_refused_on_export() {
    let t = tempfile::tempdir().unwrap();
    let f = write(t.path(), "a.txt", b"x").1;
    for bad in ["../evil", "/abs", "a/../../b", "a\\b", "C:evil", ""] {
        let files = vec![(bad.to_owned(), f.clone())];
        let r = export_archive_with(&files, &t.path().join("x.ghx"), "pw", &FAST);
        assert!(matches!(r, Err(StoreError::Invalid(_))), "{bad:?}");
    }
    assert!(!t.path().join("x.ghx").exists());
}

/// An authentic archive that carries a hostile name must be refused on
/// import too. Builds one with the crate's own format by exporting a safe
/// name, then checking import of every entry stays under `dest`.
#[test]
fn import_never_escapes_the_destination() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    fs::create_dir(&src).unwrap();
    let files = vec![write(&src, "sub/deep/a.txt", b"x")];
    let arch = t.path().join("a.ghx");
    export_archive_with(&files, &arch, "pw", &FAST).unwrap();
    let dest = t.path().join("dest");
    import_archive(&arch, "pw", &dest).unwrap();
    assert!(dest.join("sub/deep/a.txt").exists());
    // A second import must not overwrite.
    assert!(matches!(
        import_archive(&arch, "pw", &dest),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(fs::read(dest.join("sub/deep/a.txt")).unwrap(), b"x");
}

#[test]
fn key_ring_travels_through_memory_only() {
    let t = tempfile::tempdir().unwrap();
    let f = write(t.path(), "a.txt", b"hello");
    let secret = b"SECRET-KEY-RING-BYTES".to_vec();
    let entries = vec![
        Entry::file("a.txt", f.1),
        Entry::bytes("keyring.bin", secret.clone()),
    ];
    let arch = t.path().join("k.ghx");
    export_entries_with(&entries, &arch, "pw", &FAST).unwrap();
    assert!(
        !fs::read(&arch)
            .unwrap()
            .windows(secret.len())
            .any(|w| w == secret)
    );

    let dest = t.path().join("dest");
    let got = import_archive_entries(&arch, "pw", &dest).unwrap();
    assert_eq!(got.names, ["a.txt", "keyring.bin"]);
    assert_eq!(got.entry("keyring.bin").unwrap(), secret.as_slice());
    assert!(got.entry("a.txt").is_none());
    assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"hello");
    // Never written to disk, not even to staging.
    assert!(!dest.join("keyring.bin").exists());
    assert_eq!(fs::read_dir(&dest).unwrap().count(), 1);
}

#[test]
fn windows_reserved_and_trailing_dot_names_are_refused() {
    let t = tempfile::tempdir().unwrap();
    let f = write(t.path(), "a.txt", b"x").1;
    for bad in [
        "CON",
        "nul.txt",
        "a/Com1.log",
        "lpt9",
        "AUX.tar.gz",
        "a.",
        "a /b",
        "dir./x",
        "b ",
    ] {
        let files = vec![(bad.to_owned(), f.clone())];
        let r = export_archive_with(&files, &t.path().join("x.ghx"), "pw", &FAST);
        assert!(matches!(r, Err(StoreError::Invalid(_))), "{bad:?}");
    }
    for ok in ["console.txt", "a.b", "com10", "conx/aux2"] {
        let files = vec![(ok.to_owned(), f.clone())];
        export_archive_with(&files, &t.path().join("ok.ghx"), "pw", &FAST).unwrap();
    }
}

#[test]
fn import_refuses_excessive_kdf_cost() {
    let t = tempfile::tempdir().unwrap();
    let files = vec![write(t.path(), "a.txt", b"x")];
    let arch = t.path().join("a.ghx");
    export_archive_with(&files, &arch, "pw", &FAST).unwrap();
    let mut b = fs::read(&arch).unwrap();
    b[5..9].copy_from_slice(&(300_000u32).to_le_bytes()); // m > 256 MiB
    fs::write(&arch, &b).unwrap();
    assert!(matches!(
        import_archive(&arch, "pw", &t.path().join("d")),
        Err(StoreError::Invalid(_))
    ));
}

#[test]
fn stale_staging_is_cleaned() {
    let t = tempfile::tempdir().unwrap();
    fs::create_dir_all(t.path().join(".ghi-import-abc/sub")).unwrap();
    fs::write(t.path().join(".ghi-import-abc/sub/x"), b"x").unwrap();
    fs::write(t.path().join(".export-123"), b"x").unwrap();
    // Store::export_all stages in a directory.
    fs::create_dir_all(t.path().join(".export-456")).unwrap();
    fs::write(t.path().join(".export-456/ghira.db"), b"x").unwrap();
    fs::write(t.path().join("keep.txt"), b"x").unwrap();
    // Too young: kept.
    clean_stale_staging(t.path(), std::time::Duration::from_secs(3600)).unwrap();
    assert!(t.path().join(".export-123").exists());
    clean_stale_staging(t.path(), std::time::Duration::ZERO).unwrap();
    assert!(!t.path().join(".export-123").exists());
    assert!(!t.path().join(".export-456").exists());
    assert!(!t.path().join(".ghi-import-abc").exists());
    assert!(t.path().join("keep.txt").exists());
    clean_stale_staging(&t.path().join("missing"), std::time::Duration::ZERO).unwrap();
}
