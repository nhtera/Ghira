// SPDX-License-Identifier: Apache-2.0
fn main() {
    // With `--features nemo`, dev builds find NeMo-Speech.cpp where
    // tools/scripts/build-nemo.sh installed it; bundles ship it (phase 12).
    println!("cargo:rerun-if-env-changed=DEP_NEMO_SPEECH_ASR_C_LIBDIR");
    if let Ok(dir) = std::env::var("DEP_NEMO_SPEECH_ASR_C_LIBDIR")
        && std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
    {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    }
    // Declaring app commands makes each one need an explicit capability grant
    // (`allow-<command>` in capabilities/*.json) instead of being open to every window.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "app_version",
            "start_recording",
            "stop_recording",
            "pause_recording",
            "resume_recording",
            "mark_moment",
            "discard_last",
            "rename_speaker",
            "merge_speakers",
            "split_speaker",
            "speaker_not_a_person",
            "import_recording",
        ]),
    ))
    .expect("failed to run tauri-build");
}
