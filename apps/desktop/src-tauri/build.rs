// SPDX-License-Identifier: Apache-2.0
fn main() {
    // Declaring app commands makes each one need an explicit capability grant
    // (`allow-<command>` in capabilities/*.json) instead of being open to every window.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&["app_version"])),
    )
    .expect("failed to run tauri-build");
}
