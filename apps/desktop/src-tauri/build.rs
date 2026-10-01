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
            "discard_from",
            "rename_speaker",
            "merge_speakers",
            "split_speaker",
            "speaker_not_a_person",
            "import_recording",
            "quit_app",
            "list_meetings",
            "set_meeting_title",
            "note_lines",
            "add_note_line",
            "update_note_line",
            "delete_note_line",
            "discard_preview",
            "get_settings",
            "update_settings",
            "mic_permission",
            "request_mic_permission",
            "reply_meeting_detected",
            "issue_audio_sample",
            "show_main",
            "set_consent_confirmed",
            "session_snapshot",
            "models_status",
            "download_models",
            "cancel_model_download",
            "delete_meeting",
            "retry_meeting",
            "take_recovered_meetings",
            "known_speaker_names",
            "hide_popover",
            "set_mini_compact",
            "close_mini",
            "open_mini_recorder",
            "close_detect",
            "show_notification",
            "request_quit_app",
            "meeting_speakers",
            "rename_meeting_speaker",
            "has_recovery_key",
            "create_recovery_key",
            "confirm_recovery_key",
            "cancel_recovery_key",
            "open_privacy_settings",
            "test_capture",
        ]),
    ))
    .expect("failed to run tauri-build");
}
