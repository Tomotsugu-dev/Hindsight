//! The cloud storage sync runs on. Files live in one flat folder and are
//! addressed by name; the operations of [`CloudBackend`] are all the engine
//! needs, and every backend must give them the same meaning.
//!
//! `Drive` talks to the Google Drive REST API:
//!
//!   GET    https://www.googleapis.com/drive/v3/files?spaces=appDataFolder&q=...
//!   GET    https://www.googleapis.com/drive/v3/files/<id>?alt=media
//!   POST   https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart
//!   PATCH  https://www.googleapis.com/upload/drive/v3/files/<id>?uploadType=media
//!   DELETE https://www.googleapis.com/drive/v3/files/<id>
//!
//! Its folder is `appDataFolder`: hidden from the user, private to the OAuth
//! client, shared by every device signed into the same account with the same
//! client id.
//!
//! Each backend carries its own credential: [`DriveClient`] holds a `DbPool`,
//! reads the access token from `auth_state` before every request and refreshes
//! it once when Drive answers 401. That table is the only one a backend reads;
//! the outbox, the cursors and the activity tables belong to the engine.
//!
//! `InMemory` keeps the files in a map for the end-to-end tests, with its own
//! clock so that modification times only ever move forward.

#[cfg(test)]
use std::collections::HashMap;
use std::future::Future;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use tokio::sync::Mutex;

use rand::Rng;
use serde::Deserialize;
use serde_json::json;

use crate::error::{Error, Result};
use crate::storage::DbPool;
use crate::sync::auth;

const DRIVE_BASE: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_BASE: &str = "https://www.googleapis.com/upload/drive/v3";

/// Cloud file metadata (id + name + modified time), without the file content.
#[derive(Debug, Clone)]
pub struct FileMeta {
    /// The backend's own handle for the file, assigned when the file is created
    /// and opaque to us. Download, overwrite and delete address the file by it.
    pub id: String,
    /// The name Hindsight gave the file, `device.<device id>.<kind>.json`. It is
    /// what pull reads to tell what the file holds and which device wrote it.
    pub name: String,
    /// Time in RFC3339 format.
    pub modified_time: String,
    /// File size in bytes; reserved for future diagnostics / "cloud usage" display.
    #[allow(dead_code)]
    pub size: Option<u64>,
}

/// One entry of Google's file listing, as it arrives. `size` comes as a string
/// and is missing for folders; the rest of the engine sees the converted
/// `FileMeta` instead.
#[derive(Debug, Deserialize)]
struct RawFile {
    id: String,
    name: String,
    #[serde(rename = "modifiedTime")]
    modified_time: String,
    #[serde(default)]
    size: Option<String>,
}

/// One page of Google's file listing.
#[derive(Debug, Deserialize)]
struct ListResp {
    #[serde(default)]
    files: Vec<RawFile>,
    /// Present only when more files remain; the next request sends it back as
    /// `pageToken` to get the following page.
    #[serde(rename = "nextPageToken", default)]
    next_page_token: Option<String>,
}

/// The cloud behind sync. The sync engine holds one, and push, pull and the
/// cloud-clearing commands reach the cloud only through its methods.
/// `InMemory` stands in for a real cloud in tests and is the reference for
/// what each method must do.
pub enum CloudBackend {
    Drive(DriveClient),
    /// The tests' stand-in; not compiled into the shipped binary.
    #[cfg(test)]
    InMemory(Arc<InMemoryDriveStore>),
}

impl CloudBackend {
    /// The backend the app runs on: Google Drive, with the credential taken
    /// from `pool`'s `auth_state` table.
    pub fn drive(pool: DbPool) -> Self {
        CloudBackend::Drive(DriveClient { pool })
    }

    /// Readies the credential this round needs, reading the local credential
    /// table and refreshing when that is due.
    ///
    /// `Ok(false)` means the user is not signed in to this backend: push and
    /// pull skip the round and it does not count as a failure. `Err` means a
    /// credential is there but no usable one came of it, a refused refresh for
    /// instance; the round fails and the Devices page shows the error.
    pub async fn ensure_credential(&self) -> Result<bool> {
        match self {
            CloudBackend::Drive(c) => c.ensure_credential().await,
            #[cfg(test)]
            CloudBackend::InMemory(store) => Ok(store.signed_in()),
        }
    }

    /// Lists the files modified strictly after `modified_after`, oldest first;
    /// an empty string lists everything. Pull passes its cursor here, so
    /// "strictly after" and the ordering are part of the contract.
    pub async fn list(&self, modified_after: &str) -> Result<Vec<FileMeta>> {
        match self {
            CloudBackend::Drive(c) => c.list(modified_after).await,
            #[cfg(test)]
            CloudBackend::InMemory(store) => store.list_appdata_files(modified_after).await,
        }
    }

    /// Downloads a file's whole content into memory.
    pub async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        match self {
            CloudBackend::Drive(c) => c.download(file_id).await,
            #[cfg(test)]
            CloudBackend::InMemory(store) => store.download(file_id).await,
        }
    }

    /// Writes a file by name: replaces the content when the name exists,
    /// creates the file otherwise. Either way the modification time moves to
    /// now. Returns the file's id.
    pub async fn upsert_by_name(&self, name: &str, content: &[u8]) -> Result<String> {
        match self {
            CloudBackend::Drive(c) => c.upsert_by_name(name, content).await,
            #[cfg(test)]
            CloudBackend::InMemory(store) => store.upsert_by_name(name, content).await,
        }
    }

    /// Deletes a file for good; there is no trash to recover it from. Deleting
    /// a file that is already gone counts as success.
    pub async fn delete(&self, file_id: &str) -> Result<()> {
        match self {
            CloudBackend::Drive(c) => c.delete(file_id).await,
            #[cfg(test)]
            CloudBackend::InMemory(store) => store.delete(file_id).await,
        }
    }
}

