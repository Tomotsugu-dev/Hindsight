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

pub mod auth;
mod auth_callback;
#[cfg(test)]
mod fake;

#[cfg(test)]
pub use fake::InMemoryDriveStore;

use std::future::Future;
use std::time::Duration;

use rand::Rng;
use serde::Deserialize;
use serde_json::json;

use crate::error::{Error, Result};
use crate::storage::DbPool;
use crate::sync::cloud::{http_client, FailureKind, FileMeta};
use crate::sync::file_name::{Dataset, FileKind, FileName};
use crate::sync::payload::TombstonePayload;

const DRIVE_BASE: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_BASE: &str = "https://www.googleapis.com/upload/drive/v3";

/// Drive's sync cadence: 5 minutes.
const DRIVE_PUSH_INTERVAL: Duration = Duration::from_secs(300);
const DRIVE_PULL_INTERVAL: Duration = Duration::from_secs(300);

/// Drive uses zero because its timestamps are precise enough, and pull already
/// stops before failed files instead of advancing past them.
const TIME_PRECISION: Duration = Duration::ZERO;

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

fn failure_kind(e: &Error) -> FailureKind {
    match e {
        // 400 and 401 are Google saying it no longer accepts this refresh token:
        // the user revoked the grant, or the token expired.
        Error::OAuthHttp {
            operation: "refresh",
            status,
            ..
        } if *status == 400 || *status == 401 => FailureKind::CredentialInvalid,
        // AES cannot decrypt: the local key or the ciphertext is damaged, and
        // signing in again is the only way out.
        Error::Crypto(_) => FailureKind::CredentialInvalid,
        // Scope missing: the token has no drive.appdata permission, and only
        // the consent page can grant it.
        Error::DriveScopeInsufficient => FailureKind::CredentialInvalid,
        // Everything else is retried by the next tick.
        _ => FailureKind::Transient,
    }
}

/// Drive's names in `sync_cursor` have no prefix: they existed before each
/// backend got its own.
fn pull_cursor_name(dataset: Dataset) -> String {
    // `Core` used this name before cursors were split by dataset. Keep it so
    // existing databases find their old row instead of downloading every
    // cloud file again after an upgrade.
    if dataset == Dataset::Core {
        return "drive_files".to_string();
    }
    // Newer datasets keep separate pull progress under a namespaced key.
    format!("pull.{}", dataset.name())
}

fn push_fingerprint_name(dataset: Dataset) -> String {
    format!("push.{}", dataset.name())
}

/// The earliest of the streams' cursors: a listing by time from there covers
/// every stream.
fn earliest_cursor<'a>(streams: &[(Dataset, &'a str)]) -> &'a str {
    streams
        .iter()
        .map(|(_, cursor)| *cursor)
        .min()
        .unwrap_or("")
}

/// The Google Drive backend. `pool` serves one purpose: reading and writing
/// `auth_state`, which holds the access token and when it expires.
pub struct DriveClient {
    pool: DbPool,
}

impl DriveClient {
    pub(crate) fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub(super) fn time_precision(&self) -> Duration {
        TIME_PRECISION
    }

    pub(super) fn push_interval(&self) -> Duration {
        DRIVE_PUSH_INTERVAL
    }

    pub(super) fn pull_interval(&self) -> Duration {
        DRIVE_PULL_INTERVAL
    }

