// SPDX-License-Identifier: Apache-2.0
//! Pinned downloads: Hugging Face first, then the model's mirrors, through
//! `ghi-net`. The registry's size and SHA-256 decide what is accepted; a source
//! only ever supplies bytes.

use std::fs;
use std::path::Path;

use ghi_net::NetPolicy;
use ghi_net::fetch::{Control, FetchError, FetchOpts, FetchReport, Progress, Transport, fetch};

use crate::verify::verify_file;
use crate::{Model, path_in};

#[derive(Debug)]
pub enum DownloadError {
    /// Refused by policy (strict offline, host not allowed); no source was
    /// tried further.
    Denied(String),
    /// Every source failed; the last error is kept.
    Failed(FetchError),
    /// Stopped through [`Control::cancel`]; the `.part` is kept for a resume.
    Cancelled,
    Io(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Denied(d) => write!(f, "not allowed: {d}"),
            DownloadError::Failed(e) => write!(f, "download failed: {e}"),
            DownloadError::Cancelled => f.write_str("download cancelled"),
            DownloadError::Io(d) => write!(f, "file error: {d}"),
        }
    }
}

impl std::error::Error for DownloadError {}

/// The URLs to try, in order: Hugging Face `resolve/` at the pinned revision,
/// then each mirror (`<base>/<file>`).
pub fn source_urls(m: &Model) -> Vec<String> {
    let mut urls = vec![format!(
        "https://huggingface.co/{}/resolve/{}/{}",
        m.repo, m.revision, m.file
    )];
    urls.extend(
        m.mirrors
            .iter()
            .map(|base| format!("{}/{}", base.trim_end_matches('/'), m.file)),
    );
    urls
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split('/').next()?;
    Some(authority.split(':').next()?.to_ascii_lowercase())
}

