// SPDX-License-Identifier: Apache-2.0
//! Installing a downloaded update (macOS): unpack next to the installed app,
//! check the new bundle, swap by rename, relaunch.
//!
//! - The archive is a `ditto -c -k --keepParent` zip of the signed, stapled
//!   `.app` (signatures and extended attributes survive `ditto -x -k`).
//! - Unpacked into a hidden folder in the installed app's own folder: the
//!   swap is then two renames on one volume.
//! - The new bundle must pass [`Verify`] (the real one: `codesign --verify
//!   --deep --strict` and the same Team ID as the running app) before the
//!   installed app is touched.
//! - A translocated app (run from the DMG or Downloads before being moved)
//!   can't be updated in place: the user moves it to Applications first.
//! - The old app stays as `.<name>.old` until the new one has started
//!   ([`cleanup`]); if the second rename fails it is put back.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub enum InstallError {
    /// Run from a translocated copy: move the app to Applications first.
    Translocated,
    /// Not inside an `.app` bundle (e.g. a dev build).
    NotABundle,
    Unpack(String),
    /// The new bundle failed its signature or Team ID check.
    Rejected(String),
    /// No permission to replace the app (owned by another user).
    Permission,
    Io(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Translocated => {
                f.write_str("move the app to the Applications folder, then update")
            }
            InstallError::NotABundle => f.write_str("this build can't update itself"),
            InstallError::Unpack(e) => write!(f, "the update can't be unpacked: {e}"),
            InstallError::Rejected(e) => write!(f, "the update isn't signed by us: {e}"),
            InstallError::Permission => {
                f.write_str("no permission to replace the app; download the new version instead")
            }
            InstallError::Io(e) => write!(f, "the update can't be installed: {e}"),
        }
    }
}

impl std::error::Error for InstallError {}

fn io(e: std::io::Error) -> InstallError {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        InstallError::Permission
    } else {
        InstallError::Io(e.to_string())
    }
}

/// Checks an unpacked bundle before it replaces the installed one.
pub trait Verify {
    fn verify(&self, new_app: &Path, installed_app: &Path) -> Result<(), InstallError>;
}

/// `codesign`: the new bundle is validly signed, and by the same team as the
/// installed one.
pub struct Codesign;

fn team_id(app: &Path) -> Result<String, InstallError> {
    let out = Command::new("/usr/bin/codesign")
        .args(["-dv", "--verbose=2"])
        .arg(app)
        .output()
        .map_err(|e| InstallError::Rejected(e.to_string()))?;
    // codesign prints the details on stderr.
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .find_map(|l| l.strip_prefix("TeamIdentifier="))
        .map(str::to_string)
        .filter(|t| t != "not set")
        .ok_or_else(|| InstallError::Rejected("no Team ID".into()))
}

impl Verify for Codesign {
    fn verify(&self, new_app: &Path, installed_app: &Path) -> Result<(), InstallError> {
        let ok = Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(new_app)
            .status()
            .map_err(|e| InstallError::Rejected(e.to_string()))?
            .success();
        if !ok {
            return Err(InstallError::Rejected("invalid signature".into()));
        }
        let (new, ours) = (team_id(new_app)?, team_id(installed_app)?);
        if new != ours {
            return Err(InstallError::Rejected(format!(
                "Team ID {new}, expected {ours}"
            )));
        }
        Ok(())
    }
}

/// The `.app` bundle containing `exe` (`…/Ghira.app/Contents/MacOS/ghi-desktop`).
pub fn bundle_of(exe: &Path) -> Result<PathBuf, InstallError> {
    if exe.to_string_lossy().contains("/AppTranslocation/") {
        return Err(InstallError::Translocated);
    }
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
        .ok_or(InstallError::NotABundle)
}

fn hidden(installed: &Path, tag: &str) -> Result<PathBuf, InstallError> {
    let name = installed
        .file_name()
        .ok_or(InstallError::NotABundle)?
        .to_string_lossy();
    let dir = installed.parent().ok_or(InstallError::NotABundle)?;
    Ok(dir.join(format!(".{name}.{tag}")))
}

/// Unpacks `archive` next to `installed`, checks it, and swaps it in.
/// Returns the path of the old bundle (removed by [`cleanup`] after the new
/// one has started).
pub fn install(
    archive: &Path,
    installed: &Path,
    verify: &dyn Verify,
) -> Result<PathBuf, InstallError> {
    let new_app = prepare(archive, installed, verify)?;
    swap(&new_app, installed)
}

