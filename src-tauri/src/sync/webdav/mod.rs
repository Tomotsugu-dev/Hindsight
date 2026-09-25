//! The WebDAV backend (ADR-0007, ADR-0008). [`WebDavClient`] is what the
//! engine sees: it reads the peers' manifests to find changes, downloads and
//! deletes. Its own state lives in `sync_cursor` under `webdav.` keys, which
//! the engine never reads.

// Removed when the client is wired in (ADR-0007 follow-up 3).
#![allow(dead_code)]

pub(crate) mod account_hash;
mod dav;
#[cfg(test)]
mod fake;
pub mod layout;
mod manifest;

#[cfg(test)]
pub(crate) use fake::{Call, FakeDav};

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use dav::{Dav, DavOps, HttpDav};
use layout::{flat_name_to_path, path_to_flat_name, temporary_upload_path};
use manifest::{manifest_path, manifest_to_device_id, Manifest};

use crate::error::{Error, Result};
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::cloud::{FailureKind, FileMeta};
use crate::sync::engine::{flat_name_to_dataset, rewind_cursor};

/// WebDAV Server only provides modification times with second precision (ADR-0007 §4).
pub(crate) const TIME_PRECISION: Duration = Duration::from_secs(1);

/// Sync interval (ADR-0007 §3): Nutstore limits
/// the number of requests, both push and pull are 5 minutes.
pub(crate) const PUSH_INTERVAL: Duration = Duration::from_secs(300);
pub(crate) const PULL_INTERVAL: Duration = Duration::from_secs(300);

/// How a file already on the server is replaced so that other devices never
/// read half of it. Servers differ here (ADR-0009).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UploadMethod {
    /// `PUT` straight onto the file name. Nutstore swaps a file in only after
    /// the whole body has arrived, and its `MOVE` refuses to overwrite.
    DirectPut,
    /// `PUT` to a `.tmp-` name, then `MOVE` onto the file name. For servers
    /// whose `PUT` can leave a half-written file behind, such as Nextcloud.
    TempThenMove,
}

pub(crate) fn failure_kind(e: &Error) -> FailureKind {
    match e {
        // WebDAV 401: the user name or app password is wrong or was revoked.
        // There is no refresh; the user has to enter it again (ADR-0007).
        Error::WebDavHttp { status: 401, .. } => FailureKind::CredentialInvalid,
        // Everything else is retried by the next tick.
        // TODO: WebDAV 507 (out of space) should stop retrying and tell the
        // user; that needs a third kind (ADR-0007 error table).
        _ => FailureKind::Transient,
    }
}

/// Nutstore's WebDAV host.
const NUTSTORE_HOST: &str = "dav.jianguoyun.com";

/// The upload method a server needs, told by its host.
fn upload_method_for(host: &str) -> UploadMethod {
    if host == NUTSTORE_HOST {
        UploadMethod::DirectPut
    } else {
        UploadMethod::TempThenMove
    }
}

/// The WebDAV backend. `pool` serves one purpose: reading and writing this
/// device's own rows in `sync_cursor` — its progress on each peer's manifest
/// (`webdav.peer.*`) and the changes not yet written into its own manifest
/// file (`webdav.pending`).
pub(crate) struct WebDavClient {
    dav: Dav,
    upload_method: UploadMethod,
    pool: DbPool,
    self_id: String,
    /// When the last manifest upload attempt ended. A failed attempt counts: the
    /// server may have written the file and only the response was lost.
    last_manifest_attempt: Mutex<Option<Instant>>,
}

/// This device's own files uploaded or deleted and not yet written into the
/// manifest file. Every change goes straight to the `webdav.pending`
/// row of `sync_cursor`, so closing the app loses nothing;
/// [`WebDavClient::end_push_round`] clears it once the manifest file is written.
/// Paths are relative to the device directory, like the manifest file's keys.
#[derive(Default, Serialize, Deserialize)]
struct PendingChanges {
    uploaded: BTreeSet<String>,
    removed: BTreeSet<String>,
}

impl PendingChanges {
    fn is_empty(&self) -> bool {
        self.uploaded.is_empty() && self.removed.is_empty()
    }
}

impl WebDavClient {
    pub(crate) fn new(
        dav: Dav,
        upload_method: UploadMethod,
        pool: DbPool,
        self_id: String,
    ) -> Self {
        Self {
            dav,
            upload_method,
            pool,
            self_id,
            last_manifest_attempt: Mutex::new(None),
        }
    }

    /// 测试用：连假服务器，按通用服务器的做法上传。几台设备传同一个 `dav`，就是共用
    /// 一个服务器。
    #[cfg(test)]
    pub(crate) fn with_fake_server(
        dav: std::sync::Arc<FakeDav>,
        pool: DbPool,
        self_id: String,
    ) -> Self {
        Self::new(Dav::Fake(dav), UploadMethod::TempThenMove, pool, self_id)
    }

    pub(crate) fn connect(
        server_url: &str,
        username: &str,
        password: &str,
        pool: DbPool,
        self_id: String,
    ) -> Result<Self> {
        let http = HttpDav::new(server_url, username, password)?;
        let upload_method = upload_method_for(http.host());
        Ok(Self::new(Dav::Http(http), upload_method, pool, self_id))
    }

    /// WebDAV doesn't have a concept of logging in;
    /// if the credentials are set in the configuration,
    /// it is considered logged in.
    pub(crate) async fn ensure_credential(&self) -> Result<bool> {
        Ok(true)
    }