/// Installs `m` into `dir`. A file already there that verifies is left alone
/// (`downloaded == 0`). Resumes a `.part`; on a failed or stalled source
/// (`ctl.idle_timeout`), tries the next, which continues from the same `.part`.
/// `progress` is rate limited and ends with [`Progress::Verifying`]; `ctl.cancel`
/// stops with [`DownloadError::Cancelled`], no further source tried.
pub fn download(
    m: &Model,
    dir: &Path,
    policy: NetPolicy,
    ctl: &Control,
    progress: &mut dyn FnMut(Progress<'_>),
    transport: &dyn Transport,
) -> Result<FetchReport, DownloadError> {
    let dest = path_in(dir, m);
    if dest.exists() {
        progress(Progress::Verifying);
    }
    if verify_file(&dest, m).is_ok() {
        return Ok(FetchReport {
            downloaded: 0,
            resumed_from: 0,
            redirects: 0,
            final_host: String::new(),
        });
    }
    // A wrong file in place (e.g. an old revision) is replaced, not resumed.
    if dest.exists() {
        fs::remove_file(&dest).map_err(|e| DownloadError::Io(e.to_string()))?;
    }
    let mut last = None;
    for url in source_urls(m) {
        let opts = FetchOpts {
            expected_sha256: m.sha256.clone(),
            expected_size: m.size,
            resume: true,
            // Mirror hosts come from the embedded registry, which is trusted.
            extra_hosts: m.mirrors.iter().filter_map(|b| host_of(b)).collect(),
            control: ctl.clone(),
        };
        match fetch(policy, &url, &dest, &opts, progress, transport) {
            Ok(report) => return Ok(report),
            Err(FetchError::Cancelled) => return Err(DownloadError::Cancelled),
            Err(FetchError::Denied(d)) if policy == NetPolicy::StrictOffline => {
                return Err(DownloadError::Denied(d));
            }
            Err(e) => last = Some(e),
        }
    }
    match last {
        Some(FetchError::Denied(d)) => Err(DownloadError::Denied(d)),
        Some(e) => Err(DownloadError::Failed(e)),
        None => Err(DownloadError::Io("no source".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::Cursor;

    use ghi_net::NetError;
    use ghi_net::fetch::Response;
    use sha2::{Digest, Sha256};

    const DATA: &[u8] = b"model bytes, twenty+";

    fn model(mirrors: Vec<String>) -> Model {
        Model {
            id: "t".into(),
            role: "asr".into(),
            repo: "org/repo".into(),
            revision: "a".repeat(40),
            file: "t.gguf".into(),
            sha256: Sha256::digest(DATA)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            size: DATA.len() as u64,
            license: "MIT".into(),
            chat_format: None,
            mirrors,
            optional: false,
        }
    }

    /// Serves `DATA` for hosts in `good`, a 503 for any other host.
    struct Routed {
        good: Vec<&'static str>,
        urls: RefCell<Vec<String>>,
    }

    impl Transport for Routed {
        fn get(&self, url: &str, _: Option<u64>) -> Result<Response, NetError> {
            self.urls.borrow_mut().push(url.to_owned());
            let host = host_of(url).unwrap();
            let (status, body) = if self.good.contains(&host.as_str()) {
                (200, DATA.to_vec())
            } else {
                (503, vec![])
            };
            Ok(Response {
                status,
                location: None,
                content_length: Some(body.len() as u64),
                content_range: None,
                body: Box::new(Cursor::new(body)),
            })
        }
    }

    #[test]
    fn builds_pinned_urls() {
        let m = model(vec!["https://mirror.example.org/m/".into()]);
        assert_eq!(
            source_urls(&m),
            [
                format!(
                    "https://huggingface.co/org/repo/resolve/{}/t.gguf",
                    "a".repeat(40)
                ),
                "https://mirror.example.org/m/t.gguf".to_owned(),
            ]
        );
    }

    #[test]
    fn falls_back_to_a_mirror_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let m = model(vec!["https://mirror.example.org/m".into()]);
        let t = Routed {
            good: vec!["mirror.example.org"],
            urls: RefCell::new(vec![]),
        };
        let r = download(
            &m,
            dir.path(),
            NetPolicy::Default,
            &Control::default(),
            &mut |_| {},
            &t,
        )
        .unwrap();
        assert_eq!(r.downloaded, DATA.len() as u64);
        assert_eq!(t.urls.borrow().len(), 2);
        verify_file(&path_in(dir.path(), &m), &m).unwrap();
    }

    #[test]
    fn all_sources_failing_reports_the_last_error() {
        let dir = tempfile::tempdir().unwrap();
        let m = model(vec![]);
        let t = Routed {
            good: vec![],
            urls: RefCell::new(vec![]),
        };
        let e = download(
            &m,
            dir.path(),
            NetPolicy::Default,
            &Control::default(),
            &mut |_| {},
            &t,
        )
        .unwrap_err();
        assert!(matches!(e, DownloadError::Failed(FetchError::Status(503))));
    }

    #[test]
    fn strict_offline_stops_at_the_first_source() {
        let dir = tempfile::tempdir().unwrap();
        let m = model(vec!["https://mirror.example.org/m".into()]);
        let t = Routed {
            good: vec!["huggingface.co"],
            urls: RefCell::new(vec![]),
        };
        let e = download(
            &m,
            dir.path(),
            NetPolicy::StrictOffline,
            &Control::default(),
            &mut |_| {},
            &t,
        )
        .unwrap_err();
        assert!(matches!(e, DownloadError::Denied(_)));
        assert!(t.urls.borrow().is_empty());
    }

    #[test]
    fn same_size_file_with_a_bad_hash_is_downloaded_again() {
        let dir = tempfile::tempdir().unwrap();
        let m = model(vec![]);
        let mut bad = DATA.to_vec();
        bad[0] ^= 1;
        fs::write(path_in(dir.path(), &m), &bad).unwrap();
        let t = Routed {
            good: vec!["huggingface.co"],
            urls: RefCell::new(vec![]),
        };
        let r = download(
            &m,
            dir.path(),
            NetPolicy::Default,
            &Control::default(),
            &mut |_| {},
            &t,
        )
        .unwrap();
        assert_eq!(r.downloaded, DATA.len() as u64);
        verify_file(&path_in(dir.path(), &m), &m).unwrap();
    }

    #[test]
    fn installed_and_verified_is_not_downloaded_again() {
        let dir = tempfile::tempdir().unwrap();
        let m = model(vec![]);
        fs::write(path_in(dir.path(), &m), DATA).unwrap();
        let t = Routed {
            good: vec![],
            urls: RefCell::new(vec![]),
        };
        let r = download(
            &m,
            dir.path(),
            NetPolicy::StrictOffline,
            &Control::default(),
            &mut |_| {},
            &t,
        )
        .unwrap();
        assert_eq!(r.downloaded, 0);
        assert!(t.urls.borrow().is_empty());
    }
}
