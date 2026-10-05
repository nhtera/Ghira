// SPDX-License-Identifier: Apache-2.0
//! The per-field merge rules of doc 07 §7.4 (slice 15-C2): pure functions, no
//! database.

use std::cmp::Ordering;

use sha2::{Digest, Sha256};

use super::records::Version;

/// Which of two concurrent versions wins: the higher `(lamport, origin gid)`.
pub fn winner<'a>(a: &'a Version, b: &'a Version) -> &'a Version {
    match a.cmp(b) {
        Ordering::Less => b,
        _ => a,
    }
}

/// Rank of a meeting status:
/// `recording < importing < done < processing < ready`. (`done` is what
/// `finish_meeting` sets before a pass takes over; the recorder that never
/// queues one leaves it there.) `None` for an unknown status.
pub fn status_rank(status: &str) -> Option<u8> {
    match status {
        "recording" => Some(0),
        "importing" => Some(1),
        "done" => Some(2),
        "processing" => Some(3),
        "ready" => Some(4),
        _ => None,
    }
}

/// The higher-ranked of two known statuses (`a` if equal or `b` is unknown).
pub fn merge_status<'a>(a: &'a str, b: &'a str) -> &'a str {
    match (status_rank(a), status_rank(b)) {
        (Some(x), Some(y)) if y > x => b,
        (None, Some(_)) => b,
        _ => a,
    }
}

/// `cut_pages` merges by minimum; `None` means "never cut".
pub fn merge_cut(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

/// What the epoch fence says about an incoming row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fence {
    /// Same generation as the meeting: apply.
    Accept,
    /// Newer than the meeting knows: the meeting row may still be on its way,
    /// so hold the row (amendment #1).
    Park,
    /// Older: a later result replaced it. Drop it and tombstone it.
    Superseded,
}

/// Compares a row's `(version, epoch)` (or a bare epoch, with a `0` first
/// element on both sides) to the meeting's.
pub fn fence(row: (i64, i64), meeting: (i64, i64)) -> Fence {
    match row.cmp(&meeting) {
        Ordering::Equal => Fence::Accept,
        Ordering::Greater => Fence::Park,
        Ordering::Less => Fence::Superseded,
    }
}

/// The gid of the conflict copy of `field` of `target_gid`, whose text lost to
/// another version: a UUIDv8 over `H(gid, field, loser version)`, so every
/// device (and every redelivery) names the same copy the same way.
pub fn copy_gid(target_gid: &str, field: &str, loser_lamport: i64, loser_origin: &str) -> String {
    let mut h = Sha256::new();
    for part in [
        target_gid.as_bytes(),
        field.as_bytes(),
        &loser_lamport.to_be_bytes(),
        loser_origin.as_bytes(),
    ] {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part);
    }
    let digest = h.finalize();
    let mut b = [0u8; 16];
    b.copy_from_slice(&digest[..16]);
    uuid::Uuid::new_v8(b).to_string()
}

/// The gid of a link re-pointed from a merged-away tag to its survivor: a
/// UUIDv8 over `H(old link gid, survivor tag gid)`. Every device that folds
/// the same pair makes the same new link, so they do not pile up.
pub fn relink_gid(old_link_gid: &str, survivor_tag_gid: &str) -> String {
    let mut h = Sha256::new();
    for part in [
        b"relink".as_slice(),
        old_link_gid.as_bytes(),
        survivor_tag_gid.as_bytes(),
    ] {
        h.update((part.len() as u64).to_be_bytes());
        h.update(part);
    }
    let digest = h.finalize();
    let mut b = [0u8; 16];
    b.copy_from_slice(&digest[..16]);
    uuid::Uuid::new_v8(b).to_string()
}

/// The merged-into edge to clear in a cycle: the one with the lowest version
/// `(lamport, origin gid)`. `edges` are the versions of the cycle's rows, in
/// any order; returns the index.
pub fn weakest_edge(edges: &[Version]) -> usize {
    let mut best = 0;
    for (i, e) in edges.iter().enumerate() {
        if *e < edges[best] {
            best = i;
        }
    }
    best
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
        assert_eq!(merge_status("processing", "recording"), "processing");
        assert_eq!(merge_status("done", "recording"), "done");
        assert_eq!(merge_status("done", "processing"), "processing");
        assert_eq!(merge_status("importing", "ready"), "ready");
        assert_eq!(merge_status("ready", "bogus"), "ready");
    }

    #[test]
    fn cut_pages_take_the_minimum() {
        assert_eq!(merge_cut(Some(9), Some(4)), Some(4));
        assert_eq!(merge_cut(None, Some(4)), Some(4));
        assert_eq!(merge_cut(Some(9), None), Some(9));
        assert_eq!(merge_cut(None, None), None);
    }

    #[test]
    fn fence_compares_generations() {
        assert_eq!(fence((1, 1), (1, 1)), Fence::Accept);
        assert_eq!(fence((2, 0), (1, 5)), Fence::Park);
        assert_eq!(fence((1, 2), (1, 1)), Fence::Park);
        assert_eq!(fence((1, 0), (1, 1)), Fence::Superseded);
        assert_eq!(fence((0, 3), (0, 4)), Fence::Superseded);
    }

    #[test]
    fn relink_gids_are_deterministic_and_distinct() {
        let a = relink_gid("l1", "s");
        assert_eq!(a, relink_gid("l1", "s"));
        assert_ne!(a, relink_gid("l2", "s"));
        assert_ne!(a, relink_gid("l1", "t"));
        assert!(uuid::Uuid::parse_str(&a).is_ok());
    }

    #[test]
    fn weakest_edge_is_the_lowest_version() {
        let v = |lamport, origin: &str| Version {
            lamport,
            origin: origin.into(),
        };
        assert_eq!(weakest_edge(&[v(5, "a"), v(3, "z"), v(3, "b")]), 2);
        assert_eq!(weakest_edge(&[v(1, "a")]), 0);
    }

    #[test]
    fn copy_gids_are_deterministic_and_distinct() {
        let a = copy_gid("g", "title_ct", 7, "dev");
        assert_eq!(a, copy_gid("g", "title_ct", 7, "dev"));
        assert_ne!(a, copy_gid("g", "title_ct", 8, "dev"));
        assert_ne!(a, copy_gid("g", "body_ct", 7, "dev"));
        assert_ne!(a, copy_gid("g", "title_ct", 7, "dev2"));
        assert!(uuid::Uuid::parse_str(&a).is_ok());
    }
}
