//! ISO-8601 UTC timestamps in the shape of JavaScript's `toISOString()`
//! (`2026-11-04T12:00:00.000Z`), and reading them back for `--weekly`.

use std::time::{SystemTime, UNIX_EPOCH};

/// Milliseconds since 1970 now.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Format milliseconds since 1970 as `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC).
pub fn iso_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (y, m, day) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        ms.rem_euclid(1000)
    )
}

/// Read `YYYY-MM-DDTHH:MM:SS[.fff]Z` back into milliseconds since 1970.
pub fn parse_iso(s: &str) -> Option<i64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.splitn(3, '-');
    let y: i64 = d.next()?.parse().ok()?;
    let m: u32 = d.next()?.parse().ok()?;
    let day: u32 = d.next()?.parse().ok()?;
    let (hms, frac) = match time.split_once('.') {
        Some((hms, frac)) => (hms, frac),
        None => (time, ""),
    };
    let mut t = hms.splitn(3, ':');
    let h: i64 = t.next()?.parse().ok()?;
    let min: i64 = t.next()?.parse().ok()?;
    let sec: i64 = t.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) || h > 23 || min > 59 || sec > 60 {
        return None;
    }
    let ms: i64 = if frac.is_empty() {
        0
    } else {
        let digits: String = frac.chars().take(3).collect();
        if !frac.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        format!("{digits:0<3}").parse().ok()?
    };
    let days = days_from_civil(y, m, day);
    Some(((days * 86_400 + h * 3600 + min * 60 + sec) * 1000) + ms)
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

/// (year, month, day) → days since 1970-01-01 (`days_from_civil`).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_javascript_to_iso_string() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_utc(1_793_793_600_123), "2026-11-04T12:00:00.123Z");
        assert_eq!(iso_utc(951_868_799_999), "2000-02-29T23:59:59.999Z");
    }

    #[test]
    fn parses_back() {
        for ms in [0, 1_793_793_600_123, 951_868_799_999] {
            assert_eq!(parse_iso(&iso_utc(ms)), Some(ms));
        }
        assert_eq!(parse_iso("2026-11-04T12:00:00Z"), Some(1_793_793_600_000));
        assert_eq!(parse_iso("2026-11-04T12:00:00.5Z"), Some(1_793_793_600_500));
        for bad in [
            "",
            "2026-11-04",
            "2026-13-04T12:00:00Z",
            "2026-11-04T12:00:00",
            "x",
        ] {
            assert_eq!(parse_iso(bad), None, "{bad}");
        }
    }
}
