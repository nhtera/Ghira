// SPDX-License-Identifier: Apache-2.0
//! Audio pages (`TrackPages`) into the receiver's `RawImport`: a genuine
//! sealed bundle, damaged by the fuzzer (flipped bytes, dropped, repeated or
//! swapped records, a cut tail) and pushed in batches. A corrupted record is
//! never accepted: the verified prefix is exactly the leading records that
//! are unchanged, a complete import is byte-identical to the original, and
//! nothing panics.
#![no_main]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use arbitrary::Arbitrary;
use ghi_store::bundle::{raw_header, raw_records, BundleWriter, RawBegin, RawImport};
use ghi_store::rowcrypt::Dek;
use libfuzzer_sys::fuzz_target;

const PAGES: usize = 24;
const AAD: &[u8] = b"fuzz-track";

struct Genuine {
    dir: tempfile::TempDir,
    key: Dek,
    header: Vec<u8>,
    records: Vec<Vec<u8>>,
    file: Vec<u8>,
}

fn genuine() -> &'static Genuine {
    static G: OnceLock<Genuine> = OnceLock::new();
    G.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let key = Dek::from_bytes([42; 32]);
        let path = dir.path().join("genuine.ghb");
        let mut w = BundleWriter::create(&path, &key, AAD).unwrap();
        for i in 0..PAGES {
            w.append(&vec![i as u8; 200 + i * 13]).unwrap();
        }
        w.finish().unwrap();
        drop(w);
        Genuine {
            header: raw_header(&path).unwrap(),
            records: raw_records(&path, 0, 1000).unwrap(),
            file: std::fs::read(&path).unwrap(),
            key,
            dir,
        }
    })
}

#[derive(Debug, Arbitrary)]
struct Case {
    /// `(record, offset, xor)`: bytes flipped inside records.
    flips: Vec<(u8, u16, u8)>,
    /// A record repeated in place of the next one.
    repeat: Option<u8>,
    /// Two records swapped.
    swap: Option<(u8, u8)>,
    /// Records dropped from the end.
    cut_tail: u8,
    /// Records per push.
    batch: u8,
}

fn dest(dir: &Path) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    dir.join(format!(
        "recv-{}.ghb",
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}

fuzz_target!(|case: Case| {
    let g = genuine();
    let mut records = g.records.clone();
    for (r, off, xor) in &case.flips {
        let r = *r as usize % records.len();
        if !records[r].is_empty() {
            let at = *off as usize % records[r].len();
            records[r][at] ^= xor;
        }
    }
    if let Some(r) = case.repeat {
        let r = r as usize % records.len();
        if r + 1 < records.len() {
            records[r + 1] = records[r].clone();
        }
    }
    if let Some((a, b)) = case.swap {
        let (a, b) = (a as usize % records.len(), b as usize % records.len());
        records.swap(a, b);
    }
    let keep = records.len().saturating_sub(case.cut_tail as usize % 4);
    records.truncate(keep);
    // The leading records that are exactly the genuine ones.
    let leading = records
        .iter()
        .zip(&g.records)
        .take_while(|(a, b)| a == b)
        .count();

    let out = dest(g.dir.path());
    let RawBegin::Resume(mut import) = RawImport::begin(&out, &g.key, AAD, &g.header).unwrap()
    else {
        panic!("nothing was there to be complete");
    };
    let batch = (case.batch as usize % 8) + 1;
    for chunk in records.chunks(batch) {
        if import.push(chunk).is_err() {
            break;
        }
    }
    let verified = import.have() as usize + usize::from(import.is_complete());
    assert_eq!(
        verified, leading,
        "the verified prefix is not exactly the unchanged leading records"
    );
    if import.is_complete() {
        import.finish().unwrap();
        assert_eq!(
            std::fs::read(&out).unwrap(),
            g.file,
            "a finished import differs"
        );
        let _ = std::fs::remove_file(&out);
    } else {
        drop(import);
        let _ = std::fs::remove_file(ghi_store::bundle::part_path(&out));
    }
    let _ = std::fs::remove_file(ghi_store::bundle::index_path(&out));
});
