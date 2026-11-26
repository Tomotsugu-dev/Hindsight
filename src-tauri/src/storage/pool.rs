use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::storage::{Connection, SqliteResultExt};

/// The SQLite connection pool: a thin wrapper around [`Connection`].
/// There is one per process, passed in as Tauri State. Commands get it through
/// `State<'_, DbPool>`.
#[derive(Clone)]
pub struct DbPool(pub Connection);

impl DbPool {
    /// Opens (or creates) the database. Does not run migrations: call
    /// [`crate::storage::migrations::run`] next.
    ///
    /// WAL: capture commits several times every 5 seconds, and in the default
    /// rollback journal each commit writes its pages twice and waits for two
    /// fsyncs. `synchronous = NORMAL` drops the fsync per commit; a power cut can
    /// lose the last commits but leaves the file intact.
    ///
    /// `optimize`: without statistics, a query that filters by both date and
    /// device picks the device index and reads every record of that device. The
    /// first open analyzes the tables. Once the statistics exist, running it again
    /// takes almost no time.
    pub async fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).await?;
        conn.call(|c| {
            c.execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = NORMAL;
                 PRAGMA optimize = 0x10002;",
            )
            .db()
        })
        .await?;
        Ok(Self(conn))
    }

    /// 在内存里开一个临时 SQLite DB——给单元测试用，调用方需自行跑 migrations。
    /// 每次返回一个全新独立的连接，互相之间不共享 schema / 数据。
    #[cfg(test)]
    pub async fn open_in_memory() -> Result<Self> {
        let conn = Connection::open(":memory:").await?;
        Ok(Self(conn))
    }
}

/// The main database file this run uses: `hindsight.<uid>.sqlite`, or
/// `hindsight.sqlite` when the database belongs to no account yet. The uid is
/// [`crate::account::db_uid`].
pub fn db_path() -> Result<PathBuf> {
    db_path_for(crate::account::db_uid().as_deref())
}

/// Where an account's database lives: `hindsight.<uid>.sqlite`. `None` gives
/// `hindsight.sqlite`, the database that belongs to no account yet.
pub fn db_path_for(uid: Option<&str>) -> Result<PathBuf> {
    let dir = db_path_dir()?;
    let name = match uid {
        Some(uid) => format!("hindsight.{uid}.sqlite"),
        None => "hindsight.sqlite".to_string(),
    };
    Ok(dir.join(name))
}

/// 数据根目录（[`crate::bootstrap::data_root`]）；不存在时自动创建。
pub fn db_path_dir() -> Result<PathBuf> {
    let dir = crate::bootstrap::data_root();
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 主库按 WAL + NORMAL 打开；WAL 记在文件头里，不经 DbPool 再打开（如旧版本）也还是 WAL。
    #[tokio::test]
    async fn open_uses_wal_and_normal_sync() {
        let dir = std::env::temp_dir().join(format!("hindsight-pool-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("main.sqlite");

        let pool = DbPool::open(&path).await.unwrap();
        let (mode, sync): (String, i64) = pool
            .0
            .call(|c| {
                let mode = c.query_row("PRAGMA journal_mode", [], |r| r.get(0)).db()?;
                let sync = c.query_row("PRAGMA synchronous", [], |r| r.get(0)).db()?;
                Ok((mode, sync))
            })
            .await
            .unwrap();
        assert_eq!(mode, "wal");
        assert_eq!(sync, 1, "1 = NORMAL");
        pool.0.close().await.unwrap();

        let raw = rusqlite::Connection::open(&path).unwrap();
        let mode: String = raw
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 打开时给有数据的表生成统计信息，查询规划器才能在日期索引和设备索引之间选对。
    /// 空表不生成统计，所以先插数据。
    #[tokio::test]
    async fn open_analyzes_tables_with_rows() {
        let dir =
            std::env::temp_dir().join(format!("hindsight-pool-stats-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("main.sqlite");

        let raw = rusqlite::Connection::open(&path).unwrap();
        raw.execute_batch(
            "CREATE TABLE t(day TEXT, device TEXT);
             CREATE INDEX t_day ON t(day);
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100)
             INSERT INTO t SELECT 'd' || (i % 10), 'dev' FROM n;",
        )
        .unwrap();
        drop(raw);

        let pool = DbPool::open(&path).await.unwrap();
        let rows: i64 = pool
            .0
            .call(|c| {
                // 统计表不存在时直接查会报 no such table，先看它在不在
                let has_table: bool = c
                    .query_row(
                        "SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE name = 'sqlite_stat1')",
                        [],
                        |r| r.get(0),
                    )
                    .db()?;
                if !has_table {
                    return Ok(0);
                }
                c.query_row(
                    "SELECT COUNT(*) FROM sqlite_stat1 WHERE tbl = 't'",
                    [],
                    |r| r.get(0),
                )
                .db()
            })
            .await
            .unwrap();
        assert!(rows > 0, "t 应该有统计信息");
        pool.0.close().await.unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }
}
