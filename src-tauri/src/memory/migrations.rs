//! Migrations of the screen memory database's tables.
//!
//! They change a database made by an older version to the current layout. Each migration looks at
//! the tables to decide whether it is needed, so running it again does nothing. Each one runs in a
//! single transaction.

use std::time::Instant;

use rusqlite::Connection;

use super::{FRAMES_TABLE, SESSION_FRAMES_TABLE};
use crate::storage::{DbResult, SqliteResultExt};

/// Migrates the screen memory database from `session_lines` to `session_frames` (ADR-0021).
///
/// In a single transaction, it adds an explicit integer primary key `id` to `frames` and drops
/// `local_date`, merges the lines of `session_lines` into runs in `session_frames`, and then drops
/// the old tables. If the database has no `session_lines`, it does nothing.
///
/// Returns `Ok(true)` if it migrated, and `Ok(false)` if the database already has the new layout.
/// If it fails, the transaction rolls back, and the database stays as it was.
#[cfg_attr(not(test), allow(dead_code))]
pub fn to_session_frames(conn: &mut Connection) -> DbResult<bool> {
    let tx = conn.transaction().db()?;
    // A database that still has session_lines has the old layout and needs the migration.
    let old: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master
                            WHERE type = 'table' AND name = 'session_lines')",
            [],
            |r| r.get(0),
        )
        .db()?;
    if !old {
        return Ok(false);
    }
    let started = Instant::now();

    // SQLite cannot change a table's primary key, so the old table is renamed and a new one is made.
    // The index keeps its name and stays with `frames_old`. If it were not dropped first,
    // `CREATE INDEX IF NOT EXISTS` in FRAMES_TABLE would be skipped, and the new table would lose
    // the index when the old table is dropped.
    tx.execute_batch(
        "DROP INDEX IF EXISTS idx_frames_state;
         ALTER TABLE frames RENAME TO frames_old;",
    )
    .db()?;
    tx.execute_batch(FRAMES_TABLE).db()?;
    tx.execute_batch(SESSION_FRAMES_TABLE).db()?;
    tx.execute(
        "INSERT INTO frames(path, ts, app_id, title, ocr_state, attempts, session_id)
         SELECT path, ts, app_id, title, ocr_state, attempts, session_id
           FROM frames_old ORDER BY rowid",
        [],
    )
    .db()?;
    // Merge session_lines by (session, screenshot path first_path): for each screenshot, keep only
    // the first line number of its text, and look up its integer frame_id in frames. LEFT JOIN
    // keeps the runs whose screenshot is no longer in frames (frame_id is NULL). If they were
    // dropped, their lines would belong to the screenshot before them.
    let runs = tx
        .execute(
            "INSERT INTO session_frames(session_id, first_line_no, frame_id)
             SELECT l.session_id, MIN(l.line_no) AS first_line_no, f.id
               FROM session_lines l LEFT JOIN frames f ON f.path = l.first_path
              GROUP BY l.session_id, l.first_path
              ORDER BY l.session_id, first_line_no",
            [],
        )
        .db()?;
    let without_frame: u64 = tx
        .query_row(
            "SELECT COUNT(*) FROM session_frames WHERE frame_id IS NULL",
            [],
            |r| r.get(0),
        )
        .db()?;
    tx.execute_batch("DROP TABLE session_lines; DROP TABLE frames_old;")
        .db()?;
    tx.commit().db()?;

    log::info!(
        "memory: session_lines replaced by session_frames: {runs} runs, {without_frame} without a frame, {} ms",
        started.elapsed().as_millis()
    );
    Ok(true)
}

#[cfg(test)]
mod tests {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use rusqlite::types::Value;
    use rusqlite::{params, OptionalExtension};

    use super::*;
    use crate::memory::MemoryDb;
    use crate::storage::SqliteResultExt;