    /// Finds the peers' files the running datasets still have to merge, per
    /// ADR-0008. `streams` names each running dataset by its cursor key, with
    /// its local cursor: how far the engine merged; a dataset not in it is
    /// left alone. Each file's `modified_time` is the server time of its
    /// manifest.
    pub(crate) async fn list(&self, streams: &[(&str, &str)]) -> Result<Vec<FileMeta>> {
        let entries = match self.dav.propfind("").await {
            Ok(entries) => entries,
            // A new account: nobody has pushed, so there is nothing to pull.
            Err(Error::WebDavHttp { status: 404, .. }) => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut out = Vec::new();
        for entry in entries {
            if entry.is_dir {
                continue;
            }
            let Some(path) = href_to_relative_path(&entry.href) else {
                continue;
            };
            let Some(device_id) = manifest_to_device_id(&path) else {
                continue;
            };
            if device_id == self.self_id {
                continue;
            }
            let Some(manifest_time) = entry.modified else {
                log::warn!("webdav: manifest of {device_id} has no modified time, skipped");
                continue;
            };

            // The running datasets that have not recorded this version yet.
            let mut bookmarks = read_bookmarks(&self.pool, device_id).await?;
            let behind: Vec<(&str, &str)> = streams
                .iter()
                .copied()
                .filter(|(dataset, _)| {
                    bookmarks
                        .get(*dataset)
                        .is_none_or(|b| b.manifest_time != manifest_time)
                })
                .collect();
            if behind.is_empty() {
                continue;
            }
            let manifest = match self.dav.get(&path).await {
                Ok(bytes) => match Manifest::parse(&bytes) {
                    Ok(m) => m,
                    Err(e) => {
                        log::warn!("webdav: manifest of {device_id} skipped: {e}");
                        continue;
                    }
                },
                Err(e) => {
                    log::warn!("webdav: manifest of {device_id} not fetched: {e}");
                    continue;
                }
            };

            // All files of one version are reported at `manifest_time`, and the
            // engine stores a dataset's local cursor rewound one second (#56), so
            // once a dataset merged this version its local cursor reads
            // `merged_cursor`. A dataset there or past it gets a bookmark and no
            // files; one short of it gets the files above its bookmark's count.
            let merged_cursor = rewind_cursor(&manifest_time, TIME_PRECISION)?;
            // Dataset → the count its bookmark holds; files above it get merged.
            let mut need_merge: BTreeMap<&str, u64> = BTreeMap::new();
            let mut bookmark_changed = false;
            for (dataset, local_cursor) in &behind {
                if *local_cursor >= merged_cursor.as_str() {
                    bookmarks.insert(
                        dataset.to_string(),
                        Bookmark {
                            processed: manifest.latest_push_count(),
                            manifest_time: manifest_time.clone(),
                        },
                    );
                    bookmark_changed = true;
                } else {
                    let processed = bookmarks.get(*dataset).map_or(0, |b| b.processed);
                    need_merge.insert(dataset, processed);
                }
            }
            if bookmark_changed {
                write_bookmarks(&self.pool, device_id, &bookmarks).await?;
            }

            for (file, push_count) in &manifest.files {
                let file_path = format!("{device_id}/{file}");
                let Some(name) = path_to_flat_name(&file_path) else {
                    log::debug!("webdav: {file_path} is not a sync file, skipped");
                    continue;
                };
                let Some(dataset) = flat_name_to_dataset(&name) else {
                    log::debug!("webdav: {name} is not a file this version knows, skipped");
                    continue;
                };
                let Some(processed) = need_merge.get(dataset) else {
                    continue;
                };
                if push_count <= processed {
                    continue;
                }
                out.push(FileMeta {
                    id: file_path,
                    name,
                    modified_time: manifest_time.clone(),
                    size: None,
                });
            }
        }
        out.sort_by(|a, b| a.modified_time.cmp(&b.modified_time));
        Ok(out)
    }

    pub(crate) async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        self.dav.get(file_id).await
    }

    /// Uploads one sync file to this device's own directory and adds it to this
    /// round's changes, which [`Self::end_push_round`] writes into the manifest.
    /// `name` is the engine's flat file name (shape defined in
    /// [`flat_name_to_path`]); a name that does not match is an error.
    /// Returns the path relative to the sync root.
    pub(crate) async fn upsert_by_name(&self, name: &str, content: &[u8]) -> Result<String> {
        let Some(path) = flat_name_to_path(name) else {
            return Err(Error::InvalidInputDyn(format!(
                "not a sync file name: {name}"
            )));
        };
        self.upload(&path, content).await?;
        if let Some(device_relative_path) = self.path_to_device_relative_path(&path) {
            let mut pending = load_pending(&self.pool).await?;
            pending.removed.remove(device_relative_path);
            pending.uploaded.insert(device_relative_path.to_string());
            store_pending(&self.pool, &pending).await?;
        }
        Ok(path)
    }

    pub(crate) async fn delete(&self, file_id: &str) -> Result<()> {
        self.dav.delete(file_id).await?;
        if let Some(device_relative_path) = self.path_to_device_relative_path(file_id) {
            let mut pending = load_pending(&self.pool).await?;
            pending.uploaded.remove(device_relative_path);
            pending.removed.insert(device_relative_path.to_string());
            store_pending(&self.pool, &pending).await?;
        }
        Ok(())
    }

    /// Applies the changes in [`PendingChanges`] to this device's manifest (read
    /// by [`Self::own_manifest`]) and uploads the result to the sync root as the
    /// manifest file. With no changes, it returns at once and sends no request.
    ///
    /// This round's push count = the highest push count in the manifest file + 1.
    /// Uploaded files get this round's push count; deleted files are removed from
    /// the manifest file. Example: the highest is 5, and this round uploaded
    /// `categories.json` and deleted `icons.json` → `categories.json` gets 6 and
    /// `icons.json` is removed.
    ///
    /// The pending changes are cleared only after the manifest file is uploaded.
    /// If the upload fails, nothing changes locally, and the next round tries
    /// again with the same pending changes.
    pub(crate) async fn end_push_round(&self) -> Result<()> {
        let pending = load_pending(&self.pool).await?;
        if pending.is_empty() {
            return Ok(());
        }
        let mut manifest = self.own_manifest().await?;
        let push_count = manifest.latest_push_count() + 1;
        for file in &pending.uploaded {
            manifest.files.insert(file.clone(), push_count);
        }
        for file in &pending.removed {
            manifest.files.remove(file);
        }
        let bytes = manifest.to_bytes()?;

        // Ensure that at least one second has passed since
        // the last manifest upload. (Ensures server time difference for consecutive uploads.)
        let mut last_attempt = self.last_manifest_attempt.lock().await;
        if let Some(at) = *last_attempt {
            if at.elapsed() < TIME_PRECISION {
                tokio::time::sleep(TIME_PRECISION - at.elapsed()).await;
            }
        }
        let uploaded = self.upload(&manifest_path(&self.self_id), &bytes).await;
        *last_attempt = Some(Instant::now());
        uploaded?;

        clear_pending(&self.pool).await
    }

