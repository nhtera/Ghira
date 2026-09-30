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

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ghi-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn record_replay_writes_eval_layout() {
    let wav = tone_wav("record-replay");
    let wav = wav.to_str().unwrap();
    let out_dir = temp_dir("record-wav");
    let replay = format!("{wav},{wav}");
    let out = ghi(&[
        "record",
        "--mode",
        "call",
        "--replay",
        &replay,
        "--out",
        out_dir.to_str().unwrap(),
        "--id",
        "r1",
        "--json",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["schema"], "ghi.record/1");
    assert_eq!(doc["stopped"], "source_ended");
    assert_eq!(doc["mix"], "r1.wav");
    for name in [
        "r1.wav",
        "r1.mic.wav",
        "r1.system.wav",
        "r1.session.json",
        "r1.markers.jsonl",
    ] {
        assert!(out_dir.join(name).is_file(), "{name} missing");
    }
    // The recording is valid input for the engine commands' WAV reader.
    let mic = hound::WavReader::open(out_dir.join("r1.mic.wav")).unwrap();
    assert_eq!(mic.spec().sample_rate, 16_000);
    let secs = mic.duration() as f64 / 16_000.0;
    assert!((secs - 1.0).abs() < 0.05, "{secs}");
    std::fs::remove_dir_all(&out_dir).unwrap();
}

#[test]
fn record_opus_then_recover() {
    let wav = tone_wav("record-opus");
    let out_dir = temp_dir("record-opus");
    let out = ghi(&[
        "record",
        "--mode",
        "room",
        "--format",
        "opus",
        "--replay",
        wav.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
        "--id",
        "o1",
        "--duration",
        "0.5",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["stopped"], "duration");
    assert_eq!(doc["tracks"].as_array().unwrap().len(), 1);

    let out = ghi(&["recover", out_dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["schema"], "ghi.recover/1");
    let file = &doc["files"][0];
    assert_eq!(file["file"], "o1.mic.opus");
    assert_eq!(file["complete"], true);
    let secs = file["duration_s"].as_f64().unwrap();
    assert!((0.45..=0.6).contains(&secs), "{secs}");
    assert!(out_dir.join("o1.mic.recovered.wav").is_file());
    std::fs::remove_dir_all(&out_dir).unwrap();
}

#[test]
fn record_rejects_bad_arguments() {
    let wav = tone_wav("record-bad");
    let wav = wav.to_str().unwrap();
    let out_dir = temp_dir("record-bad");
    let dir = out_dir.to_str().unwrap();
    // Call mode needs two replay files.
    let out = ghi(&["record", "--mode", "call", "--replay", wav, "--out", dir]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(error_doc(&out)["code"], "bad_input");
    // Ids become file names: a bad one is a usage error.
    let out = ghi(&[
        "record", "--mode", "room", "--replay", wav, "--out", dir, "--id", "../x",
    ]);
    assert_eq!(out.status.code(), Some(2));
    let out = ghi(&[
        "record",
        "--mode",
        "room",
        "--replay",
        wav,
        "--out",
        dir,
        "--duration",
        "0",
    ]);
    assert_eq!(out.status.code(), Some(2));
    // An existing recording is never overwritten.
    let run = || {
        ghi(&[
            "record", "--mode", "room", "--replay", wav, "--out", dir, "--id", "same",
        ])
    };
    assert!(run().status.success());
    let out = run();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(error_doc(&out)["code"], "bad_input");
    std::fs::remove_dir_all(&out_dir).unwrap();
}

#[test]
fn recover_reports_bad_files_and_goes_on() {
    let wav = tone_wav("recover-bad");
    let out_dir = temp_dir("recover-bad");
    let dir = out_dir.to_str().unwrap();
    let out = ghi(&[
        "record",
        "--mode",
        "room",
        "--format",
        "opus",
        "--replay",
        wav.to_str().unwrap(),
        "--out",
        dir,
        "--id",
        "good",
        "--duration",
        "0.3",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::write(out_dir.join("bad.mic.opus"), b"not ogg").unwrap();
    let out = ghi(&["recover", dir]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    let files = doc["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert!(files[0]["error"].is_string(), "{doc}");
    assert_eq!(files[1]["file"], "good.mic.opus");
    assert!(files[1]["error"].is_null());
    std::fs::remove_dir_all(&out_dir).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn detect_lists_audio_processes() {
    let out = ghi(&["detect"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let doc: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(doc["schema"], "ghi.detect/1");
    assert!(doc["processes"].is_array());
}

#[cfg(not(target_os = "macos"))]
#[test]
fn capture_is_unavailable_off_macos() {
    let out_dir = temp_dir("record-capture");
    let out = ghi(&[
        "record",
        "--out",
        out_dir.to_str().unwrap(),
        "--duration",
        "1",
    ]);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(error_doc(&out)["code"], "capture_unavailable");
    std::fs::remove_dir_all(&out_dir).unwrap();
}

#[test]
fn store_record_transcript_search_export_delete() {
    use std::io::Write;
    let wav = tone_wav("store");
    let wav = wav.to_str().unwrap();
    let root = temp_dir("store");
    let data = root.join("data");
    let dir = data.to_str().unwrap();
    let json = |out: Output| -> Value {
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let stdin_ghi = |args: &[&str], input: &str| -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ghi"))
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };

    // Record a call into the store (debug builds use a key file, not the Keychain).
    let rec = json(ghi(&[
        "record",
        "--mode",
        "call",
        "--format",
        "store",
        "--replay",
        &format!("{wav},{wav}"),
        "--out",
        dir,
        "--title",
        "Họp tuần",
    ]));
    let gid = rec["id"].as_str().unwrap().to_owned();
    assert!(
        rec["tracks"][0]["file"]
            .as_str()
            .unwrap()
            .ends_with("mic.ghb")
    );
    let list = json(ghi(&["store", "--dir", dir, "list"]));
    assert_eq!(list["meetings"][0]["title"], "Họp tuần");
    assert_eq!(list["meetings"][0]["tracks"].as_array().unwrap().len(), 2);

    // Audio comes back out of the encrypted bundle.
    let out_wav = root.join("mic.wav");
    let audio = json(ghi(&[
        "store",
        "--dir",
        dir,
        "audio",
        &gid,
        "--out",
        out_wav.to_str().unwrap(),
    ]));
    assert_eq!(audio["complete"], true);
    assert!((audio["duration_s"].as_f64().unwrap() - 1.0).abs() < 0.05);

    // Transcript → accent-insensitive search with highlights on the original.
    let t = root.join("t.json");
    std::fs::write(&t, r#"{"schema":"ghi.transcript/1","audio":"a.wav","duration_s":1.0,"pass":"final","lang":"auto","engine":{"name":"t","version":"0"},"segments":[{"id":0,"start":0.0,"end":1.0,"text":"Chốt kế hoạch ở Đà Nẵng","lang":"vi","speaker":null,"words":null}],"perf":{"wall_s":0.0,"rtf":null,"peak_rss_mb":null}}"#).unwrap();
    json(ghi(&[
        "store",
        "--dir",
        dir,
        "add-transcript",
        t.to_str().unwrap(),
        "--meeting",
        &gid,
    ]));
    let hits = json(ghi(&["store", "--dir", dir, "search", "da nang"]));
    let hit = &hits["hits"][0];
    let snippet: Vec<char> = hit["snippet"].as_str().unwrap().chars().collect();
    let [a, b] = [0, 1].map(|i| hit["highlights"][0][i].as_u64().unwrap() as usize);
    assert_eq!(snippet[a..b].iter().collect::<String>(), "Đà Nẵng");

    // Export with a password, restore elsewhere; a wrong password fails.
    let archive = root.join("all.ghx");
    let arch = archive.to_str().unwrap();
    json(stdin_ghi(
        &["store", "--dir", dir, "export", "--out", arch],
        "pw 1\n",
    ));
    let restored = root.join("restored");
    let bad = stdin_ghi(
        &["store", "--dir", restored.to_str().unwrap(), "import", arch],
        "nope\n",
    );
    assert_eq!(bad.status.code(), Some(1));
    let imp = json(stdin_ghi(
        &["store", "--dir", restored.to_str().unwrap(), "import", arch],
        "pw 1\n",
    ));
    assert_eq!(imp["meetings"], 1);

    // Delete: gone from list, search and audio.
    json(ghi(&["store", "--dir", dir, "delete", &gid]));
    assert_eq!(
        json(ghi(&["store", "--dir", dir, "list"]))["meetings"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        json(ghi(&["store", "--dir", dir, "search", "chot"]))["hits"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let gone = ghi(&[
        "store",
        "--dir",
        dir,
        "audio",
        &gid,
        "--out",
        out_wav.to_str().unwrap(),
    ]);
    assert_eq!(gone.status.code(), Some(1));
    std::fs::remove_dir_all(&root).unwrap();
}
