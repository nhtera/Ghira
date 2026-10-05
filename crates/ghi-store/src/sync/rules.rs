// SPDX-License-Identifier: Apache-2.0
//! The per-field merge rules of doc 07 §7.4 (slice 15-C2).

use std::cmp::Ordering;

use super::records::Version;

/// Which of two concurrent versions wins: the higher `(lamport, origin gid)`.
pub fn winner<'a>(a: &'a Version, b: &'a Version) -> &'a Version {
    match a.cmp(b) {
        Ordering::Less => b,
        _ => a,
    }
}

/// Rank of a meeting status: `recording < importing < processing < ready`.
/// `None` for an unknown status.
pub fn status_rank(status: &str) -> Option<u8> {
    match status {
        "recording" => Some(0),
        "importing" => Some(1),
        "processing" => Some(2),
        "ready" => Some(3),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn higher_lamport_wins_then_origin() {
        let v = |lamport, origin: &str| Version {
            lamport,
            origin: origin.into(),
        };
        assert_eq!(winner(&v(3, "a"), &v(2, "z")), &v(3, "a"));
        assert_eq!(winner(&v(2, "a"), &v(2, "b")), &v(2, "b"));
        assert_eq!(winner(&v(2, "b"), &v(2, "a")), &v(2, "b"));
    }

    #[test]
    fn status_ranks_are_ordered() {
        assert!(status_rank("recording") < status_rank("ready"));
        assert_eq!(status_rank("bogus"), None);
    }
}
