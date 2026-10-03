// SPDX-License-Identifier: Apache-2.0
//! What the share extension and the share sheet need from the app: the App
//! Group container (where the extension drops files, `inbox/<uuid>/`) and
//! the system share sheet for a file we made (an export).

use std::path::{Path, PathBuf};

/// The App Group shared with the share extension.
#[cfg(target_os = "ios")]
pub const APP_GROUP: &str = "group.com.nhtera.ghira";

/// The App Group container, or `None` outside iOS and when the entitlement
/// is missing (an ad-hoc simulator build without it).
#[cfg(target_os = "ios")]
pub fn app_group_dir() -> Option<PathBuf> {
    use objc2_foundation::{NSFileManager, NSString};
    let manager = NSFileManager::defaultManager();
    let url = manager
        .containerURLForSecurityApplicationGroupIdentifier(&NSString::from_str(APP_GROUP))?;
    Some(PathBuf::from(url.path()?.to_string()))
}

#[cfg(not(target_os = "ios"))]
pub fn app_group_dir() -> Option<PathBuf> {
    None
}

/// Where the extension's `inbox/<uuid>/` folders are: the App Group container
/// on iOS; `<data>/inbox` where there is none (host tests, the dev build).
pub fn inbox_root(data: &Path) -> PathBuf {
    app_group_dir()
        .unwrap_or_else(|| data.to_path_buf())
        .join("inbox")
}

/// Presents the system share sheet for `path`. Swift deletes the file when
/// the sheet closes (its completion handler); [`crate::privacy_cmd`] also
/// sweeps leftovers.
pub fn share_file(path: &Path) -> Result<(), String> {
    crate::platform::share_file(path)
}

/// Renders one meeting (notes and transcript) as Markdown or text into the
/// temporary share folder and presents it. Returns once the sheet is shown;
/// Swift deletes the file when it closes, Rust after an hour or at launch.
/// A failed share leaves nothing behind.
pub fn share_meeting(
    store: &ghi_store::store::Store,
    data: &Path,
    meeting: &str,
    format: ghi_core::export::Format,
    vietnamese: bool,
    present: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    use ghi_core::export::{self, ExportOptions, Lang};
    let opts = ExportOptions {
        include_notes: true,
        include_transcript: true,
        ui_lang: if vietnamese { Lang::Vi } else { Lang::En },
    };
    let bytes = export::render(store, meeting, format, &opts)?;
    let m = store.get_meeting(meeting).map_err(|e| e.to_string())?;
    let dir = crate::privacy_cmd::share_dir(data);
    crate::privacy_cmd::sweep_exports(data, std::time::Duration::from_secs(3600));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = export::write_new(
        &dir,
        &export::file_stem(&m),
        export::extension(format),
        &bytes,
    )?;
    let path = dir.join(name);
    if let Err(e) = present(&path) {
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    crate::privacy_cmd::forget_later(path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shared_meeting_is_written_for_the_sheet_and_removed_when_it_fails() {
        use ghi_store::keys::MemoryKeyStore;
        use ghi_store::store::NewMeeting;
        let t = tempfile::tempdir().unwrap();
        let store = ghi_store::store::Store::open(
            &t.path().join("store"),
            std::sync::Arc::new(MemoryKeyStore::default()),
            Default::default(),
        )
        .unwrap();
        let gid = store
            .create_meeting(NewMeeting {
                title: "Họp tuần".into(),
                ..Default::default()
            })
            .unwrap()
            .gid;
        let seen = std::sync::Mutex::new(None);
        share_meeting(
            &store,
            t.path(),
            &gid,
            ghi_core::export::Format::Markdown,
            true,
            &|p| {
                let text = std::fs::read_to_string(p).unwrap();
                *seen.lock().unwrap() = Some((p.to_path_buf(), text));
                Ok(())
            },
        )
        .unwrap();
        let (path, text) = seen.lock().unwrap().clone().unwrap();
        assert!(text.contains("Họp tuần"));
        assert_eq!(path.extension().unwrap(), "md");
        assert!(path.starts_with(crate::privacy_cmd::share_dir(t.path())));
        // Still there for the sheet; the launch sweep removes it.
        assert!(path.exists());
        crate::privacy_cmd::sweep_exports(t.path(), std::time::Duration::ZERO);
        assert!(!path.exists());
        // A sheet that cannot open leaves no file; text format uses .txt.
        let failed = std::sync::Mutex::new(None);
        let r = share_meeting(
            &store,
            t.path(),
            &gid,
            ghi_core::export::Format::Text,
            false,
            &|p| {
                *failed.lock().unwrap() = Some(p.to_path_buf());
                Err("no sheet".into())
            },
        );
        assert_eq!(r, Err("no sheet".into()));
        let p = failed.lock().unwrap().clone().unwrap();
        assert_eq!(p.extension().unwrap(), "txt");
        assert!(!p.exists());
        // An unknown meeting writes nothing.
        assert!(
            share_meeting(
                &store,
                t.path(),
                "nope",
                ghi_core::export::Format::Text,
                false,
                &|_| Ok(())
            )
            .is_err()
        );
    }

    #[test]
    fn the_inbox_is_under_the_data_dir_without_an_app_group() {
        let data = Path::new("/data/Ghira");
        assert_eq!(inbox_root(data), Path::new("/data/Ghira/inbox"));
    }
}
