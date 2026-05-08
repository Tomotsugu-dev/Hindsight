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
use crate::storage::SqliteResultExt;
use crate::storage::{db_path_dir, DbPool};

/// 文件 cache 路径：`<data_root>/icons/<sanitized>.png`。
/// process_name 里的非 ASCII alnum/. /-/_ 字符替换成 `_`，避免文件名歧义。
pub fn icon_cache_path(process_name: &str) -> Result<PathBuf> {
    let dir = db_path_dir()?.join("icons");
    Ok(dir.join(format!("{}.png", sanitize(process_name))))
}

/// 把 process_name 转成可作为文件名的字符串。非 ASCII / 非安全字符按 codepoint
/// 编码成 `uXX-`（hex + `-`分隔），保证不同字符不冲突，且不产生连续 `_`。
///
/// 历史 bug：之前所有非 ASCII 字符都换成 `_`，导致"提示" / "音乐" / "微信" 等
/// 任意 2 字符纯中文 app 都共用 `__.png`，后写的覆盖前写的，前端图标互相串。
fn sanitize(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push_str(&format!("u{:X}-", c as u32));
        }
    }
    out
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
            .db()?;

            // outbox payload 用不到 BLOB 内容，build 时会重新去 DB 查 —— 这里只放 process_name
            // 让 group_outbox 能定位到 (DirtyKey::AppIcons)。
            let payload = serde_json::json!({ "processName": p }).to_string();
            enqueue(conn, OutboxOp::Upsert, OutboxEntity::AppIcon, &p, &payload).db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// 启动时一次性 backfill：把老用户已经在文件 cache 里、但 app_icons 表里没记录的
/// 图标灌进 DB。否则这些"开启同步前提取过的图标"永远不会入 outbox，对端永远拉不到。
///
/// 顺序：
///   1. 列所有 process_paths.process_name
///   2. 对每一个，看 app_icons 是否已经有 active 行 —— 有就跳过
///   3. 优先用文件 cache（直接读 PNG 字节）
///   4. fallback 到 exe 提取（SHGetFileInfo / icns），同步阻塞所以放 spawn_blocking
///   5. 拿到字节 → upsert_local（写 DB + 入 outbox + 写文件 cache 由 upsert_local 的调用者负责，
///      但 backfill 场景文件 cache 已经在，跳过这一步）
///
/// 返回新写入的行数。每次启动都跑（已存在的会跳过，开销 = 一遍 SQL 查询）。
pub async fn backfill_db_from_cache_or_extract(pool: &DbPool) -> Result<usize> {
    let process_names: Vec<String> = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare("SELECT process_name FROM process_paths")
                .db()?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).db()?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.db()?);
            }
            Ok(out)
        })
        .await?;

    let mut added = 0usize;
    for name in process_names {
        // 已经有 active 行就跳过（包括从对端 sync 拉过来的）
        let p = name.clone();
        let exists: bool = pool
            .0
            .call(move |conn| {
                let r = conn
                    .query_row(
                        "SELECT 1 FROM app_icons
                         WHERE process_name = ?1 AND deleted_at IS NULL",
                        rusqlite::params![p],
                        |_| Ok(true),
                    )
                    .optional()
                    .db()?
                    .unwrap_or(false);
                Ok(r)
            })
            .await
            .unwrap_or(false);
        if exists {
            continue;
        }

        // 1) 文件 cache 命中 → 读字节
        let cache_path = match icon_cache_path(&name) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let bytes_opt: Option<Vec<u8>> = if cache_path.exists() {
            std::fs::read(&cache_path).ok()
        } else {
            // 2) exe 提取 fallback
            let exe = match crate::repo::process_paths::get_path(pool, &name).await {
                Ok(Some(p)) if !p.is_empty() => std::path::PathBuf::from(p),
                _ => continue,
            };
            match tokio::task::spawn_blocking(move || crate::icons::extract_png(&exe)).await {
                Ok(Ok(Some(bytes))) => {
                    write_cache_file(&cache_path, &bytes);
                    Some(bytes)
                }
                _ => None,
            }
        };

        if let Some(bytes) = bytes_opt {
            if upsert_local(pool, &name, &bytes).await.is_ok() {
                added += 1;
            }
        }
    }

    Ok(added)
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
                .db()?;
            Ok(r)
        })
        .await?;
    Ok(bytes)
}
