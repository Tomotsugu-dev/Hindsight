//! 同步引擎：登录后台跑两件事
//!   - push：每 30 秒把 sync_outbox 翻成"哪些文件脏了"，对每个脏文件全量重写到 Drive appDataFolder
//!   - pull：每 60 秒列 Drive 上其他设备的文件，按 modifiedTime 增量下载并 LWW merge 到本地
//!
//! 失败走指数退避（最多 1 小时），attempts > 10 留在 outbox 作为 dead-letter，UI 可以看见。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use rand::Rng;
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::error::{Error, Result};
use crate::storage::DbPool;
use crate::sync::auth::{self, TokenInfo};
use crate::sync::drive;

const PUSH_INTERVAL_SECS: u64 = 30;
const PULL_INTERVAL_SECS: i64 = 60;
const PUSH_BATCH_SIZE: usize = 200;
const MAX_ATTEMPTS: i64 = 10;
const RETRY_BASE_SECS: i64 = 5;
const RETRY_MAX_SECS: i64 = 60 * 60;
const PULL_CURSOR_KEY: &str = "drive_files";

#[derive(Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub running: bool,
    pub last_pushed_at: Option<String>,
    pub last_pulled_at: Option<String>,
    pub last_error: Option<String>,
    pub pending: u64,
    pub dead_letter: u64,
}

struct Inner {
    pool: DbPool,
    handle: Mutex<Option<JoinHandle<()>>>,
    status: RwLock<SyncStatus>,
}

pub struct SyncEngine {
    inner: Arc<Inner>,
}

impl SyncEngine {
    pub fn new(pool: DbPool) -> Self {
        Self {
            inner: Arc::new(Inner {
                pool,
                handle: Mutex::new(None),
                status: RwLock::new(SyncStatus::default()),
            }),
        }
    }

    pub async fn start(&self) {
        let mut h = self.inner.handle.lock().await;
        if h.is_some() {
            return;
        }
        let inner = Arc::clone(&self.inner);
        *h = Some(tokio::spawn(async move {
            run_loop(inner).await;
        }));
        log::info!("sync engine 已启动");
    }

    /// 停止后台 push/pull 循环。当前没有 UI 入口；保留给将来"sign_out 后停 engine"的场景。
    #[allow(dead_code)]
    pub async fn stop(&self) {
        let mut h = self.inner.handle.lock().await;
        if let Some(handle) = h.take() {
            handle.abort();
            log::info!("sync engine 已停止");
        }
    }

    pub async fn is_running(&self) -> bool {
        self.inner.handle.lock().await.is_some()
    }

    pub async fn status(&self) -> SyncStatus {
        let mut s = self.inner.status.read().await.clone();
        s.running = self.is_running().await;
        s.pending = count_outbox(&self.inner.pool).await.unwrap_or(0);
        s.dead_letter = count_dead_letter(&self.inner.pool).await.unwrap_or(0);
        s
    }

    /// UI "立即同步" 按钮：跑一次 push + pull，不等下个 30s tick。
    pub async fn sync_now(&self) -> Result<()> {
        // 清掉上次的错误，否则即使这次成功，UI 也会留着旧 last_error
        self.inner.status.write().await.last_error = None;
        flush_push(&self.inner).await?;
        flush_pull(&self.inner).await?;
        // push/pull 内部如果 token 拿不到会写 last_error 但 return Ok；这里统一暴露给 UI
        let last_err = self.inner.status.read().await.last_error.clone();
        if let Some(e) = last_err {
            return Err(crate::error::Error::Other(e));
        }
        Ok(())
    }
}

async fn run_loop(inner: Arc<Inner>) {
    let mut last_pull: Option<DateTime<Utc>> = None;
    loop {
        if let Err(e) = flush_push(&inner).await {
            log::warn!("sync push 失败: {e}");
            inner.status.write().await.last_error = Some(e.to_string());
        }

        let now = Utc::now();
        let should_pull = match last_pull {
            None => true,
            Some(t) => (now - t).num_seconds() >= PULL_INTERVAL_SECS,
        };
        if should_pull {
            if let Err(e) = flush_pull(&inner).await {
                log::warn!("sync pull 失败: {e}");
                inner.status.write().await.last_error = Some(e.to_string());
            }
            last_pull = Some(now);
        }

        tokio::time::sleep(Duration::from_secs(PUSH_INTERVAL_SECS)).await;
    }
}

// ───────────────────── Push ─────────────────────

#[derive(Debug, Clone)]
struct OutboxRow {
    id: i64,
    entity: String,
    payload: String,
}

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

