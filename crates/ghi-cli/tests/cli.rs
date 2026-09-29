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

#[test]
fn engine_commands_are_not_implemented_yet() {
    // Any existing file will do: the stubs only check that the input exists.
    let audio = format!("{FIXTURES}version.json");
    let transcript = format!("{FIXTURES}transcript.json");
    for args in [
        vec!["transcribe", audio.as_str(), "--json"],
        vec![
            "transcribe",
            audio.as_str(),
            "--stream",
            "--realtime",
            "--lang",
            "vi",
        ],
        vec![
            "diarize",
            audio.as_str(),
            "--pass",
            "live",
            "--max-speakers",
            "8",
            "--json",
        ],
        vec!["notes", transcript.as_str(), "--json"],
        vec!["bench", audio.as_str(), "--json"],
    ] {
        let out = ghi(&args);
        assert_eq!(out.status.code(), Some(3), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        assert_eq!(error_doc(&out)["code"], "not_implemented", "{args:?}");
    }
}

#[test]
fn bad_input_exits_1() {
    let out = ghi(&["transcribe", "no-such-file.wav", "--json"]);
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
