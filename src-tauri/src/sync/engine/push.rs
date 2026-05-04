//! Push 路径：把 sync_outbox 翻译成"哪些 device-scoped 文件需要重写"，每个 dirty key
//! 调一次 build_* 全量重新生成 JSON / NDJSON 内容，再 upload 到 Drive。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};

use super::io::{self, OutboxRow};
use super::Inner;
use crate::error::{Error, Result};
use crate::storage::DbPool;
use crate::sync::auth::{self, TokenInfo};
use crate::sync::drive;

const PUSH_BATCH_SIZE: usize = 200;

/// Outbox entity 行翻成 dirty key —— 同一个 dirty key 触发对应文件的全量重写。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum DirtyKey {
    ActivityDay(String), // local_date
    Categories,
    AppCategories,
    ProcessPaths,
    DeviceMeta,
    AppIcons,
    AppGroups,
    AppGroupMembers,
}

pub(super) async fn flush_push(inner: &Arc<Inner>) -> Result<()> {
    let token: TokenInfo = match auth::ensure_valid_token(&inner.pool).await {
        Ok(t) => t,
        Err(e) => {
            let msg = e.to_string();
            // "未登录" 是预期状态，不当错误显示；其它（续期失败 / refresh_token 失效）要让用户看见
            if msg.contains("未登录") {
                log::debug!("sync 跳过 push（未登录）");
                return Ok(());
            }
            log::warn!("sync push 拿不到有效 token: {msg}");
            inner.status.write().await.last_error = Some(msg);
            return Ok(());
        }
    };

    let rows = io::read_due_outbox(&inner.pool, PUSH_BATCH_SIZE).await?;
    if rows.is_empty() {
        return Ok(());
    }

    // 把 outbox 行分组到"脏文件"
    let groups = group_outbox(&rows);
    if groups.is_empty() {
        // 所有行都没法分组（entity 未知）→ 全部 drop
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        io::delete_outbox_rows(&inner.pool, &ids).await?;
        return Ok(());
    }

    let self_id = crate::device::self_id();
    let mut succeeded_ids: Vec<i64> = Vec::new();
    let mut failed_ids: Vec<i64> = Vec::new();
    let mut last_err: Option<String> = None;

    for (key, ids) in groups {
        let name = file_name_for(self_id, &key);
        let content = match build_content(&inner.pool, self_id, &key).await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("生成 {} 内容失败: {e}", name);
                failed_ids.extend(&ids);
                last_err = Some(e.to_string());
                continue;
            }
        };
        match drive::upsert_by_name(&token.access_token, &name, &content).await {
            Ok(_) => succeeded_ids.extend(&ids),
            Err(e) => {
                log::warn!("上传 {} 失败: {e}", name);
                failed_ids.extend(&ids);
                last_err = Some(e.to_string());
            }
        }
    }

    if !succeeded_ids.is_empty() {
        io::delete_outbox_rows(&inner.pool, &succeeded_ids).await?;
        let mut s = inner.status.write().await;
        s.last_pushed_at = Some(chrono::Utc::now().to_rfc3339());
        s.last_error = None;
    }

    if !failed_ids.is_empty() {
        let err = last_err.clone().unwrap_or_else(|| "未知错误".into());
        io::bump_outbox_retry(&inner.pool, &failed_ids, &err).await?;
        inner.status.write().await.last_error = last_err.clone();
        return Err(Error::Other(
            last_err.unwrap_or_else(|| "push 失败".into()),
        ));
    }

    log::info!("sync push 成功，共 {} 行 outbox 出队", succeeded_ids.len());
    Ok(())
}

fn group_outbox(rows: &[OutboxRow]) -> HashMap<DirtyKey, Vec<i64>> {
    let mut groups: HashMap<DirtyKey, Vec<i64>> = HashMap::new();
    for row in rows {
        let key = match row.entity.as_str() {
            "activity" => match serde_json::from_str::<Value>(&row.payload)
                .ok()
                .and_then(|p| p.get("localDate").and_then(|v| v.as_str()).map(String::from))
            {
                Some(d) => DirtyKey::ActivityDay(d),
                None => {
                    log::warn!("outbox row {} 是 activity 但 payload 缺 localDate", row.id);
                    continue;
                }
            },
            "category" => DirtyKey::Categories,
            "app_category" => DirtyKey::AppCategories,
            "process_path" => DirtyKey::ProcessPaths,
            "device" => DirtyKey::DeviceMeta,
            "app_icon" => DirtyKey::AppIcons,
            "app_group" => DirtyKey::AppGroups,
            "app_group_member" => DirtyKey::AppGroupMembers,
            _ => {
                log::warn!("outbox row {} entity 未知: {}", row.id, row.entity);
                continue;
            }
        };
        groups.entry(key).or_default().push(row.id);
    }
    groups
}

