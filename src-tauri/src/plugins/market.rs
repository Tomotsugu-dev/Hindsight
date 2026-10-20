//! The plugin packages listed on the plugin market page.

use std::cmp::{Ordering, Reverse};
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::plugins::install;
use crate::plugins::manifest::{check_id, check_version, Manifest};
use crate::repo::plugins::{self as repo, Plugin};
use crate::storage::DbPool;

/// The URL of the online plugin list.
pub const PLUGINS_JSON_URL: &str =
    "https://raw.githubusercontent.com/Tomotsugu-dev/hindsight-plugins/main/plugins.json";

/// The folder of plugin icons in the official repository. A plugin's icon is `<id>.png` in it.
const ICONS_URL: &str =
    "https://raw.githubusercontent.com/Tomotsugu-dev/hindsight-plugins/main/icons/";

/// The GitHub account that holds the code of official plugins.
const OFFICIAL_ACCOUNT: &str = "Tomotsugu-dev";

/// The current plugins.json format version.
const PLUGINS_JSON_FORMAT_VERSION: u32 = 1;

/// The timeout for network requests from the market page.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// The size limit of a plugin zip.
const MAX_ZIP_BYTES: u64 = 500 * 1024 * 1024;

/// The timeout for connecting to the server when downloading a zip.
const DOWNLOAD_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest allowed gap between two pieces of data when downloading a zip.
const DOWNLOAD_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The plugins being downloaded, each recorded by its `.partial` file. See [`DownloadLock`].
static DOWNLOADING: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// Items listed on the plugin market page.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Market {
    pub installed: Vec<Plugin>,
    /// Online versions of the installed plugins
    pub installed_online: Vec<OnlinePlugin>,
    /// Online plugins that are not installed. An installed plugin is never here, whatever its
    /// version
    pub downloadable: Vec<OnlinePlugin>,
    pub local_zips: Vec<LocalZip>,
    /// Error encountered when fetching the online list, if any.
    pub online_error: Option<String>,
}

/// How the version to install relates to the installed version.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InstallKind {
    /// The plugin is not installed
    #[default]
    New,
    Update,
    Downgrade,
    /// The same version is already installed
    Same,
}

/// Lists the items displayed on the plugin market page.
///
/// `online` is the result of [`fetch_online`].
pub async fn list(
    pool: &DbPool,
    zips_dir: &Path,
    online: Result<Vec<OnlinePlugin>>,
) -> Result<Market> {
    let installed = repo::list(pool).await?;
    let mut local_zips = scan_local_zips(zips_dir)?;
    // If the same plugin version is already installed, include the install error in its listing.
    for zip in &mut local_zips {
        let Some(m) = &zip.manifest else { continue };
        let kind = install_kind(&installed, &m.id, &m.version);
        if kind == Some(InstallKind::Same) {
            zip.error = Some(Error::PluginAlreadyInstalled(m.version.clone()).to_string());
        }
        zip.kind = kind;
    }
    let (online, online_error) = match online {
        Ok(plugins) => (mark_versions(&installed, plugins), None),
        Err(e) => (Vec::new(), Some(e.to_string())),
    };
    let (installed_online, downloadable) = online
        .into_iter()
        .partition(|o| installed.iter().any(|p| p.id == o.id));
    Ok(Market {
        installed,
        installed_online,
        downloadable,
        local_zips,
        online_error,
    })
}

/// Returns how `version` of plugin `id` relates to the installed version.
///
/// Versions are compared as numbers. Returns `None` if either version is not valid.
fn install_kind(installed: &[Plugin], id: &str, version: &str) -> Option<InstallKind> {
    let new = version_numbers(version)?;
    let Some(old) = installed.iter().find(|p| p.id == id) else {
        return Some(InstallKind::New);
    };
    let old = version_numbers(&old.version)?;
    Some(match new.cmp(&old) {
        Ordering::Greater => InstallKind::Update,
        Ordering::Less => InstallKind::Downgrade,
        Ordering::Equal => InstallKind::Same,
    })
}

/// Splits `0.10.0` into `[0, 10, 0]`, so that versions can be compared as numbers.
///
/// Returns `None` if the format is not valid.
fn version_numbers(version: &str) -> Option<Vec<u64>> {
    check_version(version).ok()?;
    version.split('.').map(|p| p.parse().ok()).collect()
}

