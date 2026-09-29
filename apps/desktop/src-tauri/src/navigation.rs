// SPDX-License-Identifier: Apache-2.0
//! Navigation guard (RT-6). The CSP does not cover top-level navigation
//! (`location =`, `<meta http-equiv=refresh>`, links), so the webview may only
//! navigate within the app's own origin.

use tauri::{Runtime, Url, plugin::TauriPlugin};

/// Dev server origin from `build.devUrl`; only allowed in debug builds.
const DEV_ORIGIN: (&str, &str, u16) = ("http", "localhost", 1420);

pub fn is_allowed(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    let app_origin = match url.scheme() {
        // macOS/iOS/Linux serve the bundled frontend as tauri://localhost;
        // Windows/Android as http(s)://tauri.localhost.
        "tauri" => host == "localhost",
        "http" | "https" => host == "tauri.localhost",
        _ => false,
    };
    let dev_origin =
        cfg!(debug_assertions) && (url.scheme(), host, url.port().unwrap_or(0)) == DEV_ORIGIN;
    app_origin || dev_origin
}

pub fn guard<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("navigation-guard")
        .on_navigation(|_webview, url| is_allowed(url))
        .build()
}

#[cfg(test)]
mod tests {
    use super::is_allowed;
    use tauri::Url;

    fn allowed(url: &str) -> bool {
        is_allowed(&Url::parse(url).unwrap())
    }

    #[test]
    fn app_origins_are_allowed() {
        assert!(allowed("tauri://localhost/index.html"));
        assert!(allowed("http://tauri.localhost/meetings"));
        assert!(allowed("https://tauri.localhost/"));
    }

    #[test]
    fn remote_and_lookalike_origins_are_blocked() {
        for url in [
            "https://example.com/",
            "http://tauri.localhost.example.com/",
            "tauri://evil/",
            "file:///etc/passwd",
            "data:text/html,hi",
            "javascript:alert(1)",
        ] {
            assert!(!allowed(url), "{url}");
        }
    }

    #[test]
    fn dev_server_only_in_debug() {
        assert_eq!(allowed("http://localhost:1420/"), cfg!(debug_assertions));
        assert!(!allowed("http://localhost:8080/"));
    }
}
