// SPDX-License-Identifier: Apache-2.0
//! The macOS app menu. Its accelerators fire before the webview sees a key,
//! so every app shortcut that could collide lives here and reaches the UI as
//! a typed `MenuAction` event (the webview handles the same keys itself on
//! Windows, which has no menu bar). ⌘M marks a moment (brief §8), so Minimize
//! moves to ⌥⌘M. Labels are English until the native menu is localized
//! (phase 12).

use serde::{Deserialize, Serialize};
use specta::Type;
#[cfg(target_os = "macos")]
use tauri::menu::{Menu, MenuBuilder, MenuEvent, MenuItemBuilder, SubmenuBuilder};
#[cfg(target_os = "macos")]
use tauri::{AppHandle, Manager, Runtime};
use tauri_specta::Event;

/// A menu command for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub enum MenuAction {
    /// ⌘⇧R: start or stop recording.
    ToggleRecording,
    /// ⌘M: mark this moment.
    Mark,
    /// ⌘K: command palette.
    CommandPalette,
    /// ⌘,: settings.
    Settings,
}

// The menu exists on macOS only (Windows has no menu bar; the webview handles
// the same keys there); `MenuAction` stays for the typed bindings.
#[cfg(target_os = "macos")]
const ACTIONS: [(&str, &str, &str, MenuAction); 4] = [
    (
        "toggle-recording",
        "Start or Stop Recording",
        "CmdOrCtrl+Shift+R",
        MenuAction::ToggleRecording,
    ),
    ("mark", "Mark Moment", "CmdOrCtrl+M", MenuAction::Mark),
    (
        "command-palette",
        "Search or Jump To…",
        "CmdOrCtrl+K",
        MenuAction::CommandPalette,
    ),
    ("settings", "Settings…", "CmdOrCtrl+,", MenuAction::Settings),
];

#[cfg(target_os = "macos")]
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let item = |id: &str| {
        let (id, text, accel, _) = ACTIONS.iter().find(|a| a.0 == id).expect("known action");
        MenuItemBuilder::with_id(*id, *text)
            .accelerator(*accel)
            .build(app)
    };
    let minimize = MenuItemBuilder::with_id("minimize", "Minimize")
        .accelerator("Alt+CmdOrCtrl+M")
        .build(app)?;
    let name = app.package_info().name.clone();
    let app_menu = SubmenuBuilder::new(app, name)
        .about(None)
        .separator()
        .item(&item("settings")?)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;
    let edit = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let meeting = SubmenuBuilder::new(app, "Meeting")
        .item(&item("toggle-recording")?)
        .item(&item("mark")?)
        .separator()
        .item(&item("command-palette")?)
        .build()?;
    let window = SubmenuBuilder::new(app, "Window")
        .item(&minimize)
        .maximize()
        .fullscreen()
        .separator()
        .close_window()
        .build()?;
    MenuBuilder::new(app)
        .items(&[&app_menu, &edit, &meeting, &window])
        .build()
}

#[cfg(target_os = "macos")]
pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let id = event.id().as_ref();
    if id == "minimize" {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.minimize();
        }
        return;
    }
    if let Some((.., action)) = ACTIONS.iter().find(|a| a.0 == id) {
        let _ = action.emit(app);
    }
}