fn file_name_for(self_id: &str, key: &DirtyKey) -> String {
    match key {
        DirtyKey::ActivityDay(day) => format!("device.{self_id}.activities.{day}.ndjson"),
        DirtyKey::Categories => format!("device.{self_id}.categories.json"),
        DirtyKey::AppCategories => format!("device.{self_id}.app_categories.json"),
        DirtyKey::ProcessPaths => format!("device.{self_id}.process_paths.json"),
        DirtyKey::DeviceMeta => format!("device.{self_id}.meta.json"),
        DirtyKey::AppIcons => format!("device.{self_id}.icons.json"),
        DirtyKey::AppGroups => format!("device.{self_id}.app_groups.json"),
        DirtyKey::AppGroupMembers => format!("device.{self_id}.app_group_members.json"),
    }
}

async fn build_content(pool: &DbPool, self_id: &str, key: &DirtyKey) -> Result<Vec<u8>> {
    match key {
        DirtyKey::ActivityDay(day) => build_activities_day(pool, self_id, day).await,
        DirtyKey::Categories => build_categories(pool).await,
        DirtyKey::AppCategories => build_app_categories(pool).await,
        DirtyKey::ProcessPaths => build_process_paths(pool).await,
        DirtyKey::DeviceMeta => build_device_meta(pool, self_id).await,
        DirtyKey::AppIcons => build_app_icons(pool).await,
        DirtyKey::AppGroups => build_app_groups(pool).await,
        DirtyKey::AppGroupMembers => build_app_group_members(pool).await,
    }
}

async fn build_activities_day(pool: &DbPool, self_id: &str, day: &str) -> Result<Vec<u8>> {
    let self_id = self_id.to_string();
    let day = day.to_string();
    let lines = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id, updated_at
                     FROM activities
                     WHERE device_id = ?1 AND local_date = ?2 AND origin = 'local'
                     ORDER BY id",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let rows = stmt
                .query_map(rusqlite::params![self_id, day], |r| {
                    Ok(json!({
                        "id":            r.get::<_, i64>(0)?,
                        "startedAt":     r.get::<_, String>(1)?,
                        "endedAt":       r.get::<_, String>(2)?,
                        "durationSecs":  r.get::<_, i64>(3)?,
                        "localDate":     r.get::<_, String>(4)?,
                        "localHour":     r.get::<_, i64>(5)?,
                        "processName":   r.get::<_, String>(6)?,
                        "windowTitle":   r.get::<_, Option<String>>(7)?,
                        "categoryId":    r.get::<_, String>(8)?,
                        "updatedAt":     r.get::<_, String>(9)?,
                    }))
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(rows)
        })
        .await?;

    let mut out = Vec::with_capacity(lines.len() * 200);
    for line in &lines {
        let s = serde_json::to_string(line)?;
        out.extend_from_slice(s.as_bytes());
        out.push(b'\n');
    }
    Ok(out)
}

async fn build_categories(pool: &DbPool) -> Result<Vec<u8>> {
    let arr = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, color, icon, builtin, updated_at, deleted_at
                     FROM categories ORDER BY id",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(json!({
                        "id":        r.get::<_, String>(0)?,
                        "name":      r.get::<_, String>(1)?,
                        "color":     r.get::<_, String>(2)?,
                        "icon":      r.get::<_, String>(3)?,
                        "builtin":   r.get::<_, i64>(4)? != 0,
                        "updatedAt": r.get::<_, String>(5)?,
                        "deletedAt": r.get::<_, Option<String>>(6)?,
                    }))
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(rows)
        })
        .await?;
    Ok(serde_json::to_vec(&Value::Array(arr))?)
}

