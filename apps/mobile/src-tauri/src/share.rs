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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inbox_is_under_the_data_dir_without_an_app_group() {
        let data = Path::new("/data/Ghira");
        assert_eq!(inbox_root(data), Path::new("/data/Ghira/inbox"));
    }
}
