// SPDX-License-Identifier: Apache-2.0
//! Tauri shell. Commands are typed with tauri-specta; regenerate the TypeScript
//! bindings with `GHI_UPDATE_BINDINGS=1 cargo test -p ghi-desktop`.

mod core;
mod navigation;

use std::sync::Arc;

use serde::Serialize;
use specta::Type;
use tauri::Manager;

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

/// Recording mode (doc 02 §A).
#[derive(Debug, Clone, Copy, serde::Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum RecordMode {
    /// Mic (you) + the call's audio.
    Call,
    /// One mic in a room.
    Room,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Stopped {
    pub meeting: String,
    pub duration_ms: f64,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Imported {
    pub meeting: String,
    pub duplicate: bool,
    pub duration_ms: f64,
}

type CoreState<'a> = tauri::State<'a, Arc<core::Core>>;

/// Runs a blocking core call off the async runtime's workers.
async fn blocking<T: Send + 'static>(
    core: &Arc<core::Core>,
    f: impl FnOnce(&core::Core) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let core = core.clone();
    tauri::async_runtime::spawn_blocking(move || f(&core))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
async fn start_recording(
    core: CoreState<'_>,
    mode: RecordMode,
    language: Option<String>,
    title: String,
) -> Result<String, String> {
    let mode = match mode {
        RecordMode::Call => ghi_core::live::Mode::Call,
        RecordMode::Room => ghi_core::live::Mode::Room,
    };
    blocking(&core, move |c| c.start(mode, language, title)).await
}

#[tauri::command]
#[specta::specta]
async fn stop_recording(core: CoreState<'_>) -> Result<Stopped, String> {
    let r = blocking(&core, |c| c.stop()).await?;
    Ok(Stopped {
        meeting: r.meeting,
        duration_ms: r.duration_ms as f64,
    })
}

#[tauri::command]
#[specta::specta]
async fn pause_recording(core: CoreState<'_>) -> Result<(), String> {
    core.with_session(|s| s.pause())
}

#[tauri::command]
#[specta::specta]
async fn resume_recording(core: CoreState<'_>) -> Result<(), String> {
    core.with_session(|s| s.resume())
}

/// Marks this moment; returns its meeting time (ms).
#[tauri::command]
#[specta::specta]
async fn mark_moment(core: CoreState<'_>) -> Result<f64, String> {
    core.with_session(|s| s.mark() as f64)
}

/// Discards the last `seconds` [RT-1]; returns where the cut landed (ms).
#[tauri::command]
#[specta::specta]
async fn discard_last(core: CoreState<'_>, seconds: f64) -> Result<f64, String> {
    blocking(&core, move |c| {
        c.with_session(|s| {
            s.discard(seconds)
                .map(|t| t as f64)
                .map_err(|e| e.to_string())
        })?
    })
    .await
}

#[tauri::command]
#[specta::specta]
async fn rename_speaker(core: CoreState<'_>, id: u32, name: String) -> Result<(), String> {
    core.with_session(|s| s.rename(id, &name))
}

#[tauri::command]
#[specta::specta]
async fn merge_speakers(core: CoreState<'_>, from: u32, into: u32) -> Result<(), String> {
    core.with_session(|s| s.merge(from, into))
}

/// Moves the given lines (segment gids) to a new speaker; returns its id.
#[tauri::command]
#[specta::specta]
async fn split_speaker(
    core: CoreState<'_>,
    from: u32,
    lines: Vec<String>,
) -> Result<Option<u32>, String> {
    blocking(&core, move |c| c.with_session(|s| s.split(from, lines))).await
}

#[tauri::command]
#[specta::specta]
async fn speaker_not_a_person(core: CoreState<'_>, id: u32) -> Result<(), String> {
    core.with_session(|s| s.not_a_person(id))
}

/// Imports an audio/video file as a meeting (transcribed by the job runner).
#[tauri::command]
#[specta::specta]
async fn import_recording(
    core: CoreState<'_>,
    path: String,
    split_channels: bool,
) -> Result<Imported, String> {
    let r = blocking(&core, move |c| {
        c.import(std::path::Path::new(&path), split_channels)
    })
    .await?;
    Ok(Imported {
        meeting: r.meeting,
        duplicate: r.duplicate,
        duration_ms: r.duration_ms as f64,
    })
}

fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new()
        .commands(tauri_specta::collect_commands![
            app_version,
            start_recording,
            stop_recording,
            pause_recording,
            resume_recording,
            mark_moment,
            discard_last,
            rename_speaker,
            merge_speakers,
            split_speaker,
            speaker_not_a_person,
            import_recording
        ])
        .events(tauri_specta::collect_events![core::CoreEvent])
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
            app.manage(Arc::new(core::Core::new(app.handle())?));
            // Created here rather than in tauri.conf.json so `window.open` can
            // be denied; the navigation guard plugin covers in-place navigation.
            tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("Ghira")
            .inner_size(1200.0, 800.0)
            .min_inner_size(900.0, 600.0)
            .on_new_window(|_url, _features| tauri::webview::NewWindowResponse::Deny)
            .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Ghira desktop app");
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