/// Marks each online version with its [`InstallKind`], then sorts the versions from newest to
/// oldest.
///
/// Keeps only plugins with at least one valid version.
fn mark_versions(installed: &[Plugin], plugins: Vec<OnlinePlugin>) -> Vec<OnlinePlugin> {
    plugins
        .into_iter()
        .filter_map(|mut plugin| {
            // Remove versions whose format is not valid
            plugin.versions.retain_mut(|v| {
                let Some(kind) = install_kind(installed, &plugin.id, &v.version) else {
                    log::warn!(
                        "plugins.json: {} has an invalid version {:?}",
                        plugin.id,
                        v.version
                    );
                    return false;
                };
                v.kind = kind;
                true
            });
            plugin
                .versions
                .sort_by_cached_key(|v| Reverse(version_numbers(&v.version)));
            (!plugin.versions.is_empty()).then_some(plugin)
        })
        .collect()
}

/// Returns the full path to a zip file by joining `file_name` to `dir`.
///
/// # Errors
///
/// Returns [`Error::InvalidInputDyn`] if `file_name` is not a single path component
/// (for example, if it contains `/`, is `..`, is an absolute path, or is empty).
pub fn zip_path(dir: &Path, file_name: &str) -> Result<PathBuf> {
    let mut parts = Path::new(file_name).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(_)), None) => Ok(dir.join(file_name)),
        _ => Err(Error::InvalidInputDyn(format!(
            "plugin zip file name: {file_name}"
        ))),
    }
}

/// One package in the local plugin zip folder.
///
/// If a package cannot be read, `manifest` is `None` and `error` is set.
/// If its version is already installed, both fields are set.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalZip {
    pub file_name: String,
    pub manifest: Option<Manifest>,
    /// `None` if the zip cannot be read
    pub kind: Option<InstallKind>,
    /// The reason why the zip cannot be used, if any.
    pub error: Option<String>,
}

/// Lists the zips in `dir` with their manifests, sorted by file name.
///
/// Zips that cannot be read are listed too, with the reason in `error`. If `dir` does not exist,
/// the list is empty.
fn scan_local_zips(dir: &Path) -> Result<Vec<LocalZip>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        // The user has not opened this folder yet
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut zips = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let is_zip = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"));
        if !is_zip || !path.is_file() {
            continue;
        }
        let file_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let (manifest, error) = match install::read_manifest(&path) {
            Ok(manifest) => (Some(manifest), None),
            Err(e) => (None, Some(e.to_string())),
        };
        zips.push(LocalZip {
            file_name,
            manifest,
            kind: None,
            error,
        });
    }
    // Sort so the frontend shows the zips in a stable order.
    zips.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    Ok(zips)
}

/// One plugin in the online list, plugins.json.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnlinePlugin {
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    /// The author's repository, such as `someone/hindsight-focus`
    pub repo: String,
    pub versions: Vec<OnlineVersion>,
    /// The URL of this plugin's icon in the official repository. It is not in plugins.json; it is
    /// made from the id when the list is parsed. `None` if the id is not valid. If the repository
    /// has no such image, the frontend shows the default icon
    #[serde(skip_deserializing)]
    pub icon_url: Option<String>,
    /// Whether this is an official Hindsight plugin: its `repo` is under the official account. It
    /// is not in plugins.json; it is set when the list is parsed. `repo` is checked in PR review,
    /// so an author cannot set it alone
    #[serde(skip_deserializing)]
    pub official: bool,
}

/// One version of an online plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnlineVersion {
    pub version: String,
    /// zip download URL
    pub url: String,
    pub sha256: String,
    pub permissions: Vec<String>,
    /// The requested disk space for this version (MB).
    /// The marketplace shows this to the user before downloading. If not specified, it is `None`.
    pub storage_mb: Option<u32>,
    /// How this version relates to the installed version. It is not in plugins.json; [`list`]
    /// sets it
    #[serde(skip_deserializing)]
    pub kind: InstallKind,
}

#[derive(Deserialize)]
struct PluginsJson {
    format_version: u32,
    plugins: Vec<OnlinePlugin>,
}

