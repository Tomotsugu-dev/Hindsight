//! Time helpers: parse an activity's start and end times, split a time range at clock hours, and
//! work out the start and end dates of a day, week or month.

use chrono::{
    DateTime, Datelike, Duration, FixedOffset, Local, Months, NaiveDate, NaiveTime, TimeZone,
    Timelike,
};

/// Parses a stored time string (RFC 3339), keeping the UTC offset it was written with. Logs an
/// error and returns None if the format is wrong.
pub(super) fn parse_stored_time(s: &str) -> Option<DateTime<FixedOffset>> {
    match DateTime::parse_from_rfc3339(s) {
        Ok(dt) => Some(dt),
        Err(e) => {
            log::error!("activity time is not RFC 3339: {s:?}: {e}");
            None
        }
    }
}

/// Converts a stored time string (RFC 3339) to this machine's time zone. Logs an error and returns
/// None if the format is wrong.
pub(super) fn parse_time_in_local(s: &str) -> Option<DateTime<Local>> {
    parse_stored_time(s).map(|dt| dt.with_timezone(&Local))
}

/// Splits an activity interval at midnight and returns the seconds assigned to each date.
///
/// Uses `start`'s UTC offset as the local time zone for the whole interval, converting `end` to
/// the same offset before splitting. This keeps the dates consistent with the stored `local_date`
/// and avoids daylight saving time changes affecting the split.
pub(super) fn split_by_date(
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
) -> Vec<(NaiveDate, u64)> {
    let mut out = Vec::new();
    let mut cur = start.naive_local();
    let end = end.with_timezone(&start.timezone()).naive_local();
    // Loop over each day, splitting at midnight.
    // For example, a time range from 10-2 23:59:50 to 10-4 00:00:20 will be split into
    // three chunks: (10-2, 10) and (10-3, 86400) and (10-4, 20).
    while cur < end {
        let date = cur.date();
        let next_midnight = (date + Duration::days(1)).and_time(NaiveTime::MIN);
        let chunk_end = next_midnight.min(end);
        out.push((date, (chunk_end - cur).num_seconds() as u64));
        cur = chunk_end;
    }
    out
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

    fn at(value: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(value).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// 同一天内不切；跨一次午夜切成两段；刚好在午夜结束的，次日没有时间。
    #[test]
    fn split_by_date_cuts_at_midnight() {
        let d1 = date(2026, 10, 1);
        let d2 = date(2026, 10, 2);
        assert_eq!(
            split_by_date(
                at("2026-10-01T10:00:00+09:00"),
                at("2026-10-01T10:30:00+09:00")
            ),
            vec![(d1, 1800)]
        );
        assert_eq!(
            split_by_date(
                at("2026-10-01T23:50:00+09:00"),
                at("2026-10-02T00:10:00+09:00")
            ),
            vec![(d1, 600), (d2, 600)]
        );
        assert_eq!(
            split_by_date(
                at("2026-10-01T23:50:00+09:00"),
                at("2026-10-02T00:00:00+09:00")
            ),
            vec![(d1, 600)]
        );
    }

    /// 跨三个日期时，中间那天是整整 86400 秒；空区间和倒挂的区间没有任何段。
    #[test]
    fn split_by_date_covers_every_date_in_between() {
        assert_eq!(
            split_by_date(
                at("2026-10-01T23:00:00+09:00"),
                at("2026-10-03T01:00:00+09:00")
            ),
            vec![
                (date(2026, 10, 1), 3600),
                (date(2026, 10, 2), 86400),
                (date(2026, 10, 3), 3600),
            ]
        );
        let t = at("2026-10-01T23:00:00+09:00");
        assert!(split_by_date(t, t).is_empty());
        assert!(split_by_date(t, at("2026-10-01T22:00:00+09:00")).is_empty());
    }

    /// 日期按开始时刻的偏移算：结束时刻用别的偏移写成，也切在开始偏移下的午夜。
    /// 美国夏令时开始那晚，03-07 23:30（-05:00）到 03-08 03:30（-04:00）实际只过了 3 小时，
    /// 其中 03-07 占 30 分钟。
    #[test]
    fn split_by_date_uses_the_offset_of_the_start() {
        assert_eq!(
            split_by_date(
                at("2026-10-01T23:50:00+09:00"),
                at("2026-10-01T15:10:00+00:00")
            ),
            vec![(date(2026, 10, 1), 600), (date(2026, 10, 2), 600)]
        );
        assert_eq!(
            split_by_date(
                at("2026-03-07T23:30:00-05:00"),
                at("2026-03-08T03:30:00-04:00")
            ),
            vec![(date(2026, 3, 7), 1800), (date(2026, 3, 8), 9000)]
        );
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
