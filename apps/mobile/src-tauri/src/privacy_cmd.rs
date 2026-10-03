// SPDX-License-Identifier: Apache-2.0
//! Privacy actions (Settings → Privacy): export everything, delete everything
//! and the audio-retention sweep.
//!
//! - **Export** makes a password-sealed archive (`Store::export_all`: Argon2id
//!   and XChaCha20-Poly1305, the key ring inside) in `<data>/export-tmp/`, hands
//!   it to the share sheet through Swift and forgets it: Swift deletes it when
//!   the sheet closes; Rust also deletes it after an hour and at every launch.
//!   The path never reaches the webview.
//! - **Delete everything** crypto-shreds the store (`Core::delete_everything`:
//!   every meeting key, the database, audio bundles and a fresh key ring in the
//!   Keychain), removes the cloud keys, and clears everything else the app
//!   keeps: the inbox, the sealed backlog, metrics and leftover exports. Models
//!   stay (they are not user data). The typed confirmation is checked again here.
//! - **Retention** deletes audio older than `AppSettings.audio_retention_days`
//!   and keeps the text; it runs at launch and then daily.

use std::path::{Path, PathBuf};
use std::time::Duration;

use ghi_app::core::Core;
use ghi_store::retention::RetentionReport;
use ghi_store::store::Store;
use rand_core::RngCore;

/// Held (shared) while a recording starts or an import runs, and (exclusive)
/// by [`delete_all`] from its busy check to its last deletion, so neither can
/// begin in the middle of a wipe or leave a half-written meeting behind.
pub static DATA_GUARD: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// Temporary exports live here, under the (backup-excluded) data directory.
pub const EXPORT_DIR: &str = "export-tmp";
/// Shortest export password.
pub const MIN_PASSWORD_CHARS: usize = 8;
/// A leftover export is removed after this long.
const EXPORT_TTL: Duration = Duration::from_secs(3600);
/// Retention runs this often.
pub const RETENTION_EVERY: Duration = Duration::from_secs(24 * 3600);

/// The words the user types to confirm, in the app's languages. The UI shows
/// these (`mobile.privacy.deleteAll.phrase`); case and accents are ignored.
pub const DELETE_PHRASES: [&str; 2] = ["DELETE", "XÓA"];

/// The typed confirmation matches a phrase.
pub fn phrase_ok(typed: &str) -> bool {
    let typed = ghi_store::fold::fold(typed.trim());
    DELETE_PHRASES
        .iter()
        .any(|p| ghi_store::fold::fold(p) == typed)
}

fn export_dir(data: &Path) -> PathBuf {
    data.join(EXPORT_DIR)
}

/// Removes exports older than `max_age` (zero: all of them).
pub fn sweep_exports(data: &Path, max_age: Duration) {
    let Ok(rd) = std::fs::read_dir(export_dir(data)) else {
        return;
    };
    for e in rd.filter_map(Result::ok) {
        let old = max_age.is_zero()
            || e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_none_or(|age| age >= max_age);
        if old {
            let p = e.path();
            let _ = if e.file_type().is_ok_and(|t| t.is_dir()) {
                std::fs::remove_dir_all(p)
            } else {
                std::fs::remove_file(p)
            };
        }
    }
}

/// Writes the sealed archive and returns its path. `password` is checked for length only.
pub fn export_archive(store: &Store, data: &Path, password: &str) -> Result<PathBuf, String> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err("passwordTooShort".into());
    }
    let dir = export_dir(data);
    sweep_exports(data, EXPORT_TTL);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // Two exports in the same second must not share a file.
    let tag = rand_core::OsRng.next_u32();
    let out = dir.join(format!("Ghira-export-{at}-{tag:08x}.ghx"));
    store
        .export_all(&out, password)
        .map_err(|e| format!("export: {e}"))?;
    Ok(out)
}

