// SPDX-License-Identifier: Apache-2.0
//! Speech models on the phone (M1 step, Settings → Models): what is
//! installed, and the download of what is missing through `ghi-models` and
//! `ghi-net` (pinned size and SHA-256, resume from `<file>.part`, the model
//! hosts only). Wi-Fi only by default: on a cellular path the download waits
//! (`WaitingForWifi`) until the user allows cellular for this time.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ghi_models::{DownloadError, Model};
use ghi_net::NetPolicy;
use ghi_net::fetch::{Control, Progress, Transport, UreqTransport, part_path};

use crate::cmd::events::{MobileEvent, emit};
use crate::cmd::models::{MobileModelItem, MobileModelRole, MobileModelState, MobileModelsStatus};

/// The models a phone needs: the live and final engines, then the voice model
/// (Me). The notes model is optional ([`notes_model`]); the embedding model
/// never comes to the phone.
pub fn needed() -> Vec<(String, MobileModelRole)> {
    let voice = ghi_models::preset(ghi_models::Tier::Light).voice_id;
    [
        (crate::engine::MODELS[0], MobileModelRole::Asr),
        (crate::engine::MODELS[1], MobileModelRole::Diarization),
        (voice, MobileModelRole::Voice),
    ]
    .into_iter()
    .map(|(id, role)| (id.to_owned(), role))
    .collect()
}

/// The notes model, on a phone that can write notes itself (8 GB; see
/// `tier::notes_capable`): downloaded only when asked for (2.5 GB).
pub fn notes_model() -> Option<(String, MobileModelRole)> {
    crate::tier::detect().notes.then(|| {
        (
            ghi_app::core::preset().llm_id.to_owned(),
            MobileModelRole::Notes,
        )
    })
}

