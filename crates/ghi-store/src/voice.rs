// SPDX-License-Identifier: Apache-2.0
//! Voice profiles (phase 14c, RT-13): biometric data with its own key.
//!
//! Each profile has a random 256-bit key, stored only wrapped by the
//! [`KeyRing`](crate::keys::KeyRing) (`voice_profiles.key_wrapped`, AAD
//! `voice:{gid}`). Its vectors (per model and language: a centroid, up to
//! [`MAX_EXEMPLARS`] exemplars and where each came from) and the optional
//! consent clip are sealed under it. Deleting a profile is a crypto-shred like
//! a meeting's: the key is zeroed and committed before the rows go, the wrap
//! secret is rotated, and a crash in between is finished on the next open.
//!
//! Vectors and names are never logged. [`speaker_voices`](Store::speaker_voices)
//! holds the voices of unnamed clusters (third-party profiles only), sealed
//! under the meeting key.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use crate::rowcrypt::{self, Dek};
use crate::store::{Store, compact_locked, id_of, now_ms};
use crate::{Result, StoreError, new_gid, tombstones};

/// Exemplars kept per profile, model and language (the newest).
pub const MAX_EXEMPLARS: usize = 20;
/// HKDF label of the profile key's subkey. Changing it makes data unreadable.
const VOICE_INFO: &[u8] = b"ghira/voice/v1";
const BLOB_V1: u8 = 1;

/// Where an exemplar's audio came from (a meeting span), for playing samples.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExemplarSource {
    pub meeting_gid: String,
    pub t0_ms: i64,
    pub t1_ms: i64,
}

/// Proof that the caller checked the third-party voice-profile flag
/// (`voice_profiles_third_party`, which a release build forces off). Every
/// store call that writes or reads a non-Me voice needs one. Only the desktop's
/// flag-checked path (its `enforce()` result) may build it; the store cannot
/// check the flag itself, so tests and the CLI must not build one casually.
#[derive(Debug, Clone, Copy)]
pub struct ThirdPartyApproved(());

impl ThirdPartyApproved {
    /// Call only right after reading the flag as on.
    pub fn assert_flag_checked() -> ThirdPartyApproved {
        ThirdPartyApproved(())
    }
}

fn need_approval(approval: Option<ThirdPartyApproved>) -> Result<()> {
    match approval {
        Some(_) => Ok(()),
        None => Err(StoreError::Invalid(
            "third-party voice profiles are not enabled".into(),
        )),
    }
}

/// One enrolled voice window: an L2-normalised vector.
#[derive(Clone, PartialEq)]
pub struct VoiceExemplar {
    pub vec: Vec<f32>,
    pub source: Option<ExemplarSource>,
}

/// The consent evidence for a profile (`consent_json`, `v` = 1). A recorded
/// clip's audio goes in the profile (see [`Store::put_voice_profile`]); `clip`
/// says which meeting span it was cut from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceConsent {
    /// `self_checkbox` or `verbal_clip`.
    pub method: String,
    pub at_ms: i64,
    /// The locale key of the consent text shown.
    pub text_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<ExemplarSource>,
}

impl VoiceConsent {
    /// D10: the method is known, and a verbal clip comes with its audio.
    fn validate(&self, clip: Option<&[u8]>) -> Result<()> {
        match (self.method.as_str(), clip) {
            ("self_checkbox", _) => Ok(()),
            ("verbal_clip", Some(c)) if !c.is_empty() => Ok(()),
            ("verbal_clip", _) => Err(StoreError::Invalid(
                "a verbal consent needs its recorded clip".into(),
            )),
            _ => Err(StoreError::Invalid("unknown consent method".into())),
        }
    }

    pub(crate) fn to_json(&self) -> String {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        if let Some(o) = v.as_object_mut() {
            o.insert("v".into(), 1.into());
        }
        v.to_string()
    }

    pub(crate) fn from_json(s: &str) -> Result<VoiceConsent> {
        serde_json::from_str(s).map_err(|e| StoreError::Invalid(e.to_string()))
    }
}

/// A profile's vectors for one model and language.
#[derive(Clone, PartialEq)]
pub struct VoiceSet {
    pub model: String,
    pub lang: String,
    /// Mean of the exemplars, L2-normalised.
    pub centroid: Vec<f32>,
    /// Oldest first, at most [`MAX_EXEMPLARS`].
    pub exemplars: Vec<VoiceExemplar>,
}

#[derive(Clone, PartialEq)]
pub struct VoiceProfile {
    pub gid: String,
    pub person_gid: String,
    pub is_me: bool,
    pub consent: VoiceConsent,
    pub created_at: i64,
    pub updated_at: i64,
    pub sets: Vec<VoiceSet>,
}

