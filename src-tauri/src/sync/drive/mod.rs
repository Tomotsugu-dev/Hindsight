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
use crate::sync::auth;
use crate::sync::cloud::FileMeta;

const DRIVE_BASE: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_BASE: &str = "https://www.googleapis.com/upload/drive/v3";

/// Drive's sync cadence: the intervals sync has always run at. Drive filters
/// listings server-side and has no request budget worth counting, so there is
/// nothing to hold back for.
pub(super) const DRIVE_PUSH_INTERVAL: Duration = Duration::from_secs(30);
pub(super) const DRIVE_PULL_INTERVAL: Duration = Duration::from_secs(60);

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

/// The Google Drive backend. `pool` serves one purpose: reading and writing
/// `auth_state`, which holds the access token and when it expires.
pub struct DriveClient {
    pool: DbPool,
}

impl DriveClient {
    pub(super) fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub(super) async fn ensure_credential(&self) -> Result<bool> {
        match auth::ensure_valid_token(&self.pool).await {
            Ok(_) => Ok(true),
            Err(Error::NotSignedIn) => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub(super) async fn list(&self, modified_after: &str) -> Result<Vec<FileMeta>> {
        self.with_token_retry(|tok| {
            let after = modified_after.to_string();
            async move { http_list_appdata_files(&tok, &after).await }
        })
        .await
    }

    pub(super) async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        self.with_token_retry(|tok| {
            let id = file_id.to_string();
            async move { http_download(&tok, &id).await }
        })
        .await
    }

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
