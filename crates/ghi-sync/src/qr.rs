// SPDX-License-Identifier: Apache-2.0
//! The pairing QR (doc 07 §3.2; slice 15-E): `"GHI1:" || base45(CBOR{v, dev,
//! pk, psk, addrs})`. It carries the desktop's key and a one-time PSK, never a
//! name. The parser rejects unknown `v`, trailing bytes and addresses that are
//! not LAN. Nothing logs a payload.
//!
//! The image is rendered here (D11): [`svg`] returns an SVG string the UI shows
//! as an `<img>` data URL; no JS QR library.

use std::net::SocketAddr;

use crate::identity::Psk;
use crate::{Result, not_yet};

/// Text prefix of the payload.
pub const QR_PREFIX: &str = "GHI1:";
/// Payload version.
pub const QR_VERSION: u8 = 1;
/// Most addresses in a payload.
pub const MAX_QR_ADDRS: usize = 4;

/// What the phone learns from the QR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrPayload {
    pub v: u8,
    /// The hub's `device_gid` (16 bytes).
    pub dev: [u8; 16],
    /// The hub's static public key.
    pub pk: [u8; 32],
    /// The one-time QR PSK (redacted in `Debug`).
    pub psk: Psk,
    /// At most [`MAX_QR_ADDRS`], all LAN.
    pub addrs: Vec<SocketAddr>,
}

/// The text to put in the QR.
pub fn encode(_payload: &QrPayload) -> Result<String> {
    not_yet("qr::encode")
}

/// Parses scanned text. Never trusts it: see the module docs.
pub fn parse(_text: &str) -> Result<QrPayload> {
    not_yet("qr::parse")
}

/// Renders `text` as an SVG QR code (SVG only; no `image` dependency).
pub fn svg(_text: &str) -> Result<String> {
    not_yet("qr::svg")
}
