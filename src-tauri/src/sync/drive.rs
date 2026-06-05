//! Drive 后端抽象：生产路径打 Google Drive REST，集成测试用 in-memory mock。
//!
//! 生产实现走 4 个 REST 端点（保留为 [`DriveBackend::Http`] 分支内部 fn）：
//!
//!   GET    https://www.googleapis.com/drive/v3/files?spaces=appDataFolder&q=...
//!   GET    https://www.googleapis.com/drive/v3/files/<id>?alt=media
//!   POST   https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart
//!   PATCH  https://www.googleapis.com/upload/drive/v3/files/<id>?uploadType=media
//!   DELETE https://www.googleapis.com/drive/v3/files/<id>
//!
//! 所有 IO 文件都落进 `appDataFolder`：每个 OAuth client 自己的隐藏目录，浏览器看不见，
//! 多设备共享，不需要 rules / index / region。
//!
//! [`DriveBackend`] 枚举注入到 [`crate::sync::engine::SyncEngine`]，生产用 `Http`，
//! 集成测试用 `InMemory`（HashMap 模拟 appDataFolder，时钟 + 唯一 id 单调）。
//! 见 `src-tauri/tests/sync_two_devices.rs`。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

use rand::Rng;
use serde::Deserialize;
use serde_json::json;

use crate::error::{Error, Result};

const DRIVE_BASE: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_BASE: &str = "https://www.googleapis.com/upload/drive/v3";