/// Deletes the notes model (and a partial download), calls off the notes jobs
/// still waiting for it and settles their meetings; refused while one runs.
pub fn remove_notes_model(store: &ghi_store::store::Store, models: &Path) -> Result<(), String> {
    use ghi_store::jobs::JobState;
    let err = |e: ghi_store::StoreError| e.to_string();
    let notes: Vec<_> = store
        .active_jobs()
        .map_err(err)?
        .into_iter()
        .filter(|j| j.kind == ghi_core::notes_job::NOTES_FINAL_JOB)
        .collect();
    if notes.iter().any(|j| j.state == JobState::Running) {
        return Err("notes are being written".into());
    }
    let m = ghi_models::find(ghi_app::core::preset().llm_id).ok_or("unknown model")?;
    let path = ghi_models::path_in(models, &m);
    for p in [part_path(&path), path] {
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    for j in notes {
        store.cancel_job(j.id).map_err(err)?;
        let Some(meeting) = j.meeting_gid else {
            continue;
        };
        let busy = store
            .jobs_for_meeting(&meeting)
            .map_err(err)?
            .iter()
            .any(|o| matches!(o.state, JobState::Queued | JobState::Running));
        if !busy {
            store.set_meeting_status(&meeting, "ready").map_err(err)?;
        }
    }
    Ok(())
}

/// What the download thread last said about a model.
#[derive(Clone, Copy)]
struct Live {
    state: MobileModelState,
    received: u64,
}

#[derive(Default)]
struct Inner {
    /// Set while a download thread runs.
    cancel: Option<Arc<AtomicBool>>,
    live: HashMap<String, Live>,
}

/// The download in progress, if any (Tauri state).
#[derive(Default)]
pub struct Downloads(Mutex<Inner>);

impl Downloads {
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn running(&self) -> bool {
        self.inner().cancel.is_some()
    }

    fn set(&self, id: &str, state: MobileModelState, received: u64) {
        self.inner()
            .live
            .insert(id.to_owned(), Live { state, received });
    }

    /// Asks the running download to stop (its `.part` stays).
    pub fn cancel(&self) {
        if let Some(c) = &self.inner().cancel {
            c.store(true, Ordering::Release);
        }
    }
}

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// The installed models and the state of the others.
pub fn status(
    dir: &Path,
    wifi_only: bool,
    downloads: &Downloads,
    damaged: &std::collections::BTreeSet<String>,
) -> MobileModelsStatus {
    let mut missing = 0u64;
    let items: Vec<MobileModelItem> = needed()
        .into_iter()
        .filter_map(|(id, role)| {
            let (item, left) = row(dir, id, role, downloads, damaged)?;
            missing += left;
            Some(item)
        })
        .collect();
    MobileModelsStatus {
        items,
        missing_bytes: missing as f64,
        wifi_only,
    }
}

/// One model's row (installed, downloading, waiting, …) and the bytes it still needs.
pub fn row(
    dir: &Path,
    id: String,
    role: MobileModelRole,
    downloads: &Downloads,
    damaged: &std::collections::BTreeSet<String>,
) -> Option<(MobileModelItem, u64)> {
    let live = downloads.inner().live.get(&id).copied();
    let running = downloads.running();
    let m = ghi_models::find(&id)?;
    let dest = ghi_models::path_in(dir, &m);
    let installed = file_len(&dest) == m.size && !damaged.contains(&id);
    let part = file_len(&part_path(&dest)).min(m.size);
    let mut missing = 0;
    let (state, received) = if installed {
        (MobileModelState::Ready, m.size)
    } else {
        missing = m.size.saturating_sub(part);
        match live {
            // A finished or stale entry says nothing once the file is gone.
            Some(l) if l.state == MobileModelState::Downloading && !running => {
                (MobileModelState::Missing, part)
            }
            Some(l) if l.state != MobileModelState::Ready => (l.state, l.received.max(part)),
            _ => (MobileModelState::Missing, part),
        }
    };
    Some((
        MobileModelItem {
            id,
            role,
            size_bytes: m.size as f64,
            received_bytes: received as f64,
            state,
        },
        missing,
    ))
}

/// The phone is on a metered path: Swift's `NWPath.isExpensive` /
/// `isConstrained` (cellular, Personal Hotspot, Low Data Mode), or else
/// `SCNetworkReachability` `IsWWAN`. Elsewhere (the simulator, tests) there
/// is none.
#[cfg(target_os = "ios")]
pub fn on_cellular() -> bool {
    use std::ffi::c_void;
    if crate::platform::on_expensive_network() {
        return true;
    }
    #[link(name = "SystemConfiguration", kind = "framework")]
    unsafe extern "C" {
        fn SCNetworkReachabilityCreateWithAddress(
            allocator: *const c_void,
            address: *const libc::sockaddr,
        ) -> *const c_void;
        fn SCNetworkReachabilityGetFlags(target: *const c_void, flags: *mut u32) -> u8;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: *const c_void);
    }
    const IS_WWAN: u32 = 1 << 18;
    // SAFETY: a zeroed sockaddr_in with its length and family set is the
    // documented "0.0.0.0" address for a general reachability check; the
    // reference is released before returning.
    unsafe {
        let mut addr: libc::sockaddr_in = std::mem::zeroed();
        addr.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
        addr.sin_family = libc::AF_INET as u8;
        let target = SCNetworkReachabilityCreateWithAddress(
            std::ptr::null(),
            (&addr as *const libc::sockaddr_in).cast(),
        );
        if target.is_null() {
            return false;
        }
        let mut flags = 0u32;
        let ok = SCNetworkReachabilityGetFlags(target, &mut flags) != 0;
        CFRelease(target);
        ok && flags & IS_WWAN != 0
    }
}

#[cfg(not(target_os = "ios"))]
pub fn on_cellular() -> bool {
    false
}

/// How often the download re-checks the network path.
const PATH_CHECK_EVERY: Duration = Duration::from_secs(2);

/// What a download run needs (the app's; tests replace the transport).
pub struct Run<'a> {
    pub dir: &'a Path,
    pub policy: NetPolicy,
    /// Wait for Wi-Fi instead of using cellular.
    pub wifi_only: bool,
    pub cellular: &'a (dyn Fn() -> bool + Sync),
    pub cancel: &'a Arc<AtomicBool>,
    pub transport: &'a dyn Transport,
    pub downloads: &'a Downloads,
}

