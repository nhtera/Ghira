// SPDX-License-Identifier: Apache-2.0
//! Export (doc 02 §N), the part the apps share: the format and content
//! choices, the text for the clipboard, and the "last export" state. The
//! files are rendered by `ghi_core::export`; saving them after a native
//! dialog is the desktop's.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ghi_core::export::{self, ExportOptions, Format, Lang};
use ghi_store::StoreError;
use ghi_store::store::Store;
use ghi_sync::SyncError;
use ghi_sync::export::{ExportReport, MIN_PASSPHRASE_CHARS, export_for_device, import_from_device};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::{CoreState, blocking};

/// The last file or folder written by an export, for "Show in Finder".
#[derive(Default)]
pub struct LastExport(pub Mutex<Option<PathBuf>>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    Markdown,
    Text,
    Srt,
    Vtt,
    Docx,
}

impl From<ExportFormat> for Format {
    fn from(f: ExportFormat) -> Format {
        match f {
            ExportFormat::Markdown => Format::Markdown,
            ExportFormat::Text => Format::Text,
            ExportFormat::Srt => Format::Srt,
            ExportFormat::Vtt => Format::Vtt,
            ExportFormat::Docx => Format::Docx,
        }
    }
}

/// What goes into the file; headings in the app's language.
#[derive(Debug, Clone, Copy, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportContent {
    pub notes: bool,
    pub transcript: bool,
    /// Headings in Vietnamese (else English).
    pub vietnamese: bool,
}

impl From<ExportContent> for ExportOptions {
    fn from(c: ExportContent) -> ExportOptions {
        ExportOptions {
            include_notes: c.notes,
            include_transcript: c.transcript,
            ui_lang: if c.vietnamese { Lang::Vi } else { Lang::En },
        }
    }
}

/// The meeting as Markdown or plain text, for the clipboard.
#[tauri::command]
#[specta::specta]
pub async fn meeting_as_text(
    core: CoreState<'_>,
    meeting: String,
    markdown: bool,
    content: ExportContent,
) -> Result<String, String> {
    blocking(&core, move |c| {
        let f = if markdown {
            Format::Markdown
        } else {
            Format::Text
        };
        let bytes = export::render(&*c.store()?, &meeting, f, &content.into())?;
        String::from_utf8(bytes).map_err(|e| e.to_string())
    })
    .await
}

// ------------------------------------------- "Export for another device"

/// Error codes of [`device_export`] / [`device_import`] the UI words itself;
/// anything else is a plain message (never meeting content).
pub const ERR_PASSPHRASE_SHORT: &str = "passphrase_short";
pub const ERR_WRONG_PASSPHRASE: &str = "wrong_passphrase";
pub const ERR_NOT_AN_EXPORT: &str = "not_an_export";

/// What an export wrote or an import took: counts and the file's name (never
/// its path: the webview doesn't see paths).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTransfer {
    pub file_name: String,
    pub meetings: u32,
    /// Meetings in the file this device had deleted, so it did not take them.
    pub refused: u32,
    pub tracks: u32,
}

fn transfer(path: &Path, r: ExportReport) -> DeviceTransfer {
    let n = |v: usize| u32::try_from(v).unwrap_or(u32::MAX);
    DeviceTransfer {
        file_name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        meetings: n(r.meetings),
        refused: n(r.refused),
        tracks: n(r.tracks),
    }
}

/// The UI-facing text of a failure: a code the UI words, or the (content-free)
/// message.
pub fn transfer_error(e: SyncError) -> String {
    match e {
        SyncError::Store(StoreError::Decrypt) => ERR_WRONG_PASSPHRASE.into(),
        SyncError::Store(StoreError::Invalid(m)) if m.contains("passphrase is too short") => {
            ERR_PASSPHRASE_SHORT.into()
        }
        SyncError::Store(StoreError::Invalid(m)) if m.contains("not an export") => {
            ERR_NOT_AN_EXPORT.into()
        }
        // Not a Ghira file at all (the container's own check).
        SyncError::Store(StoreError::Invalid(m)) if m.contains("not a Ghira archive") => {
            ERR_NOT_AN_EXPORT.into()
        }
        other => other.to_string(),
    }
}

/// Writes the sealed export (`meetings`, or every finished one if `None`) to
/// `path`. The passphrase is checked here too, so no dialog is shown for one
/// that would be refused.
pub fn device_export(
    store: &Store,
    meetings: Option<&[String]>,
    passphrase: &str,
    path: &Path,
) -> Result<DeviceTransfer, String> {
    if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(ERR_PASSPHRASE_SHORT.into());
    }
    export_for_device(store, meetings, passphrase, path)
        .map(|r| transfer(path, r))
        .map_err(transfer_error)
}

/// Merges a sealed export from another device into the store.
///
/// `process`: the computer (a device that runs final passes) also queues the
/// final pass of what a phone recorded for it and nobody processed (see
/// [`ghi_core::recover::queue_imported_passes`]). The job runner picks it up
/// within its next wake-up.
pub fn device_import(
    store: &Store,
    path: &Path,
    passphrase: &str,
    process: bool,
) -> Result<DeviceTransfer, String> {
    let report = import_from_device(store, path, passphrase).map_err(transfer_error)?;
    if process && let Err(e) = ghi_core::recover::queue_imported_passes(store, &report.taken) {
        // The meetings are in; the next import (or the phone's own sync)
        // can still get them processed.
        log::warn!("imported meetings not queued for processing: {e}");
    }
    Ok(transfer(path, report))
}

