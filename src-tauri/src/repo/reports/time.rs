//! Time helpers: parse an activity's start and end times, split a time range at clock hours, and
//! work out the start and end dates of a day, week or month.

use chrono::{DateTime, Datelike, Duration, Local, Months, NaiveDate, TimeZone, Timelike};

/// Converts a stored time string (RFC 3339) to this machine's time zone. Logs an error and returns
/// None if the format is wrong.
pub(super) fn parse_local(s: &str) -> Option<DateTime<Local>> {
    match DateTime::parse_from_rfc3339(s) {
        Ok(dt) => Some(dt.with_timezone(&Local)),
        Err(e) => {
            log::error!("activity time is not RFC 3339: {s:?}: {e}");
            None
        }
    }
}

/// Splits a time range at local clock hours and returns `(hour, seconds)` for each piece, for the
/// hourly reports to add up.
pub(super) fn slice_by_hour(start: DateTime<Local>, end: DateTime<Local>) -> Vec<(u8, u64)> {
    let mut out = Vec::new();
    let mut cur = start;
    while cur < end {
        let hour = cur.hour() as u8;
        // Where daylight saving time is used, clocks go back 1 hour in autumn, so the same hour
        // happens twice (1:00–1:59 runs twice). Take the later one: adding 1 hour then always lands
        // after `cur`, so the loop moves forward. Both runs show the same hour on the clock, so
        // both count toward it.
        let next_hour = Local
            .with_ymd_and_hms(cur.year(), cur.month(), cur.day(), cur.hour(), 0, 0)
            .latest()
            .map(|t| t + Duration::hours(1))
            .unwrap_or(end);
        let chunk_end = if next_hour < end { next_hour } else { end };
        let secs = (chunk_end - cur).num_seconds() as u64;
        if secs > 0 {
            out.push((hour, secs));
        }
        cur = chunk_end;
    }
    out
}

/// The date of one day. Adds calendar days instead of moving in 24-hour steps: a day with a clock
/// change has 23 or 25 hours.
/// - `today`: today's date.
/// - `day_offset`: days from today; 0 is today, -1 is yesterday.
pub fn day_date(today: NaiveDate, day_offset: i32) -> NaiveDate {
    today + Duration::days(day_offset as i64)
}

/// The start and end dates of one week: Monday to Sunday.
/// - `today`: today's date.
/// - `week_offset`: weeks from this week; 0 is this week, -1 is last week.
pub fn week_range(today: NaiveDate, week_offset: i32) -> (NaiveDate, NaiveDate) {
    let dow = today.weekday().num_days_from_monday() as i64;
    let monday = today - Duration::days(dow) + Duration::days(week_offset as i64 * 7);
    let sunday = monday + Duration::days(6);
    (monday, sunday)
}

/// The start and end dates of one month: the 1st to the last day.
/// - `today`: today's date.
/// - `month_offset`: months from this month; 0 is this month, -1 is last month.
pub fn month_range(today: NaiveDate, month_offset: i32) -> (NaiveDate, NaiveDate) {
    let this_month = today.with_day(1).expect("every month has a day 1");
    let months = Months::new(month_offset.unsigned_abs());
    // Panics only beyond the dates chrono can represent (about ±260,000 years); an offset from a
    // page never gets there.
    let from = if month_offset < 0 {
        this_month - months
    } else {
        this_month + months
    };
    let to = from + Months::new(1) - Duration::days(1);
    (from, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// 起止是那个月的 1 号到月底：本月、跨年往前、二月（含闰年）、往前 13 个月、往后跨年。
    #[test]
    fn month_range_covers_whole_months() {
        let cases = [
            // (今天, 偏移, 起, 止)
            (date(2026, 10, 2), 0, date(2026, 10, 1), date(2026, 10, 31)),
            (date(2026, 1, 15), -1, date(2025, 12, 1), date(2025, 12, 31)),
            (date(2026, 3, 31), -1, date(2026, 2, 1), date(2026, 2, 28)),
            (date(2024, 3, 10), -1, date(2024, 2, 1), date(2024, 2, 29)),
            (
                date(2026, 1, 15),
                -13,
                date(2024, 12, 1),
                date(2024, 12, 31),
            ),
            (date(2026, 12, 5), 1, date(2027, 1, 1), date(2027, 1, 31)),
        ];
        for (today, offset, from, to) in cases {
            assert_eq!(
                month_range(today, offset),
                (from, to),
                "{today} 偏移 {offset}"
            );
        }
    }
}