/// Row counts of the biometric tables ([`Store::raw_voice_counts`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceCounts {
    pub profiles: i64,
    /// Profiles of anyone but Me.
    pub other_profiles: i64,
    pub embedding_rows: i64,
    /// Embedding rows of profiles of anyone but Me.
    pub other_embedding_rows: i64,
    pub speaker_voices: i64,
}

/// The voice of an unnamed cluster, from the final pass.
#[derive(Clone, PartialEq)]
pub struct SpeakerVoice {
    pub model: String,
    pub lang: String,
    pub vec: Vec<f32>,
}

// Biometric vectors never reach a log or a panic message through `{:?}`.
impl std::fmt::Debug for VoiceExemplar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VoiceExemplar(dim {}, vector hidden)", self.vec.len())
    }
}

impl std::fmt::Debug for VoiceSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceSet")
            .field("model", &self.model)
            .field("lang", &self.lang)
            .field("exemplars", &self.exemplars.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for VoiceProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceProfile")
            .field("gid", &self.gid)
            .field("is_me", &self.is_me)
            .field("sets", &self.sets)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for SpeakerVoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpeakerVoice")
            .field("model", &self.model)
            .field("lang", &self.lang)
            .field("dim", &self.vec.len())
            .finish_non_exhaustive()
    }
}

fn set_aad(profile_gid: &str, model: &str, lang: &str) -> Vec<u8> {
    format!("voice_embeddings.vec_ct:{profile_gid}:{model}:{lang}").into_bytes()
}

fn clip_aad(profile_gid: &str) -> Vec<u8> {
    format!("voice_profiles.consent_clip_ct:{profile_gid}").into_bytes()
}

fn speaker_voice_aad(speaker_gid: &str, model: &str, lang: &str) -> Vec<u8> {
    format!("speaker_voices.vec_ct:{speaker_gid}:{model}:{lang}").into_bytes()
}

fn put_f32s(out: &mut Vec<u8>, v: &[f32]) {
    out.extend(v.iter().flat_map(|x| x.to_le_bytes()));
}

fn take_f32s(bytes: &[u8], pos: &mut usize, n: usize) -> Result<Vec<f32>> {
    let end = n
        .checked_mul(4)
        .and_then(|l| pos.checked_add(l))
        .filter(|e| *e <= bytes.len())
        .ok_or(StoreError::Decrypt)?;
    let v = bytes[*pos..end]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    *pos = end;
    Ok(v)
}

fn take_u32(bytes: &[u8], pos: &mut usize) -> Result<usize> {
    let end = *pos + 4;
    let b = bytes.get(*pos..end).ok_or(StoreError::Decrypt)?;
    *pos = end;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
}

/// The mean of `ex`, L2-normalised (all of one dimension, non-empty).
fn centroid_of(ex: &[VoiceExemplar]) -> Vec<f32> {
    let dim = ex.first().map_or(0, |e| e.vec.len());
    let mut c = vec![0f32; dim];
    for e in ex {
        for (a, b) in c.iter_mut().zip(&e.vec) {
            *a += b;
        }
    }
    let norm = c.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        c.iter_mut().for_each(|x| *x /= norm);
    }
    c
}

/// Plaintext of one `voice_embeddings.vec_ct`: version, dim, n, centroid,
/// exemplars, then the sources as JSON.
fn encode_set(ex: &[VoiceExemplar]) -> Vec<u8> {
    let dim = ex.first().map_or(0, |e| e.vec.len());
    let mut out = vec![BLOB_V1];
    out.extend((dim as u32).to_le_bytes());
    out.extend((ex.len() as u32).to_le_bytes());
    put_f32s(&mut out, &centroid_of(ex));
    for e in ex {
        put_f32s(&mut out, &e.vec);
    }
    let sources: Vec<Option<&ExemplarSource>> = ex.iter().map(|e| e.source.as_ref()).collect();
    let json = serde_json::to_vec(&sources).unwrap_or_default();
    out.extend((json.len() as u32).to_le_bytes());
    out.extend(json);
    out
}

fn decode_set(model: &str, lang: &str, bytes: &[u8]) -> Result<VoiceSet> {
    let (&BLOB_V1, rest) = bytes.split_first().ok_or(StoreError::Decrypt)? else {
        return Err(StoreError::Decrypt);
    };
    let mut pos = 0;
    let dim = take_u32(rest, &mut pos)?;
    let n = take_u32(rest, &mut pos)?;
    let centroid = take_f32s(rest, &mut pos, dim)?;
    let mut vecs = Vec::new();
    for _ in 0..n {
        vecs.push(take_f32s(rest, &mut pos, dim)?);
    }
    let jlen = take_u32(rest, &mut pos)?;
    let json = rest.get(pos..pos + jlen).ok_or(StoreError::Decrypt)?;
    let sources: Vec<Option<ExemplarSource>> =
        serde_json::from_slice(json).map_err(|_| StoreError::Decrypt)?;
    if sources.len() != vecs.len() {
        return Err(StoreError::Decrypt);
    }
    Ok(VoiceSet {
        model: model.to_string(),
        lang: lang.to_string(),
        centroid,
        exemplars: vecs
            .into_iter()
            .zip(sources)
            .map(|(vec, source)| VoiceExemplar { vec, source })
            .collect(),
    })
}

