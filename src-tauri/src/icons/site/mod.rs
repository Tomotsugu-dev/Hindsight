//! Website icons for the Websites tab (ADR-0014). While the switch is on, they are downloaded
//! from the websites themselves and kept as files in `site-icons/` next to the database. They
//! are never synced.

#[cfg(test)]
mod fake;
mod fetch;
mod http;
mod store;

use std::collections::HashMap;
use std::future::Future;
use std::time::SystemTime;

use futures_util::future::join_all;
use reqwest::Url;
use serde::Serialize;

pub(crate) use http::ReqwestHttp;
pub(crate) use store::{dir, Store};

use fetch::fetch_site_icon;
use http::is_local;

/// Websites downloaded per call; the caller calls again while `remaining` is above 0.
const BATCH: usize = 12;
/// Websites downloaded at the same time; the next group starts when this one is done.
const PARALLEL: usize = 4;

/// One answered GET.
pub(crate) struct Fetched {
    /// The address after redirects; a relative icon address in the page is resolved against it.
    pub url: Url,
    pub status: u16,
    pub body: Vec<u8>,
}

/// The requests this module sends; tests use a fake.
pub(crate) trait Http: Send + Sync {
    /// `None` when there is no answer: a timeout, a refused connection, or a redirect to a
    /// local address.
    fn get(&self, url: Url) -> impl Future<Output = Option<Fetched>> + Send;
}

/// What one call downloaded.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRound {
    /// The new icons by website, as file paths
    pub icons: HashMap<String, String>,
    /// How many of the given websites still need a download; the caller calls again while
    /// this is above 0.
    pub remaining: usize,
}

/// Downloads icons for up to [`BATCH`] of `hosts` that need one, taken in the given order.
pub(crate) async fn download_round(
    http: &impl Http,
    store: &Store,
    hosts: &[String],
) -> DownloadRound {
    let now = SystemTime::now();
    // Filter out local addresses and those that don't need a download.
    let mut due = hosts
        .iter()
        .filter(|h| !is_local(h) && store.needs_download(h, now));
    let batch: Vec<&String> = due.by_ref().take(BATCH).collect();
    let remaining = due.count();
    let mut icons = HashMap::new();
    for group in batch.chunks(PARALLEL) {
        let fetched: Vec<Option<Vec<u8>>> =
            join_all(group.iter().map(|host| fetch_site_icon(http, host))).await;
        for (host, png) in group.iter().zip(fetched) {
            if let Some(path) = store.save(host, png) {
                icons.insert(host.to_string(), path);
            }
        }
    }
    DownloadRound { icons, remaining }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::fake::{ico, FakeHttp};
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hindsight-site-icons-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn a_round_saves_icons_and_failures() {
        let dir = temp_dir();
        let store = Store::at(dir.clone());
        let http = FakeHttp::default()
            .answer("https://github.com/favicon.ico", 200, ico(32))
            .answer("https://openai.com/favicon.ico", 403, "blocked")
            .answer("https://openai.com/", 403, "blocked");
        let hosts = ["github.com", "openai.com", "localhost"].map(String::from);

        let round = download_round(&http, &store, &hosts).await;

        assert_eq!(round.icons.keys().collect::<Vec<_>>(), ["github.com"]);
        assert_eq!(round.remaining, 0);
        assert!(dir.join("openai.com.failed").exists());
        assert_eq!(
            store.icons().keys().collect::<Vec<_>>(),
            ["github.com"],
            "失败的不算图标"
        );
        assert!(
            !http.requested().iter().any(|u| u.contains("localhost")),
            "本地地址不请求"
        );
        assert!(!dir.join("localhost.failed").exists(), "本地地址也不记失败");

        // 再来一轮：github 有图标，openai 刚失败，都不该再请求
        let again = FakeHttp::default();
        let round = download_round(&again, &store, &hosts).await;
        assert!(round.icons.is_empty());
        assert!(again.requested().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_round_takes_a_batch_and_reports_the_rest() {
        let dir = temp_dir();
        let hosts: Vec<String> = (0..BATCH + 1).map(|i| format!("site{i}.com")).collect();
        let http = FakeHttp::default();

        let round = download_round(&http, &Store::at(dir.clone()), &hosts).await;

        assert_eq!(round.remaining, 1, "还剩一个没下载");
        // 每个网站先请求 favicon.ico，再请求首页
        assert_eq!(http.requested().len(), BATCH * 2);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
