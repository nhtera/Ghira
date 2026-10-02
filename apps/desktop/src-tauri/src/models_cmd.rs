// SPDX-License-Identifier: Apache-2.0
//! Speech and notes models for onboarding and Settings → Models: what this
//! machine's tier needs, and an in-app download (pinned files from the
//! registry through `ghi-net`; resumable, cancellable, with an idle timeout).
//! Strict offline (a setting) refuses before any connection. When a model
//! arrives, the job runner is woken: recordings made without models get their
//! transcript and notes ("record now, process later").

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use ghi_net::NetPolicy;
use ghi_net::fetch::{Control, Progress, UreqTransport};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime};
use tauri_specta::Event;

use crate::core::Core;
use crate::system::load_settings;
use crate::{CoreState, blocking};

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    /// `asr`, `diarization`, `llm`.
    pub role: String,
    pub size: f64,
    pub installed: bool,
    /// Bytes of an unfinished download (resumes from here).
    pub partial_bytes: f64,
    /// Failed its SHA-256 at load: download it again (D12).
    pub damaged: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelsStatus {
    /// `light`, `balanced` or `max` (hardware tier).
    pub tier: String,
    pub models: Vec<ModelInfo>,
    pub downloading: bool,
}

/// Download progress for one model.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownload {
    pub model: String,
    pub phase: DownloadPhase,
    pub done: f64,
    pub total: f64,
    /// Why it stopped (failed).
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DownloadPhase {
    Downloading,
    /// Checking the SHA-256 of the finished file.
    Verifying,
    Done,
    Failed,
    Cancelled,
}

/// The one download in progress (the cancel flag of the current run).
#[derive(Default)]
pub struct Downloads(Mutex<Option<Arc<AtomicBool>>>);

fn status(core: &Core, downloads: &Downloads) -> ModelsStatus {
    let (tier, models) = ghi_models::required_for_machine(&core.models());
    let damaged = crate::core::damaged_models();
    ModelsStatus {
        tier: format!("{tier:?}").to_lowercase(),
        models: models
            .into_iter()
            .map(|m| ModelInfo {
                damaged: damaged.contains(&m.id),
                id: m.id,
                role: m.role,
                size: m.size as f64,
                installed: m.installed,
                partial_bytes: m.partial_bytes as f64,
            })
            .collect(),
        downloading: downloads
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some(),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn models_status(
    core: CoreState<'_>,
    downloads: tauri::State<'_, Arc<Downloads>>,
) -> Result<ModelsStatus, String> {
    let downloads = downloads.inner().clone();
    blocking(&core, move |c| Ok(status(c, &downloads))).await
}

/// Downloads every missing model of this machine's tier, one after another,
/// in the background (`modelDownload` events). A second call while running
/// does nothing.
#[tauri::command]
#[specta::specta]
pub async fn download_models(
    app: AppHandle,
    core: CoreState<'_>,
    downloads: tauri::State<'_, Arc<Downloads>>,
) -> Result<(), String> {
    let cancel = {
        let mut slot = downloads.0.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return Ok(());
        }
        let flag = Arc::new(AtomicBool::new(false));
        *slot = Some(flag.clone());
        flag
    };
    let core = core.inner().clone();
    let downloads = downloads.inner().clone();
    std::thread::Builder::new()
        .name("ghi-models-download".into())
        .spawn(move || {
            // Cleared however `run` ends (a panic included), or the UI would
            // show "downloading" until the next launch.
            struct Clear(Arc<Downloads>);
            impl Drop for Clear {
                fn drop(&mut self) {
                    *self.0.0.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
            }
            let _clear = Clear(downloads);
            run(&app, &core, &cancel);
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Stops the download (the partial file stays; the next download resumes).
#[tauri::command]
#[specta::specta]
pub fn cancel_model_download(downloads: tauri::State<'_, Arc<Downloads>>) {
    if let Some(flag) = downloads
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        flag.store(true, Ordering::Release);
    }
}

fn run<R: Runtime>(app: &AppHandle<R>, core: &Core, cancel: &Arc<AtomicBool>) {
    let dir = core.models();
    let strict = load_settings(core)
        .map(|s| s.strict_offline)
        .unwrap_or(true);
    let policy = if strict {
        NetPolicy::StrictOffline
    } else {
        NetPolicy::Default
    };
    let ctl = Control {
        cancel: Some(cancel.clone()),
        ..Control::default()
    };
    let (_, required) = ghi_models::required_for_machine(&dir);
    let damaged = crate::core::damaged_models();
    for r in required
        .into_iter()
        .filter(|r| !r.installed || damaged.contains(&r.id))
    {
        let Some(model) = ghi_models::find(&r.id) else {
            continue;
        };
        let emit = |phase, done: u64, error: Option<String>| {
            let _ = ModelDownload {
                model: r.id.clone(),
                phase,
                done: done as f64,
                total: r.size as f64,
                error,
            }
            .emit(app);
        };
        let mut progress = |p: Progress<'_>| match p {
            Progress::Bytes { done, .. } => emit(DownloadPhase::Downloading, done, None),
            Progress::Verifying => emit(DownloadPhase::Verifying, r.size, None),
        };
        let _ = std::fs::create_dir_all(&dir);
        match ghi_models::download(&model, &dir, policy, &ctl, &mut progress, &UreqTransport) {
            Ok(_) => {
                crate::core::clear_damaged(&r.id);
                emit(DownloadPhase::Done, r.size, None);
                // Queued transcripts and notes can run now.
                // Meetings recorded before the embedding model arrived.
                if let Ok(store) = core.store_even_locked() {
                    let _ = ghi_core::index_job::queue_missing(&store);
                }
                core.notify_jobs();
            }
            Err(ghi_models::DownloadError::Cancelled) => {
                // What is on disk now (the next download resumes from it).
                let kept = std::fs::metadata(ghi_net::fetch::part_path(&ghi_models::path_in(
                    &dir, &model,
                )))
                .map(|m| m.len())
                .unwrap_or(r.partial_bytes);
                emit(DownloadPhase::Cancelled, kept, None);
                return;
            }
            Err(e) => {
                emit(DownloadPhase::Failed, r.partial_bytes, Some(e.to_string()));
                return;
            }
        }
    }
}
