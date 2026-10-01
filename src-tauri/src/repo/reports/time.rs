//! Time helpers: parse an activity's start and end times, and split a session into clock hours.

use chrono::{DateTime, Datelike, Duration, Local, TimeZone, Timelike};

/// Converts a stored time string (RFC 3339) to this machine's time zone. Returns the current time
/// if parsing fails.
// TODO: On a parse failure, log an error and return None; callers skip the row. Returning the
// current time makes the row vanish (bad start) or stretch to now (bad end). Bad strings can only
// come from sync: `pull.rs` stores the cloud times as they are, without checking the format.
// First write a failing test: when the end time is a bad string, `day_hours` must not count it.
pub(super) fn parse_local(s: &str) -> DateTime<Local> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Local))
        .unwrap_or_else(|_| Local::now())
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
