use chrono::{DateTime, Datelike, Duration, Local, TimeZone, Timelike};
use serde::Serialize;

use crate::error::Result;
use crate::storage::DbPool;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourSegment {
    pub category_id: String,
    pub minutes: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourSlot {
    pub hour: u8,
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
                    minutes: ((*secs as f64 / 60.0).round() as u32).min(60) as u16,
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