fn item(
    id: &str,
    role: MobileModelRole,
    m: &Model,
    state: MobileModelState,
    got: u64,
) -> MobileModelItem {
    MobileModelItem {
        id: id.to_owned(),
        role,
        size_bytes: m.size as f64,
        received_bytes: got as f64,
        state,
    }
}

/// Downloads each of `models` (id, role, registry entry) that is missing, one
/// after another, telling `emit` about every change. Stops at the first
/// failure, a cancel, or the network leaving Wi-Fi.
pub fn run(
    run: &Run<'_>,
    models: &[(String, MobileModelRole, Model)],
    emit: &mut dyn FnMut(MobileModelItem),
) {
    let _ = std::fs::create_dir_all(run.dir);
    let damaged = ghi_app::core::damaged_models();
    for (id, role, m) in models {
        let dest = ghi_models::path_in(run.dir, m);
        if file_len(&dest) == m.size && !damaged.contains(id) {
            continue;
        }
        let part = file_len(&part_path(&dest)).min(m.size);
        let mut tell = |state, got: u64| {
            run.downloads.set(id, state, got);
            emit(item(id, *role, m, state, got));
        };
        if run.wifi_only && (run.cellular)() {
            tell(MobileModelState::WaitingForWifi, part);
            return;
        }
        tell(MobileModelState::Downloading, part);
        let paused = AtomicBool::new(false);
        let mut last_check = Instant::now();
        let ctl = Control {
            cancel: Some(run.cancel.clone()),
            ..Control::default()
        };
        let mut progress = |p: Progress<'_>| match p {
            Progress::Bytes { done, .. } => {
                if run.wifi_only && last_check.elapsed() >= PATH_CHECK_EVERY {
                    last_check = Instant::now();
                    if (run.cellular)() {
                        paused.store(true, Ordering::Release);
                        run.cancel.store(true, Ordering::Release);
                    }
                }
                tell(MobileModelState::Downloading, done);
            }
            Progress::Verifying => tell(MobileModelState::Downloading, m.size),
        };
        match ghi_models::download(m, run.dir, run.policy, &ctl, &mut progress, run.transport) {
            Ok(_) => {
                ghi_app::core::clear_damaged(id);
                tell(MobileModelState::Ready, m.size);
            }
            Err(DownloadError::Cancelled) => {
                let kept = file_len(&part_path(&dest));
                if paused.load(Ordering::Acquire) {
                    tell(MobileModelState::WaitingForWifi, kept);
                } else {
                    tell(MobileModelState::Missing, kept);
                }
                return;
            }
            Err(e) => {
                log::warn!("model {id}: {e}");
                tell(MobileModelState::Failed, file_len(&part_path(&dest)));
                return;
            }
        }
    }
}

