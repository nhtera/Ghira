// SPDX-License-Identifier: Apache-2.0
//! `ghi session` end to end. Strict offline (phase 8): a whole meeting —
//! live transcript, stop, notes from the live transcript, final pass, final
//! notes — runs under a sandbox that denies every socket. Needs the speech
//! models, the local LLM, the worker and a speech clip from the eval data;
//! skipped without them.

#[cfg(feature = "nemo")]
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(feature = "nemo")]
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[cfg(not(feature = "nemo"))]
#[test]
fn without_speech_engines_a_session_is_engine_unavailable_or_records_only() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("a.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&wav, spec).unwrap();
    for _ in 0..16_000 {
        w.write_sample(0i16).unwrap();
    }
    w.finalize().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ghi"))
        .args(["session", "--dir"])
        .arg(dir.path().join("store"))
        .arg("--replay")
        .arg(&wav)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // As the app does while the models are missing: audio only, the jobs wait.
    let out = Command::new(env!("CARGO_BIN_EXE_ghi"))
        .args(["session", "--record-only", "--speed", "0", "--dir"])
        .arg(dir.path().join("store"))
        .arg("--replay")
        .arg(&wav)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let doc: serde_json::Value = serde_json::from_str(stdout.lines().last().unwrap()).unwrap();
    assert_eq!(doc["schema"], "ghi.session/1");
    assert_eq!(doc["lines"], 0);
    assert_eq!(
        doc["status"], "processing",
        "jobs queued for when models arrive"
    );
    assert!(doc["duration_s"].as_f64().unwrap() > 0.9, "{doc}");
    assert!(stdout.contains("\"modelsMissing\""), "{stdout}");
}

#[cfg(all(feature = "nemo", target_os = "macos"))]
#[test]
fn a_whole_meeting_runs_with_the_network_denied() {
    let root = root();
    let worker = root.join("target/debug/ghi-llm-worker");
    let models = std::env::var_os("GHI_MODELS_DIR").map_or(root.join("models"), PathBuf::from);
    let clip = root.join("tools/eval/data/fleurs-vi/audio/fleurs_10259532564282505455.wav");
    let needed = [
        worker.clone(),
        models.join("Qwen3-4B-Q4_K_M.gguf"),
        models.join("nemotron-3.5-asr-streaming-0.6b.q8_0.gguf"),
        models.join("Nemotron-3-Diarization.q8_0.gguf"),
        clip.clone(),
    ];
    if let Some(missing) = needed.iter().find(|p| !p.is_file()) {
        eprintln!("skipped: {} is missing", missing.display());
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new("/usr/bin/sandbox-exec")
        .args(["-p", "(version 1)(allow default)(deny network*)"])
        .arg(env!("CARGO_BIN_EXE_ghi"))
        .args(["session", "--speed", "0", "--process", "--dir"])
        .arg(dir.path().join("store"))
        .arg("--replay")
        .arg(&clip)
        .env("GHI_LLM_WORKER", &worker)
        .env("GHI_MODELS_DIR", &models)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let summary = lines.last().unwrap();
    assert_eq!(summary["schema"], "ghi.session/1");
    assert_eq!(summary["status"], "ready", "{summary}");
    assert_eq!(summary["transcript_version"], 2);
    assert!(summary["lines"].as_u64().unwrap() > 0);
    for job in summary["jobs"].as_array().unwrap() {
        assert_eq!(job["result"], "done", "{job}");
    }
    let errors: Vec<&serde_json::Value> = lines
        .iter()
        .filter(|l| l["event"]["type"] == "error")
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    assert!(
        lines
            .iter()
            .any(|l| l["event"]["type"] == "transcriptFinal")
    );
}
