//! ISO-8601 UTC timestamps in the shape of JavaScript's `toISOString()`
//! (`2026-11-04T12:00:00.000Z`), so the app and the CLI write the same form.

use std::time::{SystemTime, UNIX_EPOCH};

/// The current time.
pub fn now_iso() -> String {
    iso_utc(SystemTime::now())
}

/// Format a time as `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC). Times before 1970
/// (a broken clock) read as 1970-01-01.
pub fn iso_utc(t: SystemTime) -> String {
    let d = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs();
    let (y, m, day) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        d.subsec_millis()
    )
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(ms: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(ms)
    }

    #[test]
    fn matches_javascript_to_iso_string() {
        assert_eq!(iso_utc(at(0)), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_utc(at(1_793_793_600_123)), "2026-11-04T12:00:00.123Z");
        assert_eq!(iso_utc(at(951_868_799_999)), "2000-02-29T23:59:59.999Z");
    }
}