/// Starts the download thread unless one runs. `wifi_only` is this call's
/// choice (the Wi-Fi-only setting, or the user's "use cellular this time").
pub fn start(
    core: Arc<ghi_app::core::Core>,
    downloads: Arc<Downloads>,
    wifi_only: bool,
    policy: NetPolicy,
    which: Vec<(String, MobileModelRole)>,
) -> Result<(), String> {
    let cancel = {
        let mut inner = downloads.inner();
        if inner.cancel.is_some() {
            return Ok(());
        }
        let flag = Arc::new(AtomicBool::new(false));
        inner.cancel = Some(flag.clone());
        flag
    };
    let failed = downloads.clone();
    std::thread::Builder::new()
        .name("ghi-models-download".into())
        .spawn(move || {
            // Cleared however the run ends, so the UI never shows
            // "downloading" for a thread that is gone.
            struct Clear(Arc<Downloads>);
            impl Drop for Clear {
                fn drop(&mut self) {
                    self.0.inner().cancel = None;
                }
            }
            let _clear = Clear(downloads.clone());
            // A little extra time when the app goes to the background.
            let bg = crate::platform::begin_bg_task("models");
            let models: Vec<_> = which
                .into_iter()
                .filter_map(|(id, role)| Some((id.clone(), role, ghi_models::find(&id)?)))
                .collect();
            run(
                &Run {
                    dir: &core.models(),
                    policy,
                    wifi_only,
                    cellular: &on_cellular,
                    cancel: &cancel,
                    transport: &UreqTransport,
                    downloads: &downloads,
                },
                &models,
                &mut |item| emit(MobileEvent::ModelDownload { item }),
            );
            if bg != 0 {
                crate::platform::end_bg_task(bg);
            }
            // Queued final passes can run now.
            core.notify_jobs();
        })
        .map_err(|e| {
            failed.inner().cancel = None;
            e.to_string()
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_net::NetError;
    use ghi_net::fetch::Response;
    use std::io::Cursor;

    const DATA: &[u8] = b"model bytes, twenty+ of them";

    fn model(id: &str) -> Model {
        Model {
            id: id.into(),
            role: "asr".into(),
            repo: "org/repo".into(),
            revision: "a".repeat(40),
            file: format!("{id}.gguf"),
            sha256: ghi_net::sha256_hex(DATA),
            size: DATA.len() as u64,
            license: "MIT".into(),
            chat_format: None,
            mirrors: vec![],
            optional: false,
        }
    }

    struct Serves {
        body: Vec<u8>,
        calls: Mutex<Vec<(String, Option<u64>)>>,
    }

    impl Transport for Serves {
        fn get(&self, url: &str, range_from: Option<u64>) -> Result<Response, NetError> {
            self.calls
                .lock()
                .unwrap()
                .push((url.to_owned(), range_from));
            let from = range_from.unwrap_or(0) as usize;
            let body = self.body[from..].to_vec();
            Ok(Response {
                status: if range_from.is_some() { 206 } else { 200 },
                location: None,
                content_length: Some(body.len() as u64),
                content_range: range_from
                    .map(|f| format!("bytes {f}-{}/{}", self.body.len() - 1, self.body.len())),
                body: Box::new(Cursor::new(body)),
            })
        }
    }

    fn serves(body: &[u8]) -> Serves {
        Serves {
            body: body.to_vec(),
            calls: Mutex::new(vec![]),
        }
    }

    fn setup() -> (tempfile::TempDir, Vec<(String, MobileModelRole, Model)>) {
        let dir = tempfile::tempdir().unwrap();
        let models = vec![
            ("m-asr".to_owned(), MobileModelRole::Asr, model("m-asr")),
            (
                "m-voice".to_owned(),
                MobileModelRole::Voice,
                model("m-voice"),
            ),
        ];
        (dir, models)
    }

    fn go(
        dir: &Path,
        models: &[(String, MobileModelRole, Model)],
        wifi_only: bool,
        cellular: bool,
        transport: &dyn Transport,
        downloads: &Downloads,
    ) -> Vec<(String, MobileModelState)> {
        let cancel = Arc::new(AtomicBool::new(false));
        let is_cellular = move || cellular;
        let mut events = vec![];
        run(
            &Run {
                dir,
                policy: NetPolicy::Default,
                wifi_only,
                cellular: &is_cellular,
                cancel: &cancel,
                transport,
                downloads,
            },
            models,
            &mut |i| events.push((i.id, i.state)),
        );
        events
    }

    #[test]
    fn downloads_in_order_and_reports_ready() {
        let (dir, models) = setup();
        let t = serves(DATA);
        let d = Downloads::default();
        let ev = go(dir.path(), &models, true, false, &t, &d);
        assert_eq!(
            ev.iter()
                .filter(|(_, s)| *s == MobileModelState::Ready)
                .count(),
            2
        );
        assert_eq!(ev[0], ("m-asr".to_owned(), MobileModelState::Downloading));
        for (_, _, m) in &models {
            assert_eq!(
                std::fs::read(ghi_models::path_in(dir.path(), m)).unwrap(),
                DATA
            );
        }
        // Everything is there: a second run does not touch the network.
        let t2 = serves(b"unused");
        go(dir.path(), &models, true, false, &t2, &d);
        assert!(t2.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn wifi_only_waits_on_cellular_and_cellular_this_time_downloads() {
        let (dir, models) = setup();
        let t = serves(DATA);
        let d = Downloads::default();
        let ev = go(dir.path(), &models, true, true, &t, &d);
        assert_eq!(ev, [("m-asr".to_owned(), MobileModelState::WaitingForWifi)]);
        assert!(t.calls.lock().unwrap().is_empty(), "nothing was fetched");
        let st = status(dir.path(), true, &d, &Default::default());
        assert!(st.wifi_only);
        let ev = go(dir.path(), &models, false, true, &t, &d);
        assert!(ev.iter().any(|(_, s)| *s == MobileModelState::Ready));
        assert!(!t.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn a_part_file_resumes() {
        let (dir, models) = setup();
        let m = &models[0].2;
        let dest = ghi_models::path_in(dir.path(), m);
        std::fs::write(part_path(&dest), &DATA[..10]).unwrap();
        let t = serves(DATA);
        go(
            dir.path(),
            &models[..1],
            true,
            false,
            &t,
            &Downloads::default(),
        );
        let calls = t.calls.lock().unwrap();
        assert_eq!(calls[0].1, Some(10), "asked for the rest only");
        assert_eq!(std::fs::read(&dest).unwrap(), DATA);
    }

    #[test]
    fn a_wrong_hash_fails_and_leaves_no_model() {
        let (dir, models) = setup();
        let t = serves(b"model bytes, twenty+ of THEM");
        let d = Downloads::default();
        let ev = go(dir.path(), &models, true, false, &t, &d);
        assert_eq!(
            ev.last().unwrap(),
            &("m-asr".to_owned(), MobileModelState::Failed)
        );
        assert!(!ghi_models::path_in(dir.path(), &models[0].2).exists());
        assert!(
            ev.iter().all(|(id, _)| id == "m-asr"),
            "stops at the first failure"
        );
    }

    #[test]
    fn strict_offline_downloads_nothing() {
        let (dir, models) = setup();
        let t = serves(DATA);
        let d = Downloads::default();
        let cancel = Arc::new(AtomicBool::new(false));
        let mut ev = vec![];
        run(
            &Run {
                dir: dir.path(),
                policy: NetPolicy::StrictOffline,
                wifi_only: false,
                cellular: &|| false,
                cancel: &cancel,
                transport: &t,
                downloads: &d,
            },
            &models,
            &mut |i| ev.push(i.state),
        );
        assert_eq!(ev.last(), Some(&MobileModelState::Failed));
        assert!(t.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn removing_the_notes_model_calls_off_waiting_notes() {
        use ghi_store::jobs::JobState;
        let tmp = tempfile::tempdir().unwrap();
        let store = ghi_store::store::Store::open(
            &tmp.path().join("store"),
            std::sync::Arc::new(ghi_store::keys::MemoryKeyStore::default()),
            ghi_store::keys::Protection::default(),
        )
        .unwrap();
        let models = tmp.path().join("models");
        std::fs::create_dir_all(&models).unwrap();
        let m = ghi_models::find("qwen3-4b").unwrap();
        let file = ghi_models::path_in(&models, &m);
        std::fs::write(&file, b"model").unwrap();
        let meeting = store
            .create_meeting(ghi_store::store::NewMeeting::default())
            .unwrap()
            .gid;
        let job = store
            .enqueue_job(
                Some(&meeting),
                ghi_core::notes_job::NOTES_FINAL_JOB,
                ghi_core::session::JOB_PAYLOAD_VERSION,
                &serde_json::json!({}),
            )
            .unwrap();
        store.set_meeting_status(&meeting, "processing").unwrap();
        remove_notes_model(&store, &models).unwrap();
        assert!(!file.exists());
        assert_eq!(store.job(job).unwrap().state, JobState::Cancelled);
        assert_eq!(store.get_meeting(&meeting).unwrap().status, "ready");
        // Gone already: removing again is fine.
        remove_notes_model(&store, &models).unwrap();
    }

    #[test]
    fn status_lists_the_three_phone_models() {
        let dir = tempfile::tempdir().unwrap();
        let st = status(dir.path(), true, &Downloads::default(), &Default::default());
        let roles: Vec<_> = st.items.iter().map(|i| i.role).collect();
        assert_eq!(
            roles,
            [
                MobileModelRole::Asr,
                MobileModelRole::Diarization,
                MobileModelRole::Voice
            ]
        );
        assert!(
            st.items
                .iter()
                .all(|i| i.state == MobileModelState::Missing)
        );
        assert!(st.missing_bytes > 0.0);
    }
}
