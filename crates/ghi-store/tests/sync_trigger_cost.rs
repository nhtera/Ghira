// SPDX-License-Identifier: Apache-2.0
//! Cost of the `sync_log` triggers on live segment inserts (phase 15 risk "M x M":
//! budget under 5% of the persist path). A bench, not a gate: run it by hand,
//!
//! ```text
//! cargo test -p ghi-store --release --test sync_trigger_cost -- --ignored --nocapture
//! ```
//!
//! It inserts the same segments into two stores, one as migrated and one with
//! every `sync_log_*` trigger dropped from its database, alternating rounds,
//! and prints the median throughput of each and the overhead.

mod common;

use std::path::Path;
use std::time::Instant;

use ghi_store::db;
use ghi_store::store::{NewMeeting, NewSegment, Store};

const SEGMENTS: usize = 1_500;
const ROUNDS: usize = 7;

/// A store whose `sync_log_*` triggers are gone (what a build without sync
/// would persist through).
fn without_triggers(dir: &Path) -> Store {
    let (store, keys) = common::open(dir);
    drop(store);
    let conn = db::open(&dir.join("ghira.db"), &common::db_key(&keys)).unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger' AND name LIKE 'sync_log_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(names.len() >= 28, "{} triggers found", names.len());
    for n in &names {
        conn.execute_batch(&format!("DROP TRIGGER {n}")).unwrap();
    }
    drop(conn);
    common::reopen(dir, &keys)
}

fn line(i: usize) -> NewSegment {
    NewSegment {
        t0_ms: i as i64 * 1_000,
        t1_ms: i as i64 * 1_000 + 900,
        text: format!("Dòng số {i}: chúng ta chốt lịch beta vào thứ Sáu tuần sau."),
        ..Default::default()
    }
}

/// Seconds to insert `SEGMENTS` lines into a new meeting: one transaction per
/// line (the live pipeline's persist) or in batches of 100 (a final pass or an
/// import).
fn run(store: &Store, tag: usize, batched: bool) -> f64 {
    let m = store
        .create_meeting(NewMeeting {
            title: format!("bench {tag}"),
            ..Default::default()
        })
        .unwrap();
    let t = Instant::now();
    if batched {
        for chunk in (0..SEGMENTS).collect::<Vec<_>>().chunks(100) {
            store
                .add_segments(&m.gid, chunk.iter().map(|i| line(*i)).collect())
                .unwrap();
        }
    } else {
        for i in 0..SEGMENTS {
            store.add_segment(&m.gid, line(i)).unwrap();
        }
    }
    t.elapsed().as_secs_f64()
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

/// Median seconds with and without triggers, alternating rounds.
fn compare(with: &Store, without: &Store, batched: bool) -> (f64, f64) {
    run(with, 0, batched);
    run(without, 0, batched);
    let (mut w, mut wo) = (Vec::new(), Vec::new());
    for round in 1..=ROUNDS {
        // Alternate which goes first so drift does not favour either.
        if round % 2 == 0 {
            w.push(run(with, round, batched));
            wo.push(run(without, round, batched));
        } else {
            wo.push(run(without, round, batched));
            w.push(run(with, round, batched));
        }
    }
    (median(w), median(wo))
}

#[test]
#[ignore = "bench: run by hand with --release --ignored --nocapture"]
fn sync_log_triggers_cost_on_segment_inserts() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (with, _keys) = common::open(&a.path().join("data"));
    let without = without_triggers(&b.path().join("data"));
    for (what, batched) in [
        ("one transaction per line (live persist)", false),
        ("batches of 100 (final pass, import)", true),
    ] {
        let (tw, two) = compare(&with, &without, batched);
        let added_us = (tw - two) / SEGMENTS as f64 * 1e6;
        println!(
            "{what}: {SEGMENTS} segments, median of {ROUNDS}: with triggers {:.0} seg/s, without {:.0} seg/s, \
             overhead {:.1}% = {added_us:.0} us per segment",
            SEGMENTS as f64 / tw,
            SEGMENTS as f64 / two,
            (tw - two) / two * 100.0,
        );
        // The live pipeline persists a handful of lines per second: what
        // counts is the absolute cost of one more row, well under a
        // millisecond. (The relative figure is large because the insert
        // itself is cheap.)
        assert!(
            added_us < 1_000.0,
            "{what}: the triggers add {added_us:.0} us per segment"
        );
    }
}
