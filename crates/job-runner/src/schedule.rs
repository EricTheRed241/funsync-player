//! A job's `schedule` (jobs.json): five cron fields, minute hour
//! day-of-month month day-of-week, in the Mac's local time. Each field is
//! `*`, a number, a range `a-b`, a list `a,b`, or a step `*/n`, `a-b/n`
//! (`a/n` = `a-<max>/n`). Day of week 0–6 from Sunday, 7 is Sunday too. As in
//! cron, when both day fields are restricted (neither starts with `*`), a day
//! matching either is enough.

use crate::localtime::{self, Civil};

#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    text: String,
    /// Bit n set = value n matches.
    minute: u64,
    hour: u64,
    day: u64,
    month: u64,
    weekday: u64,
    /// The day-of-month / day-of-week field starts with `*`.
    any_day: bool,
    any_weekday: bool,
}

/// How far [`Schedule::next_after`] looks: four years and a day, so 29
/// February schedules are found.
const HORIZON: i64 = (4 * 365 + 2) * 86_400;

impl Schedule {
    pub fn parse(text: &str) -> Result<Schedule, String> {
        let err = |e: String| format!("schedule \"{text}\": {e}");
        let fields: Vec<&str> = text.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields[..] else {
            return Err(err(format!(
                "needs 5 fields (minute hour day-of-month month day-of-week), has {}",
                fields.len()
            )));
        };
        let mut weekdays = field(weekday, "day-of-week", 0, 7).map_err(err)?;
        if weekdays & 1 << 7 != 0 {
            weekdays = (weekdays | 1) & !(1 << 7);
        }
        Ok(Schedule {
            text: text.to_string(),
            minute: field(minute, "minute", 0, 59).map_err(err)?,
            hour: field(hour, "hour", 0, 23).map_err(err)?,
            day: field(day, "day-of-month", 1, 31).map_err(err)?,
            month: field(month, "month", 1, 12).map_err(err)?,
            weekday: weekdays,
            any_day: day.starts_with('*'),
            any_weekday: weekday.starts_with('*'),
        })
    }

    /// The string it was parsed from.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether the minute `c` falls in matches (its seconds are ignored).
    pub fn matches(&self, c: &Civil) -> bool {
        self.day_matches(c) && has(self.hour, c.hour) && has(self.minute, c.minute)
    }

    fn day_matches(&self, c: &Civil) -> bool {
        let day = has(self.day, c.day);
        let weekday = has(self.weekday, c.weekday);
        has(self.month, c.month)
            && if self.any_day || self.any_weekday {
                day && weekday
            } else {
                day || weekday
            }
    }

    /// The first matching local minute strictly after unix second `t` (its
    /// start), within four years.
    pub fn next_after(&self, t: i64) -> Option<i64> {
        let mut t = t.div_euclid(60) * 60 + 60;
        let end = t + HORIZON;
        while t <= end {
            let c = localtime::local(t);
            let next = if !self.day_matches(&c) {
                // The next day's midnight.
                localtime::to_unix(&Civil {
                    day: c.day + 1,
                    hour: 0,
                    minute: 0,
                    second: 0,
                    ..c
                })
            } else if !has(self.hour, c.hour) {
                // The next hour's start.
                t - i64::from(c.minute * 60 + c.second) + 3600
            } else if has(self.minute, c.minute) {
                return Some(t);
            } else {
                t + 60
            };
            t = next.max(t + 60);
        }
        None
    }

    /// The latest matching local minute start at or before `now`, no earlier
    /// than `now - lookback`.
    pub fn last_due(&self, now: i64, lookback: i64) -> Option<i64> {
        let start = now - lookback;
        let mut t = now.div_euclid(60) * 60;
        while t >= start {
            let c = localtime::local(t);
            let next = if !self.day_matches(&c) {
                // The day before's last minute.
                localtime::to_unix(&Civil {
                    hour: 0,
                    minute: 0,
                    second: 0,
                    ..c
                }) - 60
            } else if !has(self.hour, c.hour) {
                // The hour before's last minute.
                t - i64::from(c.minute * 60 + c.second) - 60
            } else if has(self.minute, c.minute) {
                return Some(t);
            } else {
                t - 60
            };
            t = next.min(t - 60);
        }
        None
    }
}

fn has(bits: u64, value: u32) -> bool {
    value < 64 && bits & 1 << value != 0
}

