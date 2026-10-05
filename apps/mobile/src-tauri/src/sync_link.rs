// SPDX-License-Identifier: Apache-2.0
//! The phone's platform for LAN sync (phase 15, slice 15-J2): what
//! `ghi_app::sync_service::SyncService` (as the spoke) asks of iOS. The camera
//! scanner and the Bonjour browse are Swift (`GhiSync.swift`, over the C ABI in
//! `platform.rs`); the sockets are `ghi-net`'s (the trait's default connect).
//! The scanned text is a secret: it passes through here to the service and is
//! never logged.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ghi_app::sync_service::{ScanError, SpokeLink};
use ghi_net::lan::Discovery;

use crate::platform::{self, QrError};

/// How often a running scan looks at its cancel flag.
const SCAN_POLL: Duration = Duration::from_millis(250);

pub struct MobileLink {
    recording: Arc<dyn Fn() -> bool + Send + Sync>,
}

impl MobileLink {
    /// `recording` says whether a recording is running (changes then wait for
    /// the periodic pass).
    pub fn new(recording: Arc<dyn Fn() -> bool + Send + Sync>) -> Arc<Self> {
        Arc::new(MobileLink { recording })
    }
}

impl SpokeLink for MobileLink {
    fn discovered(&self) -> Vec<SocketAddr> {
        platform::pushed_discovery().candidates()
    }

    fn browse(&self, on: bool) {
        if on {
            platform::browse_start();
        } else {
            platform::browse_stop();
        }
    }

    fn scan(&self, cancel: &AtomicBool) -> Result<String, ScanError> {
        // A result nobody asked for must not pass as this scan's.
        platform::qr_drain();
        platform::qr_scan_start();
        let until = Instant::now() + ghi_app::sync_service::spoke::SCAN_TIMEOUT;
        let result = loop {
            if cancel.load(Ordering::Acquire) {
                break Err(ScanError::Cancelled);
            }
            if Instant::now() >= until {
                break Err(ScanError::Timeout);
            }
            match platform::qr_recv(SCAN_POLL) {
                None => {}
                Some(Ok(text)) => break Ok(text),
                Some(Err(QrError::Denied)) => break Err(ScanError::Denied),
                Some(Err(QrError::Unavailable)) => break Err(ScanError::Unavailable),
                Some(Err(QrError::Cancelled)) => break Err(ScanError::Cancelled),
            }
        };
        platform::qr_scan_stop();
        result
    }

    fn begin_bg(&self) -> u64 {
        platform::begin_bg_task("sync")
    }

    fn end_bg(&self, token: u64) {
        platform::end_bg_task(token);
    }

    fn capable(&self) -> bool {
        crate::cmd::meetings::device_is_live()
    }

    fn recording(&self) -> bool {
        (self.recording)()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    fn link() -> Arc<MobileLink> {
        MobileLink::new(Arc::new(|| false))
    }

    fn inject(text: &str) {
        let c = CString::new(text).unwrap();
        // SAFETY: a valid NUL-terminated string for the duration of the call.
        unsafe { platform::ghi_ios_qr_scanned(c.as_ptr()) };
    }

    /// Runs a scan, and has the scanner answer with `text` a moment after it
    /// started (a result sent before is dropped as left over).
    fn scan_answered(link: &Arc<MobileLink>, text: &str) -> Result<String, ScanError> {
        let l = link.clone();
        let t = std::thread::spawn(move || l.scan(&AtomicBool::new(false)));
        std::thread::sleep(Duration::from_millis(300));
        inject(text);
        t.join().unwrap()
    }

    /// The scanner's results pass through the platform channel to the service.
    /// The channel is process-wide: these steps take turns in one test.
    #[test]
    fn a_scan_returns_the_code_or_the_reason_it_failed() {
        let link = link();
        assert_eq!(scan_answered(&link, "GHI1:FAKE"), Ok("GHI1:FAKE".into()));
        // Left over from an earlier scan: dropped at the start of the next.
        inject("GHI1:STALE");
        assert_eq!(
            scan_answered(&link, "\u{1}ghi-error:denied"),
            Err(ScanError::Denied)
        );
        assert_eq!(
            scan_answered(&link, "\u{1}ghi-error:unavailable"),
            Err(ScanError::Unavailable)
        );
        assert_eq!(
            scan_answered(&link, "\u{1}ghi-error:cancelled"),
            Err(ScanError::Cancelled)
        );
        // The service closing the scanner.
        let stop = Arc::new(AtomicBool::new(false));
        let (l, s2) = (link.clone(), stop.clone());
        let t = std::thread::spawn(move || l.scan(&s2));
        std::thread::sleep(Duration::from_millis(300));
        stop.store(true, Ordering::SeqCst);
        assert_eq!(t.join().unwrap(), Err(ScanError::Cancelled));
    }

    #[test]
    fn the_discovery_the_browse_pushed_is_what_the_service_tries() {
        let link = link();
        platform::pushed_discovery().set(["192.168.1.9:7000".parse().unwrap()]);
        assert_eq!(link.discovered().len(), 1);
        platform::pushed_discovery().set([]);
        assert!(link.discovered().is_empty());
    }
}
