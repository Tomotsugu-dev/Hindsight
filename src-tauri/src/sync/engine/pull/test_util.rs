//! 几个种类的合并测试共用的 fixture 和查询 helper。

use crate::storage::{DbPool, SqliteResultExt};

/// 三个梯度时间戳:T_OLD < T_MID < T_NEW,期望值全部手推。
pub(super) const T_OLD: &str = "2026-06-01T00:00:00Z";
pub(super) const T_MID: &str = "2026-06-02T00:00:00Z";
pub(super) const T_NEW: &str = "2026-06-03T00:00:00Z";

/// 通用 fixture:一条参数化 SQL(全 String 参数)直写表。
pub(super) async fn exec_sql(pool: &DbPool, sql: &'static str, params: Vec<Option<String>>) {
    pool.0
        .call(move |conn| {
            let refs: Vec<&dyn rusqlite::ToSql> =
                params.iter().map(|p| p as &dyn rusqlite::ToSql).collect();
            conn.execute(sql, refs.as_slice()).db()?;
            Ok(())
        })
        .await
        .unwrap();
}

pub(super) fn s(v: &str) -> Option<String> {
    Some(v.to_string())
}

/// 读一行的若干 TEXT 列(逐列 Option<String>),行不存在返回 None。
pub(super) async fn read_row(
    pool: &DbPool,
    sql: &'static str,
    key: &str,
    cols: usize,
) -> Option<Vec<Option<String>>> {
    let key = key.to_string();
    pool.0
        .call(move |conn| {
            use rusqlite::OptionalExtension;
            let r = conn
                .query_row(sql, rusqlite::params![key], |r| {
                    (0..cols).map(|i| r.get::<_, Option<String>>(i)).collect()
                })
                .optional()
                .db()?;
            Ok(r)
        })
        .await
        .unwrap()
}

/// outbox 现有行的 (entity, entity_pk) 列表,按 id 序 —— 断言回灌行为用。
pub(super) async fn outbox_entries(pool: &DbPool) -> Vec<(String, String)> {
    pool.0
        .call(|conn| {
            let mut stmt = conn
                .prepare("SELECT entity, entity_pk FROM sync_outbox ORDER BY id")
                .db()?;
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await
        .unwrap()
}