// ─────────────── Google Drive ───────────────

/// The Google Drive backend. `pool` serves one purpose: reading and writing
/// `auth_state`, which holds the access token and when it expires.
pub struct DriveClient {
    pool: DbPool,
}

impl DriveClient {
    async fn ensure_credential(&self) -> Result<bool> {
        match auth::ensure_valid_token(&self.pool).await {
            Ok(_) => Ok(true),
            Err(Error::NotSignedIn) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn list(&self, modified_after: &str) -> Result<Vec<FileMeta>> {
        self.with_token_retry(|tok| {
            let after = modified_after.to_string();
            async move { http_list_appdata_files(&tok, &after).await }
        })
        .await
    }

    async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        self.with_token_retry(|tok| {
            let id = file_id.to_string();
            async move { http_download(&tok, &id).await }
        })
        .await
    }

    async fn upsert_by_name(&self, name: &str, content: &[u8]) -> Result<String> {
        self.with_token_retry(|tok| {
            let name = name.to_string();
            let content = content.to_vec();
            async move { http_upsert_by_name(&tok, &name, &content).await }
        })
        .await
    }

    async fn delete(&self, file_id: &str) -> Result<()> {
        self.with_token_retry(|tok| {
            let id = file_id.to_string();
            async move { http_delete(&tok, &id).await }
        })
        .await
    }

    /// Why the retry exists: `ensure_valid_token` only checks the expiry time
    /// stored locally, and Google can reject a token before that — wake from
    /// sleep, clock drift, rotation on Google's side.
    ///
    /// `force_refresh` writes the new token to `auth_state`, so the later calls
    /// of the same round read it back from there.
    async fn with_token_retry<F, Fut, T>(&self, mut op: F) -> Result<T>
    where
        F: FnMut(String) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let token = auth::ensure_valid_token(&self.pool).await?;
        match op(token.access_token).await {
            Err(Error::DriveHttp {
                status: 401,
                stage,
                body,
            }) => {
                log::info!("drive {stage}: 401, refreshing the access token and retrying once");
                log::debug!("drive {stage}: 401 body: {body}");
                let fresh = auth::force_refresh(&self.pool).await?;
                op(fresh.access_token).await
            }
            other => other,
        }
    }
}

// ─────────────── Drive REST calls ───────────────

async fn http_list_appdata_files(token: &str, modified_after: &str) -> Result<Vec<FileMeta>> {
    let client = reqwest::Client::new();
    let mut out = Vec::new();
    let mut page_token: Option<String> = None;

    let q = if modified_after.is_empty() {
        "trashed = false".to_string()
    } else {
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
    // A quote or backslash in the name would break the query, so escape them.
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

// ─────────────── Error helpers ───────────────

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

#[cfg(test)]
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
#[cfg(test)]
pub struct InMemoryDriveStore {
    files: Mutex<HashMap<String, StoredFile>>,
    next_id: AtomicU64,
    clock: Mutex<i64>,
    /// 测试注入开关：>0 时接下来 N 次 `upsert_by_name` 直接返回 500（模拟 Drive
    /// 瞬时故障），每失败一次消耗一次配额，归零后自动恢复正常。默认 0 = 不注入，
    /// 生产路径（HTTP 分支）完全不经过这里，默认行为不变。
    fail_next_upserts: AtomicU64,
    /// 这个假云端认不认当前用户。真后端从自己的凭证表判断，假后端没有那张表，
    /// 所以由测试用 [`Self::sign_out`] 直接拨。
    signed_in: AtomicBool,
}

#[cfg(test)]
impl InMemoryDriveStore {
    pub fn new() -> Self {
        Self {
            files: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            clock: Mutex::new(0),
            fail_next_upserts: AtomicU64::new(0),
            signed_in: AtomicBool::new(true),
        }
    }

    /// 让这个假云端当作用户没登录。仅测试用。
    #[allow(dead_code)]
    pub fn sign_out(&self) {
        self.signed_in.store(false, Ordering::SeqCst);
    }

    fn signed_in(&self) -> bool {
        self.signed_in.load(Ordering::SeqCst)
    }

    /// 注入：让接下来 `n` 次 [`Self::upsert_by_name`] 失败（500 瞬时错误）。
    /// 仅测试用 —— 验证 push 失败后 outbox 重试不变量。
    #[allow(dead_code)]
    pub fn fail_next_upserts(&self, n: u64) {
        self.fail_next_upserts.store(n, Ordering::SeqCst);
    }

    async fn next_modified_time(&self) -> String {
        let mut c = self.clock.lock().await;
        *c += 1;
        // 单调递增的 RFC3339，方便跟真实 Drive 的字典序一致。
        // 秒字段固定为 0、只让小数位增长：之前 `{:02}` 填 `*c % 60` 会在第 60 次
        // 上传时秒位回绕（...00.000060 < ...59.000059），破坏单调性
        format!("2026-05-15T10:00:00.{:09}Z", *c)
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
        // 失败注入：配额 > 0 时原子扣减一次并返回 500。checked_sub 在 0 时返回
        // None → fetch_update Err → 不注入，走正常路径。
        if self
            .fail_next_upserts
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| v.checked_sub(1))
            .is_ok()
        {
            return Err(Error::DriveHttp {
                stage: "InMemory upsert (injected failure)",
                status: 500,
                body: "injected transient failure".into(),
            });
        }
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

#[cfg(test)]
impl Default for InMemoryDriveStore {
    fn default() -> Self {
        Self::new()
    }
}