/// Keeps the newest [`MAX_EXEMPLARS`]; checks one non-zero dimension.
fn clamp_exemplars(mut ex: Vec<VoiceExemplar>) -> Result<Vec<VoiceExemplar>> {
    let dim = ex.first().map_or(0, |e| e.vec.len());
    if ex.iter().any(|e| e.vec.len() != dim || dim == 0) {
        return Err(StoreError::Invalid(
            "voice vectors must be non-empty and of one size".into(),
        ));
    }
    if ex.len() > MAX_EXEMPLARS {
        ex.drain(..ex.len() - MAX_EXEMPLARS);
    }
    Ok(ex)
}

impl Store {
    /// The profile's key (cached, or unwrapped from `key_wrapped`). A shredded
    /// profile gives [`StoreError::Decrypt`].
    pub(crate) fn voice_key(&self, conn: &Connection, profile_id: i64) -> Result<Dek> {
        if let Some(k) = self.voice_keys().get(&profile_id) {
            return Ok(k.clone());
        }
        let (gid, wrapped): (String, Vec<u8>) = conn
            .query_row(
                "SELECT gid, key_wrapped FROM voice_profiles WHERE id = ?1",
                [profile_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "voice profile",
                gid: profile_id.to_string(),
            })?;
        let key = self.ring().unwrap_voice_key(&wrapped, &gid)?;
        self.voice_keys().insert(profile_id, key.clone());
        Ok(key)
    }

    /// Creates the person's voice profile, replacing (and crypto-shredding)
    /// an existing one in the same transaction. `sets` are `(language,
    /// exemplars)`; `consent_clip` is the audio of a verbal consent (required
    /// for that method), sealed under the profile key so it survives audio
    /// retention. A profile for anyone but Me needs `approval`. Returns the
    /// profile gid.
    pub fn put_voice_profile(
        &self,
        person_gid: &str,
        consent: &VoiceConsent,
        consent_clip: Option<&[u8]>,
        model: &str,
        sets: Vec<(String, Vec<VoiceExemplar>)>,
        approval: Option<ThirdPartyApproved>,
    ) -> Result<String> {
        consent.validate(consent_clip)?;
        let mut prepared = Vec::new();
        for (lang, ex) in sets {
            if !ex.is_empty() {
                prepared.push((lang, clamp_exemplars(ex)?));
            }
        }
        let has_old = {
            let conn = self.conn();
            let (id, is_me): (i64, bool) = conn
                .query_row(
                    "SELECT id, is_me FROM persons WHERE gid = ?1",
                    [person_gid],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?
                .ok_or_else(|| StoreError::NotFound {
                    kind: "person",
                    gid: person_gid.to_string(),
                })?;
            if !is_me {
                need_approval(approval)?;
            }
            conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM voice_profiles WHERE person_id = ?1)",
                [id],
                |r| r.get::<_, bool>(0),
            )?
        };
        // The old key's shred needs a rotation: start it before the change, so
        // a crash leaves it to the next open.
        if has_old {
            self.begin_wrap_rotation()?;
        }
        let gid = new_gid();
        let key = Dek::generate();
        let sub = key.subkey(VOICE_INFO);
        let clip_ct = consent_clip.map(|c| rowcrypt::seal(&sub, c, &clip_aad(&gid)));
        // The connection first, then the ring (wrap with the secret current
        // for this transaction, like `create_meeting`): a rotation can't run
        // in between and leave this key under a secret it destroys.
        let mut conn = self.conn();
        let wrapped = self.ring().wrap_voice_key(&key, &gid);
        let tx = conn.transaction()?;
        let (person_id, is_me): (i64, bool) = tx
            .query_row(
                "SELECT id, is_me FROM persons WHERE gid = ?1",
                [person_gid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "person",
                gid: person_gid.to_string(),
            })?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        let old: Option<(i64, String)> = tx
            .query_row(
                "SELECT id, gid FROM voice_profiles WHERE person_id = ?1",
                [person_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((old_id, old_gid)) = &old {
            tombstones::write(&tx, old_gid, "voice_profile", lamport)?;
            zero_key(&tx, *old_id)?;
            tx.execute("DELETE FROM voice_profiles WHERE id = ?1", [old_id])?;
        }
        let now = now_ms();
        tx.execute(
            "INSERT INTO voice_profiles
                 (gid, person_id, is_me, consent_json, key_wrapped, created_at,
                  consent_clip_ct, updated_at, lamport)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?6, ?8)",
            params![
                gid,
                person_id,
                is_me,
                consent.to_json(),
                wrapped,
                now,
                clip_ct,
                lamport
            ],
        )?;
        let profile_id = tx.last_insert_rowid();
        for (lang, ex) in &prepared {
            write_set(&tx, &sub, profile_id, &gid, model, lang, ex)?;
        }
        tx.commit()?;
        if let Some((old_id, _)) = &old {
            self.voice_keys().remove(old_id);
        }
        self.voice_keys().insert(profile_id, key);
        if old.is_some() {
            let _ = compact_locked(&conn);
            drop(conn);
            // The new profile is committed; if the rotation fails the ring
            // stays "rotating" and the next open finishes it.
            let _ = self.rotate_wraps(true);
        }
        Ok(gid)
    }