/// One field → its values as bits.
fn field(text: &str, name: &str, min: u32, max: u32) -> Result<u64, String> {
    let mut bits = 0u64;
    for item in text.split(',') {
        let bad = |why: String| format!("{name} \"{text}\": {why}");
        let number = |s: &str| -> Result<u32, String> {
            let n: u32 = s
                .parse()
                .map_err(|_| bad(format!("\"{s}\" isn't a number")))?;
            if n < min || n > max {
                return Err(bad(format!("{n} is outside {min}-{max}")));
            }
            Ok(n)
        };
        let (range, step) = match item.split_once('/') {
            Some((range, step)) => {
                let n: u32 = step
                    .parse()
                    .map_err(|_| bad(format!("step \"{step}\" isn't a number")))?;
                if n == 0 {
                    return Err(bad("a step of 0".into()));
                }
                (range, Some(n))
            }
            None => (item, None),
        };
        let (from, to) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            let (a, b) = (number(a)?, number(b)?);
            if a > b {
                return Err(bad(format!("range {a}-{b} runs backwards")));
            }
            (a, b)
        } else {
            let a = number(range)?;
            (a, if step.is_some() { max } else { a })
        };
        for v in (from..=to).step_by(step.unwrap_or(1) as usize) {
            bits |= 1 << v;
        }
    }
    Ok(bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32, weekday: u32) -> Civil {
        Civil {
            year,
            month,
            day,
            hour,
            minute,
            second: 0,
            weekday,
        }
    }

    /// Unix seconds of a local wall-clock minute.
    fn unix(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
        localtime::to_unix(&at(year, month, day, hour, minute, 0))
    }

    fn s(text: &str) -> Schedule {
        Schedule::parse(text).unwrap()
    }

    #[test]
    fn hourly_matches_its_minute_every_hour() {
        let hourly = s("0 * * * *");
        assert_eq!(hourly.text(), "0 * * * *");
        for hour in [0, 13, 23] {
            assert!(hourly.matches(&at(2026, 11, 23, hour, 0, 1)));
            assert!(!hourly.matches(&at(2026, 11, 23, hour, 1, 1)));
        }
        let quarter = s("15 * * * *");
        assert!(quarter.matches(&at(2026, 2, 28, 7, 15, 6)));
        assert!(!quarter.matches(&at(2026, 2, 28, 7, 0, 6)));
    }

    #[test]
    fn weekly_matches_one_weekday() {
        let weekly = s("0 4 * * 1");
        assert!(weekly.matches(&at(2026, 11, 23, 4, 0, 1)));
        assert!(!weekly.matches(&at(2026, 11, 24, 4, 0, 2)));
        assert!(!weekly.matches(&at(2026, 11, 23, 4, 1, 1)));
        assert!(!weekly.matches(&at(2026, 11, 23, 5, 0, 1)));
        // 7 is Sunday too.
        assert!(s("0 4 * * 7").matches(&at(2026, 11, 22, 4, 0, 0)));
        assert!(s("0 4 * * 0").matches(&at(2026, 11, 22, 4, 0, 0)));
        assert!(!s("0 4 * * 7").matches(&at(2026, 11, 28, 4, 0, 6)));
    }

    #[test]
    fn ranges_lists_and_steps() {
        let every_15 = s("*/15 * * * *");
        let minutes: Vec<u32> = (0..60)
            .filter(|&m| every_15.matches(&at(2026, 1, 1, 0, m, 4)))
            .collect();
        assert_eq!(minutes, [0, 15, 30, 45]);
        let office = s("5,35 9-17/4 * 1-3,12 1-5");
        let hours: Vec<u32> = (0..24)
            .filter(|&h| office.matches(&at(2026, 12, 1, h, 35, 2)))
            .collect();
        assert_eq!(hours, [9, 13, 17]);
        assert!(!office.matches(&at(2026, 12, 1, 9, 6, 2)));
        assert!(!office.matches(&at(2026, 6, 2, 9, 5, 2)), "June");
        assert!(!office.matches(&at(2026, 12, 6, 9, 5, 0)), "Sunday");
        // `a/n`: from a to the end.
        let from_50 = s("50/5 * * * *");
        let minutes: Vec<u32> = (0..60)
            .filter(|&m| from_50.matches(&at(2026, 1, 1, 0, m, 4)))
            .collect();
        assert_eq!(minutes, [50, 55]);
    }

    #[test]
    fn restricted_day_fields_match_either() {
        // Both restricted: the 13th or a Friday.
        let either = s("0 0 13 * 5");
        assert!(either.matches(&at(2026, 5, 13, 0, 0, 3)));
        assert!(either.matches(&at(2026, 5, 15, 0, 0, 5)));
        assert!(!either.matches(&at(2026, 5, 14, 0, 0, 4)));
        // One starts with `*`: both must match.
        let both = s("0 0 */2 * 5");
        assert!(both.matches(&at(2026, 5, 15, 0, 0, 5)));
        assert!(!both.matches(&at(2026, 5, 22, 0, 0, 5)));
        assert!(!both.matches(&at(2026, 5, 13, 0, 0, 3)));
    }

    #[test]
    fn bad_schedules_are_rejected_naming_the_field() {
        for (bad, names) in [
            ("", "5 fields"),
            ("* * * *", "5 fields"),
            ("* * * * * *", "5 fields"),
            ("60 * * * *", "minute"),
            ("-1 * * * *", "minute"),
            ("1,,2 * * * *", "minute"),
            ("*/0 * * * *", "minute"),
            ("a * * * *", "minute"),
            ("5-1 * * * *", "minute"),
            ("* 24 * * *", "hour"),
            ("* * 0 * *", "day-of-month"),
            ("* * 32 * *", "day-of-month"),
            ("* * * 13 *", "month"),
            ("* * * 0 *", "month"),
            ("* * * * 8", "day-of-week"),
            ("* * * * mon", "day-of-week"),
            ("* * * * 1/x", "day-of-week"),
        ] {
            let e = Schedule::parse(bad).unwrap_err();
            assert!(e.contains(names), "{bad}: {e}");
        }
    }

    /// `next_after(t)` is a matching minute start after `t`, and no minute
    /// between them matches.
    fn check_next(schedule: &Schedule, t: i64) -> i64 {
        let next = schedule.next_after(t).unwrap();
        assert!(next > t && next % 60 == 0, "{t} → {next}");
        assert!(schedule.matches(&localtime::local(next)));
        let mut m = t.div_euclid(60) * 60 + 60;
        while m < next {
            assert!(
                !schedule.matches(&localtime::local(m)),
                "{} matches before {}",
                localtime::fmt_local(m),
                localtime::fmt_local(next)
            );
            m += 60;
        }
        next
    }

    /// `last_due(now)` is a matching minute start at or before `now`, and no
    /// minute between them matches.
    fn check_due(schedule: &Schedule, now: i64) -> i64 {
        let due = schedule.last_due(now, 8 * 86_400).unwrap();
        assert!(due <= now && now - due < 8 * 86_400 && due % 60 == 0);
        assert!(schedule.matches(&localtime::local(due)));
        let mut m = due + 60;
        while m <= now {
            assert!(!schedule.matches(&localtime::local(m)));
            m += 60;
        }
        due
    }

    #[test]
    fn next_after_crosses_hours_and_days() {
        let hourly = s("0 * * * *");
        assert_eq!(
            check_next(&hourly, unix(2026, 11, 23, 13, 59) + 30),
            unix(2026, 11, 23, 14, 0)
        );
        // Strictly after: on the minute itself, the next hour.
        assert_eq!(
            check_next(&hourly, unix(2026, 11, 23, 14, 0)),
            unix(2026, 11, 23, 15, 0)
        );
        assert_eq!(
            check_next(&hourly, unix(2026, 11, 23, 23, 30)),
            unix(2026, 11, 24, 0, 0)
        );
        assert_eq!(
            check_next(&s("15 * * * *"), unix(2026, 11, 23, 14, 15) + 59),
            unix(2026, 11, 23, 15, 15)
        );
        // 2026-11-23 is a Monday.
        assert_eq!(localtime::local(unix(2026, 11, 23, 12, 0)).weekday, 1);
        let weekly = s("0 4 * * 1");
        assert_eq!(
            check_next(&weekly, unix(2026, 11, 23, 3, 59)),
            unix(2026, 11, 23, 4, 0)
        );
        assert_eq!(
            check_next(&weekly, unix(2026, 11, 23, 4, 0)),
            unix(2026, 11, 30, 4, 0)
        );
        assert_eq!(
            check_next(&s("30 23 31 12 *"), unix(2026, 11, 23, 12, 0)),
            unix(2026, 12, 31, 23, 30)
        );
        for t in [0, 1_700_000_000, 1_774_000_123, 1_792_000_000] {
            check_next(&hourly, t);
            check_next(&s("*/7 9-17 * * 1-5"), t);
            check_next(&weekly, t);
        }
        assert_eq!(s("0 0 30 2 *").next_after(1_700_000_000), None);
    }

    #[test]
    fn last_due_crosses_hours_and_days() {
        let hourly = s("0 * * * *");
        assert_eq!(
            check_due(&hourly, unix(2026, 11, 23, 14, 0) + 59),
            unix(2026, 11, 23, 14, 0)
        );
        assert_eq!(
            check_due(&hourly, unix(2026, 11, 23, 13, 59) + 59),
            unix(2026, 11, 23, 13, 0)
        );
        assert_eq!(
            check_due(&hourly, unix(2026, 11, 24, 0, 30)),
            unix(2026, 11, 24, 0, 0)
        );
        assert_eq!(
            check_due(&s("45 23 * * *"), unix(2026, 11, 24, 0, 30)),
            unix(2026, 11, 23, 23, 45)
        );
        let weekly = s("0 4 * * 1");
        assert_eq!(
            check_due(&weekly, unix(2026, 11, 23, 4, 0)),
            unix(2026, 11, 23, 4, 0)
        );
        assert_eq!(
            check_due(&weekly, unix(2026, 11, 23, 3, 59)),
            unix(2026, 11, 16, 4, 0)
        );
        // Outside the lookback.
        assert_eq!(weekly.last_due(unix(2026, 11, 23, 3, 59), 6 * 86_400), None);
        for t in [1_700_000_000, 1_774_000_123, 1_792_000_000] {
            check_due(&hourly, t);
            check_due(&s("*/7 9-17 * * 1-5"), t);
            check_due(&weekly, t);
        }
    }
}
