//! The cloud storage sync runs on. Files live in one flat folder and are
//! addressed by name; the four operations of [`DriveBackend`] are all the
//! engine needs, and every backend must give them the same meaning.
//!
//! `GoogleDrive` talks to the Drive REST API:
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
//! `InMemory` keeps the files in a map for the end-to-end tests, with its own
//! clock so that modification times only ever move forward.

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use tokio::sync::Mutex;

use rand::Rng;
use serde::Deserialize;
use serde_json::json;

use crate::error::{Error, Result};

const DRIVE_BASE: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_BASE: &str = "https://www.googleapis.com/upload/drive/v3";

/// Drive file metadata (id + name + modified time), without the file content.
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
pub enum DriveBackend {
    GoogleDrive,
    /// The tests' stand-in; not compiled into the shipped binary.
    #[cfg(test)]
    InMemory(Arc<InMemoryDriveStore>),
}

impl DriveBackend {
    /// Lists the files modified strictly after `modified_after`, oldest first;
    /// an empty string lists everything. Pull passes its cursor here, so
    /// "strictly after" and the ordering are part of the contract.
    pub async fn list_files(&self, token: &str, modified_after: &str) -> Result<Vec<FileMeta>> {
        match self {
            DriveBackend::GoogleDrive => http_list_files(token, modified_after).await,
            #[cfg(test)]
            DriveBackend::InMemory(store) => store.list_files(modified_after).await,
        }
    }

    /// Downloads a file's whole content into memory.
    pub async fn download(&self, token: &str, file_id: &str) -> Result<Vec<u8>> {
        match self {
            DriveBackend::GoogleDrive => http_download(token, file_id).await,
            #[cfg(test)]
            DriveBackend::InMemory(store) => store.download(file_id).await,
        }
    }

    /// Writes a file by name: replaces the content when the name exists,
    /// creates the file otherwise. Either way the modification time moves to
    /// now. Returns the file's id.
    pub async fn upsert_by_name(&self, token: &str, name: &str, content: &[u8]) -> Result<String> {
        match self {
            DriveBackend::GoogleDrive => http_upsert_by_name(token, name, content).await,
            #[cfg(test)]
            DriveBackend::InMemory(store) => store.upsert_by_name(name, content).await,
        }
    }

    /// Deletes a file for good; there is no trash to recover it from. Deleting
    /// a file that is already gone counts as success.
    pub async fn delete(&self, token: &str, file_id: &str) -> Result<()> {
        match self {
            DriveBackend::GoogleDrive => http_delete(token, file_id).await,
            #[cfg(test)]
            DriveBackend::InMemory(store) => store.delete(file_id).await,
        }
    }
}

// ─────────────── Google Drive ───────────────

async fn http_list_files(token: &str, modified_after: &str) -> Result<Vec<FileMeta>> {
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
        update_content(token, &existing.id, content).await?;
        Ok(existing.id)
    } else {
        create_file(token, name, content).await
    }
}

async fn create_file(token: &str, name: &str, content: &[u8]) -> Result<String> {
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

async fn update_content(token: &str, file_id: &str, content: &[u8]) -> Result<()> {
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
        return Err(http_err("Drive update_content", resp).await);
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

fn net_err(stage: &'static str) -> impl Fn(reqwest::Error) -> Error {
    move |e: reqwest::Error| {
        log::error!("Network error at stage: {}", stage);
        Error::from(e)
    }
}

fn parse_err(stage: &'static str) -> impl Fn(reqwest::Error) -> Error {
    move |e: reqwest::Error| {
        log::error!("Parse error at stage: {}", stage);
        Error::from(e)
    }
}

async fn http_err(stage: &'static str, resp: reqwest::Response) -> Error {
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if status == 403 && body.contains("ACCESS_TOKEN_SCOPE_INSUFFICIENT") {
        // The token is valid but was granted without the Drive permission.
        // Neither a refresh nor a retry can fix that, only signing in again,
        // so it gets its own variant instead of the generic DriveHttp.
        return Error::DriveScopeInsufficient;
    }
    Error::DriveHttp {
        stage,
        status,
        body,
    }
}

// ─────────────── InMemoryDriveStore ───────────────

#[cfg(test)]
#[derive(Debug, Clone)]
struct StoredFile {
    name: String,
    content: Vec<u8>,
    modified_time: String,
}

/// 内存里的假云,给端到端测试用,行为与 Google Drive 后端一致——测试才能
/// 代替真后端跑:
/// - 扁平命名空间,文件名形如 `device.<uuid>.<kind>...`
/// - `upsert_by_name` 每写一次推进时钟,modifiedTime 严格递增
/// - `delete` 删不存在的文件也算成功(幂等)
/// - `list_files` 按 modifiedTime 升序,只返回严格晚于 `modified_after` 的
#[cfg(test)]
pub struct InMemoryDriveStore {
    /// 云端文件夹:文件 id → 内容。download / delete 都按 id 定位,所以按 id 存。
    files: Mutex<HashMap<String, StoredFile>>,
    /// 发号器:每建一个文件取一个唯一 id(mock-id-N),模拟 Drive 建文件时分配的 id。
    next_id: AtomicU64,
    /// 写入序号,每写一次 +1,用来造单调递增的 modifiedTime——不是真实时间。
    clock: Mutex<i64>,
    /// 故障注入:值 >0 时接下来这么多次 upsert 直接返回 500,每次扣 1。
    /// 用来测 push 上传失败后 outbox 行保留、下一轮重试。
    fail_next_upserts: AtomicU64,
}

#[cfg(test)]
impl InMemoryDriveStore {
    pub fn new() -> Self {
        Self {
            files: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            clock: Mutex::new(0),
            fail_next_upserts: AtomicU64::new(0),
        }
    }

    /// 注入：让接下来 `n` 次 [`Self::upsert_by_name`] 失败（500 瞬时错误）。
    /// 仅测试用 —— 验证 push 失败后 outbox 重试不变量。
    pub fn fail_next_upserts(&self, n: u64) {
        self.fail_next_upserts.store(n, Ordering::SeqCst);
    }

    async fn next_modified_time(&self) -> String {
        let mut c = self.clock.lock().await;
        *c += 1;
        // 这个时间戳只需满足"字符串比大小 = 写入先后"。固定秒位、只让小数位
        // 递增,字典序就跟写入顺序一致;真实时间无所谓。
        format!("2026-05-15T10:00:00.{:09}Z", *c)
    }

    pub async fn list_files(&self, modified_after: &str) -> Result<Vec<FileMeta>> {
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