/// Downloads plugins.json from GitHub and parses it.
pub async fn fetch_online(url: &str) -> Result<Vec<OnlinePlugin>> {
    let client = reqwest::Client::builder().timeout(FETCH_TIMEOUT).build()?;
    let body = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    parse_plugins_json(&body)
}

/// Parses plugins.json, then fills in each plugin's icon URL and whether it is official.
///
/// # Errors
///
/// Returns [`Error::PluginsJsonFormatVersion`] if `format_version` is not known.
fn parse_plugins_json(json: &[u8]) -> Result<Vec<OnlinePlugin>> {
    let file: PluginsJson = serde_json::from_slice(json)?;
    // Do not parse an unknown format as if it were the current one
    if file.format_version != PLUGINS_JSON_FORMAT_VERSION {
        return Err(Error::PluginsJsonFormatVersion(file.format_version));
    }
    let mut plugins = file.plugins;
    for plugin in &mut plugins {
        // The id goes into the URL. An id with `/` or `..` would point outside the official
        // repository
        plugin.icon_url = check_id(&plugin.id)
            .is_ok()
            .then(|| format!("{ICONS_URL}{}.png", plugin.id));
        // GitHub account names ignore case
        plugin.official = plugin
            .repo
            .split_once('/')
            .is_some_and(|(owner, _)| owner.eq_ignore_ascii_case(OFFICIAL_ACCOUNT));
    }
    Ok(plugins)
}

/// Finds `version` of plugin `id` in the online list.
///
/// # Errors
///
/// Returns [`Error::PluginDownload`] if the list does not have this plugin or this version.
pub fn find_version<'a>(
    online: &'a [OnlinePlugin],
    id: &str,
    version: &str,
) -> Result<&'a OnlineVersion> {
    online
        .iter()
        .find(|p| p.id == id)
        .and_then(|p| p.versions.iter().find(|v| v.version == version))
        .ok_or_else(|| Error::PluginDownload(format!("{id} {version} is not in plugins.json")))
}

/// Downloads this version of plugin `id` to `zips_dir` and returns the path of the checked zip.
///
/// The zip is first written as `.<id>.partial` and renamed to `<id>-<version>.zip` only after
/// the checks pass, so every `.zip` in the folder is complete and checked. If the checks fail,
/// the `.partial` file is deleted. A plugin downloads only one version at a time, so the
/// `.partial` name has no version.
///
/// # Errors
///
/// Returns [`Error::PluginDownload`] if the plugin is already downloading, the file is larger
/// than 500 MB, the SHA-256 does not match, or the id or version in the zip's manifest is
/// different from the list.
pub async fn download(zips_dir: &Path, id: &str, version: &OnlineVersion) -> Result<PathBuf> {
    // The id and version come from the online list and must not point outside the folder
    let partial = zip_path(zips_dir, &format!(".{id}.partial"))?;
    let zip = zip_path(zips_dir, &format!("{id}-{}.zip", version.version))?;
    let Some(_lock) = DownloadLock::try_lock(&partial) else {
        return Err(Error::PluginDownload(format!(
            "{id} is already downloading"
        )));
    };
    fs::create_dir_all(zips_dir)?;
    let checked = match save(&version.url, &partial, &version.sha256).await {
        Ok(()) => check_manifest(&partial, id, &version.version),
        Err(e) => Err(e),
    };
    if let Err(e) = checked {
        let _ = fs::remove_file(&partial);
        return Err(e);
    }
    fs::rename(&partial, &zip)?;
    Ok(zip)
}

/// The download lock of one plugin. Taking it adds the plugin's `.partial` to [`DOWNLOADING`],
/// and dropping it removes it.
struct DownloadLock(PathBuf);

impl DownloadLock {
    /// Takes the download lock. Returns `None` without waiting if the plugin is already
    /// downloading.
    fn try_lock(partial: &Path) -> Option<Self> {
        if !DOWNLOADING.lock().unwrap().insert(partial.to_path_buf()) {
            return None;
        }
        Some(Self(partial.to_path_buf()))
    }
}

impl Drop for DownloadLock {
    fn drop(&mut self) {
        DOWNLOADING.lock().unwrap().remove(&self.0);
    }
}

