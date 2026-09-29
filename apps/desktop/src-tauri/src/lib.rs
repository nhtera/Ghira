// SPDX-License-Identifier: Apache-2.0
//! Tauri shell. Commands are typed with tauri-specta; regenerate the TypeScript
//! bindings with `GHI_UPDATE_BINDINGS=1 cargo test -p ghi-desktop`.

mod navigation;

use serde::Serialize;
use specta::Type;

/// Versions shown in Settings → About.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppVersion {
    pub app: String,
    pub core: String,
}

#[tauri::command]
#[specta::specta]
fn app_version() -> AppVersion {
    AppVersion {
        app: env!("CARGO_PKG_VERSION").to_owned(),
        core: ghi_core::version().to_owned(),
    }
}

fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .commands(tauri_specta::collect_commands![app_version])
}

/// Writes the TypeScript bindings for all commands to `path`.
pub fn export_bindings(path: &str) {
    specta_builder()
        .export(
            specta_typescript::Typescript::default()
                .header("// SPDX-License-Identifier: Apache-2.0\n"),
            path,
        )
        .expect("failed to export TypeScript bindings");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = specta_builder();

    tauri::Builder::default()
        .plugin(navigation::guard())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            // Created here rather than in tauri.conf.json so `window.open` can
            // be denied; the navigation guard plugin covers in-place navigation.
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Ghi")
            .inner_size(1200.0, 800.0)
            .min_inner_size(900.0, 600.0)
            .on_new_window(|_url, _features| tauri::webview::NewWindowResponse::Deny)
            .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Ghi desktop app");
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_version_reports_core() {
        let v = super::app_version();
        assert_eq!(v.core, ghi_core::version());
        assert_eq!(v.app, env!("CARGO_PKG_VERSION"));
    }

    /// Keeps the committed bindings in sync; CI fails if they drift.
    /// Regenerate with `GHI_UPDATE_BINDINGS=1 cargo test -p ghi-desktop`.
    #[test]
    fn bindings_are_up_to_date() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/bindings.ts");
        let tmp = std::env::temp_dir().join(format!("ghi-bindings-{}.ts", std::process::id()));
        super::export_bindings(tmp.to_str().unwrap());
        let fresh = std::fs::read_to_string(&tmp).unwrap();
        let _ = std::fs::remove_file(&tmp);
        if std::env::var_os("GHI_UPDATE_BINDINGS").is_some() {
            std::fs::write(path, &fresh).unwrap();
            return;
        }
        let committed = std::fs::read_to_string(path).unwrap_or_default();
        assert_eq!(
            committed, fresh,
            "src/bindings.ts is stale: run `GHI_UPDATE_BINDINGS=1 cargo test -p ghi-desktop`"
        );
    }
}
