// SPDX-License-Identifier: Apache-2.0
//! Audio bundle: round trip, random access, tamper detection, crash recovery.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use ghi_store::StoreError;
use ghi_store::bundle::{BundleReader, BundleWriter, index_path, recover};
use ghi_store::rowcrypt::Dek;

/// The track gid.
const AAD: &[u8] = b"track-mic-1";
const HEADER: usize = 24;

fn key() -> Dek {
    Dek::from_bytes([42; 32])
}

fn payload(i: u32) -> Vec<u8> {
    (0..1000 + (i as usize % 37))
        .map(|b| (b as u32 ^ i.wrapping_mul(31)) as u8)
        .collect()
}

fn write_bundle(path: &Path, pages: u32, finish: bool) {
    let mut w = BundleWriter::create(path, &key(), AAD).unwrap();
    for i in 0..pages {
        assert_eq!(w.append(&payload(i)).unwrap(), i);
    }
    if finish {
        assert_eq!(w.finish().unwrap(), pages);
    } else {
        w.sync(true).unwrap();
    }
}

/// `(start, end)` of every record in a bundle file.
fn records(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut pos = HEADER;
    while pos + 4 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap()) as usize;
        if pos + 4 + len > bytes.len() {
            break;
        }
        out.push((pos, pos + 4 + len));
        pos += 4 + len;
    }
    out
}

fn fresh(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    dir.path().join(name)
}

#[test]
fn round_trip_and_random_access() {
    let dir = tempfile::tempdir().unwrap();
    let p = fresh(&dir, "a.ghb");
    write_bundle(&p, 50, true);
    assert!(index_path(&p).exists());

    let r = BundleReader::open(&p, &key(), AAD).unwrap();
    assert!(r.complete());
    assert_eq!(r.page_count(), 50);
    for i in [49, 0, 25, 7, 48] {
        assert_eq!(r.page(i).unwrap(), payload(i));
    }
    assert!(matches!(r.page(50), Err(StoreError::Invalid(_))));
    let all: Vec<u8> = (0..50).flat_map(payload).collect();
    assert_eq!(r.read_all().unwrap(), all);

    // The index is advisory: missing or garbage, the reader scans instead.
    fs::remove_file(index_path(&p)).unwrap();
    let r = BundleReader::open(&p, &key(), AAD).unwrap();
    assert!(r.complete());
    assert_eq!(r.page(33).unwrap(), payload(33));
    fs::write(index_path(&p), b"GHII garbage").unwrap();
    let r = BundleReader::open(&p, &key(), AAD).unwrap();
    assert_eq!((r.complete(), r.page_count()), (true, 50));
    assert_eq!(r.read_all().unwrap(), all);
}

#[test]
fn empty_bundle_and_never_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    let p = fresh(&dir, "e.ghb");
    write_bundle(&p, 0, true);
    let r = BundleReader::open(&p, &key(), AAD).unwrap();
    assert_eq!((r.complete(), r.page_count()), (true, 0));
    assert!(r.read_all().unwrap().is_empty());
    assert!(matches!(
        BundleWriter::create(&p, &key(), AAD),
        Err(StoreError::Io(_))
    ));

    let mut w = BundleWriter::create(&fresh(&dir, "f.ghb"), &key(), AAD).unwrap();
    w.finish().unwrap();
    assert!(w.append(b"x").is_err());
    assert_eq!(w.finish().unwrap(), 0);
}

#[test]
fn wrong_key_and_wrong_aad() {
    let dir = tempfile::tempdir().unwrap();
    let p = fresh(&dir, "k.ghb");
    write_bundle(&p, 5, true);
    for r in [
        BundleReader::open(&p, &Dek::from_bytes([1; 32]), AAD).unwrap(),
        BundleReader::open(&p, &key(), b"meeting-2/mic").unwrap(),
    ] {
        assert!(!r.complete());
        assert!(matches!(r.page(0), Err(StoreError::Decrypt)));
        assert!(r.read_all().is_err());
    }
}

#[test]
fn swapped_and_reordered_pages_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let p = fresh(&dir, "s.ghb");
    write_bundle(&p, 4, true);
    let bytes = fs::read(&p).unwrap();
    let recs = records(&bytes);
    let (a, b) = (recs[1], recs[2]);
    let mut swapped = bytes[..a.0].to_vec();
    swapped.extend_from_slice(&bytes[b.0..b.1]);
    swapped.extend_from_slice(&bytes[a.0..a.1]);
    swapped.extend_from_slice(&bytes[b.1..]);
    let q = fresh(&dir, "s2.ghb");
    fs::write(&q, &swapped).unwrap();
    let r = BundleReader::open(&q, &key(), AAD).unwrap();
    assert_eq!(r.page(0).unwrap(), payload(0));
    assert!(r.page(1).is_err());
    assert!(r.page(2).is_err());
    assert_eq!(r.page(3).unwrap(), payload(3));
    let rec = recover(&q, &key(), AAD).unwrap();
    assert_eq!((rec.pages, rec.complete), (1, false));
    assert!(rec.truncated_bytes > 0);
}