    /// 换成 0.12 的两张表，再放进假数据：两个会话、五张截图。会话 1 的第 0、1 行来自 a，第 2 行
    /// 来自 b；会话 2 的第 0 行来自 c。d 待识别，e 识别失败，都没有行。
    const OLD_TABLES: &str = "
        DROP TABLE IF EXISTS session_lines;
        DROP TABLE frames;
        CREATE TABLE frames (
            path        TEXT PRIMARY KEY,
            ts          TEXT NOT NULL,
            local_date  TEXT NOT NULL,
            app_id      TEXT,
            title       TEXT,
            ocr_state   INTEGER NOT NULL DEFAULT 0,
            attempts    INTEGER NOT NULL DEFAULT 0,
            session_id  INTEGER
        );
        CREATE INDEX idx_frames_state ON frames(ocr_state, ts);
        CREATE TABLE session_lines (
            session_id INTEGER NOT NULL,
            line_no    INTEGER NOT NULL,
            text       TEXT NOT NULL,
            first_path TEXT NOT NULL,
            first_ts   TEXT NOT NULL,
            PRIMARY KEY (session_id, line_no)
        );
        INSERT INTO text_sessions(id, local_date, started_ts, ended_ts, app_id, title, text) VALUES
            (1, '2026-11-20', '2026-11-20T09:00:00+09:00', '2026-11-20T09:00:05+09:00',
             'editor', 'notes.txt', 'alpha line' || char(10) || 'beta line' || char(10) || 'gamma line'),
            (2, '2026-11-20', '2026-11-20T10:00:00+09:00', '2026-11-20T10:00:00+09:00',
             'browser', NULL, 'delta line');
        INSERT INTO frames VALUES
            ('/shots/a.jpg', '2026-11-20T09:00:00+09:00', '2026-11-20', 'editor', 'notes.txt', 1, 0, 1),
            ('/shots/b.jpg', '2026-11-20T09:00:05+09:00', '2026-11-20', 'editor', 'notes.txt', 1, 0, 1),
            ('/shots/c.jpg', '2026-11-20T10:00:00+09:00', '2026-11-20', 'browser', NULL, 1, 1, 2),
            ('/shots/d.jpg', '2026-11-20T11:00:00+09:00', '2026-11-20', 'editor', NULL, 0, 0, NULL),
            ('/shots/e.jpg', '2026-11-20T11:00:05+09:00', '2026-11-20', NULL, NULL, 2, 3, NULL);
        INSERT INTO session_lines VALUES
            (1, 0, 'alpha line', '/shots/a.jpg', '2026-11-20T09:00:00+09:00'),
            (1, 1, 'beta line',  '/shots/a.jpg', '2026-11-20T09:00:00+09:00'),
            (1, 2, 'gamma line', '/shots/b.jpg', '2026-11-20T09:00:05+09:00'),
            (2, 0, 'delta line', '/shots/c.jpg', '2026-11-20T10:00:00+09:00');";

    const OLD_LINES: &str = "SELECT session_id, line_no, text, first_path, first_ts
                               FROM session_lines ORDER BY session_id, line_no";
    /// frames 里迁移后还留着的列。
    const FRAMES: &str = "SELECT path, ts, app_id, title, ocr_state, attempts, session_id
                            FROM frames ORDER BY path";
    const TEXTS: &str = "SELECT id, text FROM text_sessions ORDER BY id";
    const SCHEMA: &str = "SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name";