    pub(super) async fn ensure_credential(&self) -> Result<bool> {
        match auth::ensure_valid_token(&self.pool).await {
            Ok(_) => Ok(true),
            Err(Error::NotSignedIn) => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub(super) fn failure_kind(&self, e: &Error) -> FailureKind {
        failure_kind(e)
    }

    pub(super) fn pull_cursor_name(&self, dataset: Dataset) -> String {
        pull_cursor_name(dataset)
    }

    pub(super) fn push_fingerprint_name(&self, dataset: Dataset) -> String {
        push_fingerprint_name(dataset)
    }

    pub(super) async fn list(&self, streams: &[(Dataset, &str)]) -> Result<Vec<FileMeta>> {
        let modified_after = earliest_cursor(streams);
        self.with_token_retry(|tok| {
            let after = modified_after.to_string();
            async move { http_list_appdata_files(&tok, &after).await }
        })
        .await
    }

    /// Download a file's content from Drive by its file ID.
    ///
    /// Returns the raw file content.
    pub(super) async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        self.with_token_retry(|tok| {
            let id = file_id.to_string();
            async move { http_download(&tok, &id).await }
        })
        .await
    }

    /// Upload a file to Drive, updating an existing file with the same name or
    /// creating a new one when no match exists.
    ///
    /// Returns the Drive file ID.
    pub(super) async fn upsert_by_name(&self, name: &str, content: &[u8]) -> Result<String> {
        self.with_token_retry(|tok| {
            let name = name.to_string();
            let content = content.to_vec();
            async move { http_upsert_by_name(&tok, &name, &content).await }
        })
        .await
    }

    pub(super) async fn delete(&self, file_id: &str) -> Result<()> {
        self.with_token_retry(|tok| {
            let id = file_id.to_string();
            async move { http_delete(&tok, &id).await }
        })
        .await
    }

    pub(super) async fn end_push_round(&self) -> Result<()> {
        Ok(())
    }

    pub(super) async fn remove_device(&self, device_id: &str, cleared_at: &str) -> Result<u64> {
        remove_device(self, device_id, cleared_at).await
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

/// The file operations that [`remove_device`] needs. [`DriveClient`] does them
/// over HTTP and the test stand-in does them in memory, so the tests run the
/// same steps.
trait DriveFiles: Send + Sync {
    fn upsert_by_name(
        &self,
        name: &str,
        content: &[u8],
    ) -> impl Future<Output = Result<String>> + Send;
    /// Every file, whatever its modified time.
    fn list_all(&self) -> impl Future<Output = Result<Vec<FileMeta>>> + Send;
    fn delete(&self, file_id: &str) -> impl Future<Output = Result<()>> + Send;
}

impl DriveFiles for DriveClient {
    fn upsert_by_name(
        &self,
        name: &str,
        content: &[u8],
    ) -> impl Future<Output = Result<String>> + Send {
        DriveClient::upsert_by_name(self, name, content)
    }

    fn list_all(&self) -> impl Future<Output = Result<Vec<FileMeta>>> + Send {
        self.list(&[])
    }

    fn delete(&self, file_id: &str) -> impl Future<Output = Result<()>> + Send {
        DriveClient::delete(self, file_id)
    }
}

/// Drive's steps to remove a device: upload `device.<id>.tombstone.json`, then
/// delete the device's other files one by one. A file that fails to delete is
/// only logged: the other devices still drop its rows because of the
/// tombstone, and the file only takes up space.
async fn remove_device(drive: &impl DriveFiles, device_id: &str, cleared_at: &str) -> Result<u64> {
    let tombstone_name = FileName {
        device_id: device_id.to_string(),
        kind: FileKind::Tombstone,
    }
    .to_file_name();
    let tombstone = serde_json::to_vec(&TombstonePayload {
        cleared_at: cleared_at.to_string(),
    })?;
    drive
        .upsert_by_name(&tombstone_name, &tombstone)
        .await
        .map_err(|e| Error::Other(format!("上传 tombstone 失败（云端未动，请重试）: {e}")))?;

    // The tombstone itself stays: it is the signal.
    let prefix = FileName::device_prefix(device_id);
    let files = drive.list_all().await?;
    let device_files: Vec<_> = files
        .iter()
        .filter(|f| f.name.starts_with(&prefix) && f.name != tombstone_name)
        .collect();

    let mut deleted = 0u64;
    for f in &device_files {
        match drive.delete(&f.id).await {
            Ok(()) => deleted += 1,
            Err(e) => log::warn!("remove device from cloud: delete {} failed: {e}", f.name),
        }
    }
    Ok(deleted)
}

// ─────────────── Drive REST calls ───────────────

async fn http_list_appdata_files(token: &str, modified_after: &str) -> Result<Vec<FileMeta>> {
    let client = http_client();
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
    let client = http_client();
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
    let client = http_client();
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

    let client = http_client();
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
    let client = http_client();
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
    let client = http_client();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn oauth(operation: &'static str, status: u16) -> Error {
        Error::OAuthHttp {
            operation,
            status,
            body: String::new(),
        }
    }

    /// 要用户重新登录的只有三种：refresh 被 Google 拒（400 / 401）、本地解不开密、
    /// 权限不够。其余的，包括第一次换 token 时的 401 和 Drive 的 500，都等下一轮重试。
    #[test]
    fn failure_kind_asks_to_sign_in_only_for_a_dead_credential() {
        use FailureKind::{CredentialInvalid, Transient};
        assert_eq!(failure_kind(&oauth("refresh", 400)), CredentialInvalid);
        assert_eq!(failure_kind(&oauth("refresh", 401)), CredentialInvalid);
        assert_eq!(
            failure_kind(&Error::Crypto("aes decrypt")),
            CredentialInvalid
        );
        assert_eq!(
            failure_kind(&Error::DriveScopeInsufficient),
            CredentialInvalid
        );
        assert_eq!(failure_kind(&oauth("token", 401)), Transient);
        assert_eq!(failure_kind(&oauth("refresh", 500)), Transient);
        let drive_500 = Error::DriveHttp {
            stage: "Drive list",
            status: 500,
            body: String::new(),
        };
        assert_eq!(failure_kind(&drive_500), Transient);
    }
}
