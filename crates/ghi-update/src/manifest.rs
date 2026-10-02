// SPDX-License-Identifier: Apache-2.0
//! The signed update manifest and the update policy.

use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::Deserialize;

/// The only manifest format this build reads.
const SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    /// No trusted key signed these bytes (or the signature is malformed).
    BadSignature,
    /// The signature's trusted comment doesn't name this manifest's sequence.
    CommentMismatch,
    Malformed(String),
    /// An older manifest than one already seen (a replay).
    Replayed {
        sequence: u64,
        seen: u64,
    },
    Expired,
    WrongChannel(String),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::BadSignature => f.write_str("the update feed's signature is not valid"),
            UpdateError::CommentMismatch => {
                f.write_str("the update feed's signature doesn't match it")
            }
            UpdateError::Malformed(e) => write!(f, "the update feed can't be read: {e}"),
            UpdateError::Replayed { .. } => {
                f.write_str("the update feed is older than one already seen")
            }
            UpdateError::Expired => f.write_str("the update feed has expired"),
            UpdateError::WrongChannel(c) => {
                write!(f, "the update feed is for another channel ({c})")
            }
        }
    }
}

impl std::error::Error for UpdateError {}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    /// https, on an update host.
    pub url: String,
    /// Lowercase hex.
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Latest {
    pub version: String,
    pub min_macos: String,
    pub archive: Archive,
    #[serde(default)]
    pub notes_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    /// Grows with every manifest the owner signs.
    pub sequence: u64,
    pub channel: String,
    /// Unix ms after which the manifest is not trusted.
    pub expires_at: i64,
    pub latest: Latest,
    /// Withdrawn versions: running one of these says "update now".
    #[serde(default)]
    pub pulled: Vec<String>,
    /// Older versions can't update in place (a manual reinstall).
    #[serde(default)]
    pub min_supported: Option<String>,
}

/// Checks `bytes` against `signature` (a `.minisig` file) with any of the
/// trusted `public_keys`, then parses it. The signature's trusted comment
/// must contain `seq=<sequence>` (the owner's signing command writes it).
pub fn verify(
    bytes: &[u8],
    signature: &str,
    public_keys: &[&str],
) -> Result<Manifest, UpdateError> {
    let sig = Signature::decode(signature).map_err(|_| UpdateError::BadSignature)?;
    let trusted = public_keys.iter().any(|k| {
        PublicKey::from_base64(k)
            .ok()
            .is_some_and(|pk| pk.verify(bytes, &sig, false).is_ok())
    });
    if !trusted {
        return Err(UpdateError::BadSignature);
    }
    let m: Manifest =
        serde_json::from_slice(bytes).map_err(|e| UpdateError::Malformed(e.to_string()))?;
    if m.schema != SCHEMA {
        return Err(UpdateError::Malformed(format!("schema {}", m.schema)));
    }
    let tag = format!("seq={}", m.sequence);
    if !sig.trusted_comment().split_whitespace().any(|w| w == tag) {
        return Err(UpdateError::CommentMismatch);
    }
    for v in std::iter::once(&m.latest.version)
        .chain(m.pulled.iter())
        .chain(m.min_supported.iter())
    {
        Version::parse(v).map_err(|e| UpdateError::Malformed(format!("version {v}: {e}")))?;
    }
    if !m.latest.archive.url.starts_with("https://")
        || m.latest.archive.sha256.len() != 64
        || !m
            .latest
            .archive
            .sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err(UpdateError::Malformed("archive".into()));
    }
    Ok(m)
}

/// What the app should do about a verified manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// A newer version to offer.
    pub available: Option<Latest>,
    /// The running version was withdrawn: ask the user to update now.
    pub running_pulled: bool,
    /// The running version is too old to update in place.
    pub reinstall_needed: bool,
    /// Remember it: an older manifest is refused from now on.
    pub sequence: u64,
}

