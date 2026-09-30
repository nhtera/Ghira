// SPDX-License-Identifier: Apache-2.0
//! With `--features nemo`, lets `ghi` find the NeMo-Speech.cpp shared library
//! where tools/scripts/build-nemo.sh installed it (rpath; dev and CI builds).
//! App bundles ship the library next to the executable instead (phase 12).
//!
//! On macOS it also embeds `Info.plist` in the `ghi` binary: the Microphone and
//! System Audio Recording prompts need its usage descriptions.

fn main() {
    println!("cargo:rerun-if-changed=Info.plist");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let plist =
            std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("Info.plist");
        println!(
            "cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            plist.display()
        );
    }
    println!("cargo:rerun-if-env-changed=DEP_NEMO_SPEECH_ASR_C_LIBDIR");
    if let Ok(dir) = std::env::var("DEP_NEMO_SPEECH_ASR_C_LIBDIR")
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
    {
        // Binaries and every test target (unit tests link the library too).
        println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    }
}