async fn flush_push(inner: &Arc<Inner>) -> Result<()> {
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

    let rows = read_due_outbox(&inner.pool, PUSH_BATCH_SIZE).await?;
    if rows.is_empty() {
        return Ok(());
    }

    // 把 outbox 行分组到"脏文件"
    let groups = group_outbox(&rows);
    if groups.is_empty() {
        // 所有行都没法分组（entity 未知）→ 全部 drop
        let ids: Vec<i64> = rows.iter().map(|r| r.id).collect();
        delete_outbox_rows(&inner.pool, &ids).await?;
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
        delete_outbox_rows(&inner.pool, &succeeded_ids).await?;
        let mut s = inner.status.write().await;
        s.last_pushed_at = Some(Utc::now().to_rfc3339());
        s.last_error = None;
    }

    if !failed_ids.is_empty() {
        let err = last_err.clone().unwrap_or_else(|| "未知错误".into());
        bump_outbox_retry(&inner.pool, &failed_ids, &err).await?;
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

// ───────────────────── Pull ─────────────────────

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

async fn flush_pull(inner: &Arc<Inner>) -> Result<()> {
    let token: TokenInfo = match auth::ensure_valid_token(&inner.pool).await {
        Ok(t) => t,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("未登录") {
                return Ok(());
            }
            log::warn!("sync pull 拿不到有效 token: {msg}");
            inner.status.write().await.last_error = Some(msg);
            return Ok(());
        }
    };

    let cursor = read_cursor(&inner.pool, PULL_CURSOR_KEY).await?;
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
        write_cursor(&inner.pool, PULL_CURSOR_KEY, &t).await?;
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
    let s = std::str::from_utf8(body).map_err(|e| Error::Other(format!("ndjson UTF-8: {e}")))?;
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
    let arr: Vec<Value> = serde_json::from_slice(body).map_err(|e| Error::Other(format!("categories JSON: {e}")))?;
    for v in arr {
        let id = match v.get("id").and_then(|x| x.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let color = v.get("color").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let icon = v.get("icon").and_then(|x| x.as_str()).unwrap_or("Tag").to_string();
        let builtin = v.get("builtin").and_then(|x| x.as_bool()).unwrap_or(false);
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
                        "SELECT updated_at FROM categories WHERE id = ?1",
                        rusqlite::params![id],
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
                if cur.is_none() {
                    conn.execute(
                        "INSERT INTO categories(id, name, color, icon, builtin, updated_at, deleted_at)
                         VALUES(?, ?, ?, ?, ?, ?, ?)",
                        rusqlite::params![id, name, color, icon, builtin as i64, updated_at, deleted_at],
                    )
                    .map_err(tokio_rusqlite::Error::Rusqlite)?;
                } else {
                    conn.execute(
                        "UPDATE categories SET name = ?, color = ?, icon = ?, builtin = ?,
                                                updated_at = ?, deleted_at = ?
                         WHERE id = ?",
                        rusqlite::params![name, color, icon, builtin as i64, updated_at, deleted_at, id],
                    )
                    .map_err(tokio_rusqlite::Error::Rusqlite)?;
                }
                Ok(())
            })
            .await?;
    }
    Ok(())
}

async fn merge_app_categories(pool: &DbPool, _device_id: &str, body: &[u8]) -> Result<()> {
    let arr: Vec<Value> = serde_json::from_slice(body).map_err(|e| Error::Other(format!("app_categories JSON: {e}")))?;
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
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
                Ok(())
            })
            .await?;
    }
    Ok(())
}

async fn merge_process_paths(pool: &DbPool, _device_id: &str, body: &[u8]) -> Result<()> {
    let arr: Vec<Value> = serde_json::from_slice(body).map_err(|e| Error::Other(format!("process_paths JSON: {e}")))?;
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
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
                Ok(())
            })
            .await?;
    }
    Ok(())
}

async fn merge_app_icons(pool: &DbPool, _device_id: &str, body: &[u8]) -> Result<()> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
    let arr: Vec<Value> =
        serde_json::from_slice(body).map_err(|e| Error::Other(format!("app_icons JSON: {e}")))?;
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
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
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
        .map_err(|e| Error::Other(format!("app_groups JSON: {e}")))?;
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
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
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
                                .map_err(tokio_rusqlite::Error::Rusqlite)?;
                            let rows = stmt
                                .query_map(rusqlite::params![id_for_mirror], |r| {
                                    r.get::<_, String>(0)
                                })
                                .map_err(tokio_rusqlite::Error::Rusqlite)?;
                            let mut out = Vec::new();
                            for r in rows {
                                out.push(r.map_err(tokio_rusqlite::Error::Rusqlite)?);
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
                                    .map_err(tokio_rusqlite::Error::Rusqlite)?;
                                }
                                None => {
                                    conn.execute(
                                        "UPDATE app_categories SET deleted_at = ?, updated_at = ?
                                         WHERE process_name = ?",
                                        rusqlite::params![now, now, m],
                                    )
                                    .map_err(tokio_rusqlite::Error::Rusqlite)?;
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
        .map_err(|e| Error::Other(format!("app_group_members JSON: {e}")))?;
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
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
                Ok(())
            })
            .await?;
    }
    Ok(())
}

