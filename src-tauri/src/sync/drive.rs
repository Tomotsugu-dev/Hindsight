//! 云后端：列、下载、写、删云上的文件。引擎只调这四个动作，不知道对面是谁。
//!
//! 生产路径是 Google Drive REST 的 4 个端点（[`DriveClient`] 分支内部 fn）：
//!
//!   GET    https://www.googleapis.com/drive/v3/files?spaces=appDataFolder&q=...
//!   GET    https://www.googleapis.com/drive/v3/files/<id>?alt=media
//!   POST   https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart
//!   PATCH  https://www.googleapis.com/upload/drive/v3/files/<id>?uploadType=media
//!   DELETE https://www.googleapis.com/drive/v3/files/<id>
//!
//! 所有文件都落进 `appDataFolder`：每个 OAuth client 自己的隐藏目录，浏览器看不见，
//! 多设备共享，不需要 rules / index / region。
//!
//! 凭证由后端自己取：[`DriveClient`] 持一份 `DbPool`，每次请求前从 `auth_state`
//! 读 access token，Drive 答 401 就刷新后重试一次。**约束：后端只读凭证那张表，
//! 业务表（outbox、cursor、activities）是引擎的事。**
//!
//! [`CloudBackend`] 注入到 [`crate::sync::engine::SyncEngine`]，生产用 `Drive`，
//! 集成测试用 `InMemory`（HashMap 模拟 appDataFolder，时钟 + 唯一 id 单调），
//! 见 `engine/e2e_tests.rs`。

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

use rand::Rng;
use serde::Deserialize;
use serde_json::json;

use crate::error::{Error, Result};
use crate::storage::DbPool;
use crate::sync::auth;

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

/// 云后端：生产 = Google Drive / 测试 = InMemory。
///
/// 注入到 [`crate::sync::engine::SyncEngine`]；push/pull 只调这四个方法，
/// 不直接打 reqwest，也不经手凭证。
pub enum CloudBackend {
    Drive(DriveClient),
    /// 仅集成测试用；生产 binary 不会 match 到这条 → clippy 误报 dead_code
    #[allow(dead_code)]
    InMemory(Arc<InMemoryDriveStore>),
}

impl CloudBackend {
    /// 生产入口：打 Google Drive，凭证从 `pool` 的 `auth_state` 表取。
    pub fn drive(pool: DbPool) -> Self {
        CloudBackend::Drive(DriveClient { pool })
    }

    /// 备好这一轮要用的凭证。会读本地凭证表，必要时刷新。
    ///
    /// `Ok(false)` = 没登录这个后端，push / pull 整轮跳过，不算失败；
    /// `Err` = 凭证在但拿不到能用的（刷新被拒等），整轮失败，UI 上要出错误条幅。
    pub async fn ensure_credential(&self) -> Result<bool> {
        match self {
            CloudBackend::Drive(c) => c.ensure_credential().await,
            CloudBackend::InMemory(store) => Ok(store.signed_in()),
        }
    }

    /// 列 `modified_after` 之后修改过的文件（按修改时间升序）。
    /// `modified_after` 为空字符串或 `1970-01-01T00:00:00Z` 时，列全部。
    pub async fn list(&self, modified_after: &str) -> Result<Vec<FileMeta>> {
        match self {
            CloudBackend::Drive(c) => c.list(modified_after).await,
            CloudBackend::InMemory(store) => store.list_appdata_files(modified_after).await,
        }
    }

    /// 下载文件全部内容。
    pub async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        match self {
            CloudBackend::Drive(c) => c.download(file_id).await,
            CloudBackend::InMemory(store) => store.download(file_id).await,
        }
    }

    /// 按 name upsert：有则更新内容，没有就创建。返回文件 id。
    pub async fn upsert_by_name(&self, name: &str, content: &[u8]) -> Result<String> {
        match self {
            CloudBackend::Drive(c) => c.upsert_by_name(name, content).await,
            CloudBackend::InMemory(store) => store.upsert_by_name(name, content).await,
        }
    }

    /// 删除一个文件。404 视为成功（幂等删）。
    pub async fn delete(&self, file_id: &str) -> Result<()> {
        match self {
            CloudBackend::Drive(c) => c.delete(file_id).await,
            CloudBackend::InMemory(store) => store.delete(file_id).await,
        }
    }
}

// ─────────────── DriveClient：Google Drive 后端 ───────────────

/// Google Drive 后端。`pool` 只用来读写 `auth_state`（access token 和它的过期时间）。
pub struct DriveClient {
    pool: DbPool,
}

impl DriveClient {
    /// 三种结果对应 `auth_state` 的三种状态：四列齐且 token 可用 / 四列不齐（没登录）/
    /// 四列齐但刷新被 Google 拒（授权被撤销、refresh token 过期）。
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

    /// 带着当前 access token 跑一次 `op`；Drive 答 401 就刷新 token 再跑一次，
    /// 返回第二次的结果。只重试一次；刷新本身失败就返回那个错误，`op` 不再跑。
    ///
    /// 这层存在的理由：`ensure_valid_token` 只看本地存的过期时间，Google 可能在那
    /// 之前就拒绝一个 token（睡眠唤醒、时钟漂移、Google 侧轮换）。刷新一次就能过，
    /// 不该让用户重新登录。
    ///
    /// 刷新出来的新 token 由 `force_refresh` 写回 `auth_state`，所以本轮后面的调用
    /// 各自 `ensure_valid_token` 时读到的就是新的。
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
    /// 测试注入开关：>0 时接下来 N 次 `upsert_by_name` 直接返回 500（模拟 Drive
    /// 瞬时故障），每失败一次消耗一次配额，归零后自动恢复正常。默认 0 = 不注入，
    /// 生产路径（HTTP 分支）完全不经过这里，默认行为不变。
    fail_next_upserts: AtomicU64,
    /// 这个假云端认不认当前用户。默认 true；[`Self::sign_out`] 置 false，
    /// 让 `ensure_credential` 走"没登录"分支。真后端从自己的凭证表判断，
    /// 假后端没有那张表，所以由测试直接拨。
    signed_in: AtomicBool,
}

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

impl Default for InMemoryDriveStore {
    fn default() -> Self {
        Self::new()
    }
}
