//! Outbox + sync_cursor 的低层 SQL 操作。push/pull 路径共用。

use rand::Rng;

use crate::error::Result;
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

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
                .db()?;
            let rows = stmt
                .query_map(rusqlite::params![now_rfc, MAX_ATTEMPTS, limit], |r| {
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

    /// 测 [`bump_outbox_retry`] + [`count_dead_letter`]：连续失败 N 次后 attempts 累加，
    /// 第 10 次达到 [`MAX_ATTEMPTS`] 阈值，`read_due_outbox` 不再选中它，`count_dead_letter` += 1。
    ///
    /// 钉死 dead-letter 边界：若未来谁把 MAX_ATTEMPTS 改成 100，本测试立刻红。
    #[tokio::test]
    async fn bump_outbox_retry_dead_letter_at_10() {
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

        // 跑 10 次 bump_outbox_retry
        for _ in 0..MAX_ATTEMPTS {
            bump_outbox_retry(&pool, &[id], "test error").await.unwrap();
        }

        // attempts 应到 10 → count_dead_letter = 1
        assert_eq!(count_dead_letter(&pool).await.unwrap(), 1);
        assert_eq!(count_outbox(&pool).await.unwrap(), 0);

        // read_due_outbox 不该再选中它（attempts >= MAX_ATTEMPTS 被过滤）
        let due = read_due_outbox(&pool, 100).await.unwrap();
        assert!(
            due.iter().all(|r| r.id != id),
            "attempts >= MAX_ATTEMPTS 的行不应被 read_due_outbox 选中"
        );
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

    /// `write_cursor` UPSERT：第二次写覆盖第一次的值。
    #[tokio::test]
    async fn write_cursor_upserts_value() {
        let pool = fresh_test_pool().await;
        write_cursor(&pool, "drive_files", "2026-05-15T10:00:00Z").await.unwrap();
        assert_eq!(
            read_cursor(&pool, "drive_files").await.unwrap(),
            "2026-05-15T10:00:00Z"
        );
        write_cursor(&pool, "drive_files", "2026-05-16T11:00:00Z").await.unwrap();
        assert_eq!(
            read_cursor(&pool, "drive_files").await.unwrap(),
            "2026-05-16T11:00:00Z"
        );
    }
}
