// SPDX-License-Identifier: Apache-2.0
//! `ghi voice` (feature `voice`): consent is required; `score` and
//! `enroll` run against the real speaker model when it is installed.
#![cfg(feature = "voice")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ghi(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ghi"))
        .arg("voice")
        .args(args)
        .env("GHI_MODELS_DIR", root().join("models"))
        .output()
        .expect("run ghi")
}

#[test]
fn enroll_without_consent_is_refused_before_anything_is_read() {
    let out = ghi(&[
        "enroll",
        "--dir",
        "/nonexistent/ghi",
        "--wav",
        "/nonexistent.wav",
    ]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--consent is required"), "{err}");
}

#[test]
fn score_of_a_recording_with_itself_is_one() {
    let wav = root().join("tools/eval/data/voxconverse/audio/msbyq.wav");
    let model = root().join("models/3dspeaker_speech_campplus_sv_zh_en_16k-common_advanced.onnx");
    if !wav.exists() || !model.exists() {
        eprintln!("skipped: no eval audio or speaker model");
        return;
    }
    let w = wav.to_str().unwrap();
    let out = ghi(&["score", w, w]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["schema"], "ghi.voice-score/1");
    assert!(doc["cosine"].as_f64().unwrap() > 0.9999, "{doc}");
}
