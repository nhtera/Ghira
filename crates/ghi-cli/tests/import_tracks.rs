// SPDX-License-Identifier: Apache-2.0
//! `ghi import --tracks`: a Zoom meeting folder's participant tracks become
//! one meeting with the participants as speakers (no speech models needed).

use std::process::Command;

fn wav(path: &std::path::Path, hz: f32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for i in 0..16_000 * 4 {
        let x = (i as f32 * hz * std::f32::consts::TAU / 16_000.0).sin() * 0.2;
        w.write_sample((x * 32_767.0) as i16).unwrap();
    }
    w.finalize().unwrap();
}

/// The summary is the last line (events come before it).
fn last_doc(stdout: &[u8]) -> serde_json::Value {
    let text = String::from_utf8_lossy(stdout);
    serde_json::from_str(text.lines().rfind(|l| !l.trim().is_empty()).unwrap()).unwrap()
}

#[test]
fn a_zoom_folder_imports_as_one_meeting_with_named_speakers() {
    let tmp = tempfile::tempdir().unwrap();
    let rec = tmp
        .path()
        .join("2026-07-03 14.05.02 Sprint 81234567890/Audio Record");
    std::fs::create_dir_all(&rec).unwrap();
    wav(&rec.join("audioLinh1111.wav"), 300.0);
    wav(&rec.join("audioMinh2222.wav"), 700.0);
    let store = tmp.path().join("store");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ghi"))
            .args(args)
            .output()
            .unwrap()
    };
    let out = run(&[
        "import",
        "--tracks",
        tmp.path()
            .join("2026-07-03 14.05.02 Sprint 81234567890")
            .to_str()
            .unwrap(),
        "--dir",
        store.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc = last_doc(&out.stdout);
    assert_eq!(doc["schema"], "ghi.import/1");
    assert_eq!(doc["tracks"], 2);
    assert_eq!(doc["duplicate"], false);
    // Again: a duplicate. A file and --tracks together are refused.
    let out = run(&[
        "import",
        "--tracks",
        rec.to_str().unwrap(),
        "--dir",
        store.to_str().unwrap(),
    ]);
    let doc = last_doc(&out.stdout);
    assert_eq!(doc["duplicate"], true);
    let out = run(&[
        "import",
        "a.wav",
        "--tracks",
        rec.to_str().unwrap(),
        "--dir",
        store.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
}

#[test]
fn a_flat_folder_leaves_out_the_mixed_recording_and_lists_each_file_once() {
    let tmp = tempfile::tempdir().unwrap();
    let flat = tmp.path().join("tracks");
    std::fs::create_dir_all(&flat).unwrap();
    wav(&flat.join("audioLinh1111.wav"), 300.0);
    wav(&flat.join("audioMinh2222.wav"), 700.0);
    wav(&flat.join("audio_only.wav"), 500.0);
    let store = tmp.path().join("store");
    // The folder, a file inside it again, and the mixed file by name.
    let out = Command::new(env!("CARGO_BIN_EXE_ghi"))
        .args(["import", "--tracks"])
        .arg(&flat)
        .arg(flat.join("audioLinh1111.wav"))
        .arg(flat.join("audio_only.wav"))
        .arg("--dir")
        .arg(&store)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc = last_doc(&out.stdout);
    assert_eq!(doc["tracks"], 2, "two participants, not four files: {doc}");
}
