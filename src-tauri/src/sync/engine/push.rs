//! Push 路径：把 sync_outbox 翻译成"哪些 device-scoped 文件需要重写"，每个 dirty key
//! 调一次 build_* 全量重新生成 JSON / NDJSON 内容，再 upload 到 Drive。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use super::io::{self, OutboxRow};
use super::{format_sync_error, with_token_retry, Inner};
use crate::error::{Error, Result};
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;
use crate::sync::auth::{self, TokenInfo};
use crate::sync::drive;
use crate::sync::payload::{
    ActivityPayload, AppCategoryPayload, AppGroupMemberPayload, AppGroupPayload, AppIconPayload,
    CategoryPayload, DeviceMetaPayload, ProcessPathPayload,
};

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
    let mut token: TokenInfo = match auth::ensure_valid_token(&inner.pool).await {
        Ok(t) => t,
        // NotSignedIn 是预期状态，不当错误显示；其它（续期失败 / refresh_token 失效）让用户看见
        Err(Error::NotSignedIn) => {
            log::debug!("sync 跳过 push（未登录）");
            return Ok(());
        }
        Err(e) => {
            // 默认日志级别只记概述：避免把 OAuth body / 错误链里可能携带的
            // 用户邮箱 / account id / OAuth body 落到日志文件；
            // 错误分类后的 status 字符串已经带 [CRED_EXPIRED] / [TRANSIENT] 前缀，
            // UI 可读且不含原始 raw。排查时用 RUST_LOG=hindsight=debug 看 detail
            log::warn!("sync push 拿不到有效 token（详情见 status）");
            log::debug!("token error detail: {e}");
            inner.status.write().await.last_error = Some(format_sync_error(&e));
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

    let self_id = crate::device::self_id()?;
    let mut succeeded_ids: Vec<i64> = Vec::new();
    let mut failed_ids: Vec<i64> = Vec::new();
    let mut last_err_raw: Option<crate::error::Error> = None;
    let mut last_err_str: Option<String> = None;

    for (key, ids) in groups {
        let name = file_name_for(self_id, &key);
        let content = match build_content(&inner.pool, self_id, &key).await {
            Ok(c) => c,
            Err(e) => {
                log::warn!("生成 {} 内容失败: {e}", name);
                failed_ids.extend(&ids);
                last_err_str = Some(e.to_string());
                last_err_raw = Some(e);
                continue;
            }
        };
        let upsert_res = with_token_retry(&inner.pool, &mut token, |tok| {
            let name = name.clone();
            let content = content.clone();
            async move { drive::upsert_by_name(&tok, &name, &content).await }
        })
        .await;
        match upsert_res {
            Ok(_) => succeeded_ids.extend(&ids),
            Err(e) => {
                log::warn!("上传 {} 失败: {e}", name);
                failed_ids.extend(&ids);
                last_err_str = Some(e.to_string());
                last_err_raw = Some(e);
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
        let err_str = last_err_str.clone().unwrap_or_else(|| "未知错误".into());
        io::bump_outbox_retry(&inner.pool, &failed_ids, &err_str).await?;
        if let Some(ref e) = last_err_raw {
            inner.status.write().await.last_error = Some(format_sync_error(e));
        }
        return Err(Error::SyncIncomplete(
            last_err_str.unwrap_or_else(|| "push 失败".into()),
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
                .and_then(|p| {
                    p.get("localDate")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                }) {
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
    let rows: Vec<ActivityPayload> = pool
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
                .db()?;
            let rows = stmt
                .query_map(rusqlite::params![self_id, day], |r| {
                    Ok(ActivityPayload {
                        id: r.get(0)?,
                        started_at: r.get(1)?,
                        ended_at: r.get(2)?,
                        duration_secs: r.get(3)?,
                        local_date: r.get(4)?,
                        local_hour: r.get(5)?,
                        process_name: r.get(6)?,
                        window_title: r.get(7)?,
                        category_id: r.get(8)?,
                        updated_at: r.get(9)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;

    let mut out = Vec::with_capacity(rows.len() * 200);
    for row in &rows {
        let s = serde_json::to_string(row)?;
        out.extend_from_slice(s.as_bytes());
        out.push(b'\n');
    }
    Ok(out)
}

async fn build_categories(pool: &DbPool) -> Result<Vec<u8>> {
    let rows: Vec<CategoryPayload> = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, color, icon, builtin, sort_order, updated_at, deleted_at
                     FROM categories ORDER BY id",
                )
                .db()?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(CategoryPayload {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        color: r.get(2)?,
                        icon: r.get(3)?,
                        builtin: r.get::<_, i64>(4)? != 0,
                        sort_order: r.get(5)?,
                        updated_at: r.get(6)?,
                        deleted_at: r.get(7)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;
    Ok(serde_json::to_vec(&rows)?)
}

async fn build_app_categories(pool: &DbPool) -> Result<Vec<u8>> {
    let rows: Vec<AppCategoryPayload> = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT process_name, category_id, updated_at, deleted_at
                     FROM app_categories ORDER BY process_name",
                )
                .db()?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(AppCategoryPayload {
                        process_name: r.get(0)?,
                        category_id: r.get(1)?,
                        updated_at: r.get(2)?,
                        deleted_at: r.get(3)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;
    Ok(serde_json::to_vec(&rows)?)
}

async fn build_process_paths(pool: &DbPool) -> Result<Vec<u8>> {
    let rows: Vec<ProcessPathPayload> = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT process_name, exe_path, seen_at, updated_at
                     FROM process_paths ORDER BY process_name",
                )
                .db()?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(ProcessPathPayload {
                        process_name: r.get(0)?,
                        exe_path: r.get(1)?,
                        seen_at: r.get(2)?,
                        updated_at: r.get(3)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;
    Ok(serde_json::to_vec(&rows)?)
}

async fn build_app_icons(pool: &DbPool) -> Result<Vec<u8>> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
    let rows: Vec<AppIconPayload> = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT process_name, icon_png, updated_at, deleted_at
                     FROM app_icons ORDER BY process_name",
                )
                .db()?;
            let rows = stmt
                .query_map([], |r| {
                    let bytes: Vec<u8> = r.get(1)?;
                    Ok(AppIconPayload {
                        process_name: r.get(0)?,
                        // BLOB → base64：JSON 不支持 binary，统一用 base64 标准编码
                        icon_png_base64: BASE64.encode(&bytes),
                        updated_at: r.get(2)?,
                        deleted_at: r.get(3)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;
    Ok(serde_json::to_vec(&rows)?)
}

async fn build_app_groups(pool: &DbPool) -> Result<Vec<u8>> {
    let rows: Vec<AppGroupPayload> = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, display_name, category_id, updated_at, deleted_at
                     FROM app_groups ORDER BY id",
                )
                .db()?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(AppGroupPayload {
                        id: r.get(0)?,
                        display_name: r.get(1)?,
                        category_id: r.get(2)?,
                        updated_at: r.get(3)?,
                        deleted_at: r.get(4)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;
    Ok(serde_json::to_vec(&rows)?)
}

async fn build_app_group_members(pool: &DbPool) -> Result<Vec<u8>> {
    let rows: Vec<AppGroupMemberPayload> = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT process_name, group_id, updated_at, deleted_at
                     FROM app_group_members ORDER BY process_name",
                )
                .db()?;
            let rows = stmt
                .query_map([], |r| {
                    Ok(AppGroupMemberPayload {
                        process_name: r.get(0)?,
                        group_id: r.get(1)?,
                        updated_at: r.get(2)?,
                        deleted_at: r.get(3)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await?;
    Ok(serde_json::to_vec(&rows)?)
}

async fn build_device_meta(pool: &DbPool, self_id: &str) -> Result<Vec<u8>> {
    let self_id = self_id.to_string();
    let obj: Option<DeviceMetaPayload> = pool
        .0
        .call(move |conn| {
            let row = conn
                .query_row(
                    "SELECT device_id, display_name, color, icon, os, last_seen_at, updated_at
                     FROM devices WHERE device_id = ?1",
                    rusqlite::params![self_id],
                    |r| {
                        Ok(DeviceMetaPayload {
                            device_id: r.get(0)?,
                            display_name: r.get(1)?,
                            color: r.get(2)?,
                            icon: r.get(3)?,
                            os: r.get(4)?,
                            last_seen_at: r.get(5)?,
                            updated_at: r.get(6)?,
                        })
                    },
                )
                .ok();
            Ok(row)
        })
        .await?;
    let Some(meta) = obj else {
        return Ok(b"{}".to_vec());
    };
    Ok(serde_json::to_vec(&meta)?)
}
