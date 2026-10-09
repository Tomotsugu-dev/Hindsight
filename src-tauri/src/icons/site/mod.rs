//! Website icons for the Websites tab (ADR-0014). While the switch is on, they are downloaded
//! from the websites themselves and kept as files in `site-icons/` next to the database. They
//! are never synced.
//!
//! - `<host>.png`: the icon. The website is not requested again.
//! - `<host>.failed`: empty. Its modification time is when the download failed; the website is
//!   tried again 7 days later.

mod fetch;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use futures_util::future::join_all;
use serde::Serialize;

pub(crate) use fetch::ReqwestHttp;
use fetch::{fetch_site_icon, is_local, Http};

use crate::error::Result;

/// How long a website whose download failed waits before the next try.
const RETRY_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// Websites downloaded per call; the caller calls again while `remaining` is above 0.
const BATCH: usize = 12;
/// Websites downloaded at the same time; the next group starts when this one is done.
const PARALLEL: usize = 4;

/// `site-icons/` next to the database, beside the `icons/` folder of app icons.
pub(crate) fn dir() -> Result<PathBuf> {
    Ok(crate::storage::db_path_dir()?.join("site-icons"))
}

/// Scans `dir` (the `site-icons/` folder) for downloaded icons.
///
/// Returns a map from website host to the path of its icon file, for example
/// `"github.com"` to `".../site-icons/github.com.png"`. Only `.png` files count, and the host
/// is the file name without its extension. An unreadable folder gives an empty map.
pub(crate) fn icon_paths_by_host(dir: &Path) -> HashMap<String, String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return HashMap::new();
    };
    entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()? != "png" {
                return None;
            }
            let host = path.file_stem()?.to_str()?.to_string();
            Some((host, path.to_string_lossy().into_owned()))
        })
        .collect()
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
    dir: &Path,
    hosts: &[String],
) -> DownloadRound {
    let now = SystemTime::now();
    let mut due = hosts.iter().filter(|h| needs_download(dir, h, now));
    let batch: Vec<&String> = due.by_ref().take(BATCH).collect();
    let remaining = due.count();
    let mut icons = HashMap::new();
    for group in batch.chunks(PARALLEL) {
        let fetched: Vec<Option<Vec<u8>>> =
            join_all(group.iter().map(|host| fetch_site_icon(http, host))).await;
        for (host, png) in group.iter().zip(fetched) {
            if let Some(path) = save(dir, host, png) {
                icons.insert(host.to_string(), path);
            }
        }
    }
    DownloadRound { icons, remaining }
}

/// A website needs a download when it is not local, has no icon, and has not failed in the
/// last 7 days.
fn needs_download(dir: &Path, host: &str, now: SystemTime) -> bool {
    if !is_file_name(host) || is_local(host) || dir.join(format!("{host}.png")).exists() {
        return false;
    }
    // `<host>.failed` is an empty file whose modification time is the last failure time.
    // `m` is that file's metadata. No file (or unreadable) means no failure: download.
    match std::fs::metadata(dir.join(format!("{host}.failed"))).and_then(|m| m.modified()) {
        // A failure time in the future, after the clock was set back, also waits.
        Ok(failed_at) => now
            .duration_since(failed_at)
            .is_ok_and(|age| age >= RETRY_AFTER),
        Err(_) => true,
    }
}

/// Hosts are stored lowercase, with letters, digits, dots and dashes. Anything else would not
/// make a safe file name and is skipped.
fn is_file_name(host: &str) -> bool {
    !host.is_empty()
        && !host.starts_with('.')
        && !host.contains("..")
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
}

/// Saves the result: the icon as `<host>.png`, or an empty `<host>.failed` whose modification
/// time records the failure. Returns the icon's path.
fn save(dir: &Path, host: &str, png: Option<Vec<u8>>) -> Option<String> {
    let icon = dir.join(format!("{host}.png"));
    let failed = dir.join(format!("{host}.failed"));
    let written = std::fs::create_dir_all(dir).and_then(|()| match &png {
        // Written under another name first, so a crash mid-write leaves no broken icon.
        Some(bytes) => {
            let tmp = dir.join(format!("{host}.png.tmp"));
            std::fs::write(&tmp, bytes)?;
            std::fs::rename(&tmp, &icon)
        }
        // Writing it again moves the modification time to now.
        None => std::fs::write(&failed, []),
    });
    if let Err(e) = written {
        log::warn!("saving a website icon failed: {e}");
        return None;
    }
    png.is_some().then(|| {
        let _ = std::fs::remove_file(&failed);
        icon.to_string_lossy().into_owned()
    })
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::fetch::fake::{ico, FakeHttp};
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hindsight-site-icons-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 留一个 `days` 天前失败的 `.failed` 文件。
    fn failed_days_ago(dir: &Path, host: &str, days: u64) {
        let file = File::create(dir.join(format!("{host}.failed"))).unwrap();
        let at = SystemTime::now() - Duration::from_secs(days * 24 * 60 * 60);
        file.set_modified(at).unwrap();
    }

    #[test]
    fn which_websites_need_a_download() {
        let dir = temp_dir();
        let now = SystemTime::now();
        std::fs::write(dir.join("github.com.png"), ico(32)).unwrap();
        failed_days_ago(&dir, "openai.com", 3);
        failed_days_ago(&dir, "notion.so", 8);

        assert!(
            !needs_download(&dir, "github.com", now),
            "有 .png 的不再请求"
        );
        assert!(!needs_download(&dir, "openai.com", now), "失败不到 7 天");
        assert!(
            needs_download(&dir, "notion.so", now),
            "失败超过 7 天，再试一次"
        );
        assert!(needs_download(&dir, "bilibili.com", now), "两种文件都没有");
        assert!(!needs_download(&dir, "localhost", now), "本地地址");
        assert!(!needs_download(&dir, "../etc", now), "不能当文件名的");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_round_saves_icons_and_failures() {
        let dir = temp_dir();
        let http = FakeHttp::default()
            .answer("https://github.com/favicon.ico", 200, ico(32))
            .answer("https://openai.com/favicon.ico", 403, "blocked")
            .answer("https://openai.com/", 403, "blocked");
        let hosts = ["github.com", "openai.com", "localhost"].map(String::from);

        let round = download_round(&http, &dir, &hosts).await;

        assert_eq!(round.icons.keys().collect::<Vec<_>>(), ["github.com"]);
        assert_eq!(round.remaining, 0);
        assert!(dir.join("openai.com.failed").exists());
        assert_eq!(
            icon_paths_by_host(&dir).keys().collect::<Vec<_>>(),
            ["github.com"],
            "失败的不算图标"
        );

        // 再来一轮：github 有图标，openai 刚失败，都不该再请求
        let again = FakeHttp::default();
        let round = download_round(&again, &dir, &hosts).await;
        assert!(round.icons.is_empty());
        assert!(again.requested().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn a_round_takes_a_batch_and_reports_the_rest() {
        let dir = temp_dir();
        let hosts: Vec<String> = (0..BATCH + 1).map(|i| format!("site{i}.com")).collect();
        let http = FakeHttp::default();

        let round = download_round(&http, &dir, &hosts).await;

        assert_eq!(round.remaining, 1, "还剩一个没下载");
        // 每个网站先请求 favicon.ico，再请求首页
        assert_eq!(http.requested().len(), BATCH * 2);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
