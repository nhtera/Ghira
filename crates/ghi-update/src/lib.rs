// SPDX-License-Identifier: Apache-2.0
//! App updates (phase 12, doc 05 §4).
//!
//! - **Feed:** one JSON manifest per channel, signed offline by the owner with
//!   minisign (`<manifest>.minisig`). Both are fetched through `ghi-net`
//!   (`fetch_small`, update hosts only, nothing under strict offline). The
//!   manifest carries the archive's SHA-256, so its signature binds the
//!   archive and the kill-switch fields together.
//! - **Policy** ([`decide`]): only a strictly newer version is offered (never a
//!   downgrade: a newer store schema can't be opened by older code); a
//!   replayed older manifest (lower `sequence`) or an expired one is refused;
//!   `pulled` warns that the running version was withdrawn; `min_supported`
//!   says a manual reinstall is needed.
//! - **Install** ([`install`]): the archive (a `ditto` zip of the signed,
//!   stapled `.app`) is unpacked next to the installed app, its signature and
//!   Team ID checked, then swapped in by rename (the old app kept until the
//!   new one has started). Only on the user's "Restart to update".
//!
//! Until the owner sets [`FEED_URL`] and [`PUBLIC_KEYS`], updates are off.

pub mod install;
pub mod manifest;

pub use manifest::{Archive, Decision, Latest, Manifest, UpdateError, decide, verify};

/// The alpha channel's manifest (its signature is `<url>.minisig`). `None`
/// until the release feed exists (owner, phase 12).
pub const FEED_URL: Option<&str> = None;

/// minisign public keys (base64 key lines) trusted for the manifest; the
/// owner keeps the secret key offline. Empty: updates are off.
pub const PUBLIC_KEYS: &[&str] = &[];

/// The manifest and its signature are small.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

/// Updates can be checked in this build.
pub fn configured() -> bool {
    FEED_URL.is_some() && !PUBLIC_KEYS.is_empty()
}
