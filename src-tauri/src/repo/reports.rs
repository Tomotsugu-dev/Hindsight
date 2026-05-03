use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Timelike};
use serde::Serialize;

use crate::error::Result;
use crate::storage::DbPool;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourSegment {
    pub category_id: String,
    pub minutes: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourSlot {
    pub hour: u8,
    pub segments: Vec<HourSegment>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaySummary {
    pub date: String,
    pub segments: Vec<HourSegment>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUsage {
    pub process: String,
    pub category_id: String,
    pub minutes: u32,
}

pub async fn day_hours(pool: &DbPool, day_offset: i32) -> Result<Vec<HourSlot>> {
    let target = Local::now() + Duration::days(day_offset as i64);
    let date = target.format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String, String)> = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT a.started_at, a.ended_at,
                            COALESCE(m.category_id, 'other') AS cat
                     FROM activities a
                     LEFT JOIN app_categories m ON m.process_name = a.process_name
                     WHERE a.local_date = ?",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let it = stmt
                .query_map([&date], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.map_err(tokio_rusqlite::Error::Rusqlite)?);
            }
            Ok(out)
        })
        .await?;

    let mut buckets: [std::collections::HashMap<String, u64>; 24] =
        std::array::from_fn(|_| std::collections::HashMap::new());

    for (started, ended, cat) in rows {
        let s = parse_local(&started);
        let e = parse_local(&ended);
        if e <= s {
            continue;
        }
        for (hour, secs) in slice_by_hour(s, e) {
            *buckets[hour as usize].entry(cat.clone()).or_insert(0) += secs;
        }
    }

    let slots: Vec<HourSlot> = (0u8..24)
        .map(|h| {
            let mut segs: Vec<HourSegment> = buckets[h as usize]
                .iter()
                .map(|(cat, secs)| HourSegment {
                    category_id: cat.clone(),
                    minutes: ((*secs as f64 / 60.0).round() as u32).min(60),
                })
                .filter(|s| s.minutes > 0)
                .collect();
            segs.sort_by(|a, b| b.minutes.cmp(&a.minutes));
            HourSlot {
                hour: h,
                segments: segs,
            }
        })
        .collect();

    Ok(slots)
}

pub async fn day_apps(pool: &DbPool, day_offset: i32, limit: u32) -> Result<Vec<AppUsage>> {
    let target = Local::now() + Duration::days(day_offset as i64);
    let date = target.format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String, i64)> = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT a.process_name,
                            COALESCE(m.category_id, 'other') AS cat,
                            SUM(a.duration_secs) AS total
                     FROM activities a
                     LEFT JOIN app_categories m ON m.process_name = a.process_name
                     WHERE a.local_date = ?
                     GROUP BY a.process_name, cat
                     ORDER BY total DESC
                     LIMIT ?",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let it = stmt
                .query_map(rusqlite::params![date, limit], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.map_err(tokio_rusqlite::Error::Rusqlite)?);
            }
            Ok(out)
        })
        .await?;

    Ok(rows
        .into_iter()
        .map(|(process, cat, secs)| AppUsage {
            process,
            category_id: cat,
            minutes: ((secs as f64 / 60.0).round() as u32),
        })
        .filter(|a| a.minutes > 0)
        .collect())
}

pub async fn week_days(pool: &DbPool, week_offset: i32) -> Result<Vec<DaySummary>> {
    let (monday, sunday) = week_range(week_offset);
    days_in_range(pool, monday, sunday).await
}

pub async fn week_apps(
    pool: &DbPool,
    week_offset: i32,
    limit: u32,
) -> Result<Vec<AppUsage>> {
    let (monday, sunday) = week_range(week_offset);
    apps_in_range(pool, monday, sunday, limit).await
}

pub async fn month_days(pool: &DbPool, month_offset: i32) -> Result<Vec<DaySummary>> {
    let (first, last) = month_range(month_offset);
    days_in_range(pool, first, last).await
}

pub async fn month_apps(
    pool: &DbPool,
    month_offset: i32,
    limit: u32,
) -> Result<Vec<AppUsage>> {
    let (first, last) = month_range(month_offset);
    apps_in_range(pool, first, last, limit).await
}

