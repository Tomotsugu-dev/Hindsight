//! The plugin packages listed on the plugin market page.

use std::cmp::{Ordering, Reverse};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::plugins::install;
use crate::plugins::manifest::{check_version, Manifest};
use crate::repo::plugins::{self as repo, Plugin};
use crate::storage::DbPool;

/// The URL of the online plugin list.
pub const PLUGINS_JSON_URL: &str =
    "https://raw.githubusercontent.com/Tomotsugu-dev/hindsight-plugins/main/plugins.json";

/// The current plugins.json format version.
const PLUGINS_JSON_FORMAT_VERSION: u32 = 1;

/// The timeout for network requests from the market page.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

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
}

/// One version of an online plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnlineVersion {
    pub version: String,
    /// zip download URL
    pub url: String,
    pub sha256: String,
    pub permissions: Vec<String>,
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

/// Parses plugins.json.
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
    Ok(file.plugins)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::test_util::{focus_zip, online_plugin, write_zip, TempDir};
    use crate::repo::test_util::fresh_test_pool;

    /// 已安装的全列；本地 zip 全列，标上装它是什么；同一个版本已经装了的带上安装时会报的错。
    #[tokio::test]
    async fn list_marks_local_zips() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        install::install(&pool, &dir.plugins(), &focus_zip(&dir, "0.1.0"), vec![])
            .await
            .unwrap();
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
                    id: id.into(),
                    name: id.into(),
                    version: version.into(),
                    enabled: true,
                    permissions: vec![],
                    ui_location: "none".into(),
                    runs_in_background: true,
                    installed_at: "2026-10-18T00:00:00+00:00".into(),
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
        install::install(&pool, &dir.plugins(), &focus_zip(&dir, "0.1.0"), vec![])
            .await
            .unwrap();
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

    /// 认识的格式：按顺序读出每个插件，不认识的字段忽略。
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
                    "permissions": ["stats.read"]
                }],
                "homepage": "https://example.com"
            }]
        });

        let plugins = parse_plugins_json(json.to_string().as_bytes()).unwrap();

        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "hindsight-focus");
        assert_eq!(plugins[0].versions[0].version, "0.1.0");
        assert_eq!(plugins[0].versions[0].permissions, ["stats.read"]);
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
