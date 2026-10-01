// SPDX-License-Identifier: Apache-2.0
//! With `--features nemo`, lets the test binaries find the NeMo-Speech.cpp
//! shared library where tools/scripts/build-nemo.sh installed it (rpath).

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_NEMO_SPEECH_ASR_C_LIBDIR");
    if let Ok(dir) = std::env::var("DEP_NEMO_SPEECH_ASR_C_LIBDIR")
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
    {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    }
}
