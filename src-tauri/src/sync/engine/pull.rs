//! Pull 路径：列 Drive 文件，按 modifiedTime 增量下载，按文件名分发到 merge_*。
//! merge_* 都做 LWW（updated_at 字典序比较）+ idempotent upsert。

use std::sync::Arc;

use chrono::Utc;
use serde_json::Value;

use super::io;
use super::Inner;
use crate::error::{Error, Result};
use crate::storage::DbPool;
use crate::sync::auth::{self, TokenInfo};
use crate::sync::drive;
use crate::db::SqliteResultExt;

const PULL_CURSOR_KEY: &str = "drive_files";

enum ParsedFile {
    ActivityDay { device_id: String },
    Categories { device_id: String },
    AppCategories { device_id: String },
    ProcessPaths { device_id: String },
    DeviceMeta { device_id: String },
    AppIcons { device_id: String },
    AppGroups { device_id: String },
    AppGroupMembers { device_id: String },
}

fn parse_filename(name: &str) -> Option<ParsedFile> {
    // 形如：device.<UUID>.<KIND>.json 或 device.<UUID>.activities.<DAY>.ndjson
    let parts: Vec<&str> = name.split('.').collect();
    if parts.first().copied() != Some("device") {
        return None;
    }
    match parts.as_slice() {
        ["device", uuid, "activities", _day, "ndjson"] => Some(ParsedFile::ActivityDay {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "categories", "json"] => Some(ParsedFile::Categories {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "app_categories", "json"] => Some(ParsedFile::AppCategories {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "process_paths", "json"] => Some(ParsedFile::ProcessPaths {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "meta", "json"] => Some(ParsedFile::DeviceMeta {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "icons", "json"] => Some(ParsedFile::AppIcons {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "app_groups", "json"] => Some(ParsedFile::AppGroups {
            device_id: uuid.to_string(),
        }),
        ["device", uuid, "app_group_members", "json"] => Some(ParsedFile::AppGroupMembers {
            device_id: uuid.to_string(),
        }),
        _ => None,
    }
}

pub(super) async fn flush_pull(inner: &Arc<Inner>) -> Result<()> {
    let token: TokenInfo = match auth::ensure_valid_token(&inner.pool).await {
        Ok(t) => t,
        Err(Error::NotSignedIn) => return Ok(()),
        Err(e) => {
            let msg = e.to_string();
            log::warn!("sync pull 拿不到有效 token: {msg}");
            inner.status.write().await.last_error = Some(msg);
            return Ok(());
        }
    };

    let cursor = io::read_cursor(&inner.pool, PULL_CURSOR_KEY).await?;
    let cursor_q = if cursor.starts_with("1970-") {
        String::new()
    } else {
        cursor.clone()
    };

    let files = drive::list_appdata_files(&token.access_token, &cursor_q).await?;
    if files.is_empty() {
        return Ok(());
    }

    let self_id = crate::device::self_id();
    let local_os = std::env::consts::OS;
    let mut max_modified: Option<String> = None;
    let mut applied = 0u64;

    // Pass 1: 只跑 device.meta.json，让 devices.os 在 Pass 2 之前就位。
    // 否则一台陌生设备首次出现时，我们读 devices.os 是空的，没法做跨 OS 过滤。
    for f in &files {
        // 比较 modifiedTime 用字符串字典序就够（RFC3339 都是 ISO 8601）
        if max_modified.as_ref().map_or(true, |c| f.modified_time.as_str() > c.as_str()) {
            max_modified = Some(f.modified_time.clone());
        }

        let parsed = match parse_filename(&f.name) {
            Some(p) => p,
            None => continue,
        };
        let ParsedFile::DeviceMeta { device_id } = parsed else {
            continue;
        };
        if device_id == self_id {
            continue;
        }
        let body = match drive::download(&token.access_token, &f.id).await {
            Ok(b) => b,
            Err(e) => {
                log::warn!("下载 {} 失败: {e}", f.name);
                continue;
            }
        };
        if let Err(e) = merge_device_meta(&inner.pool, &device_id, &body).await {
            log::warn!("merge {} 失败: {e}", f.name);
            continue;
        }
        applied += 1;
    }

    // Pass 2: 其余类型；对平台特定的两类做 OS 过滤。
    for f in &files {
        let parsed = match parse_filename(&f.name) {
            Some(p) => p,
            None => continue,
        };
        if matches!(parsed, ParsedFile::DeviceMeta { .. }) {
            continue;
        }
        let device_id = match &parsed {
            ParsedFile::ActivityDay { device_id, .. }
            | ParsedFile::Categories { device_id }
            | ParsedFile::AppCategories { device_id }
            | ParsedFile::ProcessPaths { device_id }
            | ParsedFile::AppIcons { device_id }
            | ParsedFile::AppGroups { device_id }
            | ParsedFile::AppGroupMembers { device_id } => device_id.as_str(),
            ParsedFile::DeviceMeta { .. } => unreachable!(),
        };
        if device_id == self_id {
            continue;
        }

        // app_categories / process_paths 是平台特定的：
        //   Windows tracker 写 process_name = "chrome.exe"，exe_path = "C:\\..."
        //   macOS tracker  写 process_name = "Google Chrome"，exe_path = "/Applications/.../MacOS/..."
        // 跨 OS 合并要么完全无用（key 对不上），要么坏事（同名 key 撞车，把本机能用的路径覆盖掉，icon 提取失败）。
        // activities / app_icons 不过滤 —— 跨设备聚合活动是核心价值；icon 字节就是要让对方
        // 给从那台机器同步过来的 activity 行渲染图标用的。
        if matches!(
            parsed,
            ParsedFile::AppCategories { .. } | ParsedFile::ProcessPaths { .. }
        ) {
            let remote_os = remote_device_os(&inner.pool, device_id).await;
            if remote_os.as_deref() != Some(local_os) {
                log::debug!(
                    "跳过跨 OS 文件 {} (远端 os={:?}, 本机 {})",
                    f.name,
                    remote_os,
                    local_os,
                );
                continue;
            }
        }

        let body = match drive::download(&token.access_token, &f.id).await {
            Ok(b) => b,
            Err(e) => {
                log::warn!("下载 {} 失败: {e}", f.name);
                continue;
            }
        };

        let res = match parsed {
            ParsedFile::ActivityDay { device_id, .. } => {
                merge_activities(&inner.pool, &device_id, &body).await
            }
            ParsedFile::Categories { device_id } => {
                merge_categories(&inner.pool, &device_id, &body).await
            }
            ParsedFile::AppCategories { device_id } => {
                merge_app_categories(&inner.pool, &device_id, &body).await
            }
            ParsedFile::ProcessPaths { device_id } => {
                merge_process_paths(&inner.pool, &device_id, &body).await
            }
            ParsedFile::AppIcons { device_id } => {
                merge_app_icons(&inner.pool, &device_id, &body).await
            }
            ParsedFile::AppGroups { device_id } => {
                merge_app_groups(&inner.pool, &device_id, &body).await
            }
            ParsedFile::AppGroupMembers { device_id } => {
                merge_app_group_members(&inner.pool, &device_id, &body).await
            }
            ParsedFile::DeviceMeta { .. } => unreachable!(),
        };
        if let Err(e) = res {
            log::warn!("merge {} 失败: {e}", f.name);
            continue;
        }
        applied += 1;
    }

    if let Some(t) = max_modified {
        io::write_cursor(&inner.pool, PULL_CURSOR_KEY, &t).await?;
    }
    inner.status.write().await.last_pulled_at = Some(Utc::now().to_rfc3339());
    if applied > 0 {
        log::info!("sync pull 完成，应用 {} 个远端文件", applied);
    }
    Ok(())
}

/// 查 devices 表里某个远端设备的 os；没有 device_meta 同步过来时返回 None。
async fn remote_device_os(pool: &DbPool, device_id: &str) -> Option<String> {
    let id = device_id.to_string();
    pool.0
        .call(move |conn| {
            let r = conn
                .query_row(
                    "SELECT os FROM devices WHERE device_id = ?1",
                    rusqlite::params![id],
                    |r| r.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten()
                .filter(|s| !s.is_empty());
            Ok(r)
        })
        .await
        .ok()
        .flatten()
}

async fn merge_activities(pool: &DbPool, device_id: &str, body: &[u8]) -> Result<()> {
    let s = std::str::from_utf8(body).map_err(Error::from)?;
    for (lineno, line) in s.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("activities 行 {lineno} 解析失败: {e}");
                continue;
            }
        };
        let local_id = v.get("id").and_then(|x| x.as_i64()).unwrap_or(-1);
        if local_id < 0 {
            continue;
        }
        let remote_id = local_id.to_string();
        let started_at = v.get("startedAt").and_then(|x| x.as_str()).unwrap_or("");
        let ended_at = v.get("endedAt").and_then(|x| x.as_str()).unwrap_or("");
        let duration_secs = v.get("durationSecs").and_then(|x| x.as_i64()).unwrap_or(0);
        let local_date = v.get("localDate").and_then(|x| x.as_str()).unwrap_or("");
        let local_hour = v.get("localHour").and_then(|x| x.as_i64()).unwrap_or(0) as u8;
        let process_name = v.get("processName").and_then(|x| x.as_str()).unwrap_or("");
        let window_title = v.get("windowTitle").and_then(|x| x.as_str()).unwrap_or("");
        let category_id = v
            .get("categoryId")
            .and_then(|x| x.as_str())
            .unwrap_or("other");
        let updated_at = v
            .get("updatedAt")
            .and_then(|x| x.as_str())
            .unwrap_or(ended_at);

        upsert_remote_activity(
            pool,
            device_id,
            &remote_id,
            started_at,
            ended_at,
            duration_secs,
            local_date,
            local_hour,
            process_name,
            window_title,
            category_id,
            updated_at,
        )
        .await?;
    }
    Ok(())
}

async fn merge_categories(pool: &DbPool, _device_id: &str, body: &[u8]) -> Result<()> {
    let arr: Vec<Value> = serde_json::from_slice(body).map_err(|e| Error::SyncParse { kind: "categories", source: e })?;
    for v in arr {
        let id = match v.get("id").and_then(|x| x.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let color = v.get("color").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let icon = v.get("icon").and_then(|x| x.as_str()).unwrap_or("Tag").to_string();
        let builtin = v.get("builtin").and_then(|x| x.as_bool()).unwrap_or(false);
        // 老对端推过来的 payload 没有 sortOrder 字段 → fallback 0；新行随后被本端
        // 重排操作覆盖即可。
        let sort_order = v.get("sortOrder").and_then(|x| x.as_i64()).unwrap_or(0);
        let updated_at = v
            .get("updatedAt")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let deleted_at = v
            .get("deletedAt")
            .and_then(|x| x.as_str())
            .map(String::from);

        pool.0
            .call(move |conn| {
                let cur: Option<(String, Option<String>)> = conn
                    .query_row(
                        "SELECT updated_at, deleted_at FROM categories WHERE id = ?1",
                        rusqlite::params![id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .ok();
                let should_apply = match &cur {
                    None => true,
                    Some((cur_upd, _)) => updated_at.as_str() > cur_upd.as_str(),
                };
                if !should_apply {
                    return Ok(());
                }
                let prev_deleted = cur.as_ref().and_then(|(_, d)| d.clone());

                if cur.is_none() {
                    conn.execute(
                        "INSERT INTO categories(id, name, color, icon, builtin, sort_order, updated_at, deleted_at)
                         VALUES(?, ?, ?, ?, ?, ?, ?, ?)",
                        rusqlite::params![id, name, color, icon, builtin as i64, sort_order, updated_at, deleted_at],
                    )
                    .db()?;
                } else {
                    conn.execute(
                        "UPDATE categories SET name = ?, color = ?, icon = ?, builtin = ?,
                                                sort_order = ?, updated_at = ?, deleted_at = ?
                         WHERE id = ?",
                        rusqlite::params![name, color, icon, builtin as i64, sort_order, updated_at, deleted_at, id],
                    )
                    .db()?;
                }

                // 远端把这个分类删了 —— 跑一次本地 cascade，让指向它的 app_categories /
                // app_groups 也跟着清掉，否则跨 OS 设备会卡在「分类已删但本地引用没清」状态。
                // 仅在「之前没删，现在变成删了」的边沿触发；同样的 deletion 同步重复到达，
                // should_apply / 本地 cascade SQL 的 WHERE 条件会让二次操作变成 no-op（幂等）。
                let just_deleted = deleted_at.is_some() && prev_deleted.is_none();
                if just_deleted {
                    crate::repo::categories::cascade_category_deletion(conn, &id, &updated_at)
                        .db()?;
                }
                Ok(())
            })
            .await?;
    }
    Ok(())
}

async fn merge_app_categories(pool: &DbPool, _device_id: &str, body: &[u8]) -> Result<()> {
    let arr: Vec<Value> = serde_json::from_slice(body).map_err(|e| Error::SyncParse { kind: "app_categories", source: e })?;
    for v in arr {
        let process_name = match v.get("processName").and_then(|x| x.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let category_id = v
            .get("categoryId")
            .and_then(|x| x.as_str())
            .unwrap_or("other")
            .to_string();
        let updated_at = v
            .get("updatedAt")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let deleted_at = v
            .get("deletedAt")
            .and_then(|x| x.as_str())
            .map(String::from);

        pool.0
            .call(move |conn| {
                let cur: Option<String> = conn
                    .query_row(
                        "SELECT updated_at FROM app_categories WHERE process_name = ?1",
                        rusqlite::params![process_name],
                        |r| r.get(0),
                    )
                    .ok();
                let should_apply = match &cur {
                    None => true,
                    Some(c) => updated_at.as_str() > c.as_str(),
                };
                if !should_apply {
                    return Ok(());
                }
                conn.execute(
                    "INSERT INTO app_categories(process_name, category_id, updated_at, deleted_at)
                     VALUES(?, ?, ?, ?)
                     ON CONFLICT(process_name) DO UPDATE SET
                       category_id = excluded.category_id,
                       updated_at = excluded.updated_at,
                       deleted_at = excluded.deleted_at",
                    rusqlite::params![process_name, category_id, updated_at, deleted_at],
                )
                .db()?;
                Ok(())
            })
            .await?;
    }
    Ok(())
}

async fn merge_process_paths(pool: &DbPool, _device_id: &str, body: &[u8]) -> Result<()> {
    let arr: Vec<Value> = serde_json::from_slice(body).map_err(|e| Error::SyncParse { kind: "process_paths", source: e })?;
    for v in arr {
        let process_name = match v.get("processName").and_then(|x| x.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let exe_path = v.get("exePath").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let seen_at = v.get("seenAt").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let updated_at = v
            .get("updatedAt")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();

        pool.0
            .call(move |conn| {
                let cur: Option<String> = conn
                    .query_row(
                        "SELECT updated_at FROM process_paths WHERE process_name = ?1",
                        rusqlite::params![process_name],
                        |r| r.get(0),
                    )
                    .ok();
                let should_apply = match &cur {
                    None => true,
                    Some(c) => updated_at.as_str() > c.as_str(),
                };
                if !should_apply {
                    return Ok(());
                }
                conn.execute(
                    "INSERT INTO process_paths(process_name, exe_path, seen_at, updated_at)
                     VALUES(?, ?, ?, ?)
                     ON CONFLICT(process_name) DO UPDATE SET
                       exe_path = excluded.exe_path,
                       seen_at = excluded.seen_at,
                       updated_at = excluded.updated_at",
                    rusqlite::params![process_name, exe_path, seen_at, updated_at],
                )
                .db()?;
                Ok(())
            })
            .await?;
    }
    Ok(())
}

async fn merge_app_icons(pool: &DbPool, _device_id: &str, body: &[u8]) -> Result<()> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
    let arr: Vec<Value> =
        serde_json::from_slice(body).map_err(|e| Error::SyncParse { kind: "app_icons", source: e })?;
    for v in arr {
        let process_name = match v.get("processName").and_then(|x| x.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let icon_b64 = v.get("iconPngBase64").and_then(|x| x.as_str()).unwrap_or("");
        let icon_bytes = match BASE64.decode(icon_b64.as_bytes()) {
            Ok(b) => b,
            Err(e) => {
                log::warn!("app_icon process={process_name} base64 解码失败: {e}");
                continue;
            }
        };
        let updated_at = v
            .get("updatedAt")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let deleted_at = v
            .get("deletedAt")
            .and_then(|x| x.as_str())
            .map(String::from);

        let process_name_db = process_name.clone();
        let icon_bytes_db = icon_bytes.clone();
        let updated_at_db = updated_at.clone();
        let deleted_at_db = deleted_at.clone();
        let applied: bool = pool
            .0
            .call(move |conn| {
                let cur: Option<String> = conn
                    .query_row(
                        "SELECT updated_at FROM app_icons WHERE process_name = ?1",
                        rusqlite::params![process_name_db],
                        |r| r.get(0),
                    )
                    .ok();
                let should_apply = match &cur {
                    None => true,
                    Some(c) => updated_at_db.as_str() > c.as_str(),
                };
                if !should_apply {
                    return Ok(false);
                }
                conn.execute(
                    "INSERT INTO app_icons(process_name, icon_png, updated_at, deleted_at)
                     VALUES(?, ?, ?, ?)
                     ON CONFLICT(process_name) DO UPDATE SET
                       icon_png   = excluded.icon_png,
                       updated_at = excluded.updated_at,
                       deleted_at = excluded.deleted_at",
                    rusqlite::params![process_name_db, icon_bytes_db, updated_at_db, deleted_at_db],
                )
                .db()?;
                Ok(true)
            })
            .await?;

        // 把 BLOB 同步落到文件 cache —— 让 UI 后续 get_app_icon 直接命中文件 cache 返回。
        // 软删（deleted_at != NULL）时反过来：把 cache 文件清掉，避免渲染过期图标。
        if applied {
            let path = match crate::repo::app_icons::icon_cache_path(&process_name) {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("解析 icon cache 路径失败 process={process_name}: {e}");
                    continue;
                }
            };
            if deleted_at.is_some() {
                let _ = std::fs::remove_file(&path);
            } else {
                crate::repo::app_icons::write_cache_file(&path, &icon_bytes);
            }
        }
    }
    Ok(())
}

async fn merge_app_groups(pool: &DbPool, _device_id: &str, body: &[u8]) -> Result<()> {
    let arr: Vec<Value> = serde_json::from_slice(body)
        .map_err(|e| Error::SyncParse { kind: "app_groups", source: e })?;
    for v in arr {
        let id = match v.get("id").and_then(|x| x.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let display_name = v
            .get("displayName")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let category_id = v
            .get("categoryId")
            .and_then(|x| x.as_str())
            .map(String::from);
        let updated_at = v
            .get("updatedAt")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let deleted_at = v
            .get("deletedAt")
            .and_then(|x| x.as_str())
            .map(String::from);

        // 拿当前本地 category_id 用来对比 —— 远端的分类 LWW 赢了之后，要 mirror 到
        // app_categories 表里所有成员行（让 reports.rs 的 LEFT JOIN 仍能拿到正确分类）。
        let id_db = id.clone();
        let display_name_db = display_name.clone();
        let category_id_db = category_id.clone();
        let updated_at_db = updated_at.clone();
        let deleted_at_db = deleted_at.clone();
        let applied: Option<(Option<String>, Option<String>)> = pool
            .0
            .call(move |conn| {
                let prev: Option<(String, Option<String>)> = conn
                    .query_row(
                        "SELECT updated_at, category_id FROM app_groups WHERE id = ?1",
                        rusqlite::params![id_db],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .ok();
                let should_apply = match &prev {
                    None => true,
                    Some((cur_upd, _)) => updated_at_db.as_str() > cur_upd.as_str(),
                };
                if !should_apply {
                    return Ok(None);
                }
                let prev_cat = prev.map(|(_, c)| c).unwrap_or(None);
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES(?, ?, ?, ?, ?)
                     ON CONFLICT(id) DO UPDATE SET
                       display_name = excluded.display_name,
                       category_id  = excluded.category_id,
                       updated_at   = excluded.updated_at,
                       deleted_at   = excluded.deleted_at",
                    rusqlite::params![
                        id_db,
                        display_name_db,
                        category_id_db,
                        updated_at_db,
                        deleted_at_db
                    ],
                )
                .db()?;
                Ok(Some((prev_cat, category_id_db.clone())))
            })
            .await?;

        // 如果分类变了 —— 把新分类同步到组里所有 (active) 成员的 app_categories 行。
        // 用本地的 process_name 列表（成员可能是 Mac 风格也可能是 Win 风格），
        // 每行 enqueue outbox 让其它设备也拿到（同 OS 的对端会收到同样的 app_category 行）。
        if let Some((prev_cat, next_cat)) = applied {
            if prev_cat != next_cat {
                let id_for_mirror = id.clone();
                let next_for_mirror = next_cat.clone();
                let now = chrono::Utc::now().to_rfc3339();
                pool.0
                    .call(move |conn| {
                        let members: Vec<String> = {
                            let mut stmt = conn
                                .prepare(
                                    "SELECT process_name FROM app_group_members
                                     WHERE group_id = ?1 AND deleted_at IS NULL",
                                )
                                .db()?;
                            let rows = stmt
                                .query_map(rusqlite::params![id_for_mirror], |r| {
                                    r.get::<_, String>(0)
                                })
                                .db()?;
                            let mut out = Vec::new();
                            for r in rows {
                                out.push(r.db()?);
                            }
                            out
                        };
                        for m in &members {
                            match &next_for_mirror {
                                Some(cat) => {
                                    conn.execute(
                                        "INSERT INTO app_categories(process_name, category_id, updated_at, deleted_at)
                                         VALUES(?, ?, ?, NULL)
                                         ON CONFLICT(process_name) DO UPDATE SET
                                           category_id = excluded.category_id,
                                           updated_at  = excluded.updated_at,
                                           deleted_at  = NULL",
                                        rusqlite::params![m, cat, now],
                                    )
                                    .db()?;
                                }
                                None => {
                                    conn.execute(
                                        "UPDATE app_categories SET deleted_at = ?, updated_at = ?
                                         WHERE process_name = ?",
                                        rusqlite::params![now, now, m],
                                    )
                                    .db()?;
                                }
                            }
                        }
                        Ok(())
                    })
                    .await?;
            }
        }
    }
    Ok(())
}

async fn merge_app_group_members(pool: &DbPool, _device_id: &str, body: &[u8]) -> Result<()> {
    let arr: Vec<Value> = serde_json::from_slice(body)
        .map_err(|e| Error::SyncParse { kind: "app_group_members", source: e })?;
    for v in arr {
        let process_name = match v.get("processName").and_then(|x| x.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let group_id = v
            .get("groupId")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let updated_at = v
            .get("updatedAt")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let deleted_at = v
            .get("deletedAt")
            .and_then(|x| x.as_str())
            .map(String::from);

        if group_id.is_empty() {
            continue;
        }

        pool.0
            .call(move |conn| {
                let cur: Option<String> = conn
                    .query_row(
                        "SELECT updated_at FROM app_group_members WHERE process_name = ?1",
                        rusqlite::params![process_name],
                        |r| r.get(0),
                    )
                    .ok();
                let should_apply = match &cur {
                    None => true,
                    Some(c) => updated_at.as_str() > c.as_str(),
                };
                if !should_apply {
                    return Ok(());
                }
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES(?, ?, ?, ?)
                     ON CONFLICT(process_name) DO UPDATE SET
                       group_id   = excluded.group_id,
                       updated_at = excluded.updated_at,
                       deleted_at = excluded.deleted_at",
                    rusqlite::params![process_name, group_id, updated_at, deleted_at],
                )
                .db()?;
                Ok(())
            })
            .await?;
    }
    Ok(())
}

async fn merge_device_meta(pool: &DbPool, device_id: &str, body: &[u8]) -> Result<()> {
    let v: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return Err(Error::SyncParse { kind: "device_meta", source: e }),
    };
    if !v.is_object() {
        return Ok(());
    }
    let display_name = v.get("displayName").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let color = v.get("color").and_then(|x| x.as_str()).unwrap_or("#60a5fa").to_string();
    let icon = v.get("icon").and_then(|x| x.as_str()).unwrap_or("Monitor").to_string();
    let os = v.get("os").and_then(|x| x.as_str()).map(String::from);
    let last_seen_at = v.get("lastSeenAt").and_then(|x| x.as_str()).map(String::from);
    let updated_at = v
        .get("updatedAt")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let device_id = device_id.to_string();

    pool.0
        .call(move |conn| {
            let cur: Option<String> = conn
                .query_row(
                    "SELECT updated_at FROM devices WHERE device_id = ?1",
                    rusqlite::params![device_id],
                    |r| r.get(0),
                )
                .ok();
            let should_apply = match &cur {
                None => true,
                Some(c) => updated_at.as_str() > c.as_str(),
            };
            if !should_apply {
                return Ok(());
            }
            conn.execute(
                "INSERT INTO devices(device_id, display_name, color, icon, os, last_seen_at, is_self, updated_at)
                 VALUES(?, ?, ?, ?, ?, ?, 0, ?)
                 ON CONFLICT(device_id) DO UPDATE SET
                   display_name = excluded.display_name,
                   color = excluded.color,
                   icon = excluded.icon,
                   os = excluded.os,
                   last_seen_at = excluded.last_seen_at,
                   updated_at = excluded.updated_at",
                rusqlite::params![device_id, display_name, color, icon, os, last_seen_at, updated_at],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn upsert_remote_activity(
    pool: &DbPool,
    device_id: &str,
    remote_id: &str,
    started_at: &str,
    ended_at: &str,
    duration_secs: i64,
    local_date: &str,
    local_hour: u8,
    process_name: &str,
    window_title: &str,
    category_id: &str,
    updated_at: &str,
) -> Result<()> {
    let device_id = device_id.to_string();
    let remote_id = remote_id.to_string();
    let started_at = started_at.to_string();
    let ended_at = ended_at.to_string();
    let local_date = local_date.to_string();
    let process_name = process_name.to_string();
    let window_title = window_title.to_string();
    let category_id = category_id.to_string();
    let updated_at = updated_at.to_string();
    pool.0
        .call(move |conn| {
            let existing: Option<(i64, String)> = conn
                .query_row(
                    "SELECT id, updated_at FROM activities
                     WHERE device_id = ?1 AND remote_id = ?2",
                    rusqlite::params![device_id, remote_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();
            match existing {
                None => {
                    conn.execute(
                        "INSERT INTO activities(
                           started_at, ended_at, duration_secs, local_date, local_hour,
                           process_name, window_title, category_id, screenshot_path,
                           device_id, remote_id, updated_at, origin
                         ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, 'remote')",
                        rusqlite::params![
                            started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id,
                            device_id, remote_id, updated_at,
                        ],
                    )
                    .db()?;
                }
                Some((id, cur_updated)) => {
                    if updated_at > cur_updated {
                        conn.execute(
                            "UPDATE activities SET
                               started_at = ?, ended_at = ?, duration_secs = ?,
                               local_date = ?, local_hour = ?,
                               process_name = ?, window_title = ?, category_id = ?,
                               updated_at = ?
                             WHERE id = ?",
                            rusqlite::params![
                                started_at, ended_at, duration_secs,
                                local_date, local_hour,
                                process_name, window_title, category_id,
                                updated_at, id,
                            ],
                        )
                        .db()?;
                    }
                }
            }
            Ok(())
        })
        .await?;
    Ok(())
}