/// Export, share and forget. `share` presents the sheet (Swift).
pub fn export_and_share(
    store: &Store,
    data: &Path,
    password: &str,
    share: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let out = export_archive(store, data, password)?;
    if let Err(e) = share(&out) {
        let _ = std::fs::remove_file(&out);
        return Err(e);
    }
    // Swift deletes it when the sheet closes; this is the backstop.
    let _ = std::thread::Builder::new()
        .name("ghi-export-sweep".into())
        .spawn(move || {
            std::thread::sleep(EXPORT_TTL);
            let _ = std::fs::remove_file(out);
        });
    Ok(())
}

/// Deletes everything (see the module docs). `busy` says a recording or an
/// import is running (asked under [`DATA_GUARD`]): refused then. A start or an
/// import that is just beginning also counts as busy.
pub fn delete_all(
    core: &Core,
    data: &Path,
    inbox_root: &Path,
    confirm: &str,
    busy: &dyn Fn() -> bool,
) -> Result<(), String> {
    if !phrase_ok(confirm) {
        return Err("confirmMismatch".into());
    }
    let Ok(_wipe) = DATA_GUARD.try_write() else {
        return Err("busy".into());
    };
    if busy() {
        return Err("busy".into());
    }
    core.delete_everything()?;
    for dir in ["backlog", "metrics", "logs", "import-tmp", EXPORT_DIR] {
        let _ = std::fs::remove_dir_all(data.join(dir));
    }
    // The recorder keeps writing there: the next recording needs them.
    for dir in ["backlog", "metrics"] {
        std::fs::create_dir_all(data.join(dir)).map_err(|e| e.to_string())?;
    }
    // What the share extension left behind is the user's content too.
    if let Ok(rd) = std::fs::read_dir(inbox_root) {
        for e in rd.filter_map(Result::ok) {
            let p = e.path();
            let _ = if e.file_type().is_ok_and(|t| t.is_dir()) {
                std::fs::remove_dir_all(p)
            } else {
                std::fs::remove_file(p)
            };
        }
    }
    Ok(())
}

/// Applies the retention setting and deletes the audio past it (text stays).
/// Reads the stored setting directly so it also runs while the app is locked.
pub fn retention(store: &Store) -> Result<RetentionReport, String> {
    let stored = store
        .get_setting(ghi_app::system::SETTINGS_KEY)
        .map_err(|e| e.to_string())?;
    let days = ghi_app::system::from_stored(stored).audio_retention_days;
    store
        .apply_retention_days((days > 0).then_some(days))
        .map_err(|e| e.to_string())?;
    store.retention_sweep_now().map_err(|e| e.to_string())
}

