//! frames 表的读写:采集侧登记 + 消化 worker 消费。

use rusqlite::params;

use super::MemoryDb;
use crate::error::Result;
use crate::storage::SqliteResultExt;

/// 失败重试上限:超过后跳过,不让整晚消化卡死在一帧上。
pub const MAX_ATTEMPTS: i64 = 3;

/// 待消化的一帧。
#[derive(Debug, Clone)]
pub struct PendingFrame {
    pub path: String,
    pub ts: String,
    pub local_date: String,
    pub app_id: Option<String>,
    pub title: Option<String>,
}

/// 采集侧登记:截图落盘成功后调一次。幂等(同路径重复登记忽略)。
pub async fn register(
    db: &MemoryDb,
    path: String,
    ts: String,
    local_date: String,
    app_id: Option<String>,
    title: Option<String>,
) -> Result<()> {
    db.0.call(move |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO frames(path, ts, local_date, app_id, title)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![path, ts, local_date, app_id, title],
        )
        .db()?;
        Ok(())
    })
    .await?;
    Ok(())
}

/// 取一批待消化帧:待处理(0)优先,其次可重试的失败帧(2 且未超重试上限),按时间序。
pub async fn take_pending(db: &MemoryDb, limit: i64) -> Result<Vec<PendingFrame>> {
    let rows =
        db.0.call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT path, ts, local_date, app_id, title FROM frames
                     WHERE ocr_state = 0 OR (ocr_state = 2 AND attempts < ?1)
                     ORDER BY ocr_state ASC, ts ASC LIMIT ?2",
                )
                .db()?;
            let out = stmt
                .query_map(params![MAX_ATTEMPTS, limit], |r| {
                    Ok(PendingFrame {
                        path: r.get(0)?,
                        ts: r.get(1)?,
                        local_date: r.get(2)?,
                        app_id: r.get(3)?,
                        title: r.get(4)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(out)
        })
        .await?;
    Ok(rows)
}

/// 消化成功:记完成态 + 会话归属。
pub async fn mark_done(db: &MemoryDb, path: String, session_id: i64) -> Result<()> {
    db.0.call(move |conn| {
        conn.execute(
            "UPDATE frames SET ocr_state = 1, session_id = ?2 WHERE path = ?1",
            params![path, session_id],
        )
        .db()?;
        Ok(())
    })
    .await?;
    Ok(())
}

/// 消化失败:标失败态并累计重试计数(达到上限后 take_pending 不再取它)。
pub async fn mark_failed(db: &MemoryDb, path: String) -> Result<()> {
    db.0.call(move |conn| {
        conn.execute(
            "UPDATE frames SET ocr_state = 2, attempts = attempts + 1 WHERE path = ?1",
            params![path],
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

    #[tokio::test]
    async fn register_take_mark_roundtrip() {
        let db = MemoryDb::open_in_memory().await.unwrap();
        register(
            &db,
            "2026-07-05/a.jpg".into(),
            "2026-07-05T10:00:00+09:00".into(),
            "2026-07-05".into(),
            Some("code".into()),
            Some("main.rs".into()),
        )
        .await
        .unwrap();
        // 幂等:重复登记不报错不重复
        register(
            &db,
            "2026-07-05/a.jpg".into(),
            "x".into(),
            "x".into(),
            None,
            None,
        )
        .await
        .unwrap();

        let pending = take_pending(&db, 10).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].ts, "2026-07-05T10:00:00+09:00");

        mark_done(&db, "2026-07-05/a.jpg".into(), 42).await.unwrap();
        assert!(take_pending(&db, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_frame_retries_then_gives_up() {
        let db = MemoryDb::open_in_memory().await.unwrap();
        register(&db, "f.jpg".into(), "t".into(), "d".into(), None, None)
            .await
            .unwrap();
        for _ in 0..MAX_ATTEMPTS {
            assert_eq!(take_pending(&db, 10).await.unwrap().len(), 1);
            mark_failed(&db, "f.jpg".into()).await.unwrap();
        }
        // 达到重试上限 → 不再被取出
        assert!(take_pending(&db, 10).await.unwrap().is_empty());
    }
}
