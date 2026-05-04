//! Outbox + sync_cursor 的低层 SQL 操作。push/pull 路径共用。

use rand::Rng;

use crate::error::Result;
use crate::storage::DbPool;

pub(super) const MAX_ATTEMPTS: i64 = 10;
const RETRY_BASE_SECS: i64 = 5;
const RETRY_MAX_SECS: i64 = 60 * 60;

#[derive(Debug, Clone)]
pub(super) struct OutboxRow {
    pub(super) id: i64,
    pub(super) entity: String,
    pub(super) payload: String,
}

pub(super) async fn read_due_outbox(pool: &DbPool, limit: usize) -> Result<Vec<OutboxRow>> {
    let limit = limit as i64;
    // 关键：next_retry_at 是 chrono::to_rfc3339()（"2026-05-03T...+00:00"），
    // 不能跟 SQLite 的 datetime('now')（"2026-05-03 ..."，空格无 T）做字典序比较 —— 'T' > ' ' 永远不等。
    // 这里用 Rust 端生成同格式的 now 当参数。
    let now_rfc = chrono::Utc::now().to_rfc3339();
    let rows = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, entity, payload
                     FROM sync_outbox
                     WHERE next_retry_at <= ?1 AND attempts < ?2
                     ORDER BY id ASC
                     LIMIT ?3",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let rows = stmt
                .query_map(rusqlite::params![now_rfc, MAX_ATTEMPTS, limit], |r| {
                    Ok(OutboxRow {
                        id: r.get(0)?,
                        entity: r.get(1)?,
                        payload: r.get(2)?,
                    })
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(rows)
        })
        .await?;
    Ok(rows)
}

pub(super) async fn delete_outbox_rows(pool: &DbPool, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let ids = ids.to_vec();
    pool.0
        .call(move |conn| {
            let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!("DELETE FROM sync_outbox WHERE id IN ({placeholders})");
            let params: Vec<&dyn rusqlite::ToSql> =
                ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
            conn.execute(&sql, params.as_slice())
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub(super) async fn bump_outbox_retry(pool: &DbPool, ids: &[i64], err: &str) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let ids = ids.to_vec();
    let err = err.to_string();
    pool.0
        .call(move |conn| {
            for id in &ids {
                let attempts: i64 = conn
                    .query_row(
                        "SELECT attempts FROM sync_outbox WHERE id = ?",
                        [id],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                let next_attempt = attempts + 1;
                let backoff = (RETRY_BASE_SECS << next_attempt.min(12) as u32).min(RETRY_MAX_SECS);
                let jitter: i64 = rand::thread_rng().gen_range(0..30);
                let delay = backoff + jitter;
                let next_at =
                    (chrono::Utc::now() + chrono::Duration::seconds(delay)).to_rfc3339();
                conn.execute(
                    "UPDATE sync_outbox
                     SET attempts = attempts + 1, last_error = ?, next_retry_at = ?
                     WHERE id = ?",
                    rusqlite::params![err, next_at, id],
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}

pub(super) async fn count_outbox(pool: &DbPool) -> Result<u64> {
    let n = pool
        .0
        .call(|conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sync_outbox WHERE attempts < ?",
                    [MAX_ATTEMPTS],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            Ok(n)
        })
        .await?;
    Ok(n.max(0) as u64)
}

pub(super) async fn count_dead_letter(pool: &DbPool) -> Result<u64> {
    let n = pool
        .0
        .call(|conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sync_outbox WHERE attempts >= ?",
                    [MAX_ATTEMPTS],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            Ok(n)
        })
        .await?;
    Ok(n.max(0) as u64)
}

pub(super) async fn read_cursor(pool: &DbPool, entity: &str) -> Result<String> {
    let entity = entity.to_string();
    let cursor = pool
        .0
        .call(move |conn| {
            let r: Option<String> = conn
                .query_row(
                    "SELECT last_pulled_at FROM sync_cursor WHERE entity = ?",
                    [&entity],
                    |r| r.get(0),
                )
                .ok();
            Ok(r)
        })
        .await?;
    Ok(cursor.unwrap_or_else(|| "1970-01-01T00:00:00Z".into()))
}

pub(super) async fn write_cursor(pool: &DbPool, entity: &str, value: &str) -> Result<()> {
    let entity = entity.to_string();
    let value = value.to_string();
    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO sync_cursor(entity, last_pulled_at) VALUES(?, ?)
                 ON CONFLICT(entity) DO UPDATE SET last_pulled_at = excluded.last_pulled_at",
                rusqlite::params![entity, value],
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(())
        })
        .await?;
    Ok(())
}