    fn rows(conn: &Connection, sql: &str) -> Vec<Vec<Value>> {
        let mut stmt = conn.prepare(sql).unwrap();
        let n = stmt.column_count();
        stmt.query_map([], |r| (0..n).map(|i| r.get::<_, Value>(i)).collect())
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn columns(conn: &Connection, table: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
            .unwrap();
        stmt.query_map([table], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn count(conn: &Connection, sql: &str) -> i64 {
        conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    /// 旧表里每一行的会话、行号和它的截图（路径、拍摄时间）。
    fn old_line_frames(conn: &Connection) -> Vec<(i64, i64, String, String)> {
        let mut stmt = conn
            .prepare(
                "SELECT session_id, line_no, first_path, first_ts
                   FROM session_lines ORDER BY session_id, line_no",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// 新表里会话 `session` 第 `line_no` 行的截图（路径、拍摄时间）：取起始行号不超过它的最后一段。
    fn frame_of_line(conn: &Connection, session: i64, line_no: i64) -> Option<(String, String)> {
        conn.query_row(
            "SELECT f.path, f.ts
               FROM (SELECT frame_id FROM session_frames
                      WHERE session_id = ?1 AND first_line_no <= ?2
                      ORDER BY first_line_no DESC LIMIT 1) s
               LEFT JOIN frames f ON f.id = s.frame_id",
            params![session, line_no],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .unwrap()
        .and_then(|(path, ts): (Option<String>, Option<String>)| path.zip(ts))
    }

    /// 建一个旧结构的记忆库，在它的连接上跑 `check`。
    async fn on_old_database(check: impl FnOnce(&mut Connection) + Send + 'static) {
        let db = MemoryDb::open_in_memory().await.unwrap();
        db.0.call(move |conn| {
            conn.execute_batch(OLD_TABLES).db()?;
            check(conn);
            Ok(())
        })
        .await
        .unwrap();
    }

    /// 迁移后每一行还对应原来那张截图，路径和时间一字不差；会话文字和 frames 的数据不变。
    #[tokio::test]
    async fn migration_keeps_every_line_with_its_frame() {
        on_old_database(|conn| {
            let lines = old_line_frames(conn);
            let texts = rows(conn, TEXTS);
            let frames = rows(conn, FRAMES);

            assert!(to_session_frames(conn).unwrap());

            for (session, line_no, path, ts) in lines {
                assert_eq!(
                    frame_of_line(conn, session, line_no),
                    Some((path, ts)),
                    "会话 {session} 第 {line_no} 行"
                );
            }
            assert_eq!(rows(conn, TEXTS), texts);
            assert_eq!(rows(conn, FRAMES), frames);
        })
        .await;
    }

    /// 迁移后的表结构：每张截图一段，旧表都不在了。
    #[tokio::test]
    async fn migration_builds_the_new_layout() {
        on_old_database(|conn| {
            assert!(to_session_frames(conn).unwrap());

            assert_eq!(
                columns(conn, "frames"),
                ["id", "path", "ts", "app_id", "title", "ocr_state", "attempts", "session_id"]
            );
            assert_eq!(
                columns(conn, "session_frames"),
                ["session_id", "first_line_no", "frame_id"]
            );
            // 4 行文字来自 3 张截图
            assert_eq!(count(conn, "SELECT COUNT(*) FROM session_frames"), 3);
            // id 是显式的整数主键，VACUUM 不会给它重新编号
            assert_eq!(
                count(
                    conn,
                    "SELECT pk FROM pragma_table_info('frames') WHERE name = 'id' AND type = 'INTEGER'"
                ),
                1
            );
            // 按主键存，没有隐式 rowid
            assert!(conn.prepare("SELECT rowid FROM session_frames").is_err());
            // 原来的索引还在新表上，路径仍然唯一
            assert_eq!(
                count(
                    conn,
                    "SELECT COUNT(*) FROM sqlite_master
                      WHERE type = 'index' AND name = 'idx_frames_state' AND tbl_name = 'frames'"
                ),
                1
            );
            assert!(conn
                .execute(
                    "INSERT INTO frames(path, ts) VALUES ('/shots/a.jpg', 't')",
                    []
                )
                .is_err());
            // session_lines 和改名留下的旧表都删掉了
            assert_eq!(
                count(
                    conn,
                    "SELECT COUNT(*) FROM sqlite_master
                      WHERE name = 'session_lines' OR name GLOB '*_old'"
                ),
                0
            );
        })
        .await;
    }

    /// 截图已经不在 frames 里的那一段：不显示截图，也不算到前一张截图头上。
    #[tokio::test]
    async fn lines_whose_frame_is_gone_show_no_frame() {
        on_old_database(|conn| {
            conn.execute_batch(
                "INSERT INTO session_lines
                 VALUES (1, 3, 'epsilon line', '/shots/gone.jpg', '2026-11-20T09:00:09+09:00');
                 UPDATE text_sessions SET text = text || char(10) || 'epsilon line' WHERE id = 1;",
            )
            .unwrap();

            assert!(to_session_frames(conn).unwrap());

            assert_eq!(frame_of_line(conn, 1, 3), None);
            assert_eq!(
                frame_of_line(conn, 1, 2),
                Some(("/shots/b.jpg".into(), "2026-11-20T09:00:05+09:00".into()))
            );
        })
        .await;
    }

    /// 迁移在最后一步失败：表结构和数据都回到迁移前，下次还能重来。
    #[tokio::test]
    async fn failed_migration_changes_nothing() {
        on_old_database(|conn| {
            let schema = rows(conn, SCHEMA);
            let lines = rows(conn, OLD_LINES);
            let frames = rows(conn, "SELECT * FROM frames ORDER BY path");

            // 拒绝删表：新表已经建好、数据已经抄过去之后才失败
            conn.authorizer(Some(|ctx: AuthContext<'_>| match ctx.action {
                AuthAction::DropTable { .. } => Authorization::Deny,
                _ => Authorization::Allow,
            }))
            .unwrap();
            assert!(to_session_frames(conn).is_err());
            conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
                .unwrap();

            assert!(conn.is_autocommit(), "事务已经结束");
            assert_eq!(rows(conn, SCHEMA), schema);
            assert_eq!(rows(conn, OLD_LINES), lines);
            assert_eq!(rows(conn, "SELECT * FROM frames ORDER BY path"), frames);

            // 下次启动重试
            assert!(to_session_frames(conn).unwrap());
        })
        .await;
    }

    /// 要不要迁移只看 session_lines 在不在：旧版本写回的 user_version 不影响判断。
    #[tokio::test]
    async fn layout_decides_the_migration_not_user_version() {
        on_old_database(|conn| {
            // 旧结构，版本号不是 5 也迁移
            conn.execute_batch("PRAGMA user_version = 6").unwrap();
            assert!(to_session_frames(conn).unwrap());

            // 版本号写回 5，表已经是新结构
            conn.execute_batch("PRAGMA user_version = 5").unwrap();
            let schema = rows(conn, SCHEMA);
            assert!(!to_session_frames(conn).unwrap());
            assert_eq!(rows(conn, SCHEMA), schema);
        })
        .await;
    }
}
