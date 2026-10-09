// SPDX-License-Identifier: Apache-2.0
//! Tauri shell. Commands are typed with tauri-specta; regenerate the TypeScript
//! bindings with `GHI_UPDATE_BINDINGS=1 cargo test -p ghi-desktop`.

mod ask_cmd;
mod audio_protocol;
mod calendar_cmd;
#[cfg(target_os = "macos")]
mod calendar_mac;
mod cloud_cmd;
mod core;
#[cfg(all(test, unix))]
mod core_real_tests;
mod detail;
mod diag_cmd;
mod dialogs;
mod export_cmd;
mod import_cmd;
mod library;
mod lock_cmd;
mod login_item;
mod menu;
mod models_cmd;
mod navigation;
#[cfg(target_os = "macos")]
mod notify_mac;
mod organize_cmd;
mod panels;
mod people_cmd;
mod recovery_cmd;
mod settings_cmd;
mod speakers_cmd;
mod sync_cmd;
mod sync_export_cmd;
mod system;
mod tray;
mod update_cmd;
mod voice_cmd;
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

use ghi_app::{CoreState, blocking};

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
    blocking(&core, move |c| {
        // No language given: the user's default for new meetings.
        let language = language.or_else(|| {
            system::load_settings(c)
                .ok()
                .and_then(|s| s.meeting_language.hint())
        });
        let meeting = c.start(mode, language, title)?;
        // A calendar meeting in progress names the new meeting (D4); inert
        // until the calendar is connected.
        calendar_cmd::on_recording_started(c, &meeting);
        Ok(meeting)
    })
    .await
}

/// The onboarding's 10 s test: records briefly and deletes the meeting
/// afterwards; returns its id so the UI can follow its events.
#[tauri::command]
#[specta::specta]
async fn test_capture(core: CoreState<'_>, seconds: u32) -> Result<String, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || core.start_test(seconds))
        .await
        .map_err(|e| e.to_string())?
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

/// Discards everything from `from_ms` (meeting time) on [RT-1]: the span the
/// user saw in the preview, however long they took to confirm.
#[tauri::command]
#[specta::specta]
async fn discard_from(
    core: CoreState<'_>,
    tokens: tauri::State<'_, Arc<audio_protocol::AudioTokens>>,
    from_ms: f64,
) -> Result<f64, String> {
    let tokens = tokens.inner().clone();
    blocking(&core, move |c| {
        c.with_session_unlocked(|s| {
            let cut = s
                .discard_from(from_ms.max(0.0) as i64)
                .map(|t| t as f64)
                .map_err(|e| e.to_string());
            tokens.revoke_meeting(s.meeting());
            cut
        })?
    })
    .await
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
        c.with_session_unlocked(|s| {
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
    core.with_session_unlocked(|s| s.rename(id, &name))
}

#[tauri::command]
#[specta::specta]
async fn merge_speakers(core: CoreState<'_>, from: u32, into: u32) -> Result<(), String> {
    core.with_session_unlocked(|s| s.merge(from, into))
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
        c.with_session_unlocked(|s| s.split(from, lines).map_err(|e| e.to_string()))?
    })
    .await
}

#[tauri::command]
#[specta::specta]
async fn speaker_not_a_person(core: CoreState<'_>, id: u32) -> Result<(), String> {
    core.with_session_unlocked(|s| s.not_a_person(id))
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
        // Only the main window asks (the panels have no quit dialog).
        let _ = QuitRequested {}.emit_to(app, "main");
    } else {
        app.exit(0);
    }
}

