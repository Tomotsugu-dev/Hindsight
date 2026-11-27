//! 屏幕记忆库(memory.sqlite)——独立于主库的第二个数据库文件。
//!
//! 独立成库的理由(docs/design/screen-memory.md §5):用户可单独删除/迁移这份资产;
//! 避开主库单连接的写争用。**默认不进云同步**;用户在云同步设置里显式打开
//! 「聊天历史 / 屏幕记忆全文」开关后,由 sync 引擎按 guid 全量推拉(见 sync/engine/datasets.rs)。
//!
//! 本模块只提供连接与 schema;帧登记见 [`frames`],L3 折叠与 FTS 见 [`sessions`]。

pub mod digest;
pub mod frames;
pub mod migrations;
pub mod resident;
pub mod scheduled;
pub mod search;
pub mod sessions;
pub(crate) mod snippet;

use std::path::PathBuf;

use crate::error::Result;
use crate::storage::{Connection, DbError, SqliteResultExt};

/// 记忆库连接——与主库 [`crate::storage::DbPool`] 同样的薄包装,独立类型
/// 防止两个库的连接在函数签名里互相传错。
#[derive(Clone)]
pub struct MemoryDb(pub Connection);

/// 记忆库文件路径:与主库同目录,按账号隔离(`hindsight-memory.<uid>.sqlite`)。
pub fn memory_db_path() -> Result<PathBuf> {
    let dir = crate::storage::db_path_dir()?;
    let name = match crate::account::db_uid() {
        Some(uid) => format!("hindsight-memory.{uid}.sqlite"),
        None => "hindsight-memory.sqlite".to_string(),
    };
    Ok(dir.join(name))
}

/// The statements that create `frames` and its index. New databases and the migration of old ones
/// both use them. Each screenshot takes one row.
///
/// `id` is an explicit primary key: `VACUUM` can renumber the hidden row ids, and then
/// `session_frames` would point to the wrong screenshots (ADR-0021). The index on
/// `(ocr_state, ts)` lets OCR quickly find the screenshots that are not recognized yet.
const FRAMES_TABLE: &str = "
    CREATE TABLE IF NOT EXISTS frames (
        id          INTEGER PRIMARY KEY,
        path        TEXT UNIQUE,                 -- screenshot path
        ts          TEXT NOT NULL,               -- time taken (RFC 3339, local time zone; the first 10 characters are the local date)
        app_id      TEXT,
        title       TEXT,
        ocr_state   INTEGER NOT NULL DEFAULT 0,  -- 0 waiting, 1 done, 2 failed
        attempts    INTEGER NOT NULL DEFAULT 0,  -- failed attempts
        session_id  INTEGER                      -- the text session it belongs to
    );
    CREATE INDEX IF NOT EXISTS idx_frames_state ON frames(ocr_state, ts);";

/// The statement that creates `session_frames`. New databases and the migration of old ones both
/// use it. For each screenshot in a session, it keeps the line of the session text where the
/// screenshot's text starts.
///
/// For example, "0 → A, 3 → B" means lines 0–2 come from screenshot A, and the lines from 3 on
/// come from screenshot B. `WITHOUT ROWID` keeps the table in primary key order, so it needs no
/// separate primary key index.
const SESSION_FRAMES_TABLE: &str = "
    CREATE TABLE IF NOT EXISTS session_frames (
        session_id    INTEGER NOT NULL,
        first_line_no INTEGER NOT NULL,  -- the line of the session text where this run starts
        frame_id      INTEGER,           -- frames.id; NULL when the screenshot is gone
        PRIMARY KEY (session_id, first_line_no)
    ) WITHOUT ROWID;";

impl MemoryDb {
    /// Opens or creates the memory database and makes sure its tables exist.
    /// Calling it again is safe.
    ///
    /// `synchronous = NORMAL` for the same reason as the main database: see
    /// [`crate::storage::DbPool::open`].
    pub async fn open_at(path: &std::path::Path) -> Result<Self> {
        let conn = Connection::open(path).await?;
        conn.call(|c| c.execute_batch("PRAGMA synchronous = NORMAL;").db())
            .await?;
        let db = Self(conn);
        db.init_schema().await?;
        db.start_maintenance().await;
        Ok(db)
    }

