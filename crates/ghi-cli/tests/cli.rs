// SPDX-License-Identifier: Apache-2.0
//! Runs the `ghi` binary and checks exit codes and output against the contract.

use std::process::{Command, Output};

use serde_json::Value;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tools/eval/tests/fixtures/cli/"
);

fn ghi(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ghi"))
        .args(args)
        .output()
        .expect("run ghi")
}

/// The last stderr line must be a `ghi.error/1` document.
fn error_doc(out: &Output) -> Value {
    let stderr = String::from_utf8(out.stderr.clone()).unwrap();
    let last = stderr.lines().last().expect("stderr is empty");
    let doc: Value = serde_json::from_str(last).expect("last stderr line is JSON");
    assert_eq!(doc["schema"], "ghi.error/1");
    doc
}

#[test]
fn version_json() {
    let out = ghi(&["version", "--json"]);
    assert!(out.status.success());
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["schema"], "ghi.version/1");
    assert_eq!(doc["ghi"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn version_flag_still_prints_text() {
    let out = ghi(&["--version"]);
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(text.trim(), format!("ghi {}", env!("CARGO_PKG_VERSION")));
}

/// A short 16 kHz mono WAV (a 440 Hz tone) in the temp dir.
fn tone_wav(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("ghi-cli-{}-{name}.wav", std::process::id()));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&path, spec).unwrap();
    for i in 0..16_000 {
        let t = i as f32 / 16_000.0;
        w.write_sample(((t * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16)
            .unwrap();
    }
    w.finalize().unwrap();
    path
}

#[test]
fn notes_is_not_implemented_yet() {
    let out = ghi(&["notes", &format!("{FIXTURES}transcript.json"), "--json"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(out.stdout.is_empty());
    assert_eq!(error_doc(&out)["code"], "not_implemented");
}

/// Without the `nemo` feature the engine commands exit 3 `engine_unavailable`.
#[cfg(not(feature = "nemo"))]
#[test]
fn engine_commands_need_the_nemo_feature() {
    let wav = tone_wav("noengine");
    let audio = wav.to_str().unwrap();
    for args in [
        vec!["transcribe", audio, "--json"],
        vec![
            "transcribe",
            audio,
            "--stream",
            "--realtime",
            "--lang",
            "vi",
        ],
        vec![
            "diarize",
            audio,
            "--pass",
            "live",
            "--max-speakers",
            "8",
            "--json",
        ],
        vec!["bench", audio, "--json"],
    ] {
        let out = ghi(&args);
        assert_eq!(out.status.code(), Some(3), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        assert_eq!(error_doc(&out)["code"], "engine_unavailable", "{args:?}");
    }
    let _ = std::fs::remove_file(wav);
}

/// With the engines and the pinned models present, every engine command
/// prints a document of the right schema. Skipped when the models are missing.
#[cfg(feature = "nemo")]
#[test]
fn engine_commands_run_with_models() {
    let models = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models");
    if !models
        .join("nemotron-3.5-asr-streaming-0.6b.q8_0.gguf")
        .is_file()
    {
        eprintln!("skipped: run tools/scripts/fetch-models.sh");
        return;
    }
    let wav = tone_wav("engine");
    let audio = wav.to_str().unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_ghi"))
            .args(args)
            .env("GHI_MODELS_DIR", &models)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    let doc: Value = serde_json::from_str(&run(&["transcribe", audio, "--json"])).unwrap();
    assert_eq!(doc["schema"], "ghi.transcript/1");
    let doc: Value = serde_json::from_str(&run(&["diarize", audio, "--json"])).unwrap();
    assert_eq!(doc["schema"], "ghi.diarization/1");
    let doc: Value = serde_json::from_str(&run(&["bench", audio, "--topology", "call"])).unwrap();
    assert_eq!(doc["schema"], "ghi.bench/1");
    let stream = run(&["transcribe", audio, "--stream"]);
    let last: Value = serde_json::from_str(stream.lines().last().unwrap()).unwrap();
    assert_eq!(last["type"], "end");
    let _ = std::fs::remove_file(wav);
}

#[test]
fn bad_input_exits_1() {
    let out = ghi(&["transcribe", "no-such-file.wav", "--json"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(error_doc(&out)["code"], "bad_input");

    // Not a WAV file.
    let out = ghi(&["diarize", &format!("{FIXTURES}version.json"), "--json"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(error_doc(&out)["code"], "bad_input");

    // `notes` needs a ghi.transcript/1 document.
    let out = ghi(&["notes", &format!("{FIXTURES}notes.json"), "--json"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(error_doc(&out)["code"], "bad_input");
}

#[test]
fn usage_errors_exit_2() {
    for args in [
        vec![],
        vec!["transcribe"],
        vec!["transcribe", "a.wav", "--realtime"],
        vec!["diarize", "a.wav", "--max-speakers", "0"],
        vec!["transcribe", "a.wav", "--lang", "fr"],
    ] {
        assert_eq!(ghi(&args).status.code(), Some(2), "{args:?}");
    }
}
