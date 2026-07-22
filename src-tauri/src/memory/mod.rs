//! 屏幕记忆库(memory.sqlite)——独立于主库的第二个数据库文件。
//!
//! 独立成库的理由(docs/design/screen-memory.md §5):用户可单独删除/迁移这份资产;
//! 避开主库单连接的写争用。**默认不进云同步**;用户在云同步设置里显式打开
//! 「聊天历史 / 屏幕记忆全文」开关后,由 sync 引擎按 guid 全量推拉(见 sync/engine/datasets.rs)。
//!
//! 本模块只提供连接与 schema;帧登记见 [`frames`],L3 折叠与 FTS 见 [`sessions`]。

pub mod digest;
pub mod frames;
pub mod resident;
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

    /// schema v2:L2/L3 所需(帧/会话/行/FTS)+ Chat 会话历史。
    /// CREATE 全部 IF NOT EXISTS,可重复执行;旧库重跑同一批次自动补新表,
    /// 无需分支迁移,user_version 仅作世代标记。
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
                        text   TEXT NOT NULL DEFAULT '',  -- session_lines 的物化拼接
                        guid   TEXT,                      -- 跨设备全局 id(可选上云用)
                        origin_device TEXT                -- NULL=本机;非 NULL=来自该设备
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

                    -- 云端截图洞察:帧级落库,无聚合层(docs/design/cloud-insight.md §3)
                    -- state: 0待 1完 2失败(attempts<上限可重试) 3跳过(粗门/策略/隐私)
                    CREATE TABLE IF NOT EXISTS frame_insights (
                        path        TEXT PRIMARY KEY,
                        ts          TEXT NOT NULL,
                        local_date  TEXT NOT NULL,
                        app         TEXT,
                        title       TEXT,
                        insight     TEXT,
                        entities    TEXT,
                        state       INTEGER NOT NULL DEFAULT 0,
                        attempts    INTEGER NOT NULL DEFAULT 0,
                        done_at     TEXT    -- 分析完成时刻;日限额按它的日期计数(回填也占额度)
                    );
                    CREATE INDEX IF NOT EXISTS idx_frame_insights_date
                        ON frame_insights(local_date, state);

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

                    -- Chat 会话(默认只在本地;guid 供可选上云时跨设备合并,
                    -- deleted_at 软删让删除能传播到其它设备)
                    CREATE TABLE IF NOT EXISTS chat_conversations (
                        id          INTEGER PRIMARY KEY,
                        title       TEXT NOT NULL,   -- 首问截断自动生成,可重命名
                        created_ts  TEXT NOT NULL,
                        updated_ts  TEXT NOT NULL,   -- 最后一次变更时刻,列表按此倒序,同步 LWW 用
                        guid        TEXT,            -- 跨设备全局 id(hex)
                        deleted_at  TEXT             -- 软删时刻;NULL = 存活
                    );

                    -- Chat 消息:user / assistant 各一行;错误消息不落库(前端瞬态展示)
                    CREATE TABLE IF NOT EXISTS chat_messages (
                        id              INTEGER PRIMARY KEY,
                        conversation_id INTEGER NOT NULL,
                        role            TEXT NOT NULL,   -- 'user' | 'assistant'
                        content         TEXT NOT NULL,
                        citations       TEXT,            -- assistant: Vec<Citation> JSON;user: NULL
                        degraded        INTEGER NOT NULL DEFAULT 0,
                        created_ts      TEXT NOT NULL,
                        guid            TEXT,            -- 跨设备全局 id
                        conv_guid       TEXT,            -- 所属会话的 guid(跨设备解析用)
                        prompt_tokens     INTEGER,       -- 本轮上行 token(assistant)
                        completion_tokens INTEGER        -- 本轮下行 token(assistant)
                    );
                    CREATE INDEX IF NOT EXISTS idx_chat_messages_conv
                        ON chat_messages(conversation_id, id);

                    PRAGMA user_version = 4;",
                )
                .db()?;

                // ── v3 增量列(旧库补列;新库上面的 CREATE 已带) ──
                // CREATE IF NOT EXISTS 批次没法条件 ALTER,这里逐条加、
                // 重复列错误(duplicate column)视为已就绪静默跳过。
                for sql in [
                    "ALTER TABLE chat_conversations ADD COLUMN guid TEXT",
                    "ALTER TABLE chat_conversations ADD COLUMN deleted_at TEXT",
                    "ALTER TABLE chat_messages ADD COLUMN guid TEXT",
                    "ALTER TABLE chat_messages ADD COLUMN conv_guid TEXT",
                    "ALTER TABLE chat_messages ADD COLUMN prompt_tokens INTEGER",
                    "ALTER TABLE chat_messages ADD COLUMN completion_tokens INTEGER",
                    "ALTER TABLE text_sessions ADD COLUMN guid TEXT",
                    // 会话来源设备:NULL = 本机产出;非 NULL = 从该设备同步而来
                    "ALTER TABLE text_sessions ADD COLUMN origin_device TEXT",
                ] {
                    if let Err(e) = conn.execute(sql, []) {
                        let msg = e.to_string();
                        if !msg.contains("duplicate column") {
                            return Err(tokio_rusqlite::Error::Rusqlite(e));
                        }
                    }
                }
                // guid 回填(hex(randomblob) 足够全局唯一)+ 唯一索引
                conn.execute_batch(
                    "UPDATE chat_conversations SET guid = lower(hex(randomblob(16)))
                       WHERE guid IS NULL;
                     UPDATE chat_messages SET guid = lower(hex(randomblob(16)))
                       WHERE guid IS NULL;
                     UPDATE chat_messages SET conv_guid =
                         (SELECT guid FROM chat_conversations c
                           WHERE c.id = chat_messages.conversation_id)
                       WHERE conv_guid IS NULL;
                     UPDATE text_sessions SET guid = lower(hex(randomblob(16)))
                       WHERE guid IS NULL;
                     CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_conv_guid
                         ON chat_conversations(guid);
                     CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_msg_guid
                         ON chat_messages(guid);
                     CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_guid
                         ON text_sessions(guid);
                     PRAGMA user_version = 4;",
                )
                .db()?;
                Ok(())
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