async fn build_app_categories(pool: &DbPool) -> Result<Vec<u8>> {
    let arr = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT process_name, category_id, updated_at, deleted_at
                     FROM app_categories ORDER BY process_name",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(json!({
                        "processName": r.get::<_, String>(0)?,
                        "categoryId":  r.get::<_, String>(1)?,
                        "updatedAt":   r.get::<_, String>(2)?,
                        "deletedAt":   r.get::<_, Option<String>>(3)?,
                    }))
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(rows)
        })
        .await?;
    Ok(serde_json::to_vec(&Value::Array(arr))?)
}

async fn build_process_paths(pool: &DbPool) -> Result<Vec<u8>> {
    let arr = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT process_name, exe_path, seen_at, updated_at
                     FROM process_paths ORDER BY process_name",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(json!({
                        "processName": r.get::<_, String>(0)?,
                        "exePath":     r.get::<_, String>(1)?,
                        "seenAt":      r.get::<_, String>(2)?,
                        "updatedAt":   r.get::<_, String>(3)?,
                    }))
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(rows)
        })
        .await?;
    Ok(serde_json::to_vec(&Value::Array(arr))?)
}

async fn build_app_icons(pool: &DbPool) -> Result<Vec<u8>> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
    let arr = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT process_name, icon_png, updated_at, deleted_at
                     FROM app_icons ORDER BY process_name",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let rows = stmt
                .query_map([], |r| {
                    let bytes: Vec<u8> = r.get::<_, Vec<u8>>(1)?;
                    Ok(json!({
                        "processName": r.get::<_, String>(0)?,
                        // BLOB → base64：JSON 不支持 binary，统一用 base64 标准编码
                        "iconPngBase64": BASE64.encode(&bytes),
                        "updatedAt":   r.get::<_, String>(2)?,
                        "deletedAt":   r.get::<_, Option<String>>(3)?,
                    }))
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(rows)
        })
        .await?;
    Ok(serde_json::to_vec(&Value::Array(arr))?)
}

async fn build_app_groups(pool: &DbPool) -> Result<Vec<u8>> {
    let arr = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, display_name, category_id, updated_at, deleted_at
                     FROM app_groups ORDER BY id",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(json!({
                        "id":          r.get::<_, String>(0)?,
                        "displayName": r.get::<_, String>(1)?,
                        "categoryId":  r.get::<_, Option<String>>(2)?,
                        "updatedAt":   r.get::<_, String>(3)?,
                        "deletedAt":   r.get::<_, Option<String>>(4)?,
                    }))
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(rows)
        })
        .await?;
    Ok(serde_json::to_vec(&Value::Array(arr))?)
}

async fn build_app_group_members(pool: &DbPool) -> Result<Vec<u8>> {
    let arr = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT process_name, group_id, updated_at, deleted_at
                     FROM app_group_members ORDER BY process_name",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(json!({
                        "processName": r.get::<_, String>(0)?,
                        "groupId":     r.get::<_, String>(1)?,
                        "updatedAt":   r.get::<_, String>(2)?,
                        "deletedAt":   r.get::<_, Option<String>>(3)?,
                    }))
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(rows)
        })
        .await?;
    Ok(serde_json::to_vec(&Value::Array(arr))?)
}

async fn build_device_meta(pool: &DbPool, self_id: &str) -> Result<Vec<u8>> {
    let self_id = self_id.to_string();
    let obj = pool
        .0
        .call(move |conn| {
            let row: Option<(String, String, String, String, Option<String>, Option<String>, String)> = conn
                .query_row(
                    "SELECT device_id, display_name, color, icon, os, last_seen_at, updated_at
                     FROM devices WHERE device_id = ?1",
                    rusqlite::params![self_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
                )
                .ok();
            Ok(row)
        })
        .await?;
    let Some((device_id, display_name, color, icon, os, last_seen_at, updated_at)) = obj else {
        return Ok(b"{}".to_vec());
    };
    let v = json!({
        "deviceId":    device_id,
        "displayName": display_name,
        "color":       color,
        "icon":        icon,
        "os":          os,
        "lastSeenAt":  last_seen_at,
        "updatedAt":   updated_at,
    });
    Ok(serde_json::to_vec(&v)?)
}
