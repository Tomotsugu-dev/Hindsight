//! Store in the `site-icons/` directory.
//!
//! - `<host>.png`: the icon. The website is not requested again.
//! - `<host>.failed`: empty. Its modification time is when the download failed; the website is
//!   tried again 7 days later.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::error::Result;

/// How long a website whose download failed waits before the next try.
const RETRY_AFTER: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// `site-icons/` next to the database, beside the `icons/` folder of app icons.
pub(crate) fn dir() -> Result<PathBuf> {
    Ok(crate::storage::db_path_dir()?.join("site-icons"))
}

/// The `site-icons/` folder: the downloaded website icons and the marks of failed downloads.
pub(crate) struct Store {
    dir: PathBuf,
}

impl Store {
    /// Returns a Store for the `site-icons/` folder next to the database. The folder is not
    /// checked; it is created when the first icon is saved.
    pub(crate) fn open() -> Result<Self> {
        Ok(Self::at(dir()?))
    }

    /// Returns a Store for the folder `dir`.
    pub(super) fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Scans the folder for downloaded icons.
    ///
    /// Returns a map from website host to the path of its icon file, for example
    /// `"github.com"` to `".../site-icons/github.com.png"`. Only `.png` files count, and the host
    /// is the file name without its extension. An unreadable folder gives an empty map.
    pub(crate) fn icons(&self) -> HashMap<String, String> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
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

    /// Deletes the files whose website is not in `listed`.
    ///
    /// `listed` is the set of websites the page still lists. A website leaves the list when its
    /// history is deleted, and its icon would still show that it was visited. A file's website is
    /// its name without the last extension, so a leftover `.png.tmp` is deleted too.
    pub(crate) fn prune(&self, listed: &HashSet<String>) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for path in entries.filter_map(|entry| Some(entry.ok()?.path())) {
            let host = path.file_stem().and_then(|s| s.to_str());
            if !host.is_some_and(|h| listed.contains(h)) {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    /// A website needs a download when it has no icon and has not failed in the last 7 days.
    pub(super) fn needs_download(&self, host: &str, now: SystemTime) -> bool {
        if !is_file_name(host) || self.dir.join(format!("{host}.png")).exists() {
            return false;
        }
        // `<host>.failed` is an empty file whose modification time is the last failure time.
        // `m` is that file's metadata. No file (or unreadable) means no failure: download.
        match std::fs::metadata(self.dir.join(format!("{host}.failed"))).and_then(|m| m.modified())
        {
            // A failure time in the future, after the clock was set back, also waits.
            Ok(failed_at) => now
                .duration_since(failed_at)
                .is_ok_and(|age| age >= RETRY_AFTER),
            Err(_) => true,
        }
    }

    /// Saves the result: the icon as `<host>.png`, or an empty `<host>.failed` whose modification
    /// time records the failure. Returns the icon's path.
    pub(super) fn save(&self, host: &str, png: Option<Vec<u8>>) -> Option<String> {
        let dir = &self.dir;
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

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::path::Path;

    use super::*;
    use crate::icons::site::fake::ico;

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
        let store = Store::at(dir.clone());
        let now = SystemTime::now();
        std::fs::write(dir.join("github.com.png"), ico(32)).unwrap();
        failed_days_ago(&dir, "openai.com", 3);
        failed_days_ago(&dir, "notion.so", 8);

        assert!(
            !store.needs_download("github.com", now),
            "有 .png 的不再请求"
        );
        assert!(!store.needs_download("openai.com", now), "失败不到 7 天");
        assert!(
            store.needs_download("notion.so", now),
            "失败超过 7 天，再试一次"
        );
        assert!(store.needs_download("bilibili.com", now), "两种文件都没有");
        assert!(!store.needs_download("../etc", now), "不能当文件名的");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// 删了 Chrome 连同它的数据：只在 Chrome 里出现过的 bilibili、openai 不在列表里了，
    /// 它们的图标和失败记号跟着删；github 还在列表里，留着。崩溃留下的临时文件也删掉。
    #[test]
    fn prune_deletes_websites_no_longer_listed() {
        let dir = temp_dir();
        std::fs::write(dir.join("bilibili.com.png"), ico(32)).unwrap();
        std::fs::write(dir.join("github.com.png"), ico(32)).unwrap();
        std::fs::write(dir.join("github.com.png.tmp"), b"half").unwrap();
        failed_days_ago(&dir, "openai.com", 1);
        let listed = HashSet::from(["github.com".to_string()]);

        Store::at(dir.clone()).prune(&listed);

        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["github.com.png"]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