    /// 在后台跑启动维护：把旧库迁移成新结构（[`migrations`]）。
    /// 只等它开始，不等它做完。
    ///
    /// 连接一次只跑一个任务，按排队顺序跑。维护开始以后才返回，之后的读写都排在它后面，
    /// 读不到迁移到一半的表。
    async fn start_maintenance(&self) {
        let (started_tx, started) = tokio::sync::oneshot::channel();
        let conn = self.0.clone();
        tokio::spawn(async move {
            let done = conn
                .call(move |c| {
                    let _ = started_tx.send(());
                    migrations::to_session_frames(c)?;
                    Ok(())
                })
                .await;
            if let Err(e) = done {
                log::warn!("memory: maintenance failed: {e}");
            }
        });
        // 连接已经关了的话任务不会开始，这里收到的是错误，照样返回。
        let _ = started.await;
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
                // 启动时的库世代:v5(消息树)的存量串链回填只能跑一次,
                // 必须以"进程看到的旧版本"门控——下面的幂等批次会把
                // user_version 直接写成当前世代,不能事后再读。
                let prev_version: i64 = conn
                    .query_row("PRAGMA user_version", [], |r| r.get(0))
                    .db()?;
                conn.execute_batch("PRAGMA journal_mode = WAL;").db()?;
                conn.execute_batch(FRAMES_TABLE).db()?;
                conn.execute_batch(SESSION_FRAMES_TABLE).db()?;
                conn.execute_batch(
                    "-- L3 文本会话(折叠后的检索单元)
                    CREATE TABLE IF NOT EXISTS text_sessions (
                        id INTEGER PRIMARY KEY,
                        local_date TEXT NOT NULL,
                        started_ts TEXT NOT NULL,
                        ended_ts   TEXT NOT NULL,
                        app_id TEXT,
                        title  TEXT,
                        text   TEXT NOT NULL DEFAULT '',  -- 会话里的各行，按行号用换行连起来
                        guid   TEXT,                      -- 跨设备全局 id(可选上云用)
                        origin_device TEXT                -- NULL=本机;非 NULL=来自该设备
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

                    -- Chat 会话(默认只在本地;guid 供可选上云时跨设备合并,
                    -- deleted_at 软删让删除能传播到其它设备)
                    -- 定时补识别的持久记账:mark = 'YYYY-MM-DD@HH:MM',
                    -- 每个时间点每天一行;重启不丢,每天至多一次跨进程成立。
                    -- 旧日期行由调度器写入新行时顺手清理。
                    CREATE TABLE IF NOT EXISTS scheduled_ocr_marks (
                        mark TEXT PRIMARY KEY
                    );

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
                        completion_tokens INTEGER,       -- 本轮下行 token(assistant)
                        parent_guid     TEXT,            -- 消息树父指针;NULL = 会话根
                        reasoning_tokens  INTEGER,       -- 其中思考消耗(已含在 completion 内)
                        elapsed_ms        INTEGER        -- 本轮墙钟耗时(含工具执行,非纯思考)
                    );
                    CREATE INDEX IF NOT EXISTS idx_chat_messages_conv
                        ON chat_messages(conversation_id, id);

                    PRAGMA user_version = 5;",
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
                    "ALTER TABLE chat_messages ADD COLUMN parent_guid TEXT",
                    "ALTER TABLE chat_messages ADD COLUMN reasoning_tokens INTEGER",
                    "ALTER TABLE chat_messages ADD COLUMN elapsed_ms INTEGER",
                    "ALTER TABLE text_sessions ADD COLUMN guid TEXT",
                    // 会话来源设备:NULL = 本机产出;非 NULL = 从该设备同步而来
                    "ALTER TABLE text_sessions ADD COLUMN origin_device TEXT",
                ] {
                    if let Err(e) = conn.execute(sql, []) {
                        let msg = e.to_string();
                        if !msg.contains("duplicate column") {
                            return Err(DbError::Rusqlite(e));
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
                     PRAGMA user_version = 5;",
                )
                .db()?;
                // ── v5 一次性迁移:存量线性消息串成父子链(树的平凡形态)──
                // 只在旧世代跑:树世代里 parent_guid 为 NULL 是"会话根/根的
                // 编辑分支"的合法状态,重跑会把它们错误串到别的消息底下。
                if prev_version < 5 {
                    conn.execute(
                        "UPDATE chat_messages SET parent_guid =
                             (SELECT m2.guid FROM chat_messages m2
                               WHERE m2.conversation_id = chat_messages.conversation_id
                                 AND m2.id < chat_messages.id
                               ORDER BY m2.id DESC LIMIT 1)
                           WHERE parent_guid IS NULL",
                        [],
                    )
                    .db()?;
                }
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

    fn columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
            .unwrap();
        stmt.query_map([table], |r| r.get(0))
            .unwrap()
            .map(|name| name.unwrap())
            .collect()
    }

    /// 新库直接是 ADR-0021 的结构：frames 有 id、没有 local_date，没有 session_lines。
    #[tokio::test]
    async fn new_database_has_the_new_layout() {
        let db = MemoryDb::open_in_memory().await.unwrap();
        db.0.call(|conn| {
            assert_eq!(
                columns(conn, "frames"),
                [
                    "id",
                    "path",
                    "ts",
                    "app_id",
                    "title",
                    "ocr_state",
                    "attempts",
                    "session_id"
                ]
            );
            assert_eq!(
                columns(conn, "session_frames"),
                ["session_id", "first_line_no", "frame_id"]
            );
            assert!(columns(conn, "session_lines").is_empty());
            // 新库上迁移什么都不做
            assert!(!migrations::to_session_frames(conn).unwrap());
            Ok(())
        })
        .await
        .unwrap();
    }

    /// 打开旧库后马上读写：排在迁移后面，读到的已经是新结构。
    #[tokio::test]
    async fn calls_after_open_wait_for_the_migration() {
        let dir = std::env::temp_dir().join(format!(
            "hindsight-memory-migrate-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("memory.sqlite");
        {
            let old = rusqlite::Connection::open(&path).unwrap();
            old.execute_batch(
                "CREATE TABLE frames (
                     path TEXT PRIMARY KEY, ts TEXT NOT NULL, local_date TEXT NOT NULL,
                     app_id TEXT, title TEXT,
                     ocr_state INTEGER NOT NULL DEFAULT 0, attempts INTEGER NOT NULL DEFAULT 0,
                     session_id INTEGER);
                 CREATE TABLE session_lines (
                     session_id INTEGER NOT NULL, line_no INTEGER NOT NULL, text TEXT NOT NULL,
                     first_path TEXT NOT NULL, first_ts TEXT NOT NULL,
                     PRIMARY KEY (session_id, line_no));
                 INSERT INTO frames VALUES
                     ('/shots/a.jpg', '2026-11-20T09:00:00+09:00', '2026-11-20', 'editor', 't', 1, 0, 1),
                     ('/shots/b.jpg', '2026-11-20T09:00:05+09:00', '2026-11-20', 'editor', 't', 0, 0, NULL);
                 INSERT INTO session_lines VALUES
                     (1, 0, 'alpha line', '/shots/a.jpg', '2026-11-20T09:00:00+09:00');",
            )
            .unwrap();
        }

        let db = MemoryDb::open_at(&path).await.unwrap();
        // 第一个调用：取待识别截图，用的是新结构的列
        let pending = frames::take_pending(&db, 10).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].local_date(), "2026-11-20");
        let (lines_left, runs) =
            db.0.call(|conn| {
                Ok((
                    columns(conn, "session_lines").len(),
                    conn.query_row("SELECT COUNT(*) FROM session_frames", [], |r| {
                        r.get::<_, i64>(0)
                    })?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(lines_left, 0);
        assert_eq!(runs, 1);
        db.0.close().await.unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 记忆库按 WAL + NORMAL 打开，和主库一样。
    #[tokio::test]
    async fn open_at_uses_wal_and_normal_sync() {
        let dir =
            std::env::temp_dir().join(format!("hindsight-memory-open-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let db = MemoryDb::open_at(&dir.join("memory.sqlite")).await.unwrap();
        let (mode, sync): (String, i64) =
            db.0.call(|c| {
                let mode = c.query_row("PRAGMA journal_mode", [], |r| r.get(0)).db()?;
                let sync = c.query_row("PRAGMA synchronous", [], |r| r.get(0)).db()?;
                Ok((mode, sync))
            })
            .await
            .unwrap();
        assert_eq!(mode, "wal");
        assert_eq!(sync, 1, "1 = NORMAL");
        db.0.close().await.unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }
}