#[test]
fn flipped_byte_is_detected_and_recovery_keeps_the_good_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let p = fresh(&dir, "x.ghb");
    write_bundle(&p, 6, true);
    let mut bytes = fs::read(&p).unwrap();
    let recs = records(&bytes);
    bytes[recs[3].0 + 10] ^= 0x01;
    fs::write(&p, &bytes).unwrap();
    let r = BundleReader::open(&p, &key(), AAD).unwrap();
    assert_eq!(r.page(2).unwrap(), payload(2));
    assert!(matches!(r.page(3), Err(StoreError::Decrypt)));
    assert_eq!(r.page(4).unwrap(), payload(4));

    let rec = recover(&p, &key(), AAD).unwrap();
    assert_eq!((rec.pages, rec.complete), (3, false));
    assert_eq!(fs::metadata(&p).unwrap().len() as usize, recs[2].1);
    let r = BundleReader::open(&p, &key(), AAD).unwrap();
    assert_eq!(r.page_count(), 3);
    assert_eq!(
        r.read_all().unwrap(),
        (0..3).flat_map(payload).collect::<Vec<u8>>()
    );
}

#[test]
fn truncations() {
    let dir = tempfile::tempdir().unwrap();
    let p = fresh(&dir, "t.ghb");
    write_bundle(&p, 5, true);
    let full = fs::read(&p).unwrap();
    let recs = records(&full);

    // Missing final record: readable, but reported incomplete.
    let q = fresh(&dir, "t1.ghb");
    fs::write(&q, &full[..recs[4].1]).unwrap();
    let r = BundleReader::open(&q, &key(), AAD).unwrap();
    assert_eq!((r.complete(), r.page_count()), (false, 5));
    assert_eq!(r.page(4).unwrap(), payload(4));
    let rec = recover(&q, &key(), AAD).unwrap();
    assert_eq!(
        (rec.pages, rec.complete, rec.truncated_bytes),
        (5, false, 0)
    );

    // Cut in the middle of a record.
    let q = fresh(&dir, "t2.ghb");
    let cut = recs[3].0 + 100;
    fs::write(&q, &full[..cut]).unwrap();
    let r = BundleReader::open(&q, &key(), AAD).unwrap();
    assert_eq!((r.complete(), r.page_count()), (false, 3));
    let rec = recover(&q, &key(), AAD).unwrap();
    assert_eq!((rec.pages, rec.complete), (3, false));
    assert_eq!(rec.truncated_bytes as usize, cut - recs[3].0);
    assert_eq!(fs::metadata(&q).unwrap().len() as usize, recs[3].0);
    let r = BundleReader::open(&q, &key(), AAD).unwrap();
    assert_eq!(
        r.read_all().unwrap(),
        (0..3).flat_map(payload).collect::<Vec<u8>>()
    );

    // Cut inside the length prefix.
    let q = fresh(&dir, "t3.ghb");
    fs::write(&q, &full[..recs[2].0 + 2]).unwrap();
    assert_eq!(recover(&q, &key(), AAD).unwrap().pages, 2);

    // A stale index must not confuse the reader after truncation.
    let q = fresh(&dir, "t4.ghb");
    fs::write(&q, &full[..recs[2].1]).unwrap();
    fs::copy(index_path(&p), index_path(&q)).unwrap();
    let r = BundleReader::open(&q, &key(), AAD).unwrap();
    assert_eq!((r.complete(), r.page_count()), (false, 3));

    // Header only.
    let q = fresh(&dir, "t5.ghb");
    fs::write(&q, &full[..HEADER]).unwrap();
    let rec = recover(&q, &key(), AAD).unwrap();
    assert_eq!((rec.pages, rec.complete), (0, false));

    // Trailing garbage after a clean end is cut off.
    let q = fresh(&dir, "t6.ghb");
    let mut junk = full.clone();
    junk.extend_from_slice(&[9, 9, 9, 9, 9, 9, 9, 9, 9]);
    fs::write(&q, &junk).unwrap();
    let rec = recover(&q, &key(), AAD).unwrap();
    assert_eq!((rec.pages, rec.complete, rec.truncated_bytes), (5, true, 9));
    assert_eq!(fs::read(&q).unwrap(), full);
}

#[test]
fn recover_with_wrong_key_does_not_destroy_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let p = fresh(&dir, "w.ghb");
    write_bundle(&p, 5, true);
    let before = fs::read(&p).unwrap();
    assert!(matches!(
        recover(&p, &Dek::from_bytes([7; 32]), AAD),
        Err(StoreError::Decrypt)
    ));
    assert!(matches!(
        recover(&p, &key(), b"other"),
        Err(StoreError::Decrypt)
    ));
    assert_eq!(fs::read(&p).unwrap(), before);
    fs::write(fresh(&dir, "n.ghb"), b"not a bundle at all, sorry").unwrap();
    assert!(matches!(
        recover(&fresh(&dir, "n.ghb"), &key(), AAD),
        Err(StoreError::Invalid(_))
    ));
}

