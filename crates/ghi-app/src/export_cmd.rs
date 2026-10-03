// SPDX-License-Identifier: Apache-2.0
//! Export (doc 02 §N), the part the apps share: the format and content
//! choices, the text for the clipboard, and the "last export" state. The
//! files are rendered by `ghi_core::export`; saving them after a native
//! dialog is the desktop's.

use std::path::PathBuf;
use std::sync::Mutex;

use ghi_core::export::{self, ExportOptions, Format, Lang};
use serde::Deserialize;
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
