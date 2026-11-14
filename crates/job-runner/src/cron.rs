//! Five-field cron schedules (`minute hour day-of-month month day-of-week`)
//! in the Mac's local time, with the usual syntax: `*`, `n`, `a-b`, `*/n`,
//! `a-b/n` and comma lists; day-of-week 0–7 (0 and 7 are Sunday). As in cron
//! (and Kubernetes CronJobs), when both day fields are restricted a day
//! matching either one counts.

use std::ffi::{c_char, c_int, c_long};

/// A local wall-clock minute.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Civil {
    pub year: i32,
    /// 1–12.
    pub month: u32,
    /// 1–31.
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    /// 0 = Sunday.
    pub weekday: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    minute: u64,
    hour: u64,
    dom: u64,
    month: u64,
    dow: u64,
    dom_star: bool,
    dow_star: bool,
}

impl Schedule {
    pub fn parse(text: &str) -> Result<Self, String> {
        let fields: Vec<&str> = text.split_whitespace().collect();
        let [minute, hour, dom, month, dow] = fields[..] else {
            return Err(format!("schedule \"{text}\": expected 5 fields"));
        };
        let err = |e: String| format!("schedule \"{text}\": {e}");
        let mut dow_bits = field(dow, 0, 7).map_err(err)?;
        if dow_bits & (1 << 7) != 0 {
            dow_bits = (dow_bits | 1) & !(1 << 7);
        }
        Ok(Schedule {
            minute: field(minute, 0, 59).map_err(err)?,
            hour: field(hour, 0, 23).map_err(err)?,
            dom: field(dom, 1, 31).map_err(err)?,
            month: field(month, 1, 12).map_err(err)?,
            dow: dow_bits,
            dom_star: dom == "*",
            dow_star: dow == "*",
        })
    }

    pub fn matches(&self, c: &Civil) -> bool {
        let has = |bits: u64, v: u32| bits & (1 << v) != 0;
        let dom = has(self.dom, c.day);
        let dow = has(self.dow, c.weekday);
        let day = if self.dom_star || self.dow_star {
            dom && dow
        } else {
            dom || dow
        };
        day && has(self.minute, c.minute) && has(self.hour, c.hour) && has(self.month, c.month)
    }

    /// The latest scheduled minute (Unix seconds) at or before `now`, looking
    /// back at most `window_secs`.
    pub fn prev(&self, now: i64, window_secs: i64, to_civil: impl Fn(i64) -> Civil) -> Option<i64> {
        let start = now - now.rem_euclid(60);
        (0..=window_secs / 60)
            .map(|i| start - i * 60)
            .find(|&t| self.matches(&to_civil(t)))
    }

    /// The first scheduled minute after `now`, within 400 days.
    pub fn next(&self, now: i64, to_civil: impl Fn(i64) -> Civil) -> Option<i64> {
        let start = now - now.rem_euclid(60) + 60;
        (0..400 * 1440)
            .map(|i| start + i * 60)
            .find(|&t| self.matches(&to_civil(t)))
    }
}

/// One field as a bit set of the values it allows.
fn field(spec: &str, min: u32, max: u32) -> Result<u64, String> {
    let mut bits = 0u64;
    for part in spec.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, num(s)?),
            None => (part, 1),
        };
        if step == 0 {
            return Err(format!("step 0 in \"{part}\""));
        }
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (num(a)?, num(b)?)
        } else {
            let v = num(range)?;
            (v, if part.contains('/') { max } else { v })
        };
        if lo < min || hi > max || lo > hi {
            return Err(format!("\"{part}\" outside {min}-{max}"));
        }
        let mut v = lo;
        while v <= hi {
            bits |= 1 << v;
            v += step;
        }
    }
    Ok(bits)
}

fn num(s: &str) -> Result<u32, String> {
    s.parse().map_err(|_| format!("\"{s}\" isn't a number"))
}

#[repr(C)]
struct Tm {
    tm_sec: c_int,
    tm_min: c_int,
    tm_hour: c_int,
    tm_mday: c_int,
    tm_mon: c_int,
    tm_year: c_int,
    tm_wday: c_int,
    tm_yday: c_int,
    tm_isdst: c_int,
    tm_gmtoff: c_long,
    tm_zone: *const c_char,
}

unsafe extern "C" {
    fn localtime_r(t: *const i64, out: *mut Tm) -> *mut Tm;
}