/// Quit from the popover (the menu bar has no app menu on Windows).
#[tauri::command]
#[specta::specta]
fn request_quit_app(app: tauri::AppHandle) {
    request_quit(&app);
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
            discard_from,
            rename_speaker,
            merge_speakers,
            split_speaker,
            speaker_not_a_person,
            quit_app,
            request_quit_app,
            library::list_meetings,
            library::set_meeting_title,
            library::note_lines,
            library::add_note_line,
            library::update_note_line,
            library::delete_note_line,
            library::discard_preview,
            library::set_consent_confirmed,
            library::set_meeting_sensitive,
            library::sensitive_next,
            library::set_sensitive_next,
            library::session_snapshot,
            library::delete_meeting,
            library::retry_meeting,
            library::take_recovered_meetings,
            library::known_speaker_names,
            speakers_cmd::meeting_speakers,
            speakers_cmd::rename_meeting_speaker,
            recovery_cmd::has_recovery_key,
            recovery_cmd::create_recovery_key,
            recovery_cmd::confirm_recovery_key,
            recovery_cmd::cancel_recovery_key,
            system::open_privacy_settings,
            test_capture,
            models_cmd::models_status,
            models_cmd::download_models,
            models_cmd::cancel_model_download,
            system::get_settings,
            system::update_settings,
            system::mic_permission,
            system::request_mic_permission,
            system::request_notifications,
            system::probe_system_audio,
            system::retry_capture,
            system::reply_meeting_detected,
            audio_protocol::issue_audio_sample,
            windows::show_main,
            panels::hide_popover,
            panels::set_mini_compact,
            panels::close_mini,
            panels::open_mini_recorder,
            panels::close_detect,
            detail::meeting_detail,
            detail::meeting_notes,
            detail::meeting_transcript,
            detail::update_segment_text,
            detail::set_segment_speaker,
            detail::update_note_block,
            diag_cmd::diagnostics_status,
            diag_cmd::reveal_diagnostics,
            diag_cmd::acknowledge_crash,
            detail::add_note_block,
            detail::delete_note_block,
            detail::add_action_item,
            detail::update_action_item,
            detail::set_action_done,
            detail::set_action_owner,
            detail::delete_action_item,
            detail::list_templates,
            detail::regenerate_notes,
            detail::retranscribe,
            settings_cmd::transcription_engine,
            settings_cmd::set_transcription_engine,
            detail::search_meetings,
            audio_protocol::issue_audio_play,
            audio_protocol::waveform_peaks,
            export_cmd::export_meeting,
            export_cmd::export_meetings,
            export_cmd::export_destination,
            export_cmd::choose_export_folder,
            export_cmd::obsidian_vault,
            export_cmd::choose_obsidian_vault,
            export_cmd::open_mail_draft,
            ghi_app::calendar_cmd::meeting_contacts,
            export_cmd::export_obsidian,
            export_cmd::meeting_as_text,
            export_cmd::reveal_last_export,
            import_cmd::pick_import_files,
            import_cmd::staged_files,
            import_cmd::unstage_files,
            import_cmd::start_import,
            import_cmd::cancel_import,
            cloud_cmd::cloud_keys,
            cloud_cmd::set_cloud_key,
            cloud_cmd::delete_cloud_key,
            cloud_cmd::cloud_models,
            cloud_cmd::cloud_preview,
            cloud_cmd::cloud_send,
            cloud_cmd::set_meeting_cloud_locked,
            cloud_cmd::cloud_request_log,
            cloud_cmd::ask_meeting,
            settings_cmd::vocabulary,
            settings_cmd::set_vocabulary,
            settings_cmd::set_vocabulary_packs,
            settings_cmd::ignore_learned_term,
            settings_cmd::export_everything,
            settings_cmd::delete_all_data,
            import_cmd::take_dropped_files,
            cloud_cmd::draft_followup_email,
            update_cmd::update_status,
            update_cmd::check_for_updates,
            update_cmd::install_update,
            lock_cmd::lock_state,
            lock_cmd::lock_now,
            lock_cmd::unlock,
            lock_cmd::set_app_lock,
            ask_cmd::ask_all_meetings,
            ask_cmd::related_meetings,
            people_cmd::list_people,
            people_cmd::person_detail,
            people_cmd::merge_people,
            people_cmd::delete_voice_data,
            people_cmd::remove_person_name,
            voice_cmd::voice_status,
            voice_cmd::enroll_voice_start,
            voice_cmd::enroll_voice_level,
            voice_cmd::enroll_voice_finish,
            voice_cmd::enroll_voice_cancel,
            speakers_cmd::set_speaker_me,
            speakers_cmd::clear_speaker_me,
            speakers_cmd::accept_voice_suggestion,
            speakers_cmd::dismiss_voice_suggestion,
            speakers_cmd::save_voice_profile,
            speakers_cmd::merge_meeting_speakers,
            speakers_cmd::split_meeting_speaker,
            speakers_cmd::set_speaker_not_person,
            calendar_cmd::calendar_status,
            calendar_cmd::request_calendar_access,
            calendar_cmd::set_calendar,
            calendar_cmd::pick_ics_file,
            calendar_cmd::remove_ics_file,
            calendar_cmd::upcoming_events,
            calendar_cmd::set_event_ask,
            ghi_app::calendar_cmd::meeting_attendees,
            organize_cmd::list_folders,
            organize_cmd::create_folder,
            organize_cmd::rename_folder,
            organize_cmd::delete_folder,
            organize_cmd::move_to_folder,
            organize_cmd::list_tags,
            organize_cmd::create_tag,
            organize_cmd::rename_tag,
            organize_cmd::delete_tag,
            organize_cmd::tag_meetings,
            organize_cmd::untag_meetings,
            import_cmd::import_tracks_separately,
            system::show_notification,
            ghi_app::sync_cmd::sync_status,
            ghi_app::sync_cmd::sync_set_enabled,
            ghi_app::sync_cmd::sync_pair_open,
            ghi_app::sync_cmd::sync_pair_close,
            ghi_app::sync_cmd::sync_devices,
            ghi_app::sync_cmd::sync_unpair,
            ghi_app::sync_cmd::sync_unpair_and_wipe,
            ghi_app::sync_cmd::sync_now,
            ghi_app::sync_cmd::sync_conflicts,
            ghi_app::sync_cmd::sync_conflict_resolve,
            ghi_app::sync_cmd::sync_confirm_mass_delete,
            ghi_app::sync_cmd::sync_delete_everywhere_status,
            ghi_app::sync_cmd::sync_delete_everywhere_skip,
            sync_export_cmd::sync_export_for_device,
            sync_export_cmd::sync_import_from_device
        ])
        .events(tauri_specta::collect_events![
            core::CoreEvent,
            menu::MenuAction,
            system::MeetingDetected,
            models_cmd::ModelDownload,
            Navigate,
            QuitRequested,
            import_cmd::ImportStaged,
            import_cmd::ImportUpdate,
            update_cmd::UpdateChanged,
            lock_cmd::LockChanged,
            ghi_app::sync_cmd::SyncEvent
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
    let app = tauri::Builder::default().plugin(navigation::guard());
    #[cfg(target_os = "macos")]
    let app = app.plugin(tauri_nspanel::init());
    let app = app
        .plugin(tray::shortcut_plugin())
        .plugin(tauri_plugin_notification::init());
    let app = app
        .invoke_handler(builder.invoke_handler())
        // Meeting audio for the webview, by token only (audio_protocol.rs).
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
            // First: the panic hook and the log cover everything after.
            app.manage(Arc::new(diag_cmd::Diag::init(app.handle())));
            let tray_app = app.handle().clone();
            let core = Arc::new(core::Core::new(app.handle(), move |e| {
                diag_cmd::track(e);
                tray::on_event(&tray_app, e)
            })?);
            // Onboarding finished or a setting changed: (un)register ⌘⇧R.
            core.set_settings_hook(Arc::new(|app| {
                tray::sync_record_shortcut(app);
                tray::sync_visibility(app);
                login_item::sync(app);
            }));
            core.init_in_background();
            let detection = Arc::new(system::Detection::default());
            system::spawn_detection(app.handle().clone(), core.clone(), detection.clone());
            system::spawn_retention(core.clone());
            // LAN sync with paired phones (phase 15): its listener exists only
            // while sync is on, someone is paired and the app is unlocked.
            let sync = sync_cmd::start(app.handle(), &core);
            app.manage(sync);
            app.manage(core);
            app.manage(detection);
            app.manage(tokens);
            app.manage(Arc::new(models_cmd::Downloads::default()));
            app.manage(Arc::new(recovery_cmd::PendingRecovery::default()));
            app.manage(Arc::new(import_cmd::Imports::default()));
            app.manage(Arc::new(dialogs::LastExport::default()));
            app.manage(Arc::new(cloud_cmd::CloudPlans::default()));
            app.manage(Arc::new(update_cmd::Updates::default()));
            app.manage(Arc::new(lock_cmd::Lock::default()));
            lock_cmd::spawn_watch(app.handle().clone());
            update_cmd::cleanup_after_update();
            update_cmd::spawn_checks(app.handle().clone());
            #[cfg(target_os = "macos")]
            {
                app.set_menu(menu::build(app.handle())?)?;
                app.on_menu_event(menu::on_event);
            }
            // Created here rather than in tauri.conf.json so `window.open` can
            // be denied (windows.rs).
            windows::main(app.handle(), None)?;
            tray::build(app.handle())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the Ghira desktop app");
    app.run(|app, event| match event {
        // Closing the main window hides it: the app lives in the menu bar
        // (detection, the popover). While recording, the mini-recorder
        // takes over. Quit is in the menu and the popover.
        RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "main" => {
            api.prevent_close();
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.hide();
            }
            if app.state::<Arc<core::Core>>().recording() {
                panels::open_mini(app);
            }
        }
        // Audio files dropped on the main window: import them (D10).
        RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }),
            ..
        } if label == "main" => import_cmd::dropped(app, paths),
        // ... or on the Dock icon / "Open With" (bundle file associations).
        #[cfg(target_os = "macos")]
        RunEvent::Opened { urls } => {
            let paths = urls.iter().filter_map(|u| u.to_file_path().ok()).collect();
            import_cmd::dropped(app, paths);
        }
        RunEvent::ExitRequested { api, code, .. } => {
            let core = app.state::<Arc<core::Core>>();
            // A user quit while recording asks first; quit_app answers.
            if code.is_none() && core.recording() {
                api.prevent_exit();
                request_quit(app);
                return;
            }
            app.state::<Arc<ghi_app::sync_service::SyncService>>()
                .stop();
            core.shutdown(Duration::from_secs(5));
        }
        // Last chance (e.g. the system logging out): save the recording so
        // it is processed at the next launch, then stop the jobs.
        RunEvent::Exit => {
            let core = app.state::<Arc<core::Core>>();
            let diag = app.state::<Arc<diag_cmd::Diag>>();
            if core.recording() {
                let _ = core.stop();
            }
            app.state::<Arc<ghi_app::sync_service::SyncService>>()
                .stop();
            core.shutdown(Duration::from_secs(2));
            diag.release();
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
    fn the_indexer_and_the_query_embedder_use_the_tier_embedding_model() {
        use ghi_models::tier::{Tier, preset};
        for t in [Tier::Light, Tier::Balanced, Tier::Max] {
            if let Some(id) = preset(t).embed_id {
                assert_eq!(id, ghi_core::index_job::MODEL_ID);
            }
        }
    }

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
