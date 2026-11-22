//! UTC calendar arithmetic for log file names, timestamps and archive names, without a date crate.
//!
//! Every date of the journal is in UTC: the local offset of Windows is not needed, and a date never jumps twice a
//! year.

use std::time::{SystemTime, UNIX_EPOCH};

const SECS_PER_DAY: i64 = 86_400;

/// Broken-down UTC time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Utc {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millis: u32,
}

/// Seconds and milliseconds since the epoch. A time before the epoch counts as the epoch.
fn since_epoch(at: SystemTime) -> (i64, u32) {
    match at.duration_since(UNIX_EPOCH) {
        Ok(d) => (i64::try_from(d.as_secs()).unwrap_or(i64::MAX / 2), d.subsec_millis()),
        Err(_) => (0, 0),
    }
}

/// Days since 1970-01-01, in UTC.
pub(crate) fn day_number(at: SystemTime) -> i64 {
    since_epoch(at).0.div_euclid(SECS_PER_DAY)
}

/// `(year, month, day)` of a day number (`civil_from_days`, H. Hinnant).
fn civil(day: i64) -> (i64, u32, u32) {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, u32::try_from(m).unwrap_or(1), u32::try_from(d).unwrap_or(1))
}

/// Day number of a date (`days_from_civil`, H. Hinnant).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(month);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub(crate) fn utc(at: SystemTime) -> Utc {
    let (secs, millis) = since_epoch(at);
    let (year, month, day) = civil(secs.div_euclid(SECS_PER_DAY));
    let rest = secs.rem_euclid(SECS_PER_DAY);
    let part = |v: i64| u32::try_from(v).unwrap_or(0);
    Utc {
        year,
        month,
        day,
        hour: part(rest / 3600),
        minute: part(rest % 3600 / 60),
        second: part(rest % 60),
        millis,
    }
}

/// `2026-11-22`.
pub(crate) fn date(day: i64) -> String {
    let (y, m, d) = civil(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Day number of a `YYYY-MM-DD` date; `None` if it is not one, or not a real date.
pub(crate) fn parse_date(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<u32> {
        let part = text.get(range)?;
        if !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let n = days_from_civil(i64::from(year), month, day);
    (date(n) == text).then_some(n)
}

/// `2026-11-22T14:03:07.123Z`.
pub fn rfc3339(at: SystemTime) -> String {
    let t = utc(at);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        t.year, t.month, t.day, t.hour, t.minute, t.second, t.millis
    )
}

/// `20261122-1403`.
pub(crate) fn compact(at: SystemTime) -> String {
    let t = utc(at);
    format!("{:04}{:02}{:02}-{:02}{:02}", t.year, t.month, t.day, t.hour, t.minute)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn dates_round_trip() {
        // 2026-11-22T14:03:07.123Z
        let at = UNIX_EPOCH + Duration::from_millis(1_795_356_187_123);
        assert_eq!(rfc3339(at), "2026-11-22T14:03:07.123Z");
        assert_eq!(compact(at), "20261122-1403");
        assert_eq!(date(day_number(at)), "2026-11-22");
        assert_eq!(parse_date("2026-11-22"), Some(day_number(at)));
        for day in [-1, 0, 59, 365, 10_957, 20_000, 20_514, 100_000] {
            assert_eq!(parse_date(&date(day)), Some(day), "{day}");
        }
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(parse_date("2024-02-29").map(date).as_deref(), Some("2024-02-29"));
        for bad in [
            "2026-02-30",
            "2026-13-01",
            "2026-1-01",
            "20261122",
            "2026-11-2x",
            "+026-11-22",
        ] {
            assert_eq!(parse_date(bad), None, "{bad}");
        }
        assert_eq!(day_number(UNIX_EPOCH - Duration::from_secs(10)), 0);
    }
}