    /// Root-relative path → device-relative path, which is the key in the
    /// manifest: `<own device id>/categories.json` gives `categories.json`.
    /// Returns `None` for anything outside this device's directory (another
    /// device's file, the manifest itself).
    fn path_to_device_relative_path<'a>(&self, path: &'a str) -> Option<&'a str> {
        path.strip_prefix(&format!("{}/", self.self_id))
            .filter(|rest| !rest.is_empty())
    }

    /// Uploads a file to the WebDAV server by its [`UploadMethod`], so that
    /// other devices read either the old content or the new, never half of it.
    async fn upload(&self, path: &str, content: &[u8]) -> Result<()> {
        let put_path = match self.upload_method {
            UploadMethod::DirectPut => path.to_string(),
            UploadMethod::TempThenMove => temporary_upload_path(path),
        };
        match self.dav.put(&put_path, content.to_vec()).await {
            Ok(()) => {}
            // If the initial PUT fails with a 409 (conflict), it means the parent
            // directories might not exist, so we create them and retry.
            Err(Error::WebDavHttp { status: 409, .. }) => {
                self.create_parents(path).await?;
                self.dav.put(&put_path, content.to_vec()).await?;
            }
            Err(e) => return Err(e),
        }
        if self.upload_method == UploadMethod::TempThenMove {
            self.dav.mv(&put_path, path).await?;
        }
        Ok(())
    }

    /// Creates parent directories for a given path, starting from the sync root.
    async fn create_parents(&self, path: &str) -> Result<()> {
        // The sync root first: on a new account it does not exist yet.
        match self.dav.mkcol("").await {
            Ok(()) | Err(Error::WebDavHttp { status: 405, .. }) => {}
            Err(e) => return Err(e),
        }
        let parent = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
        let mut dir = String::new();
        for segment in parent.split('/').filter(|s| !s.is_empty()) {
            dir = if dir.is_empty() {
                segment.to_string()
            } else {
                format!("{dir}/{segment}")
            };
            match self.dav.mkcol(&dir).await {
                // If the directory already exists (405), we skip it.
                Ok(()) | Err(Error::WebDavHttp { status: 405, .. }) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// This device's manifest file as the cloud has it now, read fresh every
    /// time; nothing is kept locally. A 404 is the server saying "no such file":
    /// this device has not pushed yet, so the manifest is empty. A dropped
    /// connection or a timeout gets no response at all; that is a different
    /// error, so the round fails and the next round tries again.
    ///
    /// With [`UploadMethod::TempThenMove`], a 404 first falls back to the
    /// `.tmp-` copy: a server may delete the target of a `MOVE` before moving,
    /// and a failure in between leaves only that copy, the newest version.
    async fn own_manifest(&self) -> Result<Manifest> {
        let path = manifest_path(&self.self_id);
        match self.dav.get(&path).await {
            Ok(bytes) => return Manifest::parse(&bytes),
            Err(Error::WebDavHttp { status: 404, .. }) => {}
            Err(e) => return Err(e),
        }
        if self.upload_method == UploadMethod::TempThenMove {
            match self.dav.get(&temporary_upload_path(&path)).await {
                Ok(bytes) => return Manifest::parse(&bytes),
                Err(Error::WebDavHttp { status: 404, .. }) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(Manifest::new())
    }
}

/// Converts the `href` from a `PROPFIND` response into a path relative to the
/// sync root: percent-decoded, with everything up to and including the root
/// directory (`/hindsight/`) removed. Returns `None` if the `href` has no root
/// directory, or if percent-decoding fails.
fn href_to_relative_path(href: &str) -> Option<String> {
    let decoded = urlencoding::decode(href).ok()?;
    let root_in_href = format!("/{}", dav::ROOT_DIR); // "/hindsight/"
    let path_start = decoded.find(&root_in_href)? + root_in_href.len();
    Some(decoded[path_start..].to_string())
}

// ─────────────── Local state in sync_cursor ───────────────

/// `sync_cursor` key of the changes not yet written into the manifest.
const PENDING_KEY: &str = "webdav.pending";

/// How far one dataset got with one peer's manifest file: `processed` is the
/// push count it merged up to, `manifest_time` the server time of the manifest
/// version that was recorded at. That version is not fetched again for it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
struct Bookmark {
    processed: u64,
    manifest_time: String,
}

/// One row per peer, `webdav.peer.<device id>`: its bookmarks by dataset,
/// keyed by the engine's cursor key, as JSON. A dataset with no bookmark has
/// never run against that peer.
fn peer_key(device_id: &str) -> String {
    format!("webdav.peer.{device_id}")
}

async fn read_bookmarks(pool: &DbPool, device_id: &str) -> Result<BTreeMap<String, Bookmark>> {
    match load_state(pool, &peer_key(device_id)).await? {
        Some(json) => serde_json::from_str(&json).map_err(|e| Error::SyncParse {
            kind: "webdav bookmarks",
            source: e,
        }),
        None => Ok(BTreeMap::new()),
    }
}

async fn write_bookmarks(
    pool: &DbPool,
    device_id: &str,
    bookmarks: &BTreeMap<String, Bookmark>,
) -> Result<()> {
    store_state(
        pool,
        &peer_key(device_id),
        &serde_json::to_string(bookmarks)?,
    )
    .await
}

async fn load_pending(pool: &DbPool) -> Result<PendingChanges> {
    match load_state(pool, PENDING_KEY).await? {
        Some(json) => serde_json::from_str(&json).map_err(|e| Error::SyncParse {
            kind: "webdav pending changes",
            source: e,
        }),
        None => Ok(PendingChanges::default()),
    }
}

async fn store_pending(pool: &DbPool, pending: &PendingChanges) -> Result<()> {
    store_state(pool, PENDING_KEY, &serde_json::to_string(pending)?).await
}

async fn clear_pending(pool: &DbPool) -> Result<()> {
    pool.0
        .call(|conn| {
            conn.execute("DELETE FROM sync_cursor WHERE entity = ?1", [PENDING_KEY])
                .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Return the `last_pulled_at` value associated with the given key from the `sync_cursor` table.
/// Returns `None` if the key does not exist.
async fn load_state(pool: &DbPool, key: &str) -> Result<Option<String>> {
    let key = key.to_string();
    Ok(pool
        .0
        .call(move |conn| {
            conn.query_row(
                "SELECT last_pulled_at FROM sync_cursor WHERE entity = ?1",
                [&key],
                |r| r.get(0),
            )
            .optional()
            .db()
        })
        .await?)
}

/// Writes the given value associated with the given key to the `sync_cursor` table.
/// If the key already exists, its value is updated.
async fn store_state(pool: &DbPool, key: &str, value: &str) -> Result<()> {
    let key = key.to_string();
    let value = value.to_string();
    pool.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO sync_cursor(entity, last_pulled_at) VALUES(?1, ?2)
                 ON CONFLICT(entity) DO UPDATE SET last_pulled_at = excluded.last_pulled_at",
                rusqlite::params![key, value],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::fake::{Call, FakeDav};
    use super::*;
    use crate::repo::test_util::fresh_test_pool;
    use crate::sync::engine::CURSOR_CORE as CORE;

    const EPOCH: &str = "1970-01-01T00:00:00Z";

    async fn client(dav: &Arc<FakeDav>) -> WebDavClient {
        WebDavClient::with_fake_server(dav.clone(), fresh_test_pool().await, "me".into())
    }

    /// 只跑核心数据集的一轮拉取，游标是 `cursor`。
    async fn list_core(client: &WebDavClient, cursor: &str) -> Vec<FileMeta> {
        client.list(&[(CORE, cursor)]).await.unwrap()
    }

    async fn seed_manifest(dav: &FakeDav, device: &str, files: &[(&str, u64)]) -> String {
        let mut m = Manifest::new();
        for (f, n) in files {
            m.files.insert((*f).to_string(), *n);
        }
        dav.seed_file(&manifest_path(device), &m.to_bytes().unwrap())
            .await
    }

    async fn cloud_manifest(dav: &FakeDav, device: &str) -> Manifest {
        Manifest::parse(&dav.file(&manifest_path(device)).await.unwrap()).unwrap()
    }

    /// href 剥前缀、解码；根目录本身是空串；不在根目录下的是 None。
    #[test]
    fn href_to_relative_path_strips_the_server_prefix() {
        assert_eq!(
            href_to_relative_path("/dav/hindsight/manifest.abc.json").as_deref(),
            Some("manifest.abc.json")
        );
        assert_eq!(
            href_to_relative_path("https://dav.example.com/dav/hindsight/a%20b/").as_deref(),
            Some("a b/")
        );
        assert_eq!(
            href_to_relative_path("/dav/hindsight/").as_deref(),
            Some("")
        );
        assert_eq!(href_to_relative_path("/dav/other/x.json"), None);
    }

    /// 一台对端一行，里面按数据集各一个书签，写进去读出来一样；没有记录时是空的。
    #[tokio::test]
    async fn bookmarks_roundtrip_in_sync_cursor() {
        let pool = fresh_test_pool().await;
        assert!(read_bookmarks(&pool, "abc").await.unwrap().is_empty());
        let mut marks = BTreeMap::new();
        marks.insert(
            CORE.to_string(),
            Bookmark {
                processed: 4,
                manifest_time: "2026-05-15T10:00:03Z".into(),
            },
        );
        write_bookmarks(&pool, "abc", &marks).await.unwrap();
        assert_eq!(read_bookmarks(&pool, "abc").await.unwrap(), marks);
    }

    // ───── 读路径 ─────

    /// 云上什么都没有：一轮就一个 PROPFIND，报空。
    #[tokio::test]
    async fn empty_root_costs_one_request() {
        let dav = Arc::new(FakeDav::new());
        let client = client(&dav).await;
        assert!(list_core(&client, EPOCH).await.is_empty());
        assert_eq!(dav.calls().await, vec![Call::Propfind("".into())]);
    }

    /// 第一轮：对端的 manifest 文件没见过 → GET 它，报出全部文件，时间都是 manifest 文件的
    /// 服务器时间。自己的 manifest 文件不碰。
    #[tokio::test]
    async fn first_round_reports_every_file_of_a_new_peer() {
        let dav = Arc::new(FakeDav::new());
        let t = seed_manifest(
            &dav,
            "abc",
            &[
                ("categories.json", 1),
                ("activities/2026/2026-09-20.ndjson", 2),
            ],
        )
        .await;
        seed_manifest(&dav, "me", &[("categories.json", 9)]).await;
        let client = client(&dav).await;

        let files = list_core(&client, EPOCH).await;
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "device.abc.activities.2026-09-20.ndjson",
                "device.abc.categories.json"
            ]
        );
        assert!(files.iter().all(|f| f.modified_time == t));
        assert_eq!(files[0].id, "abc/activities/2026/2026-09-20.ndjson");
        assert_eq!(
            dav.calls().await,
            vec![
                Call::Propfind("".into()),
                Call::Get("manifest.abc.json".into())
            ]
        );
    }

    /// 引擎的游标过了 manifest 文件时间减一秒 → 推进本机记录、这轮不报；之后空转只剩一个 PROPFIND。
    /// 游标没过（有文件合并失败）→ 下轮重新 GET、重报。
    #[tokio::test]
    async fn advances_only_when_the_engine_got_past_the_manifest() {
        let dav = Arc::new(FakeDav::new());
        let t = seed_manifest(&dav, "abc", &[("categories.json", 1)]).await;
        let client = client(&dav).await;
        assert_eq!(list_core(&client, EPOCH).await.len(), 1);

        // 合并失败：游标还在 EPOCH → 重报
        assert_eq!(list_core(&client, EPOCH).await.len(), 1);

        // 合并成功：游标 = t 退一秒 → 推进，不报
        let cursor = rewind_cursor(&t, TIME_PRECISION).unwrap();
        assert!(list_core(&client, &cursor).await.is_empty());
        assert_eq!(
            read_bookmarks(&client.pool, "abc").await.unwrap()[CORE],
            Bookmark {
                processed: 1,
                manifest_time: t.clone()
            }
        );

        // 之后每轮只有一个 PROPFIND
        let before = dav.calls().await.len();
        assert!(list_core(&client, &cursor).await.is_empty());
        let calls = dav.calls().await;
        assert_eq!(&calls[before..], &[Call::Propfind("".into())]);
    }

    /// 没在跑的数据集，书签不动。核心跑过这一版之后 AI 才开：AI 的游标还在 EPOCH，
    /// 只报 AI 的文件，核心的不再报；AI 也跑过之后，空转只剩一个 PROPFIND。
    #[tokio::test]
    async fn a_dataset_that_was_off_gets_its_files_when_it_runs() {
        let dav = Arc::new(FakeDav::new());
        let t = seed_manifest(
            &dav,
            "abc",
            &[("categories.json", 1), ("ai_summaries.json", 1)],
        )
        .await;
        let client = client(&dav).await;
        let ai = flat_name_to_dataset("device.abc.ai_summaries.json").unwrap();

        // 只有核心在跑：只报核心的文件
        let names: Vec<String> = list_core(&client, EPOCH)
            .await
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names, vec!["device.abc.categories.json"]);
        let cursor = rewind_cursor(&t, TIME_PRECISION).unwrap();
        assert!(list_core(&client, &cursor).await.is_empty());

        // AI 开了：只报 AI 的文件
        let files = client.list(&[(CORE, &cursor), (ai, EPOCH)]).await.unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["device.abc.ai_summaries.json"]);
        assert_eq!(files[0].modified_time, t);

        // AI 也推进后：一个 GET 都不发
        client
            .list(&[(CORE, &cursor), (ai, &cursor)])
            .await
            .unwrap();
        let before = dav.calls().await.len();
        assert!(client
            .list(&[(CORE, &cursor), (ai, &cursor)])
            .await
            .unwrap()
            .is_empty());
        assert_eq!(&dav.calls().await[before..], &[Call::Propfind("".into())]);
    }

    /// 对端又 push 了：只报 push 次数比本机记的大的文件。
    #[tokio::test]
    async fn a_later_push_reports_only_the_changed_files() {
        let dav = Arc::new(FakeDav::new());
        let t1 = seed_manifest(
            &dav,
            "abc",
            &[
                ("categories.json", 1),
                ("activities/2026/2026-09-20.ndjson", 2),
            ],
        )
        .await;
        let client = client(&dav).await;
        list_core(&client, EPOCH).await;
        let cursor = rewind_cursor(&t1, TIME_PRECISION).unwrap();
        list_core(&client, &cursor).await; // 推进到 2

        let t2 = seed_manifest(
            &dav,
            "abc",
            &[
                ("categories.json", 3),
                ("activities/2026/2026-09-20.ndjson", 2),
            ],
        )
        .await;
        let files = list_core(&client, &cursor).await;
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "device.abc.categories.json");
        assert_eq!(files[0].modified_time, t2);
    }

    /// manifest 文件坏了：跳过这台设备、别的照常，不整轮失败。
    #[tokio::test]
    async fn a_broken_manifest_skips_that_peer_only() {
        let dav = Arc::new(FakeDav::new());
        dav.seed_file("manifest.bad.json", b"<html>").await;
        seed_manifest(&dav, "good", &[("categories.json", 1)]).await;
        let client = client(&dav).await;
        let files = list_core(&client, EPOCH).await;
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "device.good.categories.json");
    }

    // ───── 写路径 ─────

    /// 第一次上传：PUT 撞 409 → 从根目录起逐级 MKCOL（根目录已有，405 跳过）→ 再 PUT → MOVE；
    /// 临时文件不留。
    /// 同目录第二次上传：只有 PUT + MOVE。
    #[tokio::test]
    async fn upsert_creates_missing_directories_then_moves_into_place() {
        let dav = Arc::new(FakeDav::new());
        let client = client(&dav).await;
        let day = "me/activities/2026/2026-09-20.ndjson";
        let temp = "me/activities/2026/.tmp-2026-09-20.ndjson";

        let id = client
            .upsert_by_name("device.me.activities.2026-09-20.ndjson", b"rows")
            .await
            .unwrap();
        assert_eq!(id, day);
        assert_eq!(
            dav.calls().await,
            vec![
                Call::Put(temp.into()),
                Call::Mkcol("".into()),
                Call::Mkcol("me".into()),
                Call::Mkcol("me/activities".into()),
                Call::Mkcol("me/activities/2026".into()),
                Call::Put(temp.into()),
                Call::Move(temp.into(), day.into()),
            ]
        );
        assert_eq!(dav.file(day).await.as_deref(), Some(&b"rows"[..]));
        assert!(dav.file(temp).await.is_none());

        let before = dav.calls().await.len();
        client
            .upsert_by_name("device.me.activities.2026-09-21.ndjson", b"more")
            .await
            .unwrap();
        let calls = dav.calls().await;
        assert_eq!(
            &calls[before..],
            &[
                Call::Put("me/activities/2026/.tmp-2026-09-21.ndjson".into()),
                Call::Move(
                    "me/activities/2026/.tmp-2026-09-21.ndjson".into(),
                    "me/activities/2026/2026-09-21.ndjson".into()
                ),
            ]
        );
    }

    /// 不是同步文件名的直接拒绝，不打服务器。
    #[tokio::test]
    async fn upsert_rejects_names_the_engine_never_writes() {
        let dav = Arc::new(FakeDav::new());
        let client = client(&dav).await;
        assert!(client.upsert_by_name("readme.txt", b"x").await.is_err());
        assert!(dav.calls().await.is_empty());
    }

    /// 一轮 push 的最后一步上传 manifest 文件：本轮上传的文件记成新的 push 次数，别的不动；
    /// 没动过的一轮不上传。
    #[tokio::test]
    async fn end_push_round_writes_the_manifest_with_push_counts() {
        let dav = Arc::new(FakeDav::new());
        let client = client(&dav).await;
        client
            .upsert_by_name("device.me.categories.json", b"[]")
            .await
            .unwrap();
        client
            .upsert_by_name("device.me.activities.2026-09-20.ndjson", b"rows")
            .await
            .unwrap();
        client.end_push_round().await.unwrap();
        let m = cloud_manifest(&dav, "me").await;
        assert_eq!(
            m.files,
            [
                ("categories.json".to_string(), 1),
                ("activities/2026/2026-09-20.ndjson".to_string(), 1),
            ]
            .into_iter()
            .collect()
        );
        // manifest 文件自己也走 .tmp- + MOVE
        assert!(dav.calls().await.contains(&Call::Move(
            ".tmp-manifest.me.json".into(),
            "manifest.me.json".into()
        )));

        client
            .upsert_by_name("device.me.categories.json", b"[1]")
            .await
            .unwrap();
        client.end_push_round().await.unwrap();
        let m = cloud_manifest(&dav, "me").await;
        assert_eq!(m.files["categories.json"], 2);
        assert_eq!(m.files["activities/2026/2026-09-20.ndjson"], 1);

        let before = dav.calls().await.len();
        client.end_push_round().await.unwrap();
        assert_eq!(dav.calls().await.len(), before);
    }

    /// 连着两轮都有改动：第二轮等满一秒才传 manifest 文件。同一秒里的两版服务器时间
    /// 一样，对端会把第二版当成处理过。
    #[tokio::test]
    async fn the_next_manifest_upload_waits_for_a_new_second() {
        let dav = Arc::new(FakeDav::new());
        let client = client(&dav).await;
        client
            .upsert_by_name("device.me.categories.json", b"[]")
            .await
            .unwrap();
        client.end_push_round().await.unwrap();

        client
            .upsert_by_name("device.me.icons.json", b"[]")
            .await
            .unwrap();
        let started = Instant::now();
        client.end_push_round().await.unwrap();
        // 等的是一秒减去两轮之间已经过去的那几毫秒
        assert!(started.elapsed() >= Duration::from_millis(900));
        assert_eq!(cloud_manifest(&dav, "me").await.files["icons.json"], 2);
    }

    /// 上传之后 app 关了（manifest 文件还没上传）：再起来的客户端从库里读到本轮改动，
    /// manifest 文件照常上传。
    #[tokio::test]
    async fn pending_changes_survive_a_restart() {
        let dav = Arc::new(FakeDav::new());
        let pool = fresh_test_pool().await;
        let before = WebDavClient::with_fake_server(dav.clone(), pool.clone(), "me".into());
        before
            .upsert_by_name("device.me.categories.json", b"[]")
            .await
            .unwrap();
        drop(before);

        let after = WebDavClient::with_fake_server(dav.clone(), pool, "me".into());
        after.end_push_round().await.unwrap();
        let m = cloud_manifest(&dav, "me").await;
        assert_eq!(m.files["categories.json"], 1);

        // 写完就清了：再来一轮什么都不发
        let calls = dav.calls().await.len();
        after.end_push_round().await.unwrap();
        assert_eq!(dav.calls().await.len(), calls);
    }

    /// push 次数从云上的 manifest 文件接着数，不从 1 重来；没动过的文件保持原来的次数。
    #[tokio::test]
    async fn push_count_continues_from_the_cloud_manifest() {
        let dav = Arc::new(FakeDav::new());
        seed_manifest(&dav, "me", &[("icons.json", 5)]).await;
        let client = client(&dav).await;
        client
            .upsert_by_name("device.me.categories.json", b"[]")
            .await
            .unwrap();
        client.end_push_round().await.unwrap();
        let m = cloud_manifest(&dav, "me").await;
        assert_eq!(m.files["icons.json"], 5);
        assert_eq!(m.files["categories.json"], 6);
    }

    /// manifest 文件传上去了、清 pending 之前崩了：下一轮从云上的 6 往上数，没清掉的旧改动
    /// 和新文件都记成 7。已经处理到 6 的对端两个都会取，不会漏。
    #[tokio::test]
    async fn a_crash_before_clearing_pending_loses_nothing() {
        let dav = Arc::new(FakeDav::new());
        let client = client(&dav).await;
        client
            .upsert_by_name("device.me.categories.json", b"[]")
            .await
            .unwrap();
        // 上一轮的 manifest 文件已经在云上，pending 却没清
        seed_manifest(&dav, "me", &[("categories.json", 6)]).await;

        client
            .upsert_by_name("device.me.icons.json", b"[]")
            .await
            .unwrap();
        client.end_push_round().await.unwrap();
        let m = cloud_manifest(&dav, "me").await;
        assert_eq!(m.files["categories.json"], 7);
        assert_eq!(m.files["icons.json"], 7);
    }

    /// 删掉自己的文件，下一次上传 manifest 文件时它就不在里面了。
    #[tokio::test]
    async fn delete_drops_the_file_from_the_manifest() {
        let dav = Arc::new(FakeDav::new());
        let client = client(&dav).await;
        client
            .upsert_by_name("device.me.categories.json", b"[]")
            .await
            .unwrap();
        client
            .upsert_by_name("device.me.icons.json", b"[]")
            .await
            .unwrap();
        client.end_push_round().await.unwrap();

        client.delete("me/icons.json").await.unwrap();
        client.end_push_round().await.unwrap();
        let m = cloud_manifest(&dav, "me").await;
        assert_eq!(m.files.len(), 1);
        assert_eq!(m.files["categories.json"], 1);
        assert!(dav.file("me/icons.json").await.is_none());
    }

    /// 401 要用户重新填账号密码；别的状态码等下一轮重试。
    #[test]
    fn failure_kind_asks_to_sign_in_only_for_401() {
        let http = |status| Error::WebDavHttp {
            stage: "propfind",
            status,
            body: String::new(),
        };
        assert_eq!(failure_kind(&http(401)), FailureKind::CredentialInvalid);
        assert_eq!(failure_kind(&http(403)), FailureKind::Transient);
        assert_eq!(failure_kind(&http(507)), FailureKind::Transient);
    }

    // ───── 各家服务器的上传方式 ─────

    /// connect 按用户填的地址定上传方式：坚果云直接 PUT，其他一律临时名加 MOVE。
    #[tokio::test]
    async fn connect_picks_the_upload_method_from_the_address() {
        let pool = fresh_test_pool().await;
        let nutstore = WebDavClient::connect(
            "https://dav.jianguoyun.com/dav/",
            "a",
            "b",
            pool.clone(),
            "me".into(),
        )
        .unwrap();
        assert_eq!(nutstore.upload_method, UploadMethod::DirectPut);
        let other = WebDavClient::connect(
            "https://cloud.example.com/remote.php/dav/files/a/",
            "a",
            "b",
            pool,
            "me".into(),
        )
        .unwrap();
        assert_eq!(other.upload_method, UploadMethod::TempThenMove);
    }

    /// 坚果云的做法：直接 PUT 到真名，第二次上传覆盖旧内容，不用 MOVE。
    #[tokio::test]
    async fn direct_put_replaces_a_file_without_move() {
        let dav = Arc::new(FakeDav::new());
        dav.refuse_move_overwrite(true).await;
        let client = WebDavClient::new(
            Dav::Fake(dav.clone()),
            UploadMethod::DirectPut,
            fresh_test_pool().await,
            "me".into(),
        );
        client
            .upsert_by_name("device.me.categories.json", b"[]")
            .await
            .unwrap();
        client
            .upsert_by_name("device.me.categories.json", b"[1]")
            .await
            .unwrap();
        assert_eq!(
            dav.file("me/categories.json").await.as_deref(),
            Some(&b"[1]"[..])
        );
        assert_eq!(
            dav.calls().await,
            vec![
                Call::Put("me/categories.json".into()),
                Call::Mkcol("".into()),
                Call::Mkcol("me".into()),
                Call::Put("me/categories.json".into()),
                Call::Put("me/categories.json".into()),
            ]
        );
    }

    /// 服务器不许 MOVE 覆盖时，临时名加 MOVE 第二次上传就报错：坏法看得见，旧内容原样。
    #[tokio::test]
    async fn temp_then_move_fails_loudly_where_move_cannot_overwrite() {
        let dav = Arc::new(FakeDav::new());
        dav.refuse_move_overwrite(true).await;
        let client = client(&dav).await;
        client
            .upsert_by_name("device.me.categories.json", b"[]")
            .await
            .unwrap();
        assert!(matches!(
            client
                .upsert_by_name("device.me.categories.json", b"[1]")
                .await,
            Err(Error::WebDavHttp {
                stage: "move",
                status: 409,
                ..
            })
        ));
        assert_eq!(
            dav.file("me/categories.json").await.as_deref(),
            Some(&b"[]"[..])
        );
    }

    /// 服务器 MOVE 时先删了目标、没移过去就失败：自己的 manifest 文件只剩 `.tmp-` 那份，
    /// 从它接着数，不从 1 重来。
    #[tokio::test]
    async fn own_manifest_falls_back_to_its_temporary_copy() {
        let dav = Arc::new(FakeDav::new());
        let mut left = Manifest::new();
        left.files.insert("icons.json".into(), 5);
        dav.seed_file(".tmp-manifest.me.json", &left.to_bytes().unwrap())
            .await;
        let client = client(&dav).await;
        client
            .upsert_by_name("device.me.categories.json", b"[]")
            .await
            .unwrap();
        client.end_push_round().await.unwrap();
        let m = cloud_manifest(&dav, "me").await;
        assert_eq!(m.files["icons.json"], 5);
        assert_eq!(m.files["categories.json"], 6);
    }

    // ───── 新开的账号：同步根目录还不存在 ─────

    /// 列根目录遇到 404 当空的（ADR-0007 错误表：列目录 404 是空目录），不让整轮失败。
    #[tokio::test]
    async fn list_on_a_new_account_is_empty() {
        let dav = Arc::new(FakeDav::without_root());
        let client = client(&dav).await;
        assert!(list_core(&client, EPOCH).await.is_empty());
    }

    /// 第一次上传连根目录一起建，不然逐级 MKCOL 的第一级就撞 409；manifest 文件也写得进去。
    #[tokio::test]
    async fn the_first_upload_on_a_new_account_creates_the_root() {
        let dav = Arc::new(FakeDav::without_root());
        let client = client(&dav).await;
        client
            .upsert_by_name("device.me.categories.json", b"[]")
            .await
            .unwrap();
        client.end_push_round().await.unwrap();
        assert_eq!(
            dav.file("me/categories.json").await.as_deref(),
            Some(&b"[]"[..])
        );
        assert_eq!(cloud_manifest(&dav, "me").await.files["categories.json"], 1);
    }

    // ───── 真机探测 ─────
    //
    // 打真服务器，平时不跑：
    //   HINDSIGHT_WEBDAV_URL=https://dav.jianguoyun.com/dav/ \
    //   HINDSIGHT_WEBDAV_USER=<账号> HINDSIGHT_WEBDAV_PASSWORD=<应用密码> \
    //   cargo test webdav::tests::probe_ -- --ignored
    // 每次跑都在 `<地址>/hindsight-probe/` 下新建一个子目录，只在里面读写，跑完删掉子目录，
    // 不碰真的同步根目录。坚果云不许删 `/dav/` 下的顶层目录，`hindsight-probe/` 本身会留着，
    // 要删就在网页上删。

    fn probe_account() -> (String, String, String) {
        let var =
            |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("真机探测要先设环境变量 {k}"));
        (
            var("HINDSIGHT_WEBDAV_URL"),
            var("HINDSIGHT_WEBDAV_USER"),
            var("HINDSIGHT_WEBDAV_PASSWORD"),
        )
    }

    /// 这一次探测的目录 `<地址>/hindsight-probe/<name>-<毫秒时间戳>/`，新建、空的。返回它，
    /// 当作客户端的「用户填的地址」；同步根目录是它下面的 `hindsight/`，还不存在，跟新账号一样。
    async fn probe_setup(url: &str, name: &str, user: &str, pass: &str) -> String {
        let parent = format!("{}/hindsight-probe/", url.trim_end_matches('/'));
        let run = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let base = format!("{parent}{name}-{run}/");
        let mkcol = |dir: String| {
            reqwest::Client::new()
                .request(reqwest::Method::from_bytes(b"MKCOL").unwrap(), dir)
                .basic_auth(user, Some(pass))
                .send()
        };
        // 已经有了：坚果云回 201，别家回 405，都行
        let resp = mkcol(parent).await.unwrap();
        assert!(
            resp.status().is_success() || resp.status().as_u16() == 405,
            "建探测目录失败：{}",
            resp.status()
        );
        let resp = mkcol(base.clone()).await.unwrap();
        assert!(
            resp.status().is_success(),
            "建探测目录失败：{}",
            resp.status()
        );
        base
    }

    async fn probe_cleanup(base: &str, user: &str, pass: &str) {
        let _ = reqwest::Client::new()
            .delete(base)
            .basic_auth(user, Some(pass))
            .send()
            .await;
    }

    /// 服务器回的状态码；成功是 `None`，不是服务器回的错误直接失败。
    fn status_of<T>(r: Result<T>) -> Option<u16> {
        match r {
            Ok(_) => None,
            Err(Error::WebDavHttp { status, .. }) => Some(status),
            Err(e) => panic!("不是服务器回的状态码：{e}"),
        }
    }

    /// 假服务器照搬的规则，在真服务器上逐条核对：404 / 405 / 409、按这台服务器的上传方式
    /// 替换已有的文件、删不存在的算成功、`PROPFIND` 的 href 和时间格式。
    #[tokio::test]
    #[ignore = "打真服务器：要网络和三个环境变量"]
    async fn probe_real_server_follows_the_fake_servers_rules() {
        let (url, user, pass) = probe_account();
        let base = probe_setup(&url, "rules", &user, &pass).await;
        let dav = HttpDav::new(&base, &user, &pass).unwrap();

        // 新账号：根目录还不存在
        assert_eq!(status_of(dav.propfind("").await), Some(404));
        dav.mkcol("").await.unwrap();
        // 已有的目录再 MKCOL：RFC 4918 说回 405，坚果云回 201。客户端两种都当「已经有了」
        assert!(matches!(status_of(dav.mkcol("").await), None | Some(405)));

        // 父目录不存在
        assert_eq!(
            status_of(dav.put("a/b.json", b"x".to_vec()).await),
            Some(409)
        );
        assert_eq!(status_of(dav.mkcol("a/b").await), Some(409));

        // 按这台服务器的上传方式替换已有的文件，读到的是新内容
        dav.mkcol("a").await.unwrap();
        dav.put("a/b.json", b"old".to_vec()).await.unwrap();
        match upload_method_for(dav.host()) {
            UploadMethod::DirectPut => dav.put("a/b.json", b"new".to_vec()).await.unwrap(),
            UploadMethod::TempThenMove => {
                dav.put("a/.tmp-b.json", b"new".to_vec()).await.unwrap();
                dav.mv("a/.tmp-b.json", "a/b.json").await.unwrap();
                assert_eq!(status_of(dav.get("a/.tmp-b.json").await), Some(404));
            }
        }
        assert_eq!(dav.get("a/b.json").await.unwrap(), b"new");
        dav.delete("a/missing.json").await.unwrap();

        // PROPFIND：href 还原得成相对路径（名字带空格也行），文件带秒级 UTC 时间
        dav.put("a b.json", b"{}".to_vec()).await.unwrap();
        let entries = dav.propfind("").await.unwrap();
        let mut listed: Vec<(String, bool)> = entries
            .iter()
            .map(|e| {
                // 坚果云的目录 href 不带尾斜杠，根目录也是；`list` 先跳过目录再解析 href，不受影响
                let href = if e.is_dir && !e.href.ends_with('/') {
                    format!("{}/", e.href)
                } else {
                    e.href.clone()
                };
                let path = href_to_relative_path(&href).expect("href 在根目录下");
                (path.trim_end_matches('/').to_string(), e.is_dir)
            })
            .collect();
        listed.sort();
        assert_eq!(
            listed,
            vec![
                (String::new(), true),
                ("a".to_string(), true),
                ("a b.json".to_string(), false),
            ]
        );
        let file = entries.iter().find(|e| !e.is_dir).unwrap();
        let modified = file.modified.as_deref().expect("文件有修改时间");
        assert!(modified.ends_with('Z'), "{modified}");
        let t = chrono::DateTime::parse_from_rfc3339(modified).unwrap();
        let drift = chrono::Utc::now() - t.with_timezone(&chrono::Utc);
        assert!(
            drift.num_minutes().abs() < 10,
            "服务器时间差太多：{modified}"
        );

        probe_cleanup(&base, &user, &pass).await;
    }

    /// 两台设备走真服务器，从新账号开始：A 上传、写 manifest 文件，B 列出来、下载；
    /// A 再改一次，B 拿到新内容。
    #[tokio::test]
    #[ignore = "打真服务器：要网络和三个环境变量"]
    async fn probe_two_clients_sync_through_the_real_server() {
        let (url, user, pass) = probe_account();
        let base = probe_setup(&url, "sync", &user, &pass).await;
        let connect = |id: &str, pool| {
            WebDavClient::connect(&base, &user, &pass, pool, id.to_string()).unwrap()
        };
        let a = connect("probe-a", fresh_test_pool().await);
        let b = connect("probe-b", fresh_test_pool().await);

        assert!(list_core(&b, EPOCH).await.is_empty());
        a.upsert_by_name("device.probe-a.categories.json", b"[]")
            .await
            .unwrap();
        a.end_push_round().await.unwrap();

        let first = list_core(&b, EPOCH).await;
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].name, "device.probe-a.categories.json");
        assert_eq!(b.download(&first[0].id).await.unwrap(), b"[]");

        // 隔一秒以上再写，服务器时间才会不同
        tokio::time::sleep(Duration::from_millis(1100)).await;
        a.upsert_by_name("device.probe-a.categories.json", b"[1]")
            .await
            .unwrap();
        a.end_push_round().await.unwrap();

        let cursor = rewind_cursor(&first[0].modified_time, TIME_PRECISION).unwrap();
        let second = list_core(&b, &cursor).await;
        assert_eq!(second.len(), 1);
        assert!(second[0].modified_time > first[0].modified_time);
        assert_eq!(b.download(&second[0].id).await.unwrap(), b"[1]");

        probe_cleanup(&base, &user, &pass).await;
    }
}
