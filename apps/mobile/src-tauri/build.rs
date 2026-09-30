// SPDX-License-Identifier: Apache-2.0
fn main() {
    // Each app command needs an explicit capability grant (capabilities/*.json).
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "snapshot",
            "start_recording",
            "stop_recording",
            "mark_moment",
        ]),
    ))
    .expect("failed to run tauri-build");
}