/// The update policy. `seen`: the highest sequence accepted before.
pub fn decide(
    m: &Manifest,
    running: &str,
    channel: &str,
    seen: u64,
    now_ms: i64,
) -> Result<Decision, UpdateError> {
    if m.channel != channel {
        return Err(UpdateError::WrongChannel(m.channel.clone()));
    }
    if m.sequence < seen {
        return Err(UpdateError::Replayed {
            sequence: m.sequence,
            seen,
        });
    }
    if now_ms > m.expires_at {
        return Err(UpdateError::Expired);
    }
    let parse = |v: &str| Version::parse(v).map_err(|e| UpdateError::Malformed(e.to_string()));
    let running_v = parse(running)?;
    let latest = parse(&m.latest.version)?;
    let reinstall_needed = match &m.min_supported {
        Some(min) => running_v < parse(min)?,
        None => false,
    };
    Ok(Decision {
        available: (latest > running_v && !reinstall_needed).then(|| m.latest.clone()),
        running_pulled: m
            .pulled
            .iter()
            .any(|p| parse(p).is_ok_and(|p| p == running_v)),
        reinstall_needed,
        sequence: m.sequence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway minisign key pair made for these tests only (`minisign -G`
    // with an empty password); the secret key is not kept anywhere.
    const PK: &str = include_str!("../tests/test-key.pub");

    fn manifest(seq: u64) -> String {
        format!(
            r#"{{"schema":1,"sequence":{seq},"channel":"alpha","expires_at":4102444800000,
"latest":{{"version":"0.1.0-alpha.2","min_macos":"14.2","archive":{{"url":"https://github.com/o/r/releases/download/v0.1.0-alpha.2/Ghira.app.zip","sha256":"{}","size":123}}}},
"pulled":["0.1.0-alpha.1"],"min_supported":"0.1.0-alpha.0"}}"#,
            "a".repeat(64)
        )
    }

    fn key() -> &'static str {
        PK.lines().nth(1).unwrap().trim()
    }

    #[test]
    fn a_signed_manifest_is_read_and_anything_else_refused() {
        let body = include_bytes!("../tests/manifest.json");
        let sig = include_str!("../tests/manifest.json.minisig");
        let m = verify(body, sig, &[key()]).unwrap();
        assert_eq!(m.sequence, 7);
        // One changed byte.
        let mut evil = body.to_vec();
        let i = evil.iter().position(|&b| b == b'7').unwrap();
        evil[i] = b'8';
        assert_eq!(verify(&evil, sig, &[key()]), Err(UpdateError::BadSignature));
        // No trusted key.
        assert_eq!(verify(body, sig, &[]), Err(UpdateError::BadSignature));
        assert_eq!(
            verify(body, "garbage", &[key()]),
            Err(UpdateError::BadSignature)
        );
    }

    fn parsed(seq: u64) -> Manifest {
        serde_json::from_str(&manifest(seq)).unwrap()
    }

    #[test]
    fn only_newer_versions_are_offered_and_the_kill_switch_warns() {
        let m = parsed(7);
        let d = decide(&m, "0.1.0-alpha.1", "alpha", 6, 0).unwrap();
        assert_eq!(d.available.unwrap().version, "0.1.0-alpha.2");
        assert!(d.running_pulled, "alpha.1 was withdrawn");
        assert!(!d.reinstall_needed);
        assert_eq!(d.sequence, 7);
        // Same or newer running: nothing to offer (never a downgrade).
        assert!(
            decide(&m, "0.1.0-alpha.2", "alpha", 7, 0)
                .unwrap()
                .available
                .is_none()
        );
        assert!(
            decide(&m, "0.1.0", "alpha", 7, 0)
                .unwrap()
                .available
                .is_none()
        );
    }

    #[test]
    fn replays_expiry_channels_and_unsupported_versions() {
        let m = parsed(5);
        assert!(matches!(
            decide(&m, "0.1.0-alpha.1", "alpha", 6, 0),
            Err(UpdateError::Replayed { .. })
        ));
        assert_eq!(
            decide(&m, "0.1.0-alpha.1", "alpha", 5, 4_102_444_800_001),
            Err(UpdateError::Expired)
        );
        assert!(matches!(
            decide(&m, "0.1.0-alpha.1", "beta", 0, 0),
            Err(UpdateError::WrongChannel(_))
        ));
        let mut old = parsed(5);
        old.min_supported = Some("0.1.0-alpha.1".into());
        let d = decide(&old, "0.1.0-alpha.0", "alpha", 0, 0).unwrap();
        assert!(d.reinstall_needed && d.available.is_none());
    }
}
