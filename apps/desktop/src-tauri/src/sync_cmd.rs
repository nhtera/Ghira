// SPDX-License-Identifier: Apache-2.0
//! LAN sync, the desktop's part (phase 15): starts the shared
//! [`ghi_app::sync_service::SyncService`] as the hub and forwards its events
//! to the webview as the typed `syncEvent`. The commands are `ghi-app`'s.

use std::sync::Arc;

use ghi_app::core::Core;
use ghi_app::sync_cmd::SyncEvent;
use ghi_app::sync_service::{SyncConfig, SyncService};
use tauri::AppHandle;
use tauri_specta::Event;

/// The name a phone shows for this computer: the Mac's own name (sent to a
/// paired phone only), shortened; a plain word when it can't be read.
fn computer_name() -> String {
    #[cfg(target_os = "macos")]
    let name = std::process::Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok());
    #[cfg(not(target_os = "macos"))]
    let name = std::env::var("COMPUTERNAME").ok();
    let name: String = name
        .unwrap_or_default()
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(60)
        .collect();
    if name.is_empty() {
        if cfg!(target_os = "macos") {
            "Mac"
        } else {
            "Computer"
        }
        .to_string()
    } else {
        name
    }
}

/// Creates the service for `core`, starts its thread and returns it (managed
/// by the app).
pub fn start(app: &AppHandle, core: &Arc<Core>) -> Arc<SyncService> {
    let (events, settings) = (app.clone(), app.clone());
    let service = SyncService::new(
        core.clone(),
        SyncConfig::hub(
            computer_name(),
            move |e: SyncEvent| {
                let _ = e.emit(&events);
            },
            {
                let core = core.clone();
                move || core.settings_changed(&settings)
            },
        ),
    );
    service.start();
    service
}
