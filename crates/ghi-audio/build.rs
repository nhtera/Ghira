// SPDX-License-Identifier: Apache-2.0
//! On macOS, builds the Swift capture package (`native/macos/GhiAudioMac`) as a
//! static library with `swift build` and links it with the Swift runtime and
//! the Core Audio frameworks. Other targets build nothing here.
//!
//! Needs the Xcode (or Command Line Tools) Swift toolchain on macOS.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let package = manifest.join("../../native/macos/GhiAudioMac");
    for path in ["Package.swift", "Sources/GhiAudioMac", "include"] {
        println!("cargo:rerun-if-changed={}", package.join(path).display());
    }
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        other => panic!("unsupported macOS arch {other:?}"),
    };
    let scratch = PathBuf::from(env::var("OUT_DIR").unwrap()).join("swift");

    // Always an optimized build: the IO block runs on a realtime thread.
    let swift_build = |extra: &[&str]| {
        let mut cmd = Command::new("swift");
        cmd.args([
            "build",
            "-c",
            "release",
            "--product",
            "GhiAudioMac",
            "--arch",
            arch,
        ])
        .arg("--package-path")
        .arg(&package)
        .arg("--scratch-path")
        .arg(&scratch)
        .args(extra);
        cmd
    };
    let status = swift_build(&[])
        .status()
        .expect("run `swift build` (install Xcode)");
    assert!(status.success(), "swift build of GhiAudioMac failed");
    let bin = swift_build(&["--show-bin-path"])
        .output()
        .expect("swift build --show-bin-path");
    let bin = String::from_utf8(bin.stdout).unwrap();
    println!("cargo:rustc-link-search=native={}", bin.trim());
    println!("cargo:rustc-link-lib=static=GhiAudioMac");

    // Swift runtime: the OS copy in /usr/lib/swift, plus the toolchain's
    // compatibility libraries, as `swiftc` itself would link.
    let info = Command::new("swift")
        .arg("-print-target-info")
        .output()
        .expect("swift -print-target-info");
    let info = String::from_utf8(info.stdout).unwrap();
    for dir in json_string_array(&info, "runtimeLibraryPaths") {
        println!("cargo:rustc-link-search=native={dir}");
    }
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    for framework in [
        "CoreAudio",
        "AudioToolbox",
        "AVFoundation",
        "IOKit",
        "Foundation",
    ] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}

/// The strings of `"key": [ ... ]` in `swift -print-target-info` output. A tiny
/// scanner so the build script needs no JSON dependency.
fn json_string_array(json: &str, key: &str) -> Vec<String> {
    let Some(start) = json.find(&format!("\"{key}\"")) else {
        return Vec::new();
    };
    let rest = &json[start..];
    let (Some(open), Some(close)) = (rest.find('['), rest.find(']')) else {
        return Vec::new();
    };
    rest[open + 1..close]
        .split(',')
        .map(|s| s.trim().trim_matches('"').replace("\\/", "/"))
        .filter(|s| !s.is_empty())
        .collect()
}