    /// Adds exemplars to the profile's set for `model` and `lang` (created if
    /// new), keeping the newest [`MAX_EXEMPLARS`], and recomputes the centroid.
    /// A profile that is not Me's needs `approval`.
    pub fn add_voice_exemplars(
        &self,
        profile_gid: &str,
        model: &str,
        lang: &str,
        exemplars: Vec<VoiceExemplar>,
        approval: Option<ThirdPartyApproved>,
    ) -> Result<()> {
        if exemplars.is_empty() {
            return Ok(());
        }
        let mut conn = self.conn();
        let profile_id = id_of(&conn, "voice_profiles", profile_gid)?;
        let is_me: bool = conn.query_row(
            "SELECT is_me FROM voice_profiles WHERE id = ?1",
            [profile_id],
            |r| r.get(0),
        )?;
        if !is_me {
            need_approval(approval)?;
        }
        let sub = self.voice_key(&conn, profile_id)?.subkey(VOICE_INFO);
        let tx = conn.transaction()?;
        // A sensitive meeting never teaches a voice: checked in this
        // transaction, so turning the mode on meanwhile cannot be missed.
        let mut exemplars = exemplars;
        exemplars.retain(|e| {
            e.source.as_ref().is_none_or(|s| {
                !tx.query_row(
                    "SELECT sensitive FROM meetings WHERE gid = ?1",
                    [&s.meeting_gid],
                    |r| r.get::<_, bool>(0),
                )
                .optional()
                .ok()
                .flatten()
                .unwrap_or(false)
            })
        });
        if exemplars.is_empty() {
            return Ok(());
        }
        let mut all = read_set(&tx, &sub, profile_id, profile_gid, model, lang)?
            .map(|s| s.exemplars)
            .unwrap_or_default();
        all.extend(exemplars);
        let all = clamp_exemplars(all)?;
        write_set(&tx, &sub, profile_id, profile_gid, model, lang, &all)?;
        let lamport = Store::alloc_lamport(&tx, 1)?;
        tx.execute(
            "UPDATE voice_profiles SET updated_at = ?1, lamport = ?2 WHERE id = ?3",
            params![now_ms(), lamport, profile_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Drops the exemplars taken from `meeting_gid` (a set left empty goes).
    /// For when a meeting's speaker identity changed and those vectors may be
    /// somebody else's. Returns how many were dropped. A profile that is not
    /// Me's needs `approval`.
    pub fn drop_voice_exemplars_from(
        &self,
        profile_gid: &str,
        meeting_gid: &str,
        approval: Option<ThirdPartyApproved>,
    ) -> Result<usize> {
        let mut conn = self.conn();
        let profile_id = id_of(&conn, "voice_profiles", profile_gid)?;
        let is_me: bool = conn.query_row(
            "SELECT is_me FROM voice_profiles WHERE id = ?1",
            [profile_id],
            |r| r.get(0),
        )?;
        if !is_me {
            need_approval(approval)?;
        }
        self.drop_exemplars_locked(&mut conn, profile_id, profile_gid, meeting_gid)
    }

    /// [`drop_voice_exemplars_from`](Store::drop_voice_exemplars_from) for Me's
    /// profile, if there is one (and its key is readable).
    pub fn drop_me_exemplars_from(&self, meeting_gid: &str) -> Result<usize> {
        let mut conn = self.conn();
        let row: Option<(i64, String)> = conn
            .query_row(
                "SELECT id, gid FROM voice_profiles WHERE is_me = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((id, gid)) = row else { return Ok(0) };
        match self.drop_exemplars_locked(&mut conn, id, &gid, meeting_gid) {
            Err(StoreError::Decrypt) => Ok(0),
            r => r,
        }
    }

    fn drop_exemplars_locked(
        &self,
        conn: &mut Connection,
        profile_id: i64,
        profile_gid: &str,
        meeting_gid: &str,
    ) -> Result<usize> {
        let sub = self.voice_key(conn, profile_id)?.subkey(VOICE_INFO);
        let sets: Vec<(String, String)> = conn
            .prepare_cached("SELECT model, lang FROM voice_embeddings WHERE profile_id = ?1")?
            .query_map([profile_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let tx = conn.transaction()?;
        let mut dropped = 0;
        for (model, lang) in sets {
            let Some(set) = read_set(&tx, &sub, profile_id, profile_gid, &model, &lang)? else {
                continue;
            };
            let before = set.exemplars.len();
            let keep: Vec<VoiceExemplar> = set
                .exemplars
                .into_iter()
                .filter(|e| {
                    e.source
                        .as_ref()
                        .is_none_or(|s| s.meeting_gid != meeting_gid)
                })
                .collect();
            if keep.len() == before {
                continue;
            }
            dropped += before - keep.len();
            if keep.is_empty() {
                tx.execute(
                    "DELETE FROM voice_embeddings WHERE profile_id = ?1 AND model = ?2 AND lang = ?3",
                    params![profile_id, model, lang],
                )?;
            } else {
                write_set(&tx, &sub, profile_id, profile_gid, &model, &lang, &keep)?;
            }
        }
        if dropped > 0 {
            let lamport = Store::alloc_lamport(&tx, 1)?;
            tx.execute(
                "UPDATE voice_profiles SET updated_at = ?1, lamport = ?2 WHERE id = ?3",
                params![now_ms(), lamport, profile_id],
            )?;
        }
        tx.commit()?;
        Ok(dropped)
    }

    /// Row counts of the biometric tables, straight from the database (tests
    /// and diagnostics: no vector or name is returned).
    pub fn raw_voice_counts(&self) -> Result<VoiceCounts> {
        let conn = self.conn();
        let n = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, [], |r| r.get(0))?) };
        Ok(VoiceCounts {
            profiles: n("SELECT COUNT(*) FROM voice_profiles")?,
            other_profiles: n("SELECT COUNT(*) FROM voice_profiles WHERE is_me = 0")?,
            embedding_rows: n("SELECT COUNT(*) FROM voice_embeddings")?,
            other_embedding_rows: n(
                "SELECT COUNT(*) FROM voice_embeddings e JOIN voice_profiles p ON p.id = e.profile_id
                 WHERE p.is_me = 0",
            )?,
            speaker_voices: n("SELECT COUNT(*) FROM speaker_voices")?,
        })
    }

    /// Me's profile with its vectors for `model` (`None`: no profile, or its
    /// key is gone).
    pub fn me_voice_profile(&self, model: &str) -> Result<Option<VoiceProfile>> {
        Ok(self.profiles_of(model, true)?.into_iter().next())
    }

    /// The other people's profiles with their vectors for `model`. Needs the
    /// flag-checked token.
    pub fn third_party_voice_profiles(
        &self,
        model: &str,
        _approval: ThirdPartyApproved,
    ) -> Result<Vec<VoiceProfile>> {
        self.profiles_of(model, false)
    }

    /// Profiles whose key is gone are skipped.
    fn profiles_of(&self, model: &str, me: bool) -> Result<Vec<VoiceProfile>> {
        let conn = self.conn();
        let ids: Vec<i64> = conn
            .prepare_cached("SELECT id FROM voice_profiles WHERE is_me = ?1 ORDER BY id")?
            .query_map([me], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::new();
        for id in ids {
            match self.read_profile(&conn, id, Some(model)) {
                Ok(p) => out.push(p),
                Err(StoreError::Decrypt) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// The person's profile with every model's vectors, if they have one.
    pub fn voice_profile(&self, person_gid: &str) -> Result<Option<VoiceProfile>> {
        let conn = self.conn();
        let id: Option<i64> = conn
            .query_row(
                "SELECT v.id FROM voice_profiles v JOIN persons p ON p.id = v.person_id
                 WHERE p.gid = ?1",
                [person_gid],
                |r| r.get(0),
            )
            .optional()?;
        id.map(|id| self.read_profile(&conn, id, None)).transpose()
    }

    fn read_profile(
        &self,
        conn: &Connection,
        id: i64,
        model: Option<&str>,
    ) -> Result<VoiceProfile> {
        let (gid, person_gid, is_me, consent, created_at, updated_at): (
            String,
            String,
            bool,
            String,
            i64,
            Option<i64>,
        ) = conn.query_row(
            "SELECT v.gid, p.gid, v.is_me, v.consent_json, v.created_at, v.updated_at
             FROM voice_profiles v JOIN persons p ON p.id = v.person_id WHERE v.id = ?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )?;
        let sub = self.voice_key(conn, id)?.subkey(VOICE_INFO);
        let mut stmt = conn.prepare_cached(
            "SELECT model, lang, vec_ct FROM voice_embeddings
             WHERE profile_id = ?1 AND (?2 IS NULL OR model = ?2) ORDER BY model, lang",
        )?;
        let rows: Vec<(String, String, Vec<u8>)> = stmt
            .query_map(params![id, model], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let sets = rows
            .into_iter()
            .map(|(m, l, ct)| {
                let plain =
                    zeroize::Zeroizing::new(rowcrypt::open(&sub, &ct, &set_aad(&gid, &m, &l))?);
                decode_set(&m, &l, &plain)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(VoiceProfile {
            gid,
            person_gid,
            is_me,
            consent: VoiceConsent::from_json(&consent)?,
            created_at,
            updated_at: updated_at.unwrap_or(created_at),
            sets,
        })
    }

    /// The audio of a verbal consent, if one was stored.
    pub fn voice_consent_clip(&self, profile_gid: &str) -> Result<Option<Vec<u8>>> {
        let conn = self.conn();
        let (id, ct): (i64, Option<Vec<u8>>) = conn
            .query_row(
                "SELECT id, consent_clip_ct FROM voice_profiles WHERE gid = ?1",
                [profile_gid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "voice profile",
                gid: profile_gid.to_string(),
            })?;
        let Some(ct) = ct else { return Ok(None) };
        let sub = self.voice_key(&conn, id)?.subkey(VOICE_INFO);
        Ok(Some(rowcrypt::open(&sub, &ct, &clip_aad(profile_gid))?))
    }

    /// Deletes the voice profile (the person and their names in meetings stay,
    /// unless the person is left with nothing). A crypto-shred, in this order:
    /// 1. the wrap-secret rotation is started (a crash leaves it open);
    /// 2. a tombstone is written;
    /// 3. the profile key is zeroed and the cached key dropped (committed);
    /// 4. the rows (vectors, consent) are deleted and the database compacted;
    /// 5. the wrap secret is rotated, so a database copy taken earlier can no
    ///    longer unwrap the key.
    ///
    /// A crash after 3 is finished on the next [`Store::open`]. A copy taken
    /// before the delete stays readable while the old master key and wrap
    /// secret exist (see [`Store::delete_meeting`]'s limits).
    pub fn delete_voice_profile(&self, profile_gid: &str) -> Result<()> {
        self.delete_voice_profile_impl(profile_gid, true)
    }

    /// `gc_person`: also remove the person if the delete leaves them with no
    /// speakers and no profile (not when the profile is being replaced).
    fn delete_voice_profile_impl(&self, profile_gid: &str, gc_person: bool) -> Result<()> {
        let deleted = self.delete_voice_profile_before(profile_gid, gc_person);
        let rotated = self.rotate_wraps(true);
        deleted.and(rotated)
    }

    /// [`Store::delete_voice_profile`] without its last step. Public only so
    /// tests can simulate a crash there.
    #[doc(hidden)]
    pub fn delete_voice_profile_before_rotation(&self, profile_gid: &str) -> Result<()> {
        self.delete_voice_profile_before(profile_gid, true)
    }

    fn delete_voice_profile_before(&self, profile_gid: &str, gc_person: bool) -> Result<()> {
        id_of(&self.conn(), "voice_profiles", profile_gid)?;
        self.begin_wrap_rotation()?;
        {
            let mut conn = self.conn();
            let id = id_of(&conn, "voice_profiles", profile_gid)?;
            let tx = conn.transaction()?;
            let lamport = Store::alloc_lamport(&tx, 1)?;
            // The tombstone and the zeroed key commit together.
            tombstones::write(&tx, profile_gid, "voice_profile", lamport)?;
            zero_key(&tx, id)?;
            tx.commit()?;
            self.voice_keys().remove(&id);
        }
        self.finish_voice_delete(profile_gid, gc_person)
    }

    /// Step 3 of [`Store::delete_voice_profile`] (without the tombstone), on its
    /// own. Public only so
    /// tests can simulate a crash between the steps.
    #[doc(hidden)]
    pub fn shred_voice_key(&self, profile_gid: &str) -> Result<()> {
        let conn = self.conn();
        let id = id_of(&conn, "voice_profiles", profile_gid)?;
        zero_key(&conn, id)?;
        self.voice_keys().remove(&id);
        Ok(())
    }

    /// Step 4: removes the rows (the embeddings cascade) and compacts.
    fn finish_voice_delete(&self, profile_gid: &str, gc_person: bool) -> Result<()> {
        let mut conn = self.conn();
        let id = id_of(&conn, "voice_profiles", profile_gid)?;
        let tx = conn.transaction()?;
        let person: Option<i64> = tx.query_row(
            "SELECT person_id FROM voice_profiles WHERE id = ?1",
            [id],
            |r| r.get(0),
        )?;
        tx.execute("DELETE FROM voice_profiles WHERE id = ?1", [id])?;
        if let (true, Some(p)) = (gc_person, person) {
            crate::people::gc_persons(&tx, &[p])?;
        }
        tx.commit()?;
        self.voice_keys().remove(&id);
        compact_locked(&conn)
    }

    /// Completes profile deletes interrupted after the key was zeroed. Best
    /// effort (never fails an open).
    pub(crate) fn finish_pending_voice_deletes(&self) {
        let pending: Vec<String> = {
            let conn = self.conn();
            let Ok(mut stmt) = conn.prepare(
                "SELECT gid FROM voice_profiles
                 WHERE key_wrapped = zeroblob(length(key_wrapped))",
            ) else {
                return;
            };
            let Ok(rows) = stmt.query_map([], |r| r.get(0)) else {
                return;
            };
            rows.filter_map(|r| r.ok()).collect()
        };
        let mut done = false;
        for gid in pending {
            done |= self.finish_voice_delete(&gid, true).is_ok();
        }
        if done {
            let _ = self.rotate_wraps(true);
        }
    }

    // ------------------------------------------------- unnamed cluster voices

    /// Stores (replaces) the voice of an unnamed speaker, sealed under the
    /// meeting key. Only for third-party profiles, so it needs the flag-checked
    /// token.
    pub fn put_speaker_voice(
        &self,
        speaker_gid: &str,
        model: &str,
        lang: &str,
        vec: &[f32],
        _approval: ThirdPartyApproved,
    ) -> Result<()> {
        if vec.is_empty() {
            return Err(StoreError::Invalid("empty voice vector".into()));
        }
        let mut conn = self.conn();
        let (sid, meeting_id): (i64, i64) = conn
            .query_row(
                "SELECT id, meeting_id FROM speakers WHERE gid = ?1",
                [speaker_gid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound {
                kind: "speaker",
                gid: speaker_gid.to_string(),
            })?;
        let dek = self.dek(&conn, meeting_id)?;
        let mut plain = Vec::new();
        put_f32s(&mut plain, vec);
        let ct = rowcrypt::seal(
            &dek.subkey(rowcrypt::ROWS_INFO),
            &plain,
            &speaker_voice_aad(speaker_gid, model, lang),
        );
        let tx = conn.transaction()?;
        // Not for a sensitive meeting (checked in this transaction).
        let sensitive: bool = tx.query_row(
            "SELECT sensitive FROM meetings WHERE id = ?1",
            [meeting_id],
            |r| r.get(0),
        )?;
        if sensitive {
            return Ok(());
        }
        tx.execute(
            "INSERT OR REPLACE INTO speaker_voices (speaker_id, model, lang, dim, vec_ct)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![sid, model, lang, vec.len() as i64, ct],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn speaker_voice(&self, speaker_gid: &str) -> Result<Option<SpeakerVoice>> {
        let conn = self.conn();
        let row: Option<(i64, String, String, i64, Vec<u8>)> = conn
            .query_row(
                "SELECT s.meeting_id, v.model, v.lang, v.dim, v.vec_ct
                 FROM speaker_voices v JOIN speakers s ON s.id = v.speaker_id
                 WHERE s.gid = ?1",
                [speaker_gid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        row.map(|(meeting_id, model, lang, dim, ct)| {
            open_speaker_voice(
                &self.dek(&conn, meeting_id)?,
                speaker_gid,
                model,
                lang,
                dim,
                &ct,
            )
        })
        .transpose()
    }

    /// The stored voices of a meeting's speakers, as `(speaker gid, voice)`.
    pub fn speaker_voices(&self, meeting_gid: &str) -> Result<Vec<(String, SpeakerVoice)>> {
        let conn = self.conn();
        let m = Store::meeting_ref(&conn, meeting_gid)?;
        let rows: Vec<(String, String, String, i64, Vec<u8>)> = conn
            .prepare_cached(
                "SELECT s.gid, v.model, v.lang, v.dim, v.vec_ct
                 FROM speaker_voices v JOIN speakers s ON s.id = v.speaker_id
                 WHERE s.meeting_id = ?1 ORDER BY s.label_idx, s.id",
            )?
            .query_map([m.id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let dek = self.dek(&conn, m.id)?;
        rows.into_iter()
            .map(|(gid, model, lang, dim, ct)| {
                let v = open_speaker_voice(&dek, &gid, model, lang, dim, &ct)?;
                Ok((gid, v))
            })
            .collect()
    }

    /// Drops a speaker's stored voice (e.g. once it is named or discarded).
    pub fn clear_speaker_voice(&self, speaker_gid: &str) -> Result<()> {
        self.conn().execute(
            "DELETE FROM speaker_voices WHERE speaker_id = (SELECT id FROM speakers WHERE gid = ?1)",
            [speaker_gid],
        )?;
        Ok(())
    }
}

/// Zeroes a profile's wrapped key (the crypto-shred).
fn zero_key(conn: &Connection, profile_id: i64) -> Result<()> {
    conn.execute(
        "UPDATE voice_profiles SET key_wrapped = zeroblob(length(key_wrapped)) WHERE id = ?1",
        [profile_id],
    )?;
    Ok(())
}

fn open_speaker_voice(
    dek: &Dek,
    gid: &str,
    model: String,
    lang: String,
    dim: i64,
    ct: &[u8],
) -> Result<SpeakerVoice> {
    let plain = zeroize::Zeroizing::new(rowcrypt::open(
        &dek.subkey(rowcrypt::ROWS_INFO),
        ct,
        &speaker_voice_aad(gid, &model, &lang),
    )?);
    let mut pos = 0;
    let vec = take_f32s(
        &plain,
        &mut pos,
        usize::try_from(dim).map_err(|_| StoreError::Decrypt)?,
    )?;
    if pos != plain.len() {
        return Err(StoreError::Decrypt);
    }
    Ok(SpeakerVoice { model, lang, vec })
}

/// Writes (replaces) a profile's set; `sub` is the profile key's subkey.
fn write_set(
    tx: &Transaction,
    sub: &Dek,
    profile_id: i64,
    profile_gid: &str,
    model: &str,
    lang: &str,
    ex: &[VoiceExemplar],
) -> Result<()> {
    let plain = zeroize::Zeroizing::new(encode_set(ex));
    let ct = rowcrypt::seal(sub, &plain, &set_aad(profile_gid, model, lang));
    tx.execute(
        "INSERT OR REPLACE INTO voice_embeddings (profile_id, model, lang, dim, n, vec_ct)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            profile_id,
            model,
            lang,
            ex.first().map_or(0, |e| e.vec.len()) as i64,
            ex.len() as i64,
            ct
        ],
    )?;
    Ok(())
}

fn read_set(
    conn: &Connection,
    sub: &Dek,
    profile_id: i64,
    profile_gid: &str,
    model: &str,
    lang: &str,
) -> Result<Option<VoiceSet>> {
    let ct: Option<Vec<u8>> = conn
        .query_row(
            "SELECT vec_ct FROM voice_embeddings WHERE profile_id = ?1 AND model = ?2 AND lang = ?3",
            params![profile_id, model, lang],
            |r| r.get(0),
        )
        .optional()?;
    ct.map(|ct| {
        let plain = zeroize::Zeroizing::new(rowcrypt::open(
            sub,
            &ct,
            &set_aad(profile_gid, model, lang),
        )?);
        decode_set(model, lang, &plain)
    })
    .transpose()
}

impl Store {
    /// Merges profile `from_id` into `into_id` (both persons' profiles), in
    /// the caller's transaction: per model and language the exemplars are
    /// combined (the newest [`MAX_EXEMPLARS`] kept) and the centroid
    /// recomputed under `into`'s key; then `from` is tombstoned, its key
    /// zeroed and its rows deleted. The caller drops the cached key and
    /// rotates the wrap secret afterwards.
    pub(crate) fn merge_profile_tx(
        &self,
        tx: &Transaction,
        from_id: i64,
        into_id: i64,
        lamport: i64,
    ) -> Result<()> {
        let gid_of = |id: i64| -> Result<String> {
            Ok(
                tx.query_row("SELECT gid FROM voice_profiles WHERE id = ?1", [id], |r| {
                    r.get(0)
                })?,
            )
        };
        let (from_gid, into_gid) = (gid_of(from_id)?, gid_of(into_id)?);
        let from_sub = self.voice_key(tx, from_id)?.subkey(VOICE_INFO);
        let into_sub = self.voice_key(tx, into_id)?.subkey(VOICE_INFO);
        let sets: Vec<(String, String)> = tx
            .prepare("SELECT model, lang FROM voice_embeddings WHERE profile_id = ?1")?
            .query_map([from_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        for (model, lang) in sets {
            let Some(theirs) = read_set(tx, &from_sub, from_id, &from_gid, &model, &lang)? else {
                continue;
            };
            let mut all = read_set(tx, &into_sub, into_id, &into_gid, &model, &lang)?
                .map(|s| s.exemplars)
                .unwrap_or_default();
            let dim = all.first().map(|e| e.vec.len());
            // Vectors of another size can't be mixed in.
            all.extend(
                theirs
                    .exemplars
                    .into_iter()
                    .filter(|e| dim.is_none_or(|d| e.vec.len() == d)),
            );
            let all = clamp_exemplars(all)?;
            write_set(tx, &into_sub, into_id, &into_gid, &model, &lang, &all)?;
        }
        tx.execute(
            "UPDATE voice_profiles SET updated_at = ?1, lamport = ?2 WHERE id = ?3",
            params![now_ms(), lamport, into_id],
        )?;
        tombstones::write(tx, &from_gid, "voice_profile", lamport)?;
        tx.execute(
            "UPDATE voice_profiles SET key_wrapped = zeroblob(length(key_wrapped)) WHERE id = ?1",
            [from_id],
        )?;
        tx.execute("DELETE FROM voice_profiles WHERE id = ?1", [from_id])?;
        Ok(())
    }
}