/// Drive 文件元数据（id + name + 修改时间），不含文件内容。
#[derive(Debug, Clone)]
pub struct FileMeta {
    pub id: String,
    pub name: String,
    /// RFC3339
    pub modified_time: String,
    /// 文件大小 (bytes)；保留给将来用于诊断 / "云端用量"展示
    #[allow(dead_code)]
    pub size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawFile {
    id: String,
    name: String,
    #[serde(rename = "modifiedTime")]
    modified_time: String,
    #[serde(default)]
    size: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListResp {
    #[serde(default)]
    files: Vec<RawFile>,
    #[serde(rename = "nextPageToken", default)]
    next_page_token: Option<String>,
}

/// Drive 后端：生产 = HTTP / 测试 = InMemory。
///
/// 注入到 [`crate::sync::engine::SyncEngine`]；push/pull 路径只看 trait-like 方法接口，
/// 不直接打 reqwest。
pub enum DriveBackend {
    Http,
    /// 仅集成测试用；生产 binary 不会 match 到这条 → clippy 误报 dead_code
    #[allow(dead_code)]
    InMemory(Arc<InMemoryDriveStore>),
}

impl DriveBackend {
    /// 列 appDataFolder 下、`modified_after` 之后修改过的文件（按 modifiedTime 升序）。
    /// `modified_after` 为空字符串或 `1970-01-01T00:00:00Z` 时，列全部。
    pub async fn list_appdata_files(
        &self,
        token: &str,
        modified_after: &str,
    ) -> Result<Vec<FileMeta>> {
        match self {
            DriveBackend::Http => http_list_appdata_files(token, modified_after).await,
            DriveBackend::InMemory(store) => store.list_appdata_files(modified_after).await,
        }
    }

    /// 下载文件全部内容。
    pub async fn download(&self, token: &str, file_id: &str) -> Result<Vec<u8>> {
        match self {
            DriveBackend::Http => http_download(token, file_id).await,
            DriveBackend::InMemory(store) => store.download(file_id).await,
        }
    }

    /// 按 name upsert：有则更新内容，没有就创建到 appDataFolder。返回文件 id。
    pub async fn upsert_by_name(&self, token: &str, name: &str, content: &[u8]) -> Result<String> {
        match self {
            DriveBackend::Http => http_upsert_by_name(token, name, content).await,
            DriveBackend::InMemory(store) => store.upsert_by_name(name, content).await,
        }
    }

    /// 删除一个文件（永久删，不进回收站——appDataFolder 里没有回收站概念）。
    /// 404 视为成功（幂等删）。
    pub async fn delete(&self, token: &str, file_id: &str) -> Result<()> {
        match self {
            DriveBackend::Http => http_delete(token, file_id).await,
            DriveBackend::InMemory(store) => store.delete(file_id).await,
        }
    }
}

// ─────────────── HTTP impl（生产路径，原 pub async fn 移到这里） ───────────────

async fn http_list_appdata_files(token: &str, modified_after: &str) -> Result<Vec<FileMeta>> {
    let client = reqwest::Client::new();
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;

    let q = if modified_after.is_empty() {
        "trashed = false".to_string()
    } else {
        // 注意：modifiedTime 比较值需要带单引号
        format!("trashed = false and modifiedTime > '{}'", modified_after)
    };

    loop {
        let mut req = client
            .get(format!("{DRIVE_BASE}/files"))
            .bearer_auth(token)
            .query(&[
                ("spaces", "appDataFolder"),
                ("fields", "files(id,name,modifiedTime,size),nextPageToken"),
                ("pageSize", "1000"),
                ("orderBy", "modifiedTime"),
                ("q", q.as_str()),
            ]);
        if let Some(ref t) = page_token {
            req = req.query(&[("pageToken", t.as_str())]);
        }

        let resp = req.send().await.map_err(net_err("list"))?;
        if !resp.status().is_success() {
            return Err(http_err("Drive list", resp).await);
        }
        let parsed: ListResp = resp.json().await.map_err(parse_err("list"))?;

        for f in parsed.files {
            out.push(FileMeta {
                id: f.id,
                name: f.name,
                modified_time: f.modified_time,
                size: f.size.and_then(|s| s.parse::<u64>().ok()),
            });
        }

        match parsed.next_page_token {
            Some(t) => page_token = Some(t),
            None => break,
        }
    }

    Ok(out)
}

async fn http_find_by_name(token: &str, name: &str) -> Result<Option<FileMeta>> {
    let client = reqwest::Client::new();
    // q 里的单引号需要反斜杠转义
    let escaped = name.replace('\\', "\\\\").replace('\'', "\\'");
    let q = format!(
        "name = '{}' and 'appDataFolder' in parents and trashed = false",
        escaped
    );

    let resp = client
        .get(format!("{DRIVE_BASE}/files"))
        .bearer_auth(token)
        .query(&[
            ("spaces", "appDataFolder"),
            ("fields", "files(id,name,modifiedTime,size)"),
            ("pageSize", "1"),
            ("q", q.as_str()),
        ])
        .send()
        .await
        .map_err(net_err("find"))?;
    if !resp.status().is_success() {
        return Err(http_err("Drive find_by_name", resp).await);
    }
    let parsed: ListResp = resp.json().await.map_err(parse_err("find"))?;
    Ok(parsed.files.into_iter().next().map(|f| FileMeta {
        id: f.id,
        name: f.name,
        modified_time: f.modified_time,
        size: f.size.and_then(|s| s.parse::<u64>().ok()),
    }))
}

async fn http_download(token: &str, file_id: &str) -> Result<Vec<u8>> {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{DRIVE_BASE}/files/{file_id}"))
        .bearer_auth(token)
        .query(&[("alt", "media")])
        .send()
        .await
        .map_err(net_err("download"))?;
    if !resp.status().is_success() {
        return Err(http_err("Drive download", resp).await);
    }
    let bytes = resp.bytes().await.map_err(net_err("download body"))?;
    Ok(bytes.to_vec())
}

async fn http_upsert_by_name(token: &str, name: &str, content: &[u8]) -> Result<String> {
    if let Some(existing) = http_find_by_name(token, name).await? {
        http_update_media(token, &existing.id, content).await?;
        Ok(existing.id)
    } else {
        http_create_multipart(token, name, content).await
    }
}

async fn http_create_multipart(token: &str, name: &str, content: &[u8]) -> Result<String> {
    // multipart/related 边界
    let boundary = format!("hindsight_{}", rand::thread_rng().gen::<u128>());
    let metadata = json!({
        "name": name,
        "parents": ["appDataFolder"],
    })
    .to_string();

    let mut body: Vec<u8> = Vec::with_capacity(content.len() + metadata.len() + 256);
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
    body.extend_from_slice(metadata.as_bytes());
    body.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{UPLOAD_BASE}/files"))
        .bearer_auth(token)
        .query(&[("uploadType", "multipart"), ("fields", "id")])
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/related; boundary={boundary}"),
        )
        .body(body)
        .send()
        .await
        .map_err(net_err("create"))?;
    if !resp.status().is_success() {
        return Err(http_err("Drive create_multipart", resp).await);
    }
    let v: serde_json::Value = resp.json().await.map_err(parse_err("create"))?;
    Ok(v.get("id")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string())
}

async fn http_update_media(token: &str, file_id: &str, content: &[u8]) -> Result<()> {
    let client = reqwest::Client::new();
    let resp = client
        .patch(format!("{UPLOAD_BASE}/files/{file_id}"))
        .bearer_auth(token)
        .query(&[("uploadType", "media")])
        .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
        .body(content.to_vec())
        .send()
        .await
        .map_err(net_err("update"))?;
    if !resp.status().is_success() {
        return Err(http_err("Drive update_media", resp).await);
    }
    Ok(())
}

async fn http_delete(token: &str, file_id: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let resp = client
        .delete(format!("{DRIVE_BASE}/files/{file_id}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(net_err("delete"))?;
    if !resp.status().is_success() && resp.status().as_u16() != 404 {
        return Err(http_err("Drive delete", resp).await);
    }
    Ok(())
}

// ─────────────── 错误工具 ───────────────

fn net_err(_stage: &'static str) -> impl Fn(reqwest::Error) -> Error {
    // reqwest::Error 直接走 #[from]，stage 体现在调用栈的 chain 里足够定位
    Error::from
}

fn parse_err(_stage: &'static str) -> impl Fn(reqwest::Error) -> Error {
    Error::from
}

async fn http_err(stage: &'static str, resp: reqwest::Response) -> Error {
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if status == 403 && body.contains("ACCESS_TOKEN_SCOPE_INSUFFICIENT") {
        // 显式 variant，让上层 push/pull 能 match 然后归类成"需要重新登录"
        return Error::DriveScopeInsufficient;
    }
    Error::DriveHttp {
        stage,
        status,
        body,
    }
}

// ─────────────── InMemoryDriveStore：测试用的 mock Drive ───────────────

#[derive(Debug, Clone)]
struct StoredFile {
    name: String,
    content: Vec<u8>,
    modified_time: String,
}

/// 进程内 HashMap 模拟 Drive appDataFolder。语义精确镜像 Drive REST：
/// - 文件命名空间是扁平的（`device.<uuid>.<kind>...`）
/// - `upsert_by_name` 推进内部时钟，modifiedTime 单调递增
/// - `delete` 404 视为 Ok，与 HTTP 实现一致
/// - `list_appdata_files` 按 modifiedTime 升序 + 支持 modified_after 过滤
pub struct InMemoryDriveStore {
    files: Mutex<HashMap<String, StoredFile>>,
    next_id: AtomicU64,
    clock: Mutex<i64>,
}

impl InMemoryDriveStore {
    pub fn new() -> Self {
        Self {
            files: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            clock: Mutex::new(0),
        }
    }

    async fn next_modified_time(&self) -> String {
        let mut c = self.clock.lock().await;
        *c += 1;
        // 单调递增的 RFC3339，方便跟真实 Drive 的字典序一致
        format!("2026-05-15T10:00:{:02}.{:06}Z", *c % 60, *c)
    }

    pub async fn list_appdata_files(&self, modified_after: &str) -> Result<Vec<FileMeta>> {
        let files = self.files.lock().await;
        let mut out: Vec<FileMeta> = files
            .iter()
            .filter(|(_, f)| modified_after.is_empty() || f.modified_time.as_str() > modified_after)
            .map(|(id, f)| FileMeta {
                id: id.clone(),
                name: f.name.clone(),
                modified_time: f.modified_time.clone(),
                size: Some(f.content.len() as u64),
            })
            .collect();
        out.sort_by(|a, b| a.modified_time.cmp(&b.modified_time));
        Ok(out)
    }

    pub async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        let files = self.files.lock().await;
        match files.get(file_id) {
            Some(f) => Ok(f.content.clone()),
            None => Err(Error::DriveHttp {
                stage: "InMemory download",
                status: 404,
                body: format!("file_id {file_id} not found"),
            }),
        }
    }

    pub async fn upsert_by_name(&self, name: &str, content: &[u8]) -> Result<String> {
        let mt = self.next_modified_time().await;
        let mut files = self.files.lock().await;
        // 找现有 name 对应的 id
        let existing_id = files
            .iter()
            .find(|(_, f)| f.name == name)
            .map(|(id, _)| id.clone());
        let id = existing_id
            .unwrap_or_else(|| format!("mock-id-{}", self.next_id.fetch_add(1, Ordering::SeqCst)));
        files.insert(
            id.clone(),
            StoredFile {
                name: name.to_string(),
                content: content.to_vec(),
                modified_time: mt,
            },
        );
        Ok(id)
    }

    pub async fn delete(&self, file_id: &str) -> Result<()> {
        let mut files = self.files.lock().await;
        files.remove(file_id);
        Ok(())
    }
}

impl Default for InMemoryDriveStore {
    fn default() -> Self {
        Self::new()
    }
}
