//! The Mac's local wall clock, for log lines, log file names, `status` and
//! schedules.

use std::ffi::{c_char, c_int, c_long};

/// A local wall-clock second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Civil {
    pub year: i32,
    /// 1–12.
    pub month: u32,
    /// 1–31.
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    /// 0 = Sunday … 6 = Saturday. Ignored by [`to_unix`].
    pub weekday: u32,
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
    fn mktime(tm: *mut Tm) -> i64;
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
        second: tm.tm_sec as u32,
        weekday: tm.tm_wday as u32,
    }
}

/// Local wall clock → unix seconds (`mktime`, daylight saving decided by the
/// zone). Out-of-range fields roll over (day 32 is the next month's 1st); a
/// time skipped by a clock change lands past the gap.
pub fn to_unix(c: &Civil) -> i64 {
    let mut tm = Tm {
        tm_sec: c.second as c_int,
        tm_min: c.minute as c_int,
        tm_hour: c.hour as c_int,
        tm_mday: c.day as c_int,
        tm_mon: c.month as c_int - 1,
        tm_year: c.year - 1900,
        tm_wday: 0,
        tm_yday: 0,
        tm_isdst: -1,
        tm_gmtoff: 0,
        tm_zone: std::ptr::null(),
    };
    // SAFETY: the pointer is valid for the call; mktime only reads and
    // normalises `tm`.
    unsafe { mktime(&mut tm) }
}

/// `YYYY-MM-DD HH:MM`, local time.
pub fn fmt_local(t: i64) -> String {
    let c = local(t);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        c.year, c.month, c.day, c.hour, c.minute
    )
}

/// `HH:MM`, local time.
pub fn fmt_time(t: i64) -> String {
    let c = local(t);
    format!("{:02}:{:02}", c.hour, c.minute)
}

/// A coming time, as short as `now` allows: `HH:MM` the same day, `Mon HH:MM`
/// within a week, else `YYYY-MM-DD HH:MM`.
pub fn fmt_upcoming(t: i64, now: i64) -> String {
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let (c, today) = (local(t), local(now));
    if (c.year, c.month, c.day) == (today.year, today.month, today.day) {
        fmt_time(t)
    } else if t > now && t - now < 6 * 86_400 {
        format!("{} {}", DAYS[c.weekday as usize % 7], fmt_time(t))
    } else {
        fmt_local(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_unix_inverts_local() {
        for t in [0, 1_700_000_040, 1_780_000_020, 1_798_761_600] {
            let c = local(t);
            assert_eq!(to_unix(&c), t, "{c:?}");
        }
    }

    #[test]
    fn to_unix_rolls_days_over() {
        let t = 1_700_000_040;
        let c = local(t);
        let next = local(to_unix(&Civil {
            day: c.day + 40,
            ..c
        }));
        assert_eq!((next.hour, next.minute), (c.hour, c.minute));
        assert_eq!(next.weekday, (c.weekday + 40) % 7);
    }

    #[test]
    fn upcoming_times_get_shorter_the_closer_they_are() {
        let now = 1_700_000_040;
        let c = local(now);
        let same_day = to_unix(&Civil {
            hour: 23,
            minute: 59,
            ..c
        });
        assert_eq!(fmt_upcoming(same_day, now), "23:59");
        let in_three_days = to_unix(&Civil {
            day: c.day + 3,
            hour: 4,
            minute: 0,
            second: 0,
            ..c
        });
        assert!(fmt_upcoming(in_three_days, now).ends_with(" 04:00"));
        assert_eq!(fmt_upcoming(in_three_days, now).len(), 9);
        assert_eq!(fmt_upcoming(now + 30 * 86_400, now).len(), 16);
    }
}