/// Downloads `url` into `path` while computing its SHA-256, then compares it with `sha256`.
async fn save(url: &str, path: &Path, sha256: &str) -> Result<()> {
    use futures_util::StreamExt;
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;

    let client = reqwest::Client::builder()
        .connect_timeout(DOWNLOAD_CONNECT_TIMEOUT)
        .read_timeout(DOWNLOAD_READ_TIMEOUT)
        .build()?;
    let response = client.get(url).send().await?.error_for_status()?;
    let too_big = || Error::PluginDownload(format!("larger than {MAX_ZIP_BYTES} bytes"));
    if response.content_length().is_some_and(|n| n > MAX_ZIP_BYTES) {
        return Err(too_big());
    }
    let mut file = tokio::fs::File::create(path).await?;
    let mut hasher = Sha256::new();
    let mut size: u64 = 0;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        size += chunk.len() as u64;
        if size > MAX_ZIP_BYTES {
            return Err(too_big());
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(sha256) {
        return Err(Error::PluginDownload(format!(
            "sha256 does not match: plugins.json says {sha256}, the file is {actual}"
        )));
    }
    Ok(())
}

/// Checks that the downloaded zip is a valid plugin package and that the id and version in its
/// manifest match the online list.
fn check_manifest(path: &Path, id: &str, version: &str) -> Result<()> {
    let manifest = install::read_manifest(path)?;
    if manifest.id != id || manifest.version != version {
        return Err(Error::PluginDownload(format!(
            "the zip is {} {}, plugins.json says {id} {version}",
            manifest.id, manifest.version
        )));
    }
    Ok(())
}

/// Deletes the `.partial` files left by unfinished downloads.
///
/// Skips the plugins being downloaded.
pub fn remove_partial_downloads(zips_dir: &Path) -> Result<()> {
    // Held until done, so no plugin can take its download lock between checking and deleting
    let downloading = DOWNLOADING.lock().unwrap();
    let entries = match fs::read_dir(zips_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "partial") && !downloading.contains(&path) {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::test_util::{
        focus_zip, install_focus, names, online_plugin, plugin_row, serve, write_zip, TempDir,
    };
    use crate::repo::test_util::fresh_test_pool;

    /// 一个能下载的版本：地址指向本机的服务，SHA-256 按 `body` 算。
    async fn served_version(version: &str, body: Vec<u8>) -> OnlineVersion {
        use sha2::{Digest, Sha256};
        OnlineVersion {
            version: version.into(),
            sha256: format!("{:x}", Sha256::digest(&body)),
            url: serve(body).await,
            permissions: vec![],
            storage_mb: None,
            kind: InstallKind::default(),
        }
    }

    /// 下载：核对通过后改名成 <id>-<version>.zip，不留 .partial。
    #[tokio::test]
    async fn download_saves_checked_zip() {
        let dir = TempDir::new();
        let body = fs::read(focus_zip(&dir, "0.1.0")).unwrap();
        let zips = dir.0.join("plugin-zips");
        let version = served_version("0.1.0", body).await;

        let zip = download(&zips, "hindsight-focus", &version).await.unwrap();

        assert_eq!(zip, zips.join("hindsight-focus-0.1.0.zip"));
        assert_eq!(names(&zips), ["hindsight-focus-0.1.0.zip"]);
    }

    /// SHA-256 对不上、清单的版本跟列表不一样：报下载错误，各有各的原因，文件夹里什么都不留。
    #[tokio::test]
    async fn download_rejects_changed_files() {
        let dir = TempDir::new();
        let body = fs::read(focus_zip(&dir, "0.1.0")).unwrap();
        let zips = dir.0.join("plugin-zips");
        let mut changed = served_version("0.1.0", body.clone()).await;
        changed.sha256 = "0".repeat(64);
        let wrong_version = served_version("0.2.0", body).await;

        for (reason, version) in [("sha256", changed), ("the zip is", wrong_version)] {
            let result = download(&zips, "hindsight-focus", &version).await;
            assert!(
                matches!(&result, Err(Error::PluginDownload(m)) if m.contains(reason)),
                "{reason}: {result:?}"
            );
            assert!(names(&zips).is_empty(), "{reason}");
        }
    }

    /// 列表里的 id 带 `/`：拼不出文件名，不下载，文件夹外面什么都不写。
    #[tokio::test]
    async fn download_rejects_id_with_slash() {
        let dir = TempDir::new();
        let zips = dir.0.join("plugin-zips");
        let version = served_version("0.1.0", b"x".to_vec()).await;

        let result = download(&zips, "../evil", &version).await;

        assert!(
            matches!(result, Err(Error::InvalidInputDyn(_))),
            "{result:?}"
        );
        assert!(!dir.0.join("evil-0.1.0.zip").exists());
    }

    /// 找版本：列表里没有这个插件、或没有这个版本，报下载错误。
    #[test]
    fn find_version_in_online_list() {
        let online = vec![online_plugin("focus", &["0.1.0", "0.2.0"])];
        let found = find_version(&online, "focus", "0.2.0").unwrap();
        assert_eq!(found.version, "0.2.0");
        for (id, version) in [("focus", "0.3.0"), ("notes", "0.1.0")] {
            assert!(
                matches!(
                    find_version(&online, id, version),
                    Err(Error::PluginDownload(_))
                ),
                "{id} {version}"
            );
        }
    }

    /// 同一个插件正在下载（哪个版本都算）：再下报下载错误，正在下的 .partial 不动。
    #[tokio::test]
    async fn download_rejects_plugin_already_downloading() {
        let dir = TempDir::new();
        let partial = dir.0.join(".hindsight-focus.partial");
        fs::write(&partial, "half").unwrap();
        let _lock = DownloadLock::try_lock(&partial).unwrap();
        let online = online_plugin("hindsight-focus", &["0.2.0"]);

        let result = download(&dir.0, "hindsight-focus", &online.versions[0]).await;

        assert!(
            matches!(&result, Err(Error::PluginDownload(m)) if m.contains("already downloading")),
            "{result:?}"
        );
        assert_eq!(fs::read_to_string(&partial).unwrap(), "half");
    }

    /// 清理：正在下载的插件的 .partial 跳过，别的 .partial 删掉，.zip 不动；下载结束后再清理就删掉。
    #[test]
    fn partial_downloads_skip_plugins_being_downloaded() {
        let dir = TempDir::new();
        let focus = dir.0.join(".hindsight-focus.partial");
        fs::write(&focus, "").unwrap();
        fs::write(dir.0.join(".notes.partial"), "").unwrap();
        focus_zip(&dir, "0.1.0");

        let lock = DownloadLock::try_lock(&focus).unwrap();
        remove_partial_downloads(&dir.0).unwrap();
        assert_eq!(
            names(&dir.0),
            [".hindsight-focus.partial", "focus-0.1.0.zip"]
        );

        drop(lock);
        remove_partial_downloads(&dir.0).unwrap();
        assert_eq!(names(&dir.0), ["focus-0.1.0.zip"]);
    }

    /// 已安装的全列；本地 zip 全列，标上装它是什么；同一个版本已经装了的带上安装时会报的错。
    #[tokio::test]
    async fn list_marks_local_zips() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        install_focus(&pool, &dir, "0.1.0").await;
        // 没改版本号，又打了一次包
        focus_zip(&dir, "0.1.0");
        focus_zip(&dir, "0.2.0");

        let market = list(&pool, &dir.0, Ok(vec![])).await.unwrap();

        let installed: Vec<&str> = market
            .installed
            .iter()
            .map(|p| p.version.as_str())
            .collect();
        assert_eq!(installed, ["0.1.0"]);
        let already = Error::PluginAlreadyInstalled("0.1.0".into()).to_string();
        let listed: Vec<(&str, Option<InstallKind>, Option<&str>)> = market
            .local_zips
            .iter()
            .map(|z| (z.file_name.as_str(), z.kind, z.error.as_deref()))
            .collect();
        assert_eq!(
            listed,
            [
                (
                    "focus-0.1.0.zip",
                    Some(InstallKind::Same),
                    Some(already.as_str())
                ),
                ("focus-0.2.0.zip", Some(InstallKind::Update), None)
            ]
        );
    }

    /// 线上列表按 id 分两份：装了的进 installed_online，没装的进 downloadable。
    /// 每个版本标上装它是什么；版本从新到旧排好，按数字比（0.10.0 比 0.2.0 新），
    /// 格式不对的版本去掉；没有一个好版本的插件去掉。
    #[tokio::test]
    async fn list_splits_online_by_id() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        // list 只读表，不看插件库里的文件
        for (id, version) in [("focus", "0.2.0"), ("pomodoro", "1.0.0")] {
            repo::save(
                &pool,
                Plugin {
                    version: version.into(),
                    ..plugin_row(id, true, true)
                },
            )
            .await
            .unwrap();
        }
        let online = vec![
            online_plugin("focus", &["0.1.0", "1.0", "0.10.0", "0.2.0"]),
            online_plugin("pomodoro", &["0.9.0"]),
            online_plugin("notes", &["0.2.0", "0.10.0"]),
            online_plugin("broken", &["1.0"]),
        ];

        let market = list(&pool, &dir.0, Ok(online)).await.unwrap();

        use InstallKind::*;
        assert_eq!(
            versions(&market.installed_online),
            [
                (
                    "focus",
                    vec![("0.10.0", Update), ("0.2.0", Same), ("0.1.0", Downgrade)]
                ),
                ("pomodoro", vec![("0.9.0", Downgrade)]),
            ]
        );
        assert_eq!(
            versions(&market.downloadable),
            [("notes", vec![("0.10.0", New), ("0.2.0", New)])]
        );
        assert!(market.online_error.is_none());
    }

    /// 每个插件的 id，和它的各个版本号、标的 kind。
    fn versions(plugins: &[OnlinePlugin]) -> Vec<(&str, Vec<(&str, InstallKind)>)> {
        plugins
            .iter()
            .map(|p| {
                let versions = p.versions.iter().map(|v| (v.version.as_str(), v.kind));
                (p.id.as_str(), versions.collect())
            })
            .collect()
    }

    /// 读不到线上列表：这一栏空着，带上原因；另外两栏照常列出。
    #[tokio::test]
    async fn list_without_online_keeps_the_rest() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        install_focus(&pool, &dir, "0.1.0").await;
        focus_zip(&dir, "0.2.0");

        let market = list(&pool, &dir.0, Err(Error::PluginsJsonFormatVersion(2)))
            .await
            .unwrap();

        assert!(market.installed_online.is_empty());
        assert!(market.downloadable.is_empty());
        assert_eq!(
            market.online_error.as_deref(),
            Some("plugins.json format version 2 is not supported")
        );
        assert_eq!(market.installed.len(), 1);
        assert_eq!(market.local_zips.len(), 1);
    }

    /// 认识的格式：按顺序读出每个插件，不认识的字段忽略；版本没写容量是 None。
    #[test]
    fn parses_plugins_json_format_1() {
        let json = serde_json::json!({
            "format_version": 1,
            "plugins": [{
                "id": "hindsight-focus",
                "name": "专注",
                "author": "someone",
                "description": "离今天的目标还差多少",
                "repo": "someone/hindsight-focus",
                "versions": [{
                    "version": "0.1.0",
                    "url": "https://example.com/hindsight-focus-0.1.0.zip",
                    "sha256": "abc",
                    "permissions": ["stats.read"],
                    "storage_mb": 50
                }, {
                    "version": "0.0.1",
                    "url": "https://example.com/hindsight-focus-0.0.1.zip",
                    "sha256": "def",
                    "permissions": []
                }],
                "homepage": "https://example.com"
            }]
        });

        let plugins = parse_plugins_json(json.to_string().as_bytes()).unwrap();

        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "hindsight-focus");
        assert_eq!(plugins[0].versions[0].version, "0.1.0");
        assert_eq!(plugins[0].versions[0].permissions, ["stats.read"]);
        assert_eq!(plugins[0].versions[0].storage_mb, Some(50));
        assert_eq!(plugins[0].versions[1].storage_mb, None);
    }

    /// 图标地址按 id 拼，指向官方仓库的 icons/；id 格式不对的不给地址，插件照样列出来。
    #[test]
    fn icon_urls_come_from_valid_ids() {
        let json = serde_json::json!({
            "format_version": 1,
            "plugins": [
                { "id": "pomodoro", "name": "", "author": "", "description": "", "repo": "",
                  "versions": [] },
                { "id": "../../evil", "name": "", "author": "", "description": "", "repo": "",
                  "versions": [] }
            ]
        });

        let plugins = parse_plugins_json(json.to_string().as_bytes()).unwrap();

        let urls: Vec<Option<&str>> = plugins.iter().map(|p| p.icon_url.as_deref()).collect();
        assert_eq!(
            urls,
            [
                Some("https://raw.githubusercontent.com/Tomotsugu-dev/hindsight-plugins/main/icons/pomodoro.png"),
                None
            ]
        );
    }

    /// `repo` 在官方账号下才算官方插件，账号名不分大小写；名字只是像官方账号的不算。
    #[test]
    fn official_comes_from_the_repo_account() {
        let plugin = |repo: &str| {
            serde_json::json!({ "id": "p", "name": "", "author": "", "description": "",
                                "repo": repo, "versions": [] })
        };
        let json = serde_json::json!({
            "format_version": 1,
            "plugins": [
                plugin("Tomotsugu-dev/hindsight-focus"),
                plugin("tomotsugu-dev/hindsight-focus"),
                plugin("someone/hindsight-focus"),
                plugin("Tomotsugu-dev-fake/hindsight-focus"),
                plugin("Tomotsugu-dev"),
            ]
        });

        let plugins = parse_plugins_json(json.to_string().as_bytes()).unwrap();

        let official: Vec<bool> = plugins.iter().map(|p| p.official).collect();
        assert_eq!(official, [true, true, false, false, false]);
    }

    /// 不认识的格式：报格式错误，不硬解。
    #[test]
    fn rejects_unknown_plugins_json_format_version() {
        let json = serde_json::json!({ "format_version": 2, "plugins": [] });
        let result = parse_plugins_json(json.to_string().as_bytes());
        assert!(
            matches!(result, Err(Error::PluginsJsonFormatVersion(2))),
            "{result:?}"
        );
    }

    /// 文件名只能是单独一层：带 `/`、`..`、绝对路径、空的都拒绝。
    #[test]
    fn zip_path_takes_only_a_file_name() {
        let dir = Path::new("/data/plugin-zips");
        assert_eq!(
            zip_path(dir, "focus-0.1.0.zip").unwrap(),
            dir.join("focus-0.1.0.zip")
        );
        for bad in [
            "../hindsight.db",
            "a/focus.zip",
            "/etc/passwd",
            "..",
            ".",
            "",
        ] {
            assert!(
                matches!(zip_path(dir, bad), Err(Error::InvalidInputDyn(_))),
                "{bad}"
            );
        }
    }

    /// 好的包带清单，坏的包带原因；不是 zip 的文件、文件夹不列；按文件名排序。
    #[test]
    fn lists_zips_with_manifest_or_error() {
        let dir = TempDir::new();
        focus_zip(&dir, "0.2.0");
        focus_zip(&dir, "0.1.0");
        write_zip(&dir.0.join("broken.ZIP"), &[("background.js", "x")]);
        fs::write(dir.0.join("notes.txt"), "").unwrap();
        fs::create_dir_all(dir.0.join("folder.zip")).unwrap();

        let zips = scan_local_zips(&dir.0).unwrap();

        let listed: Vec<&str> = zips.iter().map(|z| z.file_name.as_str()).collect();
        assert_eq!(listed, ["broken.ZIP", "focus-0.1.0.zip", "focus-0.2.0.zip"]);
        assert!(zips[0].manifest.is_none());
        assert!(
            zips[0].error.as_deref().unwrap().contains("manifest.json"),
            "{:?}",
            zips[0].error
        );
        assert_eq!(zips[1].manifest.as_ref().unwrap().version, "0.1.0");
        assert!(zips[1].error.is_none());
    }

    /// 文件夹不存在：空列表，不报错。
    #[test]
    fn missing_folder_is_empty() {
        let dir = TempDir::new();
        assert!(scan_local_zips(&dir.0.join("plugin-zips"))
            .unwrap()
            .is_empty());
    }
}
