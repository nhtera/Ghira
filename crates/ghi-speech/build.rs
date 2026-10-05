// SPDX-License-Identifier: Apache-2.0
//! With `--features nemo`, links the NeMo-Speech.cpp shared C library installed
//! by `tools/scripts/build-nemo.sh` (or `$NEMO_SPEECH_DIR`), and compiles a C
//! layout check for the hand-written FFI structs.
//!
//! With `--features whisper`, links the static whisper.cpp + ggml installed by
//! `tools/scripts/build-whisper.sh` (or `$WHISPER_CPP_DIR`) and compiles the
//! C shim over its API (`src/whisper/shim.c`).

fn main() {
    println!("cargo:rerun-if-env-changed=NEMO_SPEECH_DIR");
    println!("cargo:rerun-if-env-changed=WHISPER_CPP_DIR");
    #[cfg(feature = "nemo")]
    nemo();
    #[cfg(feature = "whisper")]
    whisper();
}

/// Static, so its ggml never meets NeMo's patched `@rpath/libggml.0.dylib`.
#[cfg(feature = "whisper")]
fn whisper() {
    use std::path::PathBuf;

    let dir = std::env::var_os("WHISPER_CPP_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
            manifest.join("../../target/whisper/install")
        });
    let lib = dir.join("lib");
    let include = dir.join("include");
    if !include.join("whisper.h").is_file() {
        panic!(
            "whisper.cpp not found in {}: run tools/scripts/build-whisper.sh or set WHISPER_CPP_DIR",
            dir.display()
        );
    }
    println!(
        "cargo:rerun-if-changed={}",
        include.join("whisper.h").display()
    );
    println!("cargo:rustc-link-search=native={}", lib.display());
    // Backends exist only where the build had them (Metal and Accelerate on a Mac).
    for name in [
        "whisper",
        "ggml",
        "ggml-cpu",
        "ggml-metal",
        "ggml-blas",
        "ggml-base",
    ] {
        if lib.join(format!("lib{name}.a")).is_file() {
            println!("cargo:rustc-link-lib=static={name}");
        }
    }
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("macos") => {
            for f in ["Accelerate", "Foundation", "Metal", "MetalKit"] {
                println!("cargo:rustc-link-lib=framework={f}");
            }
            println!("cargo:rustc-link-lib=c++");
            // ggml-metal uses `@available`, which clang lowers to a call into
            // its runtime (`__isPlatformVersionAtLeast`); rustc does not link it.
            if let Ok(out) = std::process::Command::new("xcrun")
                .args(["clang", "--print-resource-dir"])
                .output()
            {
                let res = String::from_utf8_lossy(&out.stdout).trim().to_string();
                println!("cargo:rustc-link-search=native={res}/lib/darwin");
                println!("cargo:rustc-link-lib=static=clang_rt.osx");
            }
        }
        Ok("windows") => {}
        _ => println!("cargo:rustc-link-lib=stdc++"),
    }
    println!("cargo:rerun-if-changed=src/whisper/shim.c");
    cc::Build::new()
        .file("src/whisper/shim.c")
        .include(&include)
        .warnings(true)
        .compile("ghi_whisper_shim");
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
