// SPDX-License-Identifier: Apache-2.0
//! End to end: a panic with private data in its payload writes a report that
//! has none of it. One test fn, since the panic hook is process-global.

use std::panic::catch_unwind;

const KEY: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
const SENTENCE: &str = "Chúng ta sẽ chốt ngân sách quý bốn vào thứ Sáu";

fn fails() -> Result<(), String> {
    Err(std::hint::black_box(SENTENCE).to_owned())
}

fn context() -> String {
    "recording".into()
}

fn reports(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains("-ghira-test"))
        .map(|e| std::fs::read_to_string(e.path()).unwrap())
        .collect();
    v.sort();
    v
}

#[test]
fn panic_writes_a_scrubbed_report() {
    let dir = tempfile::tempdir().unwrap();
    ghi_diag::install_panic_hook("ghira-test", dir.path().to_path_buf(), context);

    let _ = catch_unwind(|| {
        panic!("store open failed for key {KEY}");
    });
    let _ = catch_unwind(|| {
        let r = fails();
        r.expect("segment text rejected");
    });
    let _ = catch_unwind(|| {
        let r = fails();
        r.unwrap();
    });

    let all = reports(dir.path());
    assert_eq!(all.len(), 3, "one report per panic");
    for body in &all {
        assert!(!body.contains(KEY), "key leaked:\n{body}");
        assert!(!body.contains("9f86d081"), "key prefix leaked:\n{body}");
        assert!(!body.contains("ngân sách"), "transcript leaked:\n{body}");
        assert!(!body.contains("Chúng"), "transcript leaked:\n{body}");
        assert!(body.contains("ghira crash report"));
        assert!(body.contains("state: recording"));
        assert!(body.contains("thread: "));
        assert!(
            body.contains("location: crates/ghi-diag/tests/panic_report.rs:")
                || body.contains("panic_report.rs:")
        );
        assert!(body.contains("backtrace:"));
        assert!(body.contains("uptime_s: "));
    }
    assert!(
        all.iter()
            .any(|b| b.contains("message: store open failed for key <hex>"))
    );
}
