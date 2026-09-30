// SPDX-License-Identifier: Apache-2.0
//! With `--features nemo`, links the NeMo-Speech.cpp shared C library installed
//! by `tools/scripts/build-nemo.sh` (or `$NEMO_SPEECH_DIR`), and compiles a C
//! layout check for the hand-written FFI structs.

fn main() {
    println!("cargo:rerun-if-env-changed=NEMO_SPEECH_DIR");
    #[cfg(feature = "nemo")]
    nemo();
}

#[cfg(feature = "nemo")]
fn nemo() {
    use std::path::PathBuf;

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        return nemo_ios();
    }
    let dir = std::env::var_os("NEMO_SPEECH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
            manifest.join("../../target/nemo/install")
        });
    let lib = dir.join("lib");
    let include = dir.join("include");
    if !include.join("nemo_speech/asr.h").is_file() {
        panic!(
            "NeMo-Speech.cpp not found in {}: run tools/scripts/build-nemo.sh or set NEMO_SPEECH_DIR",
            dir.display()
        );
    }
    println!(
        "cargo:rerun-if-changed={}",
        include.join("nemo_speech").display()
    );
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=nemo_speech_asr_c");
    // Dependents (ghi-cli) set their own rpath from this; tests here need one too.
    println!("cargo:libdir={}", lib.display());
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
    }

    println!("cargo:rerun-if-changed=src/nemo/layout_check.c");
    cc::Build::new()
        .file("src/nemo/layout_check.c")
        .include(&include)
        .compile("ghi_nemo_layout_check");
}

/// iOS: the XCFrameworks from `tools/scripts/build-nemo-ios.sh` (or
/// `$NEMO_SPEECH_IOS_DIR`). The app's Xcode target links and embeds them;
/// this only points rustc at the right slice and runs the layout check.
#[cfg(feature = "nemo")]
fn nemo_ios() {
    use std::path::PathBuf;

    println!("cargo:rerun-if-env-changed=NEMO_SPEECH_IOS_DIR");
    let dir = std::env::var_os("NEMO_SPEECH_IOS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
            manifest.join("../../target/nemo-ios")
        });
    let include = dir.join("include");
    if !include.join("nemo_speech/asr.h").is_file() {
        panic!(
            "NeMo-Speech.cpp for iOS not found in {}: run tools/scripts/build-nemo-ios.sh or set NEMO_SPEECH_IOS_DIR",
            dir.display()
        );
    }
    let sim = std::env::var("CARGO_CFG_TARGET_ABI").as_deref() == Ok("sim")
        || std::env::var("TARGET").is_ok_and(|t| t.ends_with("-sim"));
    let slice = if sim {
        "ios-arm64-simulator"
    } else {
        "ios-arm64"
    };
    println!(
        "cargo:rustc-link-search=framework={}",
        dir.join("nemo_speech_asr_c.xcframework")
            .join(slice)
            .display()
    );
    println!("cargo:rustc-link-lib=framework=nemo_speech_asr_c");
    println!(
        "cargo:rerun-if-changed={}",
        include.join("nemo_speech").display()
    );
    println!("cargo:rerun-if-changed=src/nemo/layout_check.c");
    cc::Build::new()
        .file("src/nemo/layout_check.c")
        .include(&include)
        .compile("ghi_nemo_layout_check");
}
