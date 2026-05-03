//! 跨设备同步的 app icon 数据。
//!
//! 本机提取出来的 PNG 字节存这里 + 文件 cache + outbox；其它设备 pull 后也写这里。
//! 读取时 process_name 精确匹配 —— Win 和 mac 进程名不冲突，各自上传各自的，对方拿到
//! 后能给从那台设备同步过来的 activity 行渲染出图标。

use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::OptionalExtension;

use crate::error::Result;
use crate::repo::outbox::{enqueue, OutboxEntity, OutboxOp};
use crate::storage::{db_path_dir, DbPool};

/// 文件 cache 路径：`<data_root>/icons/<sanitized>.png`。
/// process_name 里的非 ASCII alnum/. /-/_ 字符替换成 `_`，避免文件名歧义。
pub fn icon_cache_path(process_name: &str) -> Result<PathBuf> {
    let dir = db_path_dir()?.join("icons");
    Ok(dir.join(format!("{}.png", sanitize(process_name))))
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// 把字节写到 cache 文件位置（自动建目录），失败 log 一下不上抛。
pub fn write_cache_file(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, bytes);
}

/// 写入本机刚提取的 icon：upsert app_icons 行，并且同事务入 outbox。
pub async fn upsert_local(pool: &DbPool, process_name: &str, icon_png: &[u8]) -> Result<()> {
    let p = process_name.to_string();
    let bytes = icon_png.to_vec();
    let updated = Utc::now().to_rfc3339();

    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO app_icons(process_name, icon_png, updated_at, deleted_at)
                 VALUES(?, ?, ?, NULL)
                 ON CONFLICT(process_name) DO UPDATE SET
                   icon_png   = excluded.icon_png,
                   updated_at = excluded.updated_at,
                   deleted_at = NULL",
                rusqlite::params![p, bytes, updated],
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;

            // outbox payload 用不到 BLOB 内容，build 时会重新去 DB 查 —— 这里只放 process_name
            // 让 group_outbox 能定位到 (DirtyKey::AppIcons)。
            let payload = serde_json::json!({ "processName": p }).to_string();
            enqueue(
                conn,
                OutboxOp::Upsert,
                OutboxEntity::AppIcon,
                &p,
                &payload,
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// 读出 app_icons 表里某个 process_name 对应的 PNG 字节（未软删才返）。
pub async fn get_blob(pool: &DbPool, process_name: &str) -> Result<Option<Vec<u8>>> {
    let p = process_name.to_string();
    let bytes = pool
        .0
        .call(move |conn| {
            let r = conn
                .query_row(
                    "SELECT icon_png FROM app_icons
                     WHERE process_name = ? AND deleted_at IS NULL",
                    [&p],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .optional()
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(r)
        })
        .await?;
    Ok(bytes)
}
