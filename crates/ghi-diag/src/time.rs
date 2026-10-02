// SPDX-License-Identifier: Apache-2.0
//! UTC timestamps without a date crate.

use std::time::{SystemTime, UNIX_EPOCH};

/// (year, month, day, hour, minute, second) in UTC.
fn civil(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    (y, m, d, rem / 3600, rem % 3600 / 60, rem % 60)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `20261002T235900Z`, usable in file names.
pub(crate) fn utc_stamp() -> String {
    let (y, mo, d, h, mi, s) = civil(now_secs());
    format!("{y:04}{mo:02}{d:02}T{h:02}{mi:02}{s:02}Z")
}

/// `2026-10-02T23:59:00Z`.
pub(crate) fn utc_iso() -> String {
    let (y, mo, d, h, mi, s) = civil(now_secs());
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    #[test]
    fn civil_known_dates() {
        assert_eq!(super::civil(0), (1970, 1, 1, 0, 0, 0));
        // October 2026
        assert_eq!(super::civil(1_791_000_000).0, 2026);
        assert_eq!(super::civil(951_782_400), (2000, 2, 29, 0, 0, 0));
    }
}