/// Unpacks `archive` next to `installed` and checks it; nothing installed
/// changes. Returns the checked bundle for [`swap`].
pub fn prepare(
    archive: &Path,
    installed: &Path,
    verify: &dyn Verify,
) -> Result<PathBuf, InstallError> {
    let staging = hidden(installed, "update")?;
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(io)?;
    let unpacked = (|| {
        let ok = Command::new("/usr/bin/ditto")
            .args(["-x", "-k"])
            .arg(archive)
            .arg(&staging)
            .status()
            .map_err(|e| InstallError::Unpack(e.to_string()))?
            .success();
        if !ok {
            return Err(InstallError::Unpack("ditto failed".into()));
        }
        let name = installed.file_name().ok_or(InstallError::NotABundle)?;
        let new_app = staging.join(name);
        if !new_app.join("Contents").is_dir() {
            return Err(InstallError::Unpack(format!(
                "no {} in the archive",
                name.to_string_lossy()
            )));
        }
        verify.verify(&new_app, installed)?;
        Ok(new_app)
    })();
    if unpacked.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    unpacked
}

/// Swaps a [`prepare`]d bundle in by two renames; on failure the installed
/// app is put back.
pub fn swap(new_app: &Path, installed: &Path) -> Result<PathBuf, InstallError> {
    let staging = hidden(installed, "update")?;
    let old = hidden(installed, "old")?;
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(installed, &old).map_err(io)?;
    if let Err(e) = std::fs::rename(new_app, installed) {
        // Put the running version back.
        let _ = std::fs::rename(&old, installed);
        let _ = std::fs::remove_dir_all(&staging);
        return Err(io(e));
    }
    let _ = std::fs::remove_dir_all(&staging);
    Ok(old)
}

/// After a successful start: removes what an update left behind.
pub fn cleanup(installed: &Path) {
    for tag in ["old", "update"] {
        if let Ok(p) = hidden(installed, tag) {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}

/// Starts `app` again once this process has exited (the caller exits next).
pub fn relaunch(app: &Path) -> Result<(), InstallError> {
    Command::new("/bin/sh")
        .args(["-c", "sleep 1; exec /usr/bin/open -n \"$0\""])
        .arg(app)
        .spawn()
        .map(|_| ())
        .map_err(|e| InstallError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Accept;
    impl Verify for Accept {
        fn verify(&self, _: &Path, _: &Path) -> Result<(), InstallError> {
            Ok(())
        }
    }

    struct Refuse;
    impl Verify for Refuse {
        fn verify(&self, _: &Path, _: &Path) -> Result<(), InstallError> {
            Err(InstallError::Rejected("test".into()))
        }
    }

    /// An `.app` with one file saying `v`, zipped like the release does.
    fn app(dir: &Path, v: &str) -> (PathBuf, PathBuf) {
        let src = dir.join(format!("src-{v}"));
        let a = src.join("Ghira.app/Contents/MacOS");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join("version"), v).unwrap();
        let zip = dir.join(format!("{v}.zip"));
        assert!(
            Command::new("/usr/bin/ditto")
                .args(["-c", "-k", "--keepParent"])
                .arg(src.join("Ghira.app"))
                .arg(&zip)
                .status()
                .unwrap()
                .success()
        );
        (src.join("Ghira.app"), zip)
    }

    fn version(app: &Path) -> String {
        std::fs::read_to_string(app.join("Contents/MacOS/version")).unwrap()
    }

    #[test]
    fn swaps_in_a_checked_bundle_and_keeps_the_old_one_until_cleanup() {
        let tmp = tempfile::tempdir().unwrap();
        let (installed, _) = app(tmp.path(), "1");
        let (_, zip) = app(tmp.path(), "2");
        let old = install(&zip, &installed, &Accept).unwrap();
        assert_eq!(version(&installed), "2");
        assert_eq!(version(&old), "1");
        cleanup(&installed);
        assert!(!old.exists());
    }

    #[test]
    fn a_rejected_bundle_leaves_the_installed_app_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let (installed, _) = app(tmp.path(), "1");
        let (_, zip) = app(tmp.path(), "2");
        assert!(matches!(
            install(&zip, &installed, &Refuse),
            Err(InstallError::Rejected(_))
        ));
        assert_eq!(version(&installed), "1");
        assert!(!hidden(&installed, "update").unwrap().exists());
        // Not a zip at all.
        let junk = tmp.path().join("junk.zip");
        std::fs::write(&junk, b"no").unwrap();
        assert!(matches!(
            install(&junk, &installed, &Accept),
            Err(InstallError::Unpack(_))
        ));
        assert_eq!(version(&installed), "1");
    }

    #[test]
    fn finds_the_bundle_and_refuses_translocated_copies() {
        let p = Path::new("/Applications/Ghira.app/Contents/MacOS/ghi-desktop");
        assert_eq!(bundle_of(p).unwrap(), Path::new("/Applications/Ghira.app"));
        assert!(matches!(
            bundle_of(Path::new(
                "/private/var/folders/x/AppTranslocation/ABC/d/Ghira.app/Contents/MacOS/ghi-desktop"
            )),
            Err(InstallError::Translocated)
        ));
        assert!(matches!(
            bundle_of(Path::new("/Users/me/target/debug/ghi-desktop")),
            Err(InstallError::NotABundle)
        ));
    }
}
