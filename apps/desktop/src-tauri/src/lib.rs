// SPDX-License-Identifier: Apache-2.0
//! Tauri shell. Commands are typed with tauri-specta; regenerate the TypeScript
//! bindings with `GHI_UPDATE_BINDINGS=1 cargo test -p ghi-desktop`.

mod audio_protocol;
mod core;
mod library;
mod menu;
mod models_cmd;
mod navigation;
mod system;
mod windows;

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{Manager, RunEvent};
use tauri_specta::Event;

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
async fn discard_last(
    core: CoreState<'_>,
    tokens: tauri::State<'_, Arc<audio_protocol::AudioTokens>>,
    seconds: f64,
) -> Result<f64, String> {
    let tokens = tokens.inner().clone();
    blocking(&core, move |c| {
        c.with_session(|s| {
            let cut = s
                .discard(seconds)
                .map(|t| t as f64)
                .map_err(|e| e.to_string());
            // Samples issued before may cover audio that is gone now.
            tokens.revoke_meeting(s.meeting());
            cut
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
    blocking(&core, move |c| {
        c.with_session(|s| s.split(from, lines).map_err(|e| e.to_string()))?
    })
    .await
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

/// The UI should go to `route` (a window was brought forward for it).
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct Navigate {
    pub route: String,
}

/// The user quit while recording: the UI asks "Stop and quit?" and answers
/// with `quit_app`.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct QuitRequested {}

/// A user asked to quit: while recording, the main window comes forward and
/// asks "Stop and quit?" (`quit_app` answers); otherwise the app exits.
pub(crate) fn request_quit<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let core = app.state::<Arc<core::Core>>();
    if core.recording() {
        if let Ok(w) = windows::main(app, None) {
            let _ = w.show();
            let _ = w.set_focus();
        }
        let _ = QuitRequested {}.emit(app);
    } else {
        app.exit(0);
    }
}

/// Quits; `stop`: stop the recording first (it is saved and processed at the
/// next launch). Without `stop`, a running recording keeps the app open.
#[tauri::command]
#[specta::specta]
async fn quit_app(app: tauri::AppHandle, core: CoreState<'_>, stop: bool) -> Result<(), String> {
    if core.recording() {
        if !stop {
            return Ok(());
        }
        blocking(&core, |c| c.stop().map(|_| ())).await?;
    }
    app.exit(0);
    Ok(())
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
            import_recording,
            quit_app,
            library::list_meetings,
            library::set_meeting_title,
            library::note_lines,
            library::add_note_line,
            library::update_note_line,
            library::delete_note_line,
            library::discard_preview,
            library::set_consent_confirmed,
            library::session_snapshot,
            models_cmd::models_status,
            models_cmd::download_models,
            models_cmd::cancel_model_download,
            system::get_settings,
            system::update_settings,
            system::mic_permission,
            system::request_mic_permission,
            system::reply_meeting_detected,
            audio_protocol::issue_audio_sample,
            windows::show_main
        ])
        .events(tauri_specta::collect_events![
            core::CoreEvent,
            menu::MenuAction,
            system::MeetingDetected,
            models_cmd::ModelDownload,
            Navigate,
            QuitRequested
        ])
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

    let tokens = Arc::new(audio_protocol::AudioTokens::default());
    let protocol_tokens = tokens.clone();
    let app = tauri::Builder::default()
        .plugin(navigation::guard())
        .invoke_handler(builder.invoke_handler())
        // Short audio spans for the webview, by token only (audio_protocol.rs).
        .register_asynchronous_uri_scheme_protocol("ghi-audio", move |ctx, request, responder| {
            let core = ctx.app_handle().state::<Arc<core::Core>>().inner().clone();
            let tokens = protocol_tokens.clone();
            std::thread::spawn(move || {
                // A bug while decoding must still answer (or the request hangs).
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    audio_protocol::respond(core.store().ok(), &tokens, &request)
                }));
                responder.respond(r.unwrap_or_else(|_| audio_protocol::server_error()));
            });
        })
        .setup(move |app| {
            builder.mount_events(app);
            let core = Arc::new(core::Core::new(app.handle())?);
            core.init_in_background();
            let detection = Arc::new(system::Detection::default());
            system::spawn_detection(app.handle().clone(), core.clone(), detection.clone());
            app.manage(core);
            app.manage(detection);
            app.manage(tokens);
            app.manage(Arc::new(models_cmd::Downloads::default()));
            #[cfg(target_os = "macos")]
            {
                app.set_menu(menu::build(app.handle())?)?;
                app.on_menu_event(menu::on_event);
            }
            // Created here rather than in tauri.conf.json so `window.open` can
            // be denied (windows.rs).
            windows::main(app.handle(), None)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the Ghira desktop app");
    app.run(|app, event| match event {
        // Closing the main window while recording asks first (until the
        // menu-bar item exists, phase 10c, closing it would leave the
        // recording without any window).
        RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "main" && app.state::<Arc<core::Core>>().recording() => {
            api.prevent_close();
            request_quit(app);
        }
        RunEvent::ExitRequested { api, code, .. } => {
            let core = app.state::<Arc<core::Core>>();
            // A user quit while recording asks first; quit_app answers.
            if code.is_none() && core.recording() {
                api.prevent_exit();
                request_quit(app);
                return;
            }
            core.shutdown(Duration::from_secs(5));
        }
        // Last chance (e.g. the system logging out): save the recording so
        // it is processed at the next launch, then stop the jobs.
        RunEvent::Exit => {
            let core = app.state::<Arc<core::Core>>();
            if core.recording() {
                let _ = core.stop();
            }
            core.shutdown(Duration::from_secs(2));
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => {
            if let Ok(w) = windows::main(app, None) {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_version_reports_core() {
        let v = super::app_version();
        assert_eq!(v.core, ghi_core::version());
        assert_eq!(v.app, env!("CARGO_PKG_VERSION"));
    }

    /// Every command the webview can call is declared in build.rs (so it
    /// needs a grant) and granted to some window in capabilities/*.json; a
    /// missing entry would only fail at runtime.
    #[test]
    fn every_command_is_declared_and_granted() {
        let dir = env!("CARGO_MANIFEST_DIR");
        let tmp = std::env::temp_dir().join(format!("ghi-cmds-{}.ts", std::process::id()));
        super::export_bindings(tmp.to_str().unwrap());
        let ts = std::fs::read_to_string(&tmp).unwrap();
        let _ = std::fs::remove_file(&tmp);
        let commands: Vec<&str> = ts
            .split("__TAURI_INVOKE")
            .skip(1)
            // Calls only (`__TAURI_INVOKE<T>("name"` / `("name"`), not the import.
            .filter(|s| s.starts_with('<') || s.starts_with('('))
            .filter_map(|s| s.split('"').nth(1))
            .collect();
        assert!(commands.len() > 10, "{commands:?}");
        let build = std::fs::read_to_string(format!("{dir}/build.rs")).unwrap();
        let grants: String = std::fs::read_dir(format!("{dir}/capabilities"))
            .unwrap()
            .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
            .collect();
        for c in commands {
            assert!(
                build.contains(&format!("\"{c}\"")),
                "{c} is not declared in build.rs"
            );
            let grant = format!("\"allow-{}\"", c.replace('_', "-"));
            assert!(
                grants.contains(&grant),
                "{c} is granted to no window ({grant})"
            );
        }
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
