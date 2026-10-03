// SPDX-License-Identifier: Apache-2.0
//! Folders and tags (phase 14d, D7): one optional folder per meeting, many
//! tags. Names are plain text, unique ignoring case, limited in length and
//! count. Every command refuses while the app is locked. Errors are codes:
//! `duplicate`, `tooLong`, `empty`, `limit`, `notFound`, `storage`.
//!
//! Names are typed freely: the store trims, folds white space and compares
//! names ignoring case but not accents ("Họp" and "Hộp" are two names).
//! `create_tag` reuses a lone accent-variant match ("hop" finds "Họp");
//! `create_folder` refuses it as `duplicate`.

use ghi_store::{StoreError, organize};
use serde::Serialize;
use specta::Type;

use crate::{CoreState, blocking};

pub(crate) const DUPLICATE: &str = "duplicate";
pub(crate) const TOO_LONG: &str = "tooLong";
pub(crate) const LIMIT: &str = "limit";
pub(crate) const NOT_FOUND: &str = "notFound";
pub(crate) const STORAGE: &str = "storage";
/// A name with nothing visible in it.
pub(crate) const EMPTY: &str = "empty";

/// A store failure as a code the UI turns into words (details go to the log).
pub(crate) fn code(e: StoreError) -> String {
    match e {
        StoreError::Duplicate { .. } => DUPLICATE.into(),
        // A name past its length is `Limit` too, with a "characters in …" kind.
        StoreError::Limit { kind, .. } if kind.starts_with("characters") => TOO_LONG.into(),
        StoreError::Limit { .. } => LIMIT.into(),
        StoreError::NotFound { .. } => NOT_FOUND.into(),
        StoreError::Invalid(_) => EMPTY.into(),
        other => {
            log::warn!("organize: {other}");
            STORAGE.into()
        }
    }
}

fn n(count: i64) -> u32 {
    count.clamp(0, i64::from(u32::MAX)) as u32
}

impl From<organize::Folder> for FolderRow {
    fn from(f: organize::Folder) -> Self {
        FolderRow {
            gid: f.gid,
            name: f.name,
            meetings: n(f.meetings),
        }
    }
}

impl From<organize::Tag> for TagRow {
    fn from(t: organize::Tag) -> Self {
        TagRow {
            gid: t.gid,
            name: t.name,
            meetings: n(t.meetings),
        }
    }
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FolderRow {
    pub gid: String,
    pub name: String,
    /// Meetings in it.
    pub meetings: u32,
}

#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TagRow {
    pub gid: String,
    pub name: String,
    /// Meetings with it.
    pub meetings: u32,
}

fn count(r: usize) -> u32 {
    u32::try_from(r).unwrap_or(u32::MAX)
}

/// All folders, by name.
#[tauri::command]
#[specta::specta]
pub async fn list_folders(core: CoreState<'_>) -> Result<Vec<FolderRow>, String> {
    blocking(&core, |c| {
        let folders = c.store()?.folders().map_err(code)?;
        Ok(folders.into_iter().map(Into::into).collect())
    })
    .await
}

/// Makes a folder. Errors: `duplicate`, `tooLong`, `empty`, `limit`.
#[tauri::command]
#[specta::specta]
pub async fn create_folder(core: CoreState<'_>, name: String) -> Result<FolderRow, String> {
    blocking(&core, move |c| {
        Ok(c.store()?.create_folder(&name).map_err(code)?.into())
    })
    .await
}

/// Renames a folder. Errors: `duplicate`, `tooLong`, `empty`, `notFound`.
#[tauri::command]
#[specta::specta]
pub async fn rename_folder(
    core: CoreState<'_>,
    folder: String,
    name: String,
) -> Result<(), String> {
    blocking(&core, move |c| {
        c.store()?.rename_folder(&folder, &name).map_err(code)
    })
    .await
}

/// Deletes a folder; its meetings stay (in no folder). Returns how many were
/// in it. Errors: `notFound`.
#[tauri::command]
#[specta::specta]
pub async fn delete_folder(core: CoreState<'_>, folder: String) -> Result<u32, String> {
    blocking(&core, move |c| {
        Ok(count(c.store()?.delete_folder(&folder).map_err(code)?))
    })
    .await
}

/// Moves meetings into a folder (`null`: out of any folder). Returns how many
/// changed. Errors: `notFound`.
#[tauri::command]
#[specta::specta]
pub async fn move_to_folder(
    core: CoreState<'_>,
    meetings: Vec<String>,
    folder: Option<String>,
) -> Result<u32, String> {
    blocking(&core, move |c| {
        let changed = c
            .store()?
            .set_meeting_folder(&meetings, folder.as_deref())
            .map_err(code)?;
        Ok(count(changed))
    })
    .await
}

/// All tags, by name.
#[tauri::command]
#[specta::specta]
pub async fn list_tags(core: CoreState<'_>) -> Result<Vec<TagRow>, String> {
    blocking(&core, |c| {
        let tags = c.store()?.tags().map_err(code)?;
        Ok(tags.into_iter().map(Into::into).collect())
    })
    .await
}

