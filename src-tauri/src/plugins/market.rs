//! The plugin packages listed on the plugin market page.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};
use crate::plugins::install;
use crate::plugins::manifest::Manifest;
use crate::repo::plugins::{self as repo, Plugin};
use crate::storage::DbPool;

/// Items listed on the plugin market page.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Market {
    pub installed: Vec<Plugin>,
    pub local_zips: Vec<LocalZip>,
}

/// Lists the items displayed on the plugin market page.
pub async fn list(pool: &DbPool, zips_dir: &Path) -> Result<Market> {
    let installed = repo::list(pool).await?;
    let mut local_zips = scan_local_zips(zips_dir)?;
    // If the same plugin version is already installed, include the install error in its listing.
    for zip in &mut local_zips {
        let Some(m) = &zip.manifest else { continue };
        if installed
            .iter()
            .any(|p| p.id == m.id && p.version == m.version)
        {
            zip.error = Some(Error::PluginAlreadyInstalled(m.version.clone()).to_string());
        }
    }
    Ok(Market {
        installed,
        local_zips,
    })
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
            error,
        });
    }
    // Sort so the frontend shows the zips in a stable order.
    zips.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    Ok(zips)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::test_util::{focus_zip, write_zip, TempDir};
    use crate::repo::test_util::fresh_test_pool;

    /// 已安装的全列；本地 zip 全列，同一个版本已经装了的带上安装时会报的错。
    #[tokio::test]
    async fn list_marks_zips_already_installed() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        install::install(&pool, &dir.plugins(), &focus_zip(&dir, "0.1.0"), vec![])
            .await
            .unwrap();
        // 没改版本号，又打了一次包
        focus_zip(&dir, "0.1.0");
        focus_zip(&dir, "0.2.0");

        let market = list(&pool, &dir.0).await.unwrap();

        let installed: Vec<&str> = market
            .installed
            .iter()
            .map(|p| p.version.as_str())
            .collect();
        assert_eq!(installed, ["0.1.0"]);
        let already = Error::PluginAlreadyInstalled("0.1.0".into()).to_string();
        let listed: Vec<(&str, Option<&str>)> = market
            .local_zips
            .iter()
            .map(|z| (z.file_name.as_str(), z.error.as_deref()))
            .collect();
        assert_eq!(
            listed,
            [
                ("focus-0.1.0.zip", Some(already.as_str())),
                ("focus-0.2.0.zip", None)
            ]
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
