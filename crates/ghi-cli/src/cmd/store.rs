// SPDX-License-Identifier: Apache-2.0
//! `ghi store ...`: inspect and manage an encrypted Ghira store (a data
//! directory) from the command line. Dev and test tooling, not part of the
//! harness contract. Passwords are read from stdin, never from arguments.

use std::io::BufRead;
use std::path::Path;
use std::time::Instant;

use ghi_audio::encoder::read_ogg_opus_detailed;
use ghi_store::search::{HitKind, SearchQuery};
use ghi_store::store::{Store, TrackKind};
use serde_json::json;

use crate::contract::{ErrorCode, ErrorDoc};
use crate::keystore::{keystore, open_store, store_error};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum TrackArg {
    Mic,
    System,
}

impl TrackArg {
    fn kind(self) -> TrackKind {
        match self {
            TrackArg::Mic => TrackKind::Mic,
            TrackArg::System => TrackKind::System,
        }
    }
}

/// `ghi store list`: meetings, newest first, with their audio tracks.
pub fn list(dir: &Path) -> Result<(), ErrorDoc> {
    let store = open_store(dir)?;
    let mut meetings = Vec::new();
    for m in store.list_meetings(1000, 0).map_err(store_error)? {
        let tracks = store.tracks(&m.gid).map_err(store_error)?;
        meetings.push(json!({
            "gid": m.gid,
            "title": m.title,
            "started_at": m.started_at,
            "duration_s": m.duration_ms as f64 / 1000.0,
            "mode": m.mode,
            "status": m.status,
            "tracks": tracks.iter().map(|(k, pages)| json!({"kind": k.as_str(), "pages": pages})).collect::<Vec<_>>(),
        }));
    }
    crate::emit(&json!({"schema": "ghi.store-list/1", "meetings": meetings}))
}

/// `ghi store search`: accent-insensitive search over transcripts and notes.
/// Highlights are `[start, end)` char offsets into `snippet`.
pub fn search(dir: &Path, query: &str, limit: usize) -> Result<(), ErrorDoc> {
    let store = open_store(dir)?;
    let started = Instant::now();
    let mut q = SearchQuery::new(query);
    q.limit = limit;
    let hits = store.search(&q).map_err(store_error)?;
    let took_ms = started.elapsed().as_secs_f64() * 1000.0;
    let hits: Vec<_> = hits
        .iter()
        .map(|h| {
            json!({
                "kind": match h.kind { HitKind::Segment => "segment", HitKind::Note => "note" },
                "meeting_gid": h.meeting_gid,
                "meeting_title": h.meeting_title,
                "item_gid": h.item_gid,
                "t0_s": h.t0_ms.map(|t| t as f64 / 1000.0),
                "t1_s": h.t1_ms.map(|t| t as f64 / 1000.0),
                "snippet": h.snippet,
                "highlights": h.highlights.iter()
                    .map(|r| [r.start.saturating_sub(h.snippet_start), r.end.saturating_sub(h.snippet_start)])
                    .collect::<Vec<_>>(),
                "exact": h.exact,
                "score": h.score,
            })
        })
        .collect();
    crate::emit(&json!({"schema": "ghi.store-search/1", "hits": hits, "took_ms": took_ms}))
}

/// `ghi store audio`: decrypts one track of a meeting to a 16 kHz WAV file.
pub fn audio(dir: &Path, gid: &str, track: TrackArg, out: &Path) -> Result<(), ErrorDoc> {
    let store = open_store(dir)?;
    let reader = store.open_bundle(gid, track.kind()).map_err(store_error)?;
    let ogg = reader.read_all().map_err(store_error)?;
    let decoded = read_ogg_opus_detailed(ogg.as_slice())
        .map_err(|e| ErrorDoc::new(ErrorCode::Internal, format!("decode: {e}")))?;
    crate::sink::write_wav(out, &decoded.samples)
        .map_err(|e| ErrorDoc::new(ErrorCode::Internal, format!("write WAV: {e}")))?;
    crate::emit(&json!({
        "schema": "ghi.store-audio/1",
        "meeting_gid": gid,
        "track": track.kind().as_str(),
        "wav": out.display().to_string(),
        "duration_s": decoded.samples.len() as f64 / 16_000.0,
        "pages": reader.page_count(),
        "complete": reader.complete(),
    }))
}