/// Gets the tag with this name, making it if new (so the same name is never
/// two tags). Errors: `tooLong`, `empty`, `limit`.
#[tauri::command]
#[specta::specta]
pub async fn create_tag(core: CoreState<'_>, name: String) -> Result<TagRow, String> {
    blocking(&core, move |c| {
        Ok(c.store()?.create_tag(&name).map_err(code)?.into())
    })
    .await
}

/// Renames a tag. Errors: `duplicate`, `tooLong`, `empty`, `notFound`.
#[tauri::command]
#[specta::specta]
pub async fn rename_tag(core: CoreState<'_>, tag: String, name: String) -> Result<(), String> {
    blocking(&core, move |c| {
        c.store()?.rename_tag(&tag, &name).map_err(code)
    })
    .await
}

/// Deletes a tag from every meeting. Returns how many had it. Errors:
/// `notFound`.
#[tauri::command]
#[specta::specta]
pub async fn delete_tag(core: CoreState<'_>, tag: String) -> Result<u32, String> {
    blocking(&core, move |c| {
        Ok(count(c.store()?.delete_tag(&tag).map_err(code)?))
    })
    .await
}

/// Adds a tag to meetings. Returns how many gained it. Errors: `limit` (20
/// tags per meeting), `notFound`.
#[tauri::command]
#[specta::specta]
pub async fn tag_meetings(
    core: CoreState<'_>,
    meetings: Vec<String>,
    tag: String,
) -> Result<u32, String> {
    blocking(&core, move |c| {
        Ok(count(
            c.store()?.tag_meetings(&meetings, &tag).map_err(code)?,
        ))
    })
    .await
}

/// Removes a tag from meetings. Returns how many lost it. Errors: `notFound`.
#[tauri::command]
#[specta::specta]
pub async fn untag_meetings(
    core: CoreState<'_>,
    meetings: Vec<String>,
    tag: String,
) -> Result<u32, String> {
    blocking(&core, move |c| {
        Ok(count(
            c.store()?.untag_meetings(&meetings, &tag).map_err(code)?,
        ))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghi_store::keys::{MemoryKeyStore, Protection};
    use ghi_store::store::{NewMeeting, Store};
    use std::sync::Arc;

    fn open() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap();
        let s = Store::open(
            tmp.path(),
            Arc::new(MemoryKeyStore::default()),
            Protection::default(),
        )
        .unwrap();
        (tmp, s)
    }

    fn meeting(s: &Store) -> String {
        s.create_meeting(NewMeeting {
            title: "Planning".into(),
            ..Default::default()
        })
        .unwrap()
        .gid
    }

    #[test]
    fn store_failures_become_codes() {
        let (_t, s) = open();
        s.create_folder("Họp").unwrap();
        // Typing "hop" for a folder is the same name as "Họp"; "Hộp" is not.
        assert_eq!(code(s.create_folder("hop").unwrap_err()), DUPLICATE);
        assert!(s.create_folder("Hộp").is_ok());
        assert_eq!(code(s.create_folder("  ").unwrap_err()), EMPTY);
        assert_eq!(
            code(s.create_folder(&"x".repeat(61)).unwrap_err()),
            TOO_LONG
        );
        assert_eq!(code(s.rename_folder("nope", "A").unwrap_err()), NOT_FOUND);
        assert_eq!(code(s.delete_tag("nope").unwrap_err()), NOT_FOUND);
    }

    #[test]
    fn a_lone_accent_variant_is_reused_for_tags() {
        let (_t, s) = open();
        let a = s.create_tag("Họp").unwrap();
        let b: TagRow = s.create_tag("hop").unwrap().into();
        assert_eq!(a.gid, b.gid);
        assert_eq!(b.name, "Họp");
    }

    #[test]
    fn limits_are_codes() {
        let (_t, s) = open();
        let m = meeting(&s);
        for i in 0..organize::MAX_TAGS_PER_MEETING {
            let t = s.create_tag(&format!("t{i}")).unwrap();
            s.tag_meetings(std::slice::from_ref(&m), &t.gid).unwrap();
        }
        let extra = s.create_tag("one more").unwrap();
        let err = s.tag_meetings(&[m], &extra.gid).unwrap_err();
        assert_eq!(code(err), LIMIT);
        assert_eq!(code(s.create_tag(&"y".repeat(41)).unwrap_err()), TOO_LONG);
    }

    #[test]
    fn rows_carry_the_counts() {
        let (_t, s) = open();
        let (a, b) = (meeting(&s), meeting(&s));
        let f = s.create_folder("Clients").unwrap();
        assert_eq!(
            s.set_meeting_folder(&[a.clone(), b.clone()], Some(&f.gid))
                .unwrap(),
            2
        );
        let rows: Vec<FolderRow> = s.folders().unwrap().into_iter().map(Into::into).collect();
        assert_eq!(rows[0].meetings, 2);
        // Moving out of any folder, and deleting, keep the meetings.
        assert_eq!(s.set_meeting_folder(&[a], None).unwrap(), 1);
        assert_eq!(count(s.delete_folder(&f.gid).unwrap()), 1);
        let t = s.create_tag("x").unwrap();
        s.tag_meetings(&[b], &t.gid).unwrap();
        let tags: Vec<TagRow> = s.tags().unwrap().into_iter().map(Into::into).collect();
        assert_eq!(tags[0].meetings, 1);
    }
}