#[test]
fn recover_clean_bundle_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let p = fresh(&dir, "c.ghb");
    write_bundle(&p, 8, true);
    let before = fs::read(&p).unwrap();
    let rec = recover(&p, &key(), AAD).unwrap();
    assert_eq!((rec.pages, rec.complete, rec.truncated_bytes), (8, true, 0));
    assert_eq!(fs::read(&p).unwrap(), before);
}

const KILL_PATH: &str = "GHI_BUNDLE_KILL9_PATH";

/// Child half of the kill -9 test: appends forever, durable sync every 2
/// pages, and prints "S <pages>" once those are on disk. A no-op unless the
/// parent sets the env var.
#[test]
fn kill9_child() {
    let Ok(path) = std::env::var(KILL_PATH) else {
        return;
    };
    let mut w = BundleWriter::create(Path::new(&path), &key(), AAD).unwrap();
    for i in 0..200_000u32 {
        w.append(&payload(i)).unwrap();
        if (i + 1) % 2 == 0 {
            w.sync(true).unwrap();
            println!("S {}", i + 1);
        }
    }
}

#[test]
fn kill9_loses_at_most_the_unsynced_tail() {
    let exe = std::env::current_exe().unwrap();
    let dir = tempfile::tempdir().unwrap();
    for round in 0..6u32 {
        let path = fresh(&dir, &format!("k{round}.ghb"));
        let mut child = Command::new(&exe)
            .args(["--exact", "kill9_child", "--nocapture", "--test-threads=1"])
            .env(KILL_PATH, &path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let target = 4 + round * 5;
        let mut acked = 0u32;
        while acked < target {
            let line = lines.next().expect("child exited early").unwrap();
            if let Some(n) = line.strip_prefix("S ") {
                acked = n.parse().unwrap();
            }
        }
        // Let it get part-way into the next pages, then SIGKILL.
        std::thread::sleep(Duration::from_millis(u64::from(round) * 2));
        child.kill().unwrap();
        child.wait().unwrap();

        let rec = recover(&path, &key(), AAD).unwrap();
        assert!(!rec.complete);
        assert!(
            rec.pages >= acked,
            "lost synced pages: recovered {} < acked {acked}",
            rec.pages
        );
        let r = BundleReader::open(&path, &key(), AAD).unwrap();
        assert_eq!(r.page_count(), rec.pages);
        for i in 0..rec.pages {
            assert_eq!(r.page(i).unwrap(), payload(i), "page {i} is not authentic");
        }
        println!(
            "kill9 round {round}: acked {acked}, recovered {}, cut {} bytes",
            rec.pages, rec.truncated_bytes
        );
    }
}

#[test]
fn pages_cannot_move_between_files_tracks_or_meetings() {
    let dir = tempfile::tempdir().unwrap();
    let a = fresh(&dir, "a.ghb");
    let b = fresh(&dir, "b.ghb");
    write_bundle(&a, 4, true);
    write_bundle(&b, 4, true);
    let (ab, bb) = (fs::read(&a).unwrap(), fs::read(&b).unwrap());
    let (ra, rb) = (records(&ab), records(&bb));

    // Cross-file (same key, same track id): page 1 of A into B.
    let mut spliced = bb[..rb[1].0].to_vec();
    spliced.extend_from_slice(&ab[ra[1].0..ra[1].1]);
    spliced.extend_from_slice(&bb[rb[1].1..]);
    let q = fresh(&dir, "q.ghb");
    fs::write(&q, spliced).unwrap();
    let r = BundleReader::open(&q, &key(), AAD).unwrap();
    assert!(r.page(1).is_err());
    assert_eq!(r.page(0).unwrap(), payload(0));

    // Header swap: A's header on B's records.
    let mut hdr = ab[..HEADER].to_vec();
    hdr.extend_from_slice(&bb[HEADER..]);
    fs::write(&q, hdr).unwrap();
    let r = BundleReader::open(&q, &key(), AAD).unwrap();
    assert!(r.page(0).is_err() && !r.complete());

    // Flipping a header byte breaks every page.
    let mut h = ab.clone();
    h[10] ^= 1;
    fs::write(&q, h).unwrap();
    assert!(
        BundleReader::open(&q, &key(), AAD)
            .unwrap()
            .page(0)
            .is_err()
    );

    // Other track (mic vs system) and other meeting (other DEK).
    assert!(
        BundleReader::open(&a, &key(), b"track-system-1")
            .unwrap()
            .page(0)
            .is_err()
    );
    assert!(
        BundleReader::open(&a, &Dek::generate(), AAD)
            .unwrap()
            .page(0)
            .is_err()
    );
    // Pages are sealed with a subkey, not the DEK itself.
    let raw = ghi_store::rowcrypt::open(&key(), &ab[ra[0].0 + 4..ra[0].1], AAD);
    assert!(raw.is_err());
}
