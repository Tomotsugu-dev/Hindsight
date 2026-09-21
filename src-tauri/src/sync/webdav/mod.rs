//! The WebDAV backend (ADR-0007, ADR-0008). [`WebDavClient`] is what the
//! engine sees: it reads the peers' manifests to find changes, downloads and
//! deletes. Its own state lives in `sync_cursor` under `webdav.` keys, which
//! the engine never reads.

// Removed when the client is wired in (ADR-0007 follow-up 3).
#![allow(dead_code)]

mod dav;
#[cfg(test)]
mod fake;
pub mod layout;
mod manifest;

use std::time::Duration;

use rusqlite::OptionalExtension;

use dav::DavOps;
use layout::path_to_flat_name;
use manifest::{manifest_to_device_id, Manifest};

use crate::error::Result;
use crate::storage::{DbPool, SqliteResultExt};
use crate::sync::cloud::FileMeta;
use crate::sync::engine::rewind_cursor;

/// WebDAV Server only provides modification times with second precision (ADR-0007 §4).
pub(crate) const TIME_PRECISION: Duration = Duration::from_secs(1);

/// The WebDAV backend. `pool` serves one purpose: reading and writing this
/// device's progress on each peer's manifest, one `webdav.peer.*` row in
/// `sync_cursor` per peer.
pub(crate) struct WebDavClient<D> {
    dav: D,
    pool: DbPool,
    self_id: String,
}

impl<D: DavOps> WebDavClient<D> {
    pub(crate) fn new(dav: D, pool: DbPool, self_id: String) -> Self {
        Self { dav, pool, self_id }
    }

    /// WebDAV doesn't have a concept of logging in;
    /// if the credentials are set in the configuration,
    /// it is considered logged in.
    pub(crate) async fn ensure_credential(&self) -> Result<bool> {
        Ok(true)
    }