/// `ghi store delete`: crypto-shreds one meeting.
pub fn delete(dir: &Path, gid: &str) -> Result<(), ErrorDoc> {
    let store = open_store(dir)?;
    store.delete_meeting(gid).map_err(store_error)?;
    crate::emit(&json!({"schema": "ghi.store-deleted/1", "meeting_gid": gid}))
}

/// The first line of stdin, without its line ending. Empty passwords are
/// refused: an archive's security is its password.
fn read_password() -> Result<zeroize::Zeroizing<String>, ErrorDoc> {
    let mut line = zeroize::Zeroizing::new(String::new());
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| ErrorDoc::new(ErrorCode::BadInput, format!("reading the password: {e}")))?;
    let len = line.trim_end_matches(['\r', '\n']).len();
    line.truncate(len);
    if line.is_empty() {
        return Err(ErrorDoc::new(
            ErrorCode::BadInput,
            "empty password on stdin",
        ));
    }
    Ok(line)
}

/// `ghi store export`: everything, in one password-encrypted archive.
pub fn export(dir: &Path, out: &Path) -> Result<(), ErrorDoc> {
    let password = read_password()?;
    let store = open_store(dir)?;
    store.export_all(out, &password).map_err(store_error)?;
    let bytes = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    crate::emit(
        &json!({"schema": "ghi.store-export/1", "archive": out.display().to_string(), "bytes": bytes}),
    )
}

/// `ghi store import`: restores an export into an empty data directory.
pub fn import(archive: &Path, dir: &Path) -> Result<(), ErrorDoc> {
    let password = read_password()?;
    let ks = keystore(dir)?;
    let store = Store::import_archive(archive, &password, dir, ks, Default::default())
        .map_err(store_error)?;
    let meetings = store.list_meetings(100_000, 0).map_err(store_error)?.len();
    crate::emit(
        &json!({"schema": "ghi.store-import/1", "dir": dir.display().to_string(), "meetings": meetings}),
    )
}

/// `ghi store add-transcript`: stores a `ghi.transcript/1` document as the
/// transcript of a meeting (a new `file` meeting unless `meeting` is given),
/// replacing any previous version. Makes transcribe → store → search testable
/// before the live pipeline (phase 8) writes transcripts itself.
pub fn add_transcript(
    dir: &Path,
    transcript: &Path,
    meeting: Option<&str>,
) -> Result<(), ErrorDoc> {
    let doc = crate::read_transcript(transcript)?;
    let store = open_store(dir)?;
    let gid = match meeting {
        Some(gid) => gid.to_owned(),
        None => {
            let new = ghi_store::store::NewMeeting {
                title: doc.audio.clone(),
                source: "file".into(),
                mode: "room".into(),
                ..Default::default()
            };
            store.create_meeting(new).map_err(store_error)?.gid
        }
    };
    let ms = |s: f64| (s * 1000.0).round() as i64;
    let segments = doc
        .segments
        .iter()
        .map(|s| ghi_store::store::NewSegment {
            t0_ms: ms(s.start),
            t1_ms: ms(s.end),
            text: s.text.clone(),
            lang: s.lang.map(|l| format!("{l:?}").to_lowercase()),
            words: s
                .words
                .iter()
                .flatten()
                .map(|w| ghi_store::store::Word {
                    t0_ms: ms(w.start),
                    t1_ms: ms(w.end),
                    conf: None,
                })
                .collect(),
            ..Default::default()
        })
        .collect();
    let version = store
        .replace_transcript(&gid, segments)
        .map_err(store_error)?;
    crate::emit(&json!({
        "schema": "ghi.store-transcript/1",
        "meeting_gid": gid,
        "transcript_version": version,
        "segments": doc.segments.len(),
    }))
}