async fn merge_device_meta(pool: &DbPool, device_id: &str, body: &[u8]) -> Result<()> {
    let v: Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return Err(Error::Other(format!("device meta JSON: {e}"))),
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
            .map_err(tokio_rusqlite::Error::Rusqlite)?;
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
                    .map_err(tokio_rusqlite::Error::Rusqlite)?;
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
                        .map_err(tokio_rusqlite::Error::Rusqlite)?;
                    }
                }
            }
            Ok(())
        })
        .await?;
    Ok(())
}

// ───────────────────── DB helpers ─────────────────────

async fn read_due_outbox(pool: &DbPool, limit: usize) -> Result<Vec<OutboxRow>> {
    let limit = limit as i64;
    // 关键：next_retry_at 是 chrono::to_rfc3339()（"2026-05-03T...+00:00"），
    // 不能跟 SQLite 的 datetime('now')（"2026-05-03 ..."，空格无 T）做字典序比较 —— 'T' > ' ' 永远不等。
    // 这里用 Rust 端生成同格式的 now 当参数。
    let now_rfc = chrono::Utc::now().to_rfc3339();
    let rows = pool
        .0
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, entity, payload
                     FROM sync_outbox
                     WHERE next_retry_at <= ?1 AND attempts < ?2
                     ORDER BY id ASC
                     LIMIT ?3",
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            let rows = stmt
                .query_map(rusqlite::params![now_rfc, MAX_ATTEMPTS, limit], |r| {
                    Ok(OutboxRow {
                        id: r.get(0)?,
                        entity: r.get(1)?,
                        payload: r.get(2)?,
                    })
                })
                .map_err(tokio_rusqlite::Error::Rusqlite)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(rows)
        })
        .await?;
    Ok(rows)
}

async fn delete_outbox_rows(pool: &DbPool, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let ids = ids.to_vec();
    pool.0
        .call(move |conn| {
            let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!("DELETE FROM sync_outbox WHERE id IN ({placeholders})");
            let params: Vec<&dyn rusqlite::ToSql> =
                ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
            conn.execute(&sql, params.as_slice())
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(())
        })
        .await?;
    Ok(())
}

async fn bump_outbox_retry(pool: &DbPool, ids: &[i64], err: &str) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let ids = ids.to_vec();
    let err = err.to_string();
    pool.0
        .call(move |conn| {
            for id in &ids {
                let attempts: i64 = conn
                    .query_row(
                        "SELECT attempts FROM sync_outbox WHERE id = ?",
                        [id],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                let next_attempt = attempts + 1;
                let backoff = (RETRY_BASE_SECS << next_attempt.min(12) as u32).min(RETRY_MAX_SECS);
                let jitter: i64 = rand::thread_rng().gen_range(0..30);
                let delay = backoff + jitter;
                let next_at =
                    (chrono::Utc::now() + chrono::Duration::seconds(delay)).to_rfc3339();
                conn.execute(
                    "UPDATE sync_outbox
                     SET attempts = attempts + 1, last_error = ?, next_retry_at = ?
                     WHERE id = ?",
                    rusqlite::params![err, next_at, id],
                )
                .map_err(tokio_rusqlite::Error::Rusqlite)?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}

async fn count_outbox(pool: &DbPool) -> Result<u64> {
    let n = pool
        .0
        .call(|conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sync_outbox WHERE attempts < ?",
                    [MAX_ATTEMPTS],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            Ok(n)
        })
        .await?;
    Ok(n.max(0) as u64)
}

async fn count_dead_letter(pool: &DbPool) -> Result<u64> {
    let n = pool
        .0
        .call(|conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sync_outbox WHERE attempts >= ?",
                    [MAX_ATTEMPTS],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            Ok(n)
        })
        .await?;
    Ok(n.max(0) as u64)
}

async fn read_cursor(pool: &DbPool, entity: &str) -> Result<String> {
    let entity = entity.to_string();
    let cursor = pool
        .0
        .call(move |conn| {
            let r: Option<String> = conn
                .query_row(
                    "SELECT last_pulled_at FROM sync_cursor WHERE entity = ?",
                    [&entity],
                    |r| r.get(0),
                )
                .ok();
            Ok(r)
        })
        .await?;
    Ok(cursor.unwrap_or_else(|| "1970-01-01T00:00:00Z".into()))
}

async fn write_cursor(pool: &DbPool, entity: &str, value: &str) -> Result<()> {
    let entity = entity.to_string();
    let value = value.to_string();
    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO sync_cursor(entity, last_pulled_at) VALUES(?, ?)
                 ON CONFLICT(entity) DO UPDATE SET last_pulled_at = excluded.last_pulled_at",
                rusqlite::params![entity, value],
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(())
        })
        .await?;
    Ok(())
}