/// Retention at launch, then daily; exports left by a crash go at launch.
pub fn spawn_maintenance(core: std::sync::Arc<Core>) {
    let _ = std::thread::Builder::new()
        .name("ghi-retention".into())
        .spawn(move || {
            sweep_exports(core.data_dir(), Duration::ZERO);
            loop {
                match core.store_even_locked().and_then(|s| retention(&s)) {
                    Ok(r) if r.meetings > 0 => {
                        log::info!("retention removed audio of {} meeting(s)", r.meetings);
                    }
                    Ok(_) => {}
                    Err(e) => log::warn!("retention: {e}"),
                }
                std::thread::sleep(RETENTION_EVERY);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_app::core::CoreHooks;
    use ghi_store::keys::secrets::{FileSecrets, SecretStore};
    use ghi_store::keys::{KeyStore, MemoryKeyStore};
    use ghi_store::store::{NewMeeting, TrackKind};
    use std::sync::Arc;

    fn open(dir: &Path, ks: Arc<MemoryKeyStore>) -> Store {
        Store::open(dir, ks, Default::default()).unwrap()
    }

    fn record(store: &Store, gid: &str) {
        let mut w = store.open_track(gid, TrackKind::Mic).unwrap();
        w.append(b"pcm").unwrap();
        store.finish_track(gid, TrackKind::Mic, w).unwrap();
    }

    #[test]
    fn the_typed_phrase_ignores_case_accents_and_spaces() {
        for ok in ["DELETE", " delete ", "Delete", "XÓA", "xoa", "xóa"] {
            assert!(phrase_ok(ok), "{ok}");
        }
        for bad in ["", "delet", "delete everything", "no", "xóa hết"] {
            assert!(!phrase_ok(bad), "{bad}");
        }
    }

    #[test]
    fn an_export_opens_with_the_store_import_and_is_forgotten() {
        let t = tempfile::tempdir().unwrap();
        let ks = Arc::new(MemoryKeyStore::default());
        let store = open(&t.path().join("store"), ks);
        let gid = store
            .create_meeting(NewMeeting {
                title: "Cuộc họp".into(),
                ..Default::default()
            })
            .unwrap()
            .gid;
        record(&store, &gid);
        // Too short a password is refused before anything is written.
        assert_eq!(
            export_archive(&store, t.path(), "short"),
            Err("passwordTooShort".into())
        );
        assert!(!export_dir(t.path()).exists());

        let shared = std::sync::Mutex::new(None);
        export_and_share(&store, t.path(), "correct horse", &|p| {
            *shared.lock().unwrap() = Some(p.to_path_buf());
            assert!(p.starts_with(export_dir(t.path())));
            Ok(())
        })
        .unwrap();
        let archive = shared.lock().unwrap().clone().unwrap();
        assert!(archive.is_file());

        // It restores on a "new phone": a fresh key store and directory.
        let ks2 = Arc::new(MemoryKeyStore::default());
        let restored = Store::import_archive(
            &archive,
            "correct horse",
            &t.path().join("restored"),
            ks2.clone(),
            Default::default(),
        )
        .unwrap();
        let rows = restored.list_meetings(10, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].gid, gid);
        assert!(restored.audio_available(&gid).unwrap());
        assert!(
            Store::import_archive(
                &archive,
                "wrong password",
                &t.path().join("other"),
                Arc::new(MemoryKeyStore::default()),
                Default::default()
            )
            .is_err()
        );

        // A share sheet that could not open leaves nothing behind.
        let r = export_and_share(&store, t.path(), "correct horse", &|_| {
            Err("no sheet".into())
        });
        assert_eq!(r, Err("no sheet".into()));
        sweep_exports(t.path(), Duration::ZERO);
        assert_eq!(std::fs::read_dir(export_dir(t.path())).unwrap().count(), 0);
    }

    #[test]
    fn retention_follows_the_stored_setting() {
        let t = tempfile::tempdir().unwrap();
        let store = open(&t.path().join("store"), Arc::default());
        let gid = store
            .create_meeting(NewMeeting {
                title: "mới".into(),
                ..Default::default()
            })
            .unwrap()
            .gid;
        record(&store, &gid);
        let until = || store.get_meeting(&gid).unwrap().audio_retained_until;
        // No setting: audio is kept for ever.
        assert_eq!(retention(&store).unwrap().meetings, 0);
        assert_eq!(until(), None);
        // One day: a deadline is set; it has not passed, so the audio stays.
        store
            .set_setting(
                ghi_app::system::SETTINGS_KEY,
                &serde_json::json!({"audioRetentionDays": 1}),
            )
            .unwrap();
        assert_eq!(retention(&store).unwrap().meetings, 0);
        assert!(until().unwrap() > ghi_store::store::now_ms());
        assert!(store.audio_available(&gid).unwrap());
        // Back to "keep for ever" clears it.
        store
            .set_setting(
                ghi_app::system::SETTINGS_KEY,
                &serde_json::json!({"audioRetentionDays": 0}),
            )
            .unwrap();
        retention(&store).unwrap();
        assert_eq!(until(), None);
    }

    #[test]
    fn a_past_deadline_deletes_the_audio_and_keeps_the_text() {
        let t = tempfile::tempdir().unwrap();
        let store = open(&t.path().join("store"), Arc::default());
        let gid = store
            .create_meeting(NewMeeting {
                title: "hết hạn".into(),
                audio_retained_until: Some(1_000),
                ..Default::default()
            })
            .unwrap()
            .gid;
        record(&store, &gid);
        // `retention_sweep_now` is what `retention` ends with.
        assert_eq!(store.retention_sweep_now().unwrap().meetings, 1);
        assert!(!store.audio_available(&gid).unwrap());
        assert_eq!(store.get_meeting(&gid).unwrap().title, "hết hạn");
    }

    #[test]
    fn delete_all_wipes_the_store_the_keys_and_what_the_app_keeps() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("Ghira");
        let ks = Arc::new(MemoryKeyStore::default());
        let secrets_dir = t.path().join("secrets");
        let hooks = CoreHooks {
            data_dir: Some(data.clone()),
            handlers: Some(Arc::new(|_, _| vec![])),
            recover_kinds: Some(vec![]),
            keystore: Some(ks.clone()),
            secrets: Some({
                let dir = secrets_dir.clone();
                Arc::new(
                    move || Ok(Box::new(FileSecrets::new(dir.clone())) as Box<dyn SecretStore>),
                )
            }),
            ..CoreHooks::default()
        };
        let (core, _rx) = Core::for_test_with(data.clone(), hooks);
        let store = core.store_even_locked().unwrap();
        let gid = store
            .create_meeting(NewMeeting {
                title: "bí mật".into(),
                ..Default::default()
            })
            .unwrap()
            .gid;
        record(&store, &gid);
        drop(store);
        let old_ring = ks.load().unwrap().unwrap().to_bytes().to_vec();
        FileSecrets::new(secrets_dir.clone())
            .set("provider-openai", b"sk-test")
            .unwrap();
        // What the app keeps beside the store.
        for d in ["backlog", "metrics", EXPORT_DIR] {
            std::fs::create_dir_all(data.join(d)).unwrap();
            std::fs::write(data.join(d).join("x"), b"x").unwrap();
        }
        let inbox = t.path().join("group").join("inbox");
        std::fs::create_dir_all(inbox.join("0F8FAD5B-D9CB-469F-A165-70867728950E")).unwrap();
        std::fs::create_dir_all(data.join("models")).unwrap();
        std::fs::write(data.join("models").join("m.gguf"), b"model").unwrap();

        // The typed phrase and a running recording or import are checked first.
        assert_eq!(
            delete_all(&core, &data, &inbox, "nope", &|| false),
            Err("confirmMismatch".into())
        );
        assert_eq!(
            delete_all(&core, &data, &inbox, "DELETE", &|| true),
            Err("busy".into())
        );
        assert!(std::fs::read_dir(data.join("store")).unwrap().count() > 0);

        delete_all(&core, &data, &inbox, "delete", &|| false).unwrap();
        // The database and the audio are gone (the process lock file stays).
        let left: Vec<_> = std::fs::read_dir(data.join("store"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, [".lock"]);
        // Backlog and metrics are empty but there: a recording after the wipe
        // writes into them.
        for d in ["backlog", "metrics"] {
            assert_eq!(std::fs::read_dir(data.join(d)).unwrap().count(), 0, "{d}");
            std::fs::write(data.join(d).join("next"), b"x").unwrap();
        }
        // A start or an import in flight holds the guard: the wipe refuses.
        let reading = DATA_GUARD.read().unwrap();
        assert_eq!(
            delete_all(&core, &data, &inbox, "delete", &|| false),
            Err("busy".into())
        );
        drop(reading);
        assert!(!data.join(EXPORT_DIR).exists());
        assert_eq!(std::fs::read_dir(&inbox).unwrap().count(), 0);
        assert!(data.join("models").join("m.gguf").exists(), "models stay");
        // The cloud key is gone and the master key is a new one.
        assert!(
            FileSecrets::new(secrets_dir)
                .get("provider-openai")
                .unwrap()
                .is_none()
        );
        let new_ring = ks.load().unwrap().unwrap().to_bytes().to_vec();
        assert_ne!(old_ring, new_ring);
        // The app opens a fresh, empty store afterwards.
        let fresh = core.store_even_locked().unwrap();
        assert!(fresh.list_meetings(10, 0).unwrap().is_empty());
    }
}