async fn days_in_range(
    pool: &DbPool,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<DaySummary>> {
    let from_str = from.format("%Y-%m-%d").to_string();
    let to_str = to.format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String, i64)> = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT a.local_date,
                            COALESCE(m.category_id, 'other') AS cat,
                            SUM(a.duration_secs) AS total
                     FROM activities a
                     LEFT JOIN app_categories m ON m.process_name = a.process_name
                     WHERE a.local_date >= ? AND a.local_date <= ?
                     GROUP BY a.local_date, cat",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let it = stmt
                .query_map(rusqlite::params![from_str, to_str], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.map_err(tokio_rusqlite::Error::Rusqlite)?);
            }
            Ok(out)
        })
        .await?;

    let mut buckets: std::collections::HashMap<String, std::collections::HashMap<String, u32>> =
        std::collections::HashMap::new();
    for (date, cat, secs) in rows {
        let minutes = ((secs as f64 / 60.0).round() as u32).max(0);
        if minutes == 0 {
            continue;
        }
        buckets.entry(date).or_default().insert(cat, minutes);
    }

    let mut out = Vec::new();
    let mut cur = from;
    while cur <= to {
        let key = cur.format("%Y-%m-%d").to_string();
        let mut segs: Vec<HourSegment> = buckets
            .remove(&key)
            .unwrap_or_default()
            .into_iter()
            .map(|(category_id, minutes)| HourSegment {
                category_id,
                minutes,
            })
            .collect();
        segs.sort_by(|a, b| b.minutes.cmp(&a.minutes));
        out.push(DaySummary {
            date: key,
            segments: segs,
        });
        cur = cur + Duration::days(1);
    }

    Ok(out)
}

async fn apps_in_range(
    pool: &DbPool,
    from: NaiveDate,
    to: NaiveDate,
    limit: u32,
) -> Result<Vec<AppUsage>> {
    let from_str = from.format("%Y-%m-%d").to_string();
    let to_str = to.format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String, i64)> = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT a.process_name,
                            COALESCE(m.category_id, 'other') AS cat,
                            SUM(a.duration_secs) AS total
                     FROM activities a
                     LEFT JOIN app_categories m ON m.process_name = a.process_name
                     WHERE a.local_date >= ? AND a.local_date <= ?
                     GROUP BY a.process_name, cat
                     ORDER BY total DESC
                     LIMIT ?",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let it = stmt
                .query_map(rusqlite::params![from_str, to_str, limit], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.map_err(tokio_rusqlite::Error::Rusqlite)?);
            }
            Ok(out)
        })
        .await?;

    Ok(rows
        .into_iter()
        .map(|(process, cat, secs)| AppUsage {
            process,
            category_id: cat,
            minutes: (secs as f64 / 60.0).round() as u32,
        })
        .filter(|a| a.minutes > 0)
        .collect())
}

fn week_range(week_offset: i32) -> (NaiveDate, NaiveDate) {
    let today = Local::now().date_naive();
    let dow = today.weekday().num_days_from_monday() as i64;
    let monday = today - Duration::days(dow) + Duration::days(week_offset as i64 * 7);
    let sunday = monday + Duration::days(6);
    (monday, sunday)
}

fn month_range(month_offset: i32) -> (NaiveDate, NaiveDate) {
    let today = Local::now().date_naive();
    let mut year = today.year();
    let mut month = today.month() as i32 + month_offset;
    while month <= 0 {
        month += 12;
        year -= 1;
    }
    while month > 12 {
        month -= 12;
        year += 1;
    }
    let first = NaiveDate::from_ymd_opt(year, month as u32, 1).unwrap();
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(year, (month + 1) as u32, 1).unwrap()
    };
    let last = next - Duration::days(1);
    (first, last)
}

fn parse_local(s: &str) -> DateTime<Local> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Local))
        .unwrap_or_else(|_| Local::now())
}

fn slice_by_hour(start: DateTime<Local>, end: DateTime<Local>) -> Vec<(u8, u64)> {
    let mut out = Vec::new();
    let mut cur = start;
    while cur < end {
        let hour = cur.hour() as u8;
        let next_hour = Local
            .with_ymd_and_hms(cur.year(), cur.month(), cur.day(), cur.hour(), 0, 0)
            .single()
            .map(|t| t + Duration::hours(1))
            .unwrap_or(end);
        let chunk_end = if next_hour < end { next_hour } else { end };
        let secs = (chunk_end - cur).num_seconds().max(0) as u64;
        if secs > 0 {
            out.push((hour, secs));
        }
        cur = chunk_end;
    }
    out
}