#[cfg(test)]
mod device_tests {
    use std::sync::Arc;

    use ghi_store::keys::{MemoryKeyStore, Protection};
    use ghi_store::store::NewMeeting;

    use super::*;

    fn store(dir: &Path) -> Store {
        Store::open(
            dir,
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap()
    }

    #[test]
    fn export_then_import_through_the_command_helpers() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (store(&t.path().join("a")), store(&t.path().join("b")));
        let m = a
            .create_meeting(NewMeeting {
                title: "Họp tuần".into(),
                ..Default::default()
            })
            .unwrap();
        a.finish_meeting(&m.gid, 1_000).unwrap();
        let file = t.path().join("Ghira transfer.ghix");

        // Too short: refused with a code before anything is written.
        assert_eq!(
            device_export(&a, None, "short", &file).unwrap_err(),
            ERR_PASSPHRASE_SHORT
        );
        assert!(!file.exists());

        let out = device_export(&a, None, "correct horse", &file).unwrap();
        assert_eq!(out.file_name, "Ghira transfer.ghix");
        assert_eq!((out.meetings, out.refused, out.tracks), (1, 0, 0));

        assert_eq!(
            device_import(&b, &file, "wrong horse", false).unwrap_err(),
            ERR_WRONG_PASSPHRASE
        );
        assert!(b.list_meetings(10, 0).unwrap().is_empty());
        let took = device_import(&b, &file, "correct horse", false).unwrap();
        assert_eq!((took.meetings, took.refused), (1, 0));
        assert_eq!(b.get_meeting(&m.gid).unwrap().title, "Họp tuần");

        // Any other file is "not an export".
        let junk = t.path().join("junk.ghix");
        std::fs::write(&junk, b"not a Ghira archive at all, just text").unwrap();
        assert_eq!(
            device_import(&b, &junk, "correct horse", false).unwrap_err(),
            ERR_NOT_AN_EXPORT
        );
    }

    #[test]
    fn a_phone_meeting_nobody_processed_gets_its_final_pass_on_the_computer() {
        use ghi_core::session::FINAL_PASS_JOB;
        use ghi_store::store::TrackKind;

        let t = tempfile::tempdir().unwrap();
        let (phone, mac) = (store(&t.path().join("a")), store(&t.path().join("b")));
        let make = |title: &str, status: &str, audio: bool| {
            let m = phone
                .create_meeting(NewMeeting {
                    title: title.into(),
                    ..Default::default()
                })
                .unwrap()
                .gid;
            if audio {
                let mut w = phone.open_track(&m, TrackKind::Mic).unwrap();
                w.append(b"page").unwrap();
                phone.finish_track(&m, TrackKind::Mic, w).unwrap();
            }
            phone.finish_meeting(&m, 1_000).unwrap();
            phone.set_meeting_status(&m, status).unwrap();
            m
        };
        let waiting = make("chờ xử lý", "processing", true);
        let no_audio = make("không có âm thanh", "processing", false);
        let done = make("đã xong", "ready", true);
        let file = t.path().join("phone.ghix");
        device_export(&phone, None, "correct horse", &file).unwrap();

        // A phone importing it (process = false) queues nothing.
        let other = store(&t.path().join("c"));
        device_import(&other, &file, "correct horse", false).unwrap();
        assert!(other.jobs_for_meeting(&waiting).unwrap().is_empty());

        device_import(&mac, &file, "correct horse", true).unwrap();
        assert!(mac.meeting_audio_origin(&waiting).unwrap().is_some());
        let jobs = mac.jobs_for_meeting(&waiting).unwrap();
        assert_eq!(jobs.len(), 1, "{jobs:?}");
        assert_eq!(jobs[0].kind, FINAL_PASS_JOB);
        assert_eq!(jobs[0].payload["epoch"], 1);
        assert!(jobs[0].payload.get("lease").is_none(), "no live grantor");
        assert!(mac.jobs_for_meeting(&no_audio).unwrap().is_empty());
        assert!(mac.jobs_for_meeting(&done).unwrap().is_empty());

        // The same file again queues nothing twice.
        device_import(&mac, &file, "correct horse", true).unwrap();
        assert_eq!(mac.jobs_for_meeting(&waiting).unwrap().len(), 1);
    }

    #[test]
    fn failures_become_codes_the_ui_words() {
        let invalid = |m: &str| SyncError::Store(StoreError::Invalid(m.into()));
        assert_eq!(
            transfer_error(SyncError::Store(StoreError::Decrypt)),
            ERR_WRONG_PASSPHRASE
        );
        assert_eq!(
            transfer_error(invalid("the passphrase is too short")),
            ERR_PASSPHRASE_SHORT
        );
        assert_eq!(
            transfer_error(invalid("this file is not an export for another device")),
            ERR_NOT_AN_EXPORT
        );
        assert_eq!(
            transfer_error(invalid("not a Ghira archive")),
            ERR_NOT_AN_EXPORT
        );
        assert_eq!(transfer_error(SyncError::Closed), "connection closed");
    }
}
