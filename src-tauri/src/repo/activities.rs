use chrono::{DateTime, Duration, Local, TimeZone, Timelike, Utc};

use crate::capture::WindowInfo;
use crate::device;
use crate::error::Result;
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

/// 创建一条新的会话记录。device_id = self；updated_at = captured_at；
/// **不**写 outbox —— 用户明确要求只在会话结束 (seal) 时才推到云端。
pub async fn insert_new(
    pool: &DbPool,
    info: &WindowInfo,
    captured_at: DateTime<Local>,
    screenshot_path: Option<String>,
) -> Result<i64> {
    let info = info.clone();
    let started = captured_at.to_rfc3339();
    let ended = captured_at.to_rfc3339();
    let updated = ended.clone();
    let local_date = captured_at.format("%Y-%m-%d").to_string();
    let local_hour = captured_at.hour() as u8;
    let device_id = device::self_id()?.to_string();

    let id = pool
        .0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO activities(
                    started_at, ended_at, duration_secs,
                    local_date, local_hour,
                    process_name, window_title, category_id, screenshot_path,
                    device_id, updated_at, origin
                ) VALUES (?, ?, 0, ?, ?, ?, ?, 'other', ?, ?, ?, 'local')",
                rusqlite::params![
                    started,
                    ended,
                    local_date,
                    local_hour,
                    info.app_name,
                    info.title,
                    screenshot_path,
                    device_id,
                    updated,
                ],
            )
            .db()?;
            Ok(conn.last_insert_rowid())
        })
        .await?;
    Ok(id)
}

/// 会话结束（焦点切到别的窗口那一刻）。
/// 同事务里：把 ended_at 钉死成 final_ended_at，更新 duration_secs / updated_at，并写一条 outbox 推到云端。
pub async fn seal_session(pool: &DbPool, id: i64, final_ended_at: DateTime<Local>) -> Result<()> {
    let ended = final_ended_at.to_rfc3339();
    let updated = Utc::now().to_rfc3339();
    let device_id = device::self_id()?.to_string();

    pool.0
        .call(move |conn| {
            // 取整行做 outbox payload 用
            let row: Option<(
                String,
                String,
                i64,
                String,
                u8,
                String,
                Option<String>,
                String,
                String,
            )> = conn
                .query_row(
                    "SELECT started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id, device_id
                     FROM activities WHERE id = ?",
                    [id],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                            r.get(7)?,
                            r.get(8)?,
                        ))
                    },
                )
                .ok();

            let Some((started_at, _, _, local_date, local_hour, process_name, window_title, category_id, this_device)) = row else {
                // 行不存在：可能是已经被清掉了；忽略
                return Ok(());
            };

            // 重算 duration
            let started = DateTime::parse_from_rfc3339(&started_at)
                .map(|dt| dt.with_timezone(&Local))
                .unwrap_or_else(|_| Local.timestamp_opt(0, 0).unwrap());
            let ended_dt = DateTime::parse_from_rfc3339(&ended)
                .map(|dt| dt.with_timezone(&Local))
                .unwrap_or_else(|_| Local::now());
            let dur = (ended_dt - started).num_seconds().max(0);

            conn.execute(
                "UPDATE activities SET ended_at = ?, duration_secs = ?, updated_at = ? WHERE id = ?",
                rusqlite::params![ended, dur, updated, id],
            )
            .db()?;

            // 只对 local 来源的会话写 outbox：远端拉来的不要再推回去
            if this_device == device_id {
                let payload = serde_json::json!({
                    "deviceId": this_device,
                    "startedAt": started_at,
                    "endedAt": ended,
                    "durationSecs": dur,
                    "localDate": local_date,
                    "localHour": local_hour,
                    "processName": process_name,
                    "windowTitle": window_title,
                    "categoryId": category_id,
                    "updatedAt": updated,
                })
                .to_string();
                enqueue(
                    conn,
                    OutboxOp::Upsert,
                    OutboxEntity::Activity,
                    &id.to_string(),
                    &payload,
                )
                .db()?;
            }

            Ok(())
        })
        .await?;
    Ok(())
}

/// 清理超过 retention_days 的截图文件（jpg），不删 activities 行；只把对应行的 screenshot_path 置 NULL。
/// 返回成功删除的文件数。
pub async fn delete_screenshots_older_than(pool: &DbPool, retention_days: u32) -> Result<u64> {
    let days = retention_days.max(1) as i64;
    let cutoff = (Local::now() - Duration::days(days))
        .format("%Y-%m-%d")
        .to_string();

    // 先取出待清理的 (id, path) 列表
    let rows: Vec<(i64, String)> = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, screenshot_path FROM activities
                     WHERE screenshot_path IS NOT NULL AND local_date < ?",
                )
                .db()?;
            let rows = stmt
                .query_map([&cutoff], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;

    // spawn_blocking 里逐个删文件（best-effort）
    let deleted_files = tokio::task::spawn_blocking({
        let rows = rows.clone();
        move || {
            let mut n = 0u64;
            for (_, path) in &rows {
                if std::fs::remove_file(path).is_ok() {
                    n += 1;
                }
            }
            n
        }
    })
    .await
    .unwrap_or(0);

    // 把这些行的 screenshot_path 置 NULL（即使文件删除失败也清引用，避免下次反复尝试）
    if !rows.is_empty() {
        let ids: Vec<i64> = rows.into_iter().map(|(id, _)| id).collect();
        pool.0
            .call(move |conn| {
                let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
                let sql = format!(
                    "UPDATE activities SET screenshot_path = NULL WHERE id IN ({placeholders})"
                );
                let params: Vec<&dyn rusqlite::ToSql> =
                    ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
                conn.execute(&sql, params.as_slice())
                    .db()?;
                Ok(())
            })
            .await?;
    }

    Ok(deleted_files)
}

pub async fn today_count(pool: &DbPool) -> Result<u32> {
    let today = Local::now().format("%Y-%m-%d").to_string();
    let count = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare_cached("SELECT COUNT(*) FROM activities WHERE local_date = ?")
                .db()?;
            let n: i64 = stmt
                .query_row([&today], |r| r.get(0))
                .db()?;
            Ok(n as u32)
        })
        .await?;
    Ok(count)
}
