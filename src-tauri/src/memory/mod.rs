//! 屏幕记忆库(memory.sqlite)——独立于主库的第二个数据库文件。
//!
//! 独立成库的理由(docs/design/screen-memory.md §5):用户可单独删除/迁移这份资产;
//! 避开主库单连接的写争用;物理隔离保证永不进云同步(无 outbox)。
//!
//! 本模块只提供连接与 schema;帧登记见 [`frames`],L3 折叠与 FTS 见 [`sessions`]。

pub mod digest;
pub mod frames;
pub mod sessions;

use std::path::PathBuf;

use tokio_rusqlite::Connection;

use crate::error::Result;
use crate::storage::SqliteResultExt;

/// 记忆库连接——与主库 [`crate::storage::DbPool`] 同样的薄包装,独立类型
/// 防止两个库的连接在函数签名里互相传错。
#[derive(Clone)]
pub struct MemoryDb(pub Connection);

/// 记忆库文件路径:与主库同目录,按账号隔离(`hindsight-memory.<uid>.sqlite`)。
pub fn memory_db_path() -> Result<PathBuf> {
    let dir = crate::storage::db_path_dir()?;
    let name = match crate::account::active_uid() {
        Some(uid) => format!("hindsight-memory.{uid}.sqlite"),
        None => "hindsight-memory.sqlite".to_string(),
    };
    Ok(dir.join(name))
}

impl MemoryDb {
    /// 打开(或新建)记忆库并确保 schema 就绪。幂等,启动与 worker 双方都可调。
    pub async fn open_at(path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(path).await?;
        let db = Self(conn);
        db.init_schema().await?;
        Ok(db)
    }

    /// 按当前账号的默认路径打开。
    pub async fn open() -> Result<Self> {
        let path = memory_db_path()?;
        Self::open_at(&path).await
    }

    /// 内存库,单元测试用。
    #[cfg(test)]
    pub async fn open_in_memory() -> Result<Self> {
        let conn = Connection::open(":memory:").await?;
        let db = Self(conn);
        db.init_schema().await?;
        Ok(db)
    }

    /// schema v1(L2/L3 所需;L4/L5 的簇表在 P3 以 user_version 迁移追加)。
    async fn init_schema(&self) -> Result<()> {
        self.0
            .call(|conn| {
                conn.execute_batch(
                    "PRAGMA journal_mode = WAL;

                    -- 每个落盘帧一条(采集侧登记,消化 worker 消费)
                    CREATE TABLE IF NOT EXISTS frames (
                        path        TEXT PRIMARY KEY,   -- 截图相对路径(相对截图根目录)
                        ts          TEXT NOT NULL,      -- 拍摄时刻(RFC3339)
                        local_date  TEXT NOT NULL,      -- 本地日期(YYYY-MM-DD)
                        app_id      TEXT,
                        title       TEXT,
                        ocr_state   INTEGER NOT NULL DEFAULT 0,  -- 0待 1完 2失败
                        attempts    INTEGER NOT NULL DEFAULT 0,  -- 失败重试计数
                        session_id  INTEGER             -- L3 文本会话归属
                    );
                    CREATE INDEX IF NOT EXISTS idx_frames_state
                        ON frames(ocr_state, ts);

                    -- L3 文本会话(折叠后的检索单元)
                    CREATE TABLE IF NOT EXISTS text_sessions (
                        id INTEGER PRIMARY KEY,
                        local_date TEXT NOT NULL,
                        started_ts TEXT NOT NULL,
                        ended_ts   TEXT NOT NULL,
                        app_id TEXT,
                        title  TEXT,
                        text   TEXT NOT NULL DEFAULT ''  -- session_lines 的物化拼接
                    );

                    -- 行级留痕:每个唯一行 + 首次出现帧(证据卡精确到帧)
                    CREATE TABLE IF NOT EXISTS session_lines (
                        session_id INTEGER NOT NULL,
                        line_no    INTEGER NOT NULL,
                        text       TEXT NOT NULL,
                        first_path TEXT NOT NULL,
                        first_ts   TEXT NOT NULL,
                        PRIMARY KEY (session_id, line_no)
                    );

                    -- 全文索引:挂在会话文本上,trigram 支持中日文子串、语言无关
                    CREATE VIRTUAL TABLE IF NOT EXISTS text_sessions_fts USING fts5(
                        text, content='text_sessions', content_rowid='id',
                        tokenize='trigram'
                    );
                    CREATE TRIGGER IF NOT EXISTS text_sessions_ai
                    AFTER INSERT ON text_sessions BEGIN
                        INSERT INTO text_sessions_fts(rowid, text)
                        VALUES (new.id, new.text);
                    END;
                    CREATE TRIGGER IF NOT EXISTS text_sessions_au
                    AFTER UPDATE OF text ON text_sessions BEGIN
                        INSERT INTO text_sessions_fts(text_sessions_fts, rowid, text)
                        VALUES ('delete', old.id, old.text);
                        INSERT INTO text_sessions_fts(rowid, text)
                        VALUES (new.id, new.text);
                    END;
                    CREATE TRIGGER IF NOT EXISTS text_sessions_ad
                    AFTER DELETE ON text_sessions BEGIN
                        INSERT INTO text_sessions_fts(text_sessions_fts, rowid, text)
                        VALUES ('delete', old.id, old.text);
                    END;

                    PRAGMA user_version = 1;",
                )
                .db()
            })
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// schema 建得起来 + trigram FTS 可用(bundled SQLite 版本达标的证明)。
    #[tokio::test]
    async fn schema_and_trigram_fts_work() {
        let db = MemoryDb::open_in_memory().await.unwrap();
        db.0.call(|conn| {
            conn.execute(
                "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, text)
                 VALUES ('2026-07-05', 't0', 't1', 'code', '标题', '订单编号八八四二 electric keyboard')",
                [],
            )
            .db()?;
            // 中文子串(非词边界)必须可命中——trigram 的核心诉求
            let hits: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM text_sessions_fts WHERE text_sessions_fts MATCH '单编号'",
                    [],
                    |r| r.get(0),
                )
                .db()?;
            assert_eq!(hits, 1);
            Ok(())
        })
        .await
        .unwrap();
    }
}