    /// Finds the peers' files that changed and need merging, per ADR-0008.
    /// Each file's `modified_time` is the server time of its manifest.
    pub(crate) async fn list(&self, modified_after: &str) -> Result<Vec<FileMeta>> {
        let entries = self.dav.propfind("").await?;
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

            let local_state = read_peer(&self.pool, device_id).await?;
            if manifest_time == local_state.manifest_time {
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

            // All files of one manifest share the same `manifest_time`; the engine
            // moves its cursor only past files that merged, rewound one second when
            // stored (#56). A file left unmerged keeps the cursor short of
            // `rewind(manifest_time)`; reaching it means this manifest is fully
            // merged: record the progress and report none of its files this round.
            if modified_after >= rewind_cursor(&manifest_time, TIME_PRECISION)?.as_str() {
                let done = PeerState {
                    processed: manifest.latest_push_count(),
                    manifest_time,
                };
                write_peer(&self.pool, device_id, &done).await?;
                continue;
            }

            for (file, push_count) in &manifest.files {
                // Skip files that have already been processed
                // according to the local recorded peer state.
                if *push_count <= local_state.processed {
                    continue;
                }
                let file_path = format!("{device_id}/{file}");
                let Some(name) = path_to_flat_name(&file_path) else {
                    log::debug!("webdav: {file_path} is not a sync file, skipped");
                    continue;
                };
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

    pub(crate) async fn delete(&self, file_id: &str) -> Result<()> {
        self.dav.delete(file_id).await
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

// ─────────────── Peer State ───────────────

/// This device's progress on one peer, kept in `sync_cursor`: `processed` is
/// the push count handled so far, `manifest_time` is the server modification
/// time of that manifest version, used to tell whether the manifest file changed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct PeerState {
    processed: u64,
    manifest_time: String,
}

impl PeerState {
    fn encode(&self) -> String {
        format!("{}|{}", self.processed, self.manifest_time)
    }

    fn decode(raw: &str) -> Self {
        match raw.split_once('|') {
            Some((n, t)) => Self {
                processed: n.parse().unwrap_or(0),
                manifest_time: t.to_string(),
            },
            None => Self::default(),
        }
    }
}

fn peer_key(device_id: &str) -> String {
    format!("webdav.peer.{device_id}")
}

async fn read_peer(pool: &DbPool, device_id: &str) -> Result<PeerState> {
    let key = peer_key(device_id);
    let raw: Option<String> = pool
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
        .await?;
    Ok(raw.map(|s| PeerState::decode(&s)).unwrap_or_default())
}

async fn write_peer(pool: &DbPool, device_id: &str, state: &PeerState) -> Result<()> {
    let key = peer_key(device_id);
    let value = state.encode();
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
    use super::fake::{Call, FakeDav};
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    const EPOCH: &str = "1970-01-01T00:00:00Z";

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

    /// 本机状态一行一台设备，编码解码互逆；没有记录时是零值。
    #[tokio::test]
    async fn peer_state_roundtrips_in_sync_cursor() {
        let pool = fresh_test_pool().await;
        assert_eq!(read_peer(&pool, "abc").await.unwrap(), PeerState::default());
        let s = PeerState {
            processed: 4,
            manifest_time: "2026-05-15T10:00:03Z".into(),
        };
        write_peer(&pool, "abc", &s).await.unwrap();
        assert_eq!(read_peer(&pool, "abc").await.unwrap(), s);
    }

    async fn seed_manifest(dav: &FakeDav, device: &str, files: &[(&str, u64)]) -> String {
        let mut m = Manifest::new();
        for (f, n) in files {
            m.files.insert((*f).to_string(), *n);
        }
        dav.seed_file(&manifest::manifest_path(device), &m.to_bytes().unwrap())
            .await
    }

    /// 云上什么都没有：一轮就一个 PROPFIND，报空。
    #[tokio::test]
    async fn empty_root_costs_one_request() {
        let dav = FakeDav::new();
        let client = WebDavClient::new(dav, fresh_test_pool().await, "me".into());
        assert!(client.list(EPOCH).await.unwrap().is_empty());
        assert_eq!(client.dav.calls().await, vec![Call::Propfind("".into())]);
    }

    /// 第一轮：对端清单没见过 → GET 它，报出全部文件，时间都是清单的服务器时间。
    /// 自己的清单不碰。
    #[tokio::test]
    async fn first_round_reports_every_file_of_a_new_peer() {
        let dav = FakeDav::new();
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
        let client = WebDavClient::new(dav, fresh_test_pool().await, "me".into());

        let files = client.list(EPOCH).await.unwrap();
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
            client.dav.calls().await,
            vec![
                Call::Propfind("".into()),
                Call::Get("manifest.abc.json".into())
            ]
        );
    }

    /// 引擎的游标过了清单时间减一秒 → 推进本机记录、这轮不报；之后空转只剩一个 PROPFIND。
    /// 游标没过（有文件合并失败）→ 下轮重新 GET、重报。
    #[tokio::test]
    async fn advances_only_when_the_engine_got_past_the_manifest() {
        let dav = FakeDav::new();
        let t = seed_manifest(&dav, "abc", &[("categories.json", 1)]).await;
        let client = WebDavClient::new(dav, fresh_test_pool().await, "me".into());
        assert_eq!(client.list(EPOCH).await.unwrap().len(), 1);

        // 合并失败：游标还在 EPOCH → 重报
        assert_eq!(client.list(EPOCH).await.unwrap().len(), 1);

        // 合并成功：游标 = t 退一秒 → 推进，不报
        let cursor = rewind_cursor(&t, TIME_PRECISION).unwrap();
        assert!(client.list(&cursor).await.unwrap().is_empty());
        assert_eq!(
            read_peer(&client.pool, "abc").await.unwrap(),
            PeerState {
                processed: 1,
                manifest_time: t.clone()
            }
        );

        // 之后每轮只有一个 PROPFIND
        client.dav.calls().await; // 读一次，下面只看新增
        let before = client.dav.calls().await.len();
        assert!(client.list(&cursor).await.unwrap().is_empty());
        let calls = client.dav.calls().await;
        assert_eq!(&calls[before..], &[Call::Propfind("".into())]);
    }

    /// 对端又 push 了：只报 push 次数比本机记的大的文件。
    #[tokio::test]
    async fn a_later_push_reports_only_the_changed_files() {
        let dav = FakeDav::new();
        let t1 = seed_manifest(
            &dav,
            "abc",
            &[
                ("categories.json", 1),
                ("activities/2026/2026-09-20.ndjson", 2),
            ],
        )
        .await;
        let client = WebDavClient::new(dav, fresh_test_pool().await, "me".into());
        client.list(EPOCH).await.unwrap();
        let cursor = rewind_cursor(&t1, TIME_PRECISION).unwrap();
        client.list(&cursor).await.unwrap(); // 推进到 2

        let t2 = seed_manifest(
            &client.dav,
            "abc",
            &[
                ("categories.json", 3),
                ("activities/2026/2026-09-20.ndjson", 2),
            ],
        )
        .await;
        let files = client.list(&cursor).await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "device.abc.categories.json");
        assert_eq!(files[0].modified_time, t2);
    }

    /// 清单坏了：跳过这台设备、别的照常，不整轮失败。
    #[tokio::test]
    async fn a_broken_manifest_skips_that_peer_only() {
        let dav = FakeDav::new();
        dav.seed_file("manifest.bad.json", b"<html>").await;
        seed_manifest(&dav, "good", &[("categories.json", 1)]).await;
        let client = WebDavClient::new(dav, fresh_test_pool().await, "me".into());
        let files = client.list(EPOCH).await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "device.good.categories.json");
    }
}
