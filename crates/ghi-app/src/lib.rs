// SPDX-License-Identifier: Apache-2.0
//! The app layer both Ghira apps share (phase 16): [`core::Core`] (the store,
//! the job runner, recording and import lifecycles) and the Tauri commands
//! and DTOs that read and edit what is stored. The desktop registers them
//! beside its window, tray and calendar commands; iOS registers the same
//! functions, so both bindings carry identical types.
//!
//! What needs a window (native dialogs, the menu bar, panels, the tray, the
//! updater, EventKit) stays in the desktop crate.

pub mod audio_protocol;
pub mod calendar_cmd;
pub mod cloud_cmd;
pub mod core;
pub mod detail;
pub mod export_cmd;
pub mod import_cmd;
pub mod library;
pub mod lock_cmd;
pub mod settings_cmd;
pub mod speakers_cmd;
pub mod store_problem;
pub mod sync_cmd;
pub mod sync_service;
pub mod system;
pub mod voice_cmd;

use std::sync::Arc;

/// The managed state every command takes.
pub type CoreState<'a> = tauri::State<'a, Arc<core::Core>>;

/// Runs a blocking core call off the async runtime's workers.
pub async fn blocking<T: Send + 'static>(
    core: &Arc<core::Core>,
    f: impl FnOnce(&core::Core) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let core = core.clone();
    tauri::async_runtime::spawn_blocking(move || f(&core))
        .await
        .map_err(|e| e.to_string())?
}
