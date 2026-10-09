// SPDX-License-Identifier: Apache-2.0
//! Synced settings (slice 15-C1): an allowlist, last writer wins per key.
//!
//! `put_synced` records a local change in `synced_settings` (the caller has
//! already written the setting itself). `apply_synced` takes a peer's value
//! and also writes the underlying setting: a field of the `"app"` JSON, or the
//! vocabulary key.

use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::Value;

use super::records::SettingRec;
use crate::store::Store;
use crate::{Result, StoreError};

/// The only keys that sync: the `"app"` JSON fields (serde names) and the
/// vocabulary keys. Everything else is device-local.
pub const SYNCED_KEYS: [&str; 8] = [
    "meetingLanguage",
    "cloudRedact",
    "audioRetentionDays",
    "consentMessageEn",
    "consentMessageVi",
    "vocabulary",
    "vocabulary.ignored",
    "vocabulary.packs",
];

/// The glossary packs that exist (`ghi-core` `glossaries/`): `<domain>-<lang>`.
/// `vocabulary.packs` is a list of these.
pub const PACK_IDS: [&str; 8] = [
    "medical-en",
    "medical-vi",
    "legal-en",
    "legal-vi",
    "finance-en",
    "finance-vi",
    "tech-en",
    "tech-vi",
];

/// The settings key whose JSON object holds the first five keys.
const APP_KEY: &str = "app";
const MAX_TEXT: usize = 4_000;
const MAX_TERMS: usize = 10_000;
const MAX_TERM_LEN: usize = 512;

/// Whether `key` is on the allowlist.
pub fn is_synced_key(key: &str) -> bool {
    SYNCED_KEYS.contains(&key)
}

/// The gid of a key's row: UUIDv5 of the key, the same on every device.
pub fn setting_gid(key: &str) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("ghira:setting:{key}").as_bytes(),
    )
    .to_string()
}

/// Parses `value_json` and checks it has the shape of the key's setting, so a
/// peer can't put a value in `"app"` that breaks reading it.
fn parse_value(key: &str, value_json: &str) -> Result<Value> {
    let bad = || StoreError::Invalid(format!("synced setting {key}"));
    let v: Value = serde_json::from_str(value_json).map_err(|_| bad())?;
    let ok = match key {
        "meetingLanguage" => matches!(v.as_str(), Some("en" | "vi" | "auto")),
        "cloudRedact" => v.is_boolean(),
        "audioRetentionDays" => v.as_u64().is_some_and(|n| n <= 36_500),
        "consentMessageEn" | "consentMessageVi" => v.as_str().is_some_and(|s| s.len() <= MAX_TEXT),
        "vocabulary" | "vocabulary.ignored" => v.as_array().is_some_and(|a| {
            a.len() <= MAX_TERMS
                && a.iter()
                    .all(|t| t.as_str().is_some_and(|s| s.len() <= MAX_TERM_LEN))
        }),
        "vocabulary.packs" => v.as_array().is_some_and(|a| {
            a.iter()
                .all(|t| t.as_str().is_some_and(|s| PACK_IDS.contains(&s)))
        }),
        _ => false,
    };
    if ok { Ok(v) } else { Err(bad()) }
}

/// A key's current row: `(value_json, lamport, origin device gid)`.
fn current(tx: &Transaction, key: &str, own: &str) -> Result<Option<(String, i64, String)>> {
    Ok(tx
        .query_row(
            "SELECT value_json, lamport,
                    COALESCE((SELECT d.gid FROM devices d WHERE d.id = x.origin), ?2)
             FROM synced_settings x WHERE key = ?1",
            params![key, own],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?)
}

impl Store {
    /// Records a local change to a synced key (`value` is JSON text). Keys
    /// off the allowlist are an error. An unchanged value is not a change.
    pub fn put_synced(&self, key: &str, value_json: &str) -> Result<()> {
        if !is_synced_key(key) {
            return Err(StoreError::Invalid(format!(
                "{key} is not a synced setting"
            )));
        }
        // Stored in the canonical form of the parsed value.
        let value = parse_value(key, value_json)?.to_string();
        let own = self.sync_device_gid()?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if current(&tx, key, &own)?.is_some_and(|(v, _, _)| v == value) {
            return Ok(());
        }
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "INSERT INTO synced_settings (gid, key, value_json, lamport, origin)
             VALUES (?1, ?2, ?3, ?4, NULL)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json,
                 lamport = excluded.lamport, origin = NULL",
            params![setting_gid(key), key, value, lamport],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Applies a peer's value if its version is newer than ours (last writer
    /// wins per key, ties by origin gid): writes `synced_settings` and the
    /// underlying setting. Unknown keys are ignored. A known key with a value
    /// of the wrong shape is [`StoreError::Invalid`].
    pub fn apply_synced(&self, rec: &SettingRec) -> Result<()> {
        let key = rec.key.as_str();
        let Some(value_json) = rec.value_json.as_deref() else {
            return Ok(());
        };
        if !is_synced_key(key) {
            return Ok(());
        }
        let value = parse_value(key, value_json)?;
        let own = self.sync_device_gid()?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let incoming = (rec.version.lamport, rec.version.origin.as_str());
        if let Some((_, lamport, origin)) = current(&tx, key, &own)?
            && incoming <= (lamport, origin.as_str())
        {
            return Ok(());
        }
        // A device we are not paired with (relayed by the hub) has no row; the
        // origin then reads as this device, which only matters for ties.
        let origin: Option<i64> = if rec.version.origin == own {
            None
        } else {
            tx.query_row(
                "SELECT id FROM devices WHERE gid = ?1",
                [&rec.version.origin],
                |r| r.get(0),
            )
            .optional()?
        };
        tx.execute(
            "INSERT INTO synced_settings (gid, key, value_json, lamport, origin)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json,
                 lamport = excluded.lamport, origin = excluded.origin",
            params![
                setting_gid(key),
                key,
                value.to_string(),
                rec.version.lamport,
                origin
            ],
        )?;
        write_underlying(&tx, key, value)?;
        super::observe_lamport(&tx, rec.version.lamport)?;
        tx.commit()?;
        Ok(())
    }
}

/// Writes the setting a synced key stands for.
fn write_underlying(tx: &Transaction, key: &str, value: Value) -> Result<()> {
    let (row_key, json) = if key.starts_with("vocabulary") {
        (key, value.to_string())
    } else {
        let raw: Option<String> = tx
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                [APP_KEY],
                |r| r.get(0),
            )
            .optional()?;
        let mut app = raw
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| Value::Object(Default::default()));
        app[key] = value;
        (APP_KEY, app.to_string())
    };
    tx.execute(
        "INSERT INTO settings (key, value_json) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
        params![row_key, json],
    )?;
    Ok(())
}
