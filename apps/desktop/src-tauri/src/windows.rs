// SPDX-License-Identifier: Apache-2.0
//! The app's windows, built in one place so every webview gets the same
//! hardening: `window.open` denied (the navigation guard plugin covers
//! in-place navigation) and a route of the one frontend bundle. The
//! mini-recorder and the menu-bar popover (phase 10c) are added here too.

use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// A hash route the frontend knows (`/live`), nothing else.
fn safe_route(route: Option<&str>) -> Option<&str> {
    route.filter(|r| {
        r.starts_with('/')
            && r.len() < 200
            && r.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_".contains(&b))
    })
}

/// The main window (library, live view, settings); a new one opens on `route`.
pub fn main<R: Runtime>(
    app: &AppHandle<R>,
    route: Option<&str>,
) -> tauri::Result<WebviewWindow<R>> {
    if let Some(w) = app.get_webview_window("main") {
        return Ok(w);
    }
    let url = match safe_route(route) {
        Some(r) => format!("index.html#{r}"),
        None => "index.html".into(),
    };
    let window = WebviewWindowBuilder::new(app, "main", WebviewUrl::App(url.into()))
        .title("Ghira")
        .inner_size(1280.0, 800.0)
        // Brief §8: the layout works down to 960×640 (compact mode).
        .min_inner_size(960.0, 640.0)
        .on_new_window(|_url, _features| tauri::webview::NewWindowResponse::Deny);
    // mac: content under a transparent title bar (the sidebar leaves room for
    // the traffic lights; headers are drag regions).
    #[cfg(target_os = "macos")]
    let window = window
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);
    window.build()
}

/// Brings the main window forward on `route` (a hash route, e.g. `/live`).
/// Async: creating a window from a sync command deadlocks on Windows.
#[tauri::command]
#[specta::specta]
pub async fn show_main(app: AppHandle, route: Option<String>) -> Result<(), String> {
    let existed = app.get_webview_window("main").is_some();
    let w = main(&app, route.as_deref()).map_err(|e| e.to_string())?;
    // A new window opened on the route already; an existing one is told.
    if let (true, Some(r)) = (existed, safe_route(route.as_deref())) {
        let _ = tauri_specta::Event::emit(&crate::Navigate { route: r.into() }, &app);
    }
    let _ = w.unminimize();
    w.show().map_err(|e| e.to_string())?;
    w.set_focus().map_err(|e| e.to_string())
}
