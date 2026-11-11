//! Outbox + sync_cursor 的低层 SQL 操作。push/pull 路径共用。

use rand::Rng;
use rusqlite::OptionalExtension;

use crate::error::Result;
use crate::storage::{utc_now_rfc3339, DbPool, SqliteResultExt};

/// A row that has failed this many times counts as failed on the Devices page.
/// Push keeps retrying it.
const FAILED_AFTER_ATTEMPTS: i64 = 8;
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
    let now_rfc = utc_now_rfc3339();
    let rows = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, entity, payload
                     FROM sync_outbox
                     WHERE next_retry_at <= ?1
                     ORDER BY id ASC
                     LIMIT ?2",
                )
                .db()?;
            let rows = stmt
                .query_map(rusqlite::params![now_rfc, limit], |r| {
                    Ok(OutboxRow {
                        id: r.get(0)?,
                        entity: r.get(1)?,
                        payload: r.get(2)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
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
            conn.execute(&sql, params.as_slice()).db()?;
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
                    .query_row("SELECT attempts FROM sync_outbox WHERE id = ?", [id], |r| {
                        r.get(0)
                    })
                    .unwrap_or(0);
                let next_attempt = attempts + 1;
                let backoff = (RETRY_BASE_SECS << next_attempt.min(12) as u32).min(RETRY_MAX_SECS);
                let jitter: i64 = rand::thread_rng().gen_range(0..30);
                let delay = backoff + jitter;
                let next_at = (chrono::Utc::now() + chrono::Duration::seconds(delay)).to_rfc3339();
                conn.execute(
                    "UPDATE sync_outbox
                     SET attempts = attempts + 1, last_error = ?, next_retry_at = ?
                     WHERE id = ?",
                    rusqlite::params![err, next_at, id],
                )
                .db()?;
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
                    [FAILED_AFTER_ATTEMPTS],
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
                    [FAILED_AFTER_ATTEMPTS],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            Ok(n)
        })
        .await?;
    Ok(n.max(0) as u64)
}

/// A missing row is a cursor that has not started yet: 1970. Any other failure
/// is an error; read as 1970, it would make pull merge every cloud file again.
pub(super) async fn read_cursor(pool: &DbPool, entity: &str) -> Result<String> {
    let entity = entity.to_string();
    let cursor: Option<String> = pool
        .0
        .call(move |conn| {
            conn.query_row(
                "SELECT last_pulled_at FROM sync_cursor WHERE entity = ?",
                [&entity],
                |r| r.get(0),
            )
            .optional()
            .db()
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
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    /// 失败多少次都接着重试：从第 8 次起设备页把这行算进「失败」，到点了 `read_due_outbox`
    /// 照样选中它；再失败下去，等待封顶在一小时。
    ///
    /// 次数写死 8：谁改了这条线，这条测试就红。
    #[tokio::test]
    async fn a_row_counted_as_failed_is_still_retried() {
        let pool = fresh_test_pool().await;
        // 入一条 outbox（attempts=0，next_retry_at=epoch 让它立刻 due）
        let id = pool
            .0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO sync_outbox(op, entity, entity_pk, payload, created_at, attempts, next_retry_at)
                     VALUES('upsert', 'activity', 'pk-1', '{\"localDate\":\"2026-05-15\"}',
                            '1970-01-01T00:00:00+00:00', 0,
                            '1970-01-01T00:00:00+00:00')",
                    [],
                )
                .db()?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .unwrap();

        // 失败 7 次：还算待发送
        for _ in 0..7 {
            bump_outbox_retry(&pool, &[id], "test error").await.unwrap();
        }
        assert_eq!(count_dead_letter(&pool).await.unwrap(), 0);
        assert_eq!(count_outbox(&pool).await.unwrap(), 1);

        // 第 8 次：算进失败
        bump_outbox_retry(&pool, &[id], "test error").await.unwrap();
        assert_eq!(count_dead_letter(&pool).await.unwrap(), 1);
        assert_eq!(count_outbox(&pool).await.unwrap(), 0);

        // 没到点不选；到点了照样选中
        assert!(read_due_outbox(&pool, 100).await.unwrap().is_empty());
        pool.0
            .call(move |conn| {
                conn.execute(
                    "UPDATE sync_outbox SET next_retry_at = '1970-01-01T00:00:00+00:00'
                     WHERE id = ?1",
                    [id],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
        let due = read_due_outbox(&pool, 100).await.unwrap();
        assert_eq!(due.iter().map(|r| r.id).collect::<Vec<_>>(), vec![id]);

        // 再失败 4 次，一共 12 次：下一次重试在一小时后，外加不到 30 秒的抖动
        for _ in 0..4 {
            bump_outbox_retry(&pool, &[id], "test error").await.unwrap();
        }
        let next_retry_at: String = pool
            .0
            .call(move |conn| {
                conn.query_row(
                    "SELECT next_retry_at FROM sync_outbox WHERE id = ?1",
                    [id],
                    |r| r.get(0),
                )
                .db()
            })
            .await
            .unwrap();
        let wait = chrono::DateTime::parse_from_rfc3339(&next_retry_at).unwrap()
            - chrono::Utc::now().fixed_offset();
        assert!((3590..=3630).contains(&wait.num_seconds()), "{wait}");
    }

    /// `bump_outbox_retry` 第 N 次调用后 attempts 单调递增。
    #[tokio::test]
    async fn bump_outbox_retry_increments_attempts_monotonically() {
        let pool = fresh_test_pool().await;
        let id = pool
            .0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO sync_outbox(op, entity, entity_pk, payload, created_at, attempts, next_retry_at)
                     VALUES('upsert', 'activity', 'pk-2', '{\"localDate\":\"2026-05-15\"}',
                            '1970-01-01T00:00:00+00:00', 0,
                            '1970-01-01T00:00:00+00:00')",
                    [],
                )
                .db()?;
                Ok(conn.last_insert_rowid())
            })
            .await
            .unwrap();

        let read_attempts = || async {
            pool.0
                .call(move |conn| {
                    let n: i64 = conn
                        .query_row(
                            "SELECT attempts FROM sync_outbox WHERE id = ?1",
                            [id],
                            |r| r.get(0),
                        )
                        .db()?;
                    Ok(n)
                })
                .await
                .unwrap()
        };

        for expected in 1..=5 {
            bump_outbox_retry(&pool, &[id], "x").await.unwrap();
            assert_eq!(read_attempts().await, expected);
        }
    }

    /// `read_cursor` 在 sync_cursor 表无对应行时返回 epoch 默认值。
    #[tokio::test]
    async fn read_cursor_defaults_to_epoch_when_missing() {
        let pool = fresh_test_pool().await;
        let cursor = read_cursor(&pool, "drive_files").await.unwrap();
        assert_eq!(cursor, "1970-01-01T00:00:00Z");
    }

    /// 读游标时数据库出错要报错，不能当成「没有游标」从 1970 开始：那样下一轮会把云上
    /// 的文件全部重新合并，清空过的数据会跟着回来。
    #[tokio::test]
    async fn read_cursor_reports_a_database_error() {
        let pool = fresh_test_pool().await;
        pool.0
            .call(|conn| {
                conn.execute("DROP TABLE sync_cursor", []).db()?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(read_cursor(&pool, "drive_files").await.is_err());
    }

    /// `write_cursor` UPSERT：第二次写覆盖第一次的值。
    #[tokio::test]
    async fn write_cursor_upserts_value() {
        let pool = fresh_test_pool().await;
        write_cursor(&pool, "drive_files", "2026-05-15T10:00:00Z")
            .await
            .unwrap();
        assert_eq!(
            read_cursor(&pool, "drive_files").await.unwrap(),
            "2026-05-15T10:00:00Z"
        );
        write_cursor(&pool, "drive_files", "2026-05-16T11:00:00Z")
            .await
            .unwrap();
        assert_eq!(
            read_cursor(&pool, "drive_files").await.unwrap(),
            "2026-05-16T11:00:00Z"
        );
    }
}
