// SPDX-License-Identifier: Apache-2.0
//! Headless CLI used by the eval harness and tests.
//!
//! `ghi` prints the JSON documents in [`contract`]. `transcribe`, `diarize`
//! and `bench` run the speech engines when built with the `nemo` feature;
//! `notes` is a stub until phase 6 (`not_implemented`, exit code 3).

pub mod audio;
pub mod cmd;
pub mod contract;
pub mod engine;

use std::path::Path;

use contract::{ErrorCode, ErrorDoc, TRANSCRIPT, Transcript, VERSION, Version};

/// Crate version, used by `ghi --version` and the About screen.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The `ghi version --json` document.
pub fn version_doc() -> Version {
    Version {
        schema: VERSION.to_owned(),
        ghi: version().to_owned(),
        core: ghi_core::version().to_owned(),
        engines: engine::engines(),
    }
}

/// Prints one JSON document on stdout. Never panics: `println!` would abort
/// the process (release builds use `panic = "abort"`) when stdout is closed.
pub fn emit<T: serde::Serialize>(doc: &T) -> Result<(), ErrorDoc> {
    use std::io::Write;
    let line = serde_json::to_string(doc)
        .map_err(|e| ErrorDoc::new(ErrorCode::Internal, e.to_string()))?;
    writeln!(std::io::stdout().lock(), "{line}")
        .map_err(|e| ErrorDoc::new(ErrorCode::Internal, format!("writing stdout: {e}")))
}

/// Writes a line to stderr, ignoring failures (stderr may be closed too).
pub fn warn(message: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr().lock(), "{message}");
}

/// Fails with `bad_input` unless `path` is an existing file.
pub fn check_input_file(path: &Path) -> Result<(), ErrorDoc> {
    if path.is_file() {
        Ok(())
    } else {
        Err(ErrorDoc::new(
            ErrorCode::BadInput,
            format!("input file not found: {}", path.display()),
        ))
    }
}

/// Reads a `ghi.transcript/1` document, the input of `ghi notes`.
pub fn read_transcript(path: &Path) -> Result<Transcript, ErrorDoc> {
    check_input_file(path)?;
    let bad = |e: &dyn std::fmt::Display| {
        ErrorDoc::new(
            ErrorCode::BadInput,
            format!("not a {TRANSCRIPT} document: {}: {e}", path.display()),
        )
    };
    let text = std::fs::read_to_string(path).map_err(|e| bad(&e))?;
    let transcript: Transcript = serde_json::from_str(&text).map_err(|e| bad(&e))?;
    if transcript.schema != TRANSCRIPT {
        return Err(bad(&format!("schema is `{}`", transcript.schema)));
    }
    Ok(transcript)
}

/// The error every engine-backed command returns until its engine exists.
pub fn not_implemented(command: &str, missing: &str) -> ErrorDoc {
    ErrorDoc::new(ErrorCode::NotImplemented, format!("{command}: {missing}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/eval/tests/fixtures/cli/"
    );

    #[test]
    fn version_doc_reports_crate_versions() {
        let v = version_doc();
        assert_eq!(v.schema, VERSION);
        assert_eq!(v.ghi, env!("CARGO_PKG_VERSION"));
        assert_eq!(v.core, ghi_core::version());
    }

    #[test]
    fn reads_golden_transcript() {
        let t = read_transcript(Path::new(&format!("{FIXTURES}transcript.json"))).unwrap();
        assert_eq!(t.segments.len(), 2);
    }

    #[test]
    fn rejects_other_documents_as_transcript() {
        let err = read_transcript(Path::new(&format!("{FIXTURES}notes.json"))).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadInput);
        let err = read_transcript(Path::new("does-not-exist.json")).unwrap_err();
        assert_eq!(err.code, ErrorCode::BadInput);
    }
}
