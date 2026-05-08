//! `process_paths` 表：从 process_name 到 exe 绝对路径的映射。
//! 用于本机图标提取（GDI / plist 需要 exe 路径）+ 调试。
//!
//! capture 路径每次见到一个进程时调用 [`upsert`] 把当前 exe 路径登记。
//! 路径没变时不写 outbox（高频心跳级噪声过滤）。

use chrono::{Local, Utc};

use crate::error::Result;
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

/// 登记 / 更新某 process_name 对应的 exe 路径。
/// 路径未变时仅刷 seen_at，不写 outbox。
pub async fn upsert(pool: &DbPool, process_name: &str, exe_path: &str) -> Result<()> {
    let p = process_name.to_string();
    let e = exe_path.to_string();
    let seen = Local::now().to_rfc3339();
    let updated = Utc::now().to_rfc3339();
    let p_clone = p.clone();
    let e_clone = e.clone();
    let seen_clone = seen.clone();
    let updated_clone = updated.clone();

    pool.0
        .call(move |conn| {
            // 上次写入的 exe_path —— 如果路径没变就不必写 outbox（避免高频心跳级噪声）
            let prev: Option<String> = conn
                .query_row(
                    "SELECT exe_path FROM process_paths WHERE process_name = ?",
                    rusqlite::params![p_clone],
                    |r| r.get(0),
                )
                .ok();

            conn.execute(
                "INSERT INTO process_paths(process_name, exe_path, seen_at, updated_at)
                 VALUES(?, ?, ?, ?)
                 ON CONFLICT(process_name) DO UPDATE SET
                   exe_path = excluded.exe_path,
                   seen_at = excluded.seen_at,
                   updated_at = excluded.updated_at",
                rusqlite::params![p_clone, e_clone, seen_clone, updated_clone],
            )
            .db()?;

            let path_changed = prev.as_deref() != Some(&e_clone);
            if path_changed {
                let payload = serde_json::json!({
                    "processName": p_clone,
                    "exePath": e_clone,
                    "seenAt": seen_clone,
                    "updatedAt": updated_clone,
                })
                .to_string();
                enqueue(
                    conn,
                    OutboxOp::Upsert,
                    OutboxEntity::ProcessPath,
                    &p_clone,
                    &payload,
                )
                .db()?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}

/// 查某 process_name 当前的 exe 路径；表里没有返回 None。
pub async fn get_path(pool: &DbPool, process_name: &str) -> Result<Option<String>> {
    let p = process_name.to_string();
    let path = pool
        .0
        .call(move |conn| {
            let r = conn
                .query_row(
                    "SELECT exe_path FROM process_paths WHERE process_name = ?",
                    [&p],
                    |row| row.get::<_, String>(0),
                )
                .ok();
            Ok(r)
        })
        .await?;
    Ok(path)
}
