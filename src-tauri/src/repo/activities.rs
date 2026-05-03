use chrono::{DateTime, Duration, Local, TimeZone, Timelike};

use crate::capture::WindowInfo;
use crate::error::Result;
use crate::storage::DbPool;

pub async fn insert_new(
    pool: &DbPool,
    info: &WindowInfo,
    captured_at: DateTime<Local>,
    screenshot_path: Option<String>,
) -> Result<i64> {
    let info = info.clone();
    let started = captured_at.to_rfc3339();
    let ended = captured_at.to_rfc3339();
    let local_date = captured_at.format("%Y-%m-%d").to_string();
    let local_hour = captured_at.hour() as u8;

    let id = pool
        .0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO activities(
                    started_at, ended_at, duration_secs,
                    local_date, local_hour,
                    process_name, window_title, category_id, screenshot_path
                ) VALUES (?, ?, 0, ?, ?, ?, ?, 'other', ?)",
                rusqlite::params![
                    started,
                    ended,
                    local_date,
                    local_hour,
                    info.app_name,
                    info.title,
                    screenshot_path,
                ],
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(conn.last_insert_rowid())
        })
        .await?;
    Ok(id)
}

pub async fn extend(pool: &DbPool, id: i64, captured_at: DateTime<Local>) -> Result<()> {
    let ended = captured_at.to_rfc3339();
    pool.0
        .call(move |conn| {
            let mut stmt = conn
                .prepare_cached("SELECT started_at FROM activities WHERE id = ?")
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let started_at: String = stmt
                .query_row([id], |r| r.get(0))
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let started = DateTime::parse_from_rfc3339(&started_at)
                .map(|dt| dt.with_timezone(&Local))
                .unwrap_or_else(|_| Local.timestamp_opt(0, 0).unwrap());
            let ended_dt = DateTime::parse_from_rfc3339(&ended)
                .map(|dt| dt.with_timezone(&Local))
                .unwrap_or_else(|_| Local::now());
            let dur = (ended_dt - started).num_seconds().max(0);

            conn.execute(
                "UPDATE activities SET ended_at = ?, duration_secs = ? WHERE id = ?",
                rusqlite::params![ended, dur, id],
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn delete_older_than(pool: &DbPool, retention_days: u32) -> Result<u64> {
    let days = retention_days.max(1) as i64;
    let cutoff = (Local::now() - Duration::days(days))
        .format("%Y-%m-%d")
        .to_string();
    let affected = pool
        .0
        .call(move |conn| {
            let n = conn
                .execute(
                    "DELETE FROM activities WHERE local_date < ?",
                    rusqlite::params![cutoff],
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(n as u64)
        })
        .await?;
    Ok(affected)
}

pub async fn today_count(pool: &DbPool) -> Result<u32> {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let count = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare_cached("SELECT COUNT(*) FROM activities WHERE local_date = ?")
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let n: i64 = stmt
                .query_row([&today], |r| r.get(0))
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(n as u32)
        })
        .await?;
    Ok(count)
}
