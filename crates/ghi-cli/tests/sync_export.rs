// SPDX-License-Identifier: Apache-2.0
//! `ghi sync export|import` (slice 15-M): a passphrase-sealed file from one
//! data directory into another, driven through the binary.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::Value;

const PASS: &str = "correct horse battery";

fn ghi(args: &[&str], pass: Option<&str>, stdin: Option<&str>) -> Output {
    use std::io::Write;
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ghi"));
    cmd.args(args)
        .env_remove("GHI_KEYSTORE")
        .env_remove("GHI_EXPORT_PASSPHRASE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(p) = pass {
        cmd.env("GHI_EXPORT_PASSPHRASE", p);
    }
    let mut child = cmd.spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    if let Some(s) = stdin {
        input.write_all(s.as_bytes()).unwrap();
    }
    drop(input);
    child.wait_with_output().unwrap()
}

fn json(out: Output) -> Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// A finished meeting with one second of audio, recorded from a replayed file.
fn record(root: &Path, dir: &str, title: &str) {
    let wav = root.join("tone.wav");
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(&wav, spec).unwrap();
    for i in 0..16_000 {
        let t = i as f32 / 16_000.0;
        w.write_sample(((t * 440.0 * std::f32::consts::TAU).sin() * 8000.0) as i16)
            .unwrap();
    }
    w.finalize().unwrap();
    let wav = wav.to_str().unwrap();
    json(ghi(
        &[
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
            title,
        ],
        None,
        None,
    ));
}

fn gids(dir: &str) -> Vec<String> {
    let list = json(ghi(&["store", "--dir", dir, "list"], None, None));
    list["meetings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["gid"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn export_and_import_between_two_data_directories() {
    let root = std::env::temp_dir().join(format!("ghi-cli-{}-sync-export", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let (a, b) = (root.join("a"), root.join("b"));
    let (a, b) = (a.to_str().unwrap(), b.to_str().unwrap());
    let file = root.join("f.ghix");
    let file = file.to_str().unwrap();

    for title in ["Họp tuần", "Họp nhóm"] {
        record(&root, a, title);
    }
    let all = gids(a);
    assert_eq!(all.len(), 2);

    // Only the chosen meeting; the passphrase from the environment.
    let out = json(ghi(
        &[
            "sync",
            "export",
            "--dir",
            a,
            "--out",
            file,
            "--meeting",
            &all[0],
        ],
        Some(PASS),
        None,
    ));
    assert_eq!(out["schema"], "ghi.sync-export/1");
    assert_eq!(out["meetings"], 1);

    // A wrong passphrase changes nothing and is the user's error (exit 1).
    let bad = ghi(
        &["sync", "import", "--dir", b, file],
        Some("wrong wrong"),
        None,
    );
    assert_eq!(bad.status.code(), Some(1));
    assert!(!String::from_utf8_lossy(&bad.stderr).contains("wrong wrong"));
    assert!(!Path::new(b).join("bundles").exists() || gids(b).is_empty());

    let imp = json(ghi(&["sync", "import", "--dir", b, file], Some(PASS), None));
    assert_eq!(imp["schema"], "ghi.sync-import/1");
    assert_eq!(
        (imp["meetings"].as_u64(), imp["refused"].as_u64()),
        (Some(1), Some(0))
    );
    assert_eq!(gids(b), vec![all[0].clone()]);

    // Everything, the passphrase from stdin (no terminal, no variable); twice
    // is the same as once.
    std::fs::remove_file(file).unwrap();
    json(ghi(
        &["sync", "export", "--dir", a, "--out", file],
        None,
        Some(&format!("{PASS}\n")),
    ));
    for _ in 0..2 {
        json(ghi(&["sync", "import", "--dir", b, file], Some(PASS), None));
        assert_eq!(gids(b).len(), 2);
    }

    // A short passphrase is refused and nothing is written.
    let short = root.join("short.ghix");
    let r = ghi(
        &[
            "sync",
            "export",
            "--dir",
            a,
            "--out",
            short.to_str().unwrap(),
        ],
        Some("short"),
        None,
    );
    assert_eq!(r.status.code(), Some(1));
    assert!(!short.exists());
    std::fs::remove_dir_all(&root).unwrap();
}
