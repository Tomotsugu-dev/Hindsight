//! 工具测试共用：内存里的主库和记忆库。
//!
//! 各工具的行为测试用它直接调 `execute()`，盯结果里的「披露」：
//! 是全量、抽样，还是只列了前几条命中。

use chrono::NaiveDate;

use tokio::sync::OnceCell;

use super::visible::Visibility;
use super::ToolCtx;
use crate::storage::Connection;

/// 开两个内存库，建好工具查询用到的表，再跑 `mem_sql` 和 `main_sql` 造数据。
pub(super) async fn ctx_with(mem_sql: &'static str, main_sql: &'static str) -> ToolCtx {
    let mem = Connection::open(":memory:").await.unwrap();
    mem.call(move |c| {
        c.execute_batch(&format!(
            "CREATE TABLE text_sessions (
                 id INTEGER PRIMARY KEY, local_date TEXT, started_ts TEXT,
                 ended_ts TEXT, app_id TEXT, title TEXT, text TEXT DEFAULT '');
             CREATE VIRTUAL TABLE text_sessions_fts USING fts5(
                 text, content='text_sessions', content_rowid='id', tokenize='trigram');
             CREATE TABLE session_lines (
                 session_id INTEGER, line_no INTEGER, text TEXT,
                 first_path TEXT, first_ts TEXT);
             CREATE TABLE frames (
                 path TEXT PRIMARY KEY, ts TEXT, local_date TEXT,
                 ocr_state INTEGER NOT NULL DEFAULT 0);
             {mem_sql}
             INSERT INTO text_sessions_fts(rowid, text)
                 SELECT id, text FROM text_sessions;"
        ))?;
        Ok(())
    })
    .await
    .unwrap();
    let main = Connection::open(":memory:").await.unwrap();
    main.call(move |c| {
        c.execute_batch(&format!(
            "CREATE TABLE activities (
                 started_at TEXT, ended_at TEXT, duration_secs INTEGER,
                 local_date TEXT, local_hour INTEGER,
                 process_name TEXT, window_title TEXT, screenshot_path TEXT,
                 category_id TEXT, excluded INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE app_group_members (
                 process_name TEXT, group_id TEXT, deleted_at TEXT);
             CREATE TABLE app_groups (
                 id TEXT, display_name TEXT, category_id TEXT, deleted_at TEXT);
             CREATE TABLE categories (
                 id TEXT, name TEXT, deleted_at TEXT);
             {main_sql}"
        ))?;
        Ok(())
    })
    .await
    .unwrap();
    // 这里的主库没有设置表，读不出可见范围：什么都给看
    ToolCtx {
        main,
        mem,
        visible: OnceCell::from(Visibility::default()),
    }
}

/// 用 `main` 和一个新的记忆库组一个 `ToolCtx`。两个库都跑过真实的建表。
///
/// `main` 用 [`fresh_test_pool`](crate::repo::test_util::fresh_test_pool) 开。
/// 可见范围第一次用时才读，分组、分类、忽略规则要在那之前造好。
pub(super) async fn real_ctx(main: &crate::storage::DbPool) -> ToolCtx {
    let mem = crate::memory::MemoryDb::open_in_memory().await.unwrap();
    ToolCtx {
        main: main.0.clone(),
        mem: mem.0,
        visible: OnceCell::new(),
    }
}

pub(super) fn day() -> (NaiveDate, NaiveDate) {
    let d = NaiveDate::from_ymd_opt(2026, 7, 8).unwrap();
    (d, d)
}
