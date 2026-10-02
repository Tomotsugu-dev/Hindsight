//! Time helpers: parse an activity's start and end times, split a time range at clock hours, and
//! work out the start and end dates of a day, week or month.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Timelike};

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
        // TODO: Remove `.max(0)`. The loop guarantees `chunk_end > cur`, so the difference is never
        // negative; a test (10:30→11:30 in `app_detail.rs`) covers splitting across hours.
        let secs = (chunk_end - cur).num_seconds().max(0) as u64;
        if secs > 0 {
            out.push((hour, secs));
        }
        cur = chunk_end;
    }
    out
}

/// The date of one day. `day_offset = 0` is today, -1 is yesterday.
// TODO: Count calendar days instead: `Local::now().date_naive() + Duration::days(..)`. This moves
// n × 24 hours from now, so where daylight saving time is used, clicking "yesterday" at 23:30 on
// the day the clocks go back still gives today. Two places in `by_hour.rs` and one in
// `app_detail.rs` use the same expression; change them to call this function. This changes
// behavior, so do it in its own commit, test first.
pub fn day_date(day_offset: i32) -> NaiveDate {
    (Local::now() + Duration::days(day_offset as i64)).date_naive()
}

// TODO: Add a doc comment that says what it does, and rename the function.
pub fn week_range(week_offset: i32) -> (NaiveDate, NaiveDate) {
    let today = Local::now().date_naive();
    let dow = today.weekday().num_days_from_monday() as i64;
    let monday = today - Duration::days(dow) + Duration::days(week_offset as i64 * 7);
    let sunday = monday + Duration::days(6);
    (monday, sunday)
}

// TODO: Add a doc comment that says what it does.
pub fn month_range(month_offset: i32) -> (NaiveDate, NaiveDate) {
    let today = Local::now().date_naive();
    let mut year = today.year();
    let mut month = today.month() as i32 + month_offset;
    // TODO: There should already be a mature way to add a month offset to a date.
    while month <= 0 {
        month += 12;
        year -= 1;
    }
    while month > 12 {
        month -= 12;
        year += 1;
    }
    // TODO: Check whether panicking here is reasonable.
    let first = NaiveDate::from_ymd_opt(year, month as u32, 1)
        .expect("month_range: year/month must be within chrono's valid range");
    // TODO: Could be clearer: finish normalizing the year and month first (a modulo would do),
    // then write the if/else.
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1).expect("month_range: rolls over to January")
    } else {
        NaiveDate::from_ymd_opt(year, (month + 1) as u32, 1)
            .expect("month_range: month + 1 must be within 1..=12")
    };
    let last = next - Duration::days(1);
    // TODO: Name these `from` and `to`, as elsewhere in this module.
    (first, last)
}