/// Unix seconds → the Mac's local wall clock (its time zone, daylight saving
/// included).
pub fn local(t: i64) -> Civil {
    let mut tm = Tm {
        tm_sec: 0,
        tm_min: 0,
        tm_hour: 0,
        tm_mday: 1,
        tm_mon: 0,
        tm_year: 70,
        tm_wday: 4,
        tm_yday: 0,
        tm_isdst: 0,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null(),
    };
    // SAFETY: both pointers are valid for the call; localtime_r only writes `tm`.
    unsafe { localtime_r(&t, &mut tm) };
    Civil {
        year: tm.tm_year + 1900,
        month: (tm.tm_mon + 1) as u32,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
        weekday: tm.tm_wday as u32,
    }
}

/// `YYYY-MM-DD HH:MM`, local time.
pub fn fmt_local(t: i64) -> String {
    let c = local(t);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        c.year, c.month, c.day, c.hour, c.minute
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A UTC clock, so tests don't depend on the machine's zone.
    fn utc(t: i64) -> Civil {
        let days = t.div_euclid(86_400);
        let rem = t.rem_euclid(86_400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        Civil {
            year: (yoe + era * 400 + i64::from(month <= 2)) as i32,
            month,
            day,
            hour: (rem / 3600) as u32,
            minute: (rem % 3600 / 60) as u32,
            weekday: (days + 4).rem_euclid(7) as u32,
        }
    }

    /// 2026-11-14 00:00 UTC (a Saturday).
    const NOV14: i64 = 1_794_614_400;

    #[test]
    fn daily_at_a_time() {
        let s = Schedule::parse("5 0 * * *").unwrap();
        assert_eq!(
            s.prev(NOV14 + 4 * 60, 86_400, utc),
            Some(NOV14 - 86_400 + 5 * 60)
        );
        assert_eq!(
            s.prev(NOV14 + 5 * 60 + 59, 86_400, utc),
            Some(NOV14 + 5 * 60)
        );
        assert_eq!(s.next(NOV14 + 5 * 60, utc), Some(NOV14 + 86_400 + 5 * 60));
    }

    #[test]
    fn a_missed_run_is_found_only_inside_the_window() {
        let s = Schedule::parse("0 19 * * *").unwrap();
        // 20:00 the next day: yesterday's 19:00 is 25 h back.
        let now = NOV14 + 86_400 + 18 * 3600;
        assert_eq!(s.prev(now, 20 * 3600, utc), None);
        assert_eq!(s.prev(now, 26 * 3600, utc), Some(NOV14 + 19 * 3600));
    }

    #[test]
    fn every_third_day_of_the_month() {
        let s = Schedule::parse("0 3 */3 * *").unwrap();
        let days: Vec<u32> = (1..=31).filter(|&d| s.dom & (1 << d) != 0).collect();
        assert_eq!(days, [1, 4, 7, 10, 13, 16, 19, 22, 25, 28, 31]);
        // Saturday the 14th: the last run was the 13th at 03:00.
        assert_eq!(
            s.prev(NOV14 + 12 * 3600, 48 * 3600, utc),
            Some(NOV14 - 86_400 + 3 * 3600)
        );
    }

    #[test]
    fn both_day_fields_restricted_means_either() {
        // The 1st of the month or any Monday, at noon.
        let s = Schedule::parse("0 12 1 * 1").unwrap();
        let at = |day, weekday| Civil {
            year: 2026,
            month: 11,
            day,
            hour: 12,
            minute: 0,
            weekday,
        };
        assert!(s.matches(&at(1, 0)));
        assert!(s.matches(&at(16, 1)));
        assert!(!s.matches(&at(17, 2)));
        // One restricted, the other `*`: only the restricted one counts.
        let mondays = Schedule::parse("0 12 * * 1").unwrap();
        assert!(!mondays.matches(&at(1, 0)));
        assert!(mondays.matches(&at(16, 1)));
    }

    #[test]
    fn lists_ranges_steps_and_sunday_as_7() {
        let s = Schedule::parse("0,30 9-17/4 * * 7").unwrap();
        let at = |hour, minute, weekday| Civil {
            year: 2026,
            month: 11,
            day: 15,
            hour,
            minute,
            weekday,
        };
        assert!(s.matches(&at(9, 30, 0)));
        assert!(s.matches(&at(13, 0, 0)));
        assert!(s.matches(&at(17, 0, 0)));
        assert!(!s.matches(&at(11, 0, 0)));
        assert!(!s.matches(&at(9, 15, 0)));
        assert!(!s.matches(&at(9, 0, 6)));
    }

    #[test]
    fn rejects_bad_schedules() {
        for bad in [
            "* * * *",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "*/0 * * * *",
            "5-1 * * * *",
            "x * * * *",
        ] {
            assert!(Schedule::parse(bad).is_err(), "{bad}");
        }
    }
}
