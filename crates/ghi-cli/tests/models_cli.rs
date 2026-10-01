// SPDX-License-Identifier: Apache-2.0
//! `ghi models`: offline paths only (no network in tests).

use std::process::Command;

/// A fresh scratch directory, removed on drop (no tempfile dev-dependency here).
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let p = std::env::temp_dir().join(format!("ghi-models-cli-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn ghi(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ghi"))
        .arg("models")
        .args(args)
        .output()
        .expect("run ghi")
}

fn json(out: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).expect("stdout is one JSON document")
}

#[test]
fn status_lists_registry_models_as_not_installed() {
    let dir = Scratch::new("status");
    let out = ghi(&["status", "--dir", dir.path().to_str().unwrap()]);
    assert!(out.status.success());
    let doc = json(&out);
    assert_eq!(doc["schema"], "ghi.models/1");
    assert!(["light", "balanced", "max"].contains(&doc["tier"].as_str().unwrap()));
    assert_eq!(doc["presets"].as_array().unwrap().len(), 3);
    let models = doc["models"].as_array().unwrap();
    assert!(models.iter().any(|m| m["id"] == "qwen3-4b"));
    assert!(
        models
            .iter()
            .all(|m| m["installed"] == false && m["verified"].is_null())
    );
}

#[test]
fn verify_of_a_missing_model_fails_and_unknown_id_is_bad_input() {
    let dir = Scratch::new("verify");
    let d = dir.path().to_str().unwrap();
    let out = ghi(&["verify", "qwen3-4b", "--dir", d]);
    assert!(!out.status.success());
    assert_eq!(json(&out)["results"][0]["ok"], false);
    let out = ghi(&["verify", "nope", "--dir", d]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn import_of_an_unknown_file_is_refused() {
    let dir = Scratch::new("import");
    let file = dir.path().join("x.gguf");
    std::fs::write(&file, b"not a model").unwrap();
    let out = ghi(&[
        "import",
        file.to_str().unwrap(),
        "--dir",
        dir.path().join("m").to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("bad_input"));
}

#[test]
fn strict_offline_fetch_is_refused_before_any_connection() {
    let dir = Scratch::new("fetch");
    let out = ghi(&[
        "fetch",
        "qwen3-4b",
        "--strict-offline",
        "--dir",
        dir.path().to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("strict offline"));
}
