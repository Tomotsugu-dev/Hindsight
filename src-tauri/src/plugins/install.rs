//! Installs and updates plugins.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use zip::ZipArchive;

use crate::error::{Error, Result};
use crate::plugins::manifest::{self, Manifest};
use crate::repo::plugins::{self as repo, Plugin};
use crate::storage::{utc_now_rfc3339, DbPool};

/// Returns the checked manifest from the zip without unpacking it. Used to show the plugin's
/// details in the frontend before it is installed.
pub fn read_manifest(zip_path: &Path) -> Result<Manifest> {
    read_checked_manifest(&mut open(zip_path)?)
}

/// Installs a plugin from a zip. If another version is installed, this is an update, and a
/// downgrade counts too. `approved` is the permissions the user approved; they are stored in the
/// table as they are.
pub async fn install(
    pool: &DbPool,
    plugins_dir: &Path,
    zip_path: &Path,
    approved: Vec<String>,
) -> Result<()> {
    let mut archive = open(zip_path)?;
    let manifest = read_checked_manifest(&mut archive)?;
    let old = repo::get(pool, &manifest.id).await?;
    if old.as_ref().is_some_and(|p| p.version == manifest.version) {
        return Err(Error::PluginAlreadyInstalled(manifest.version));
    }

    // Unpack into a temp folder, then rename it. On the same disk a rename happens in one step,
    // so the plugin library never has a half-unpacked version.
    let tmp = plugins_dir.join(format!(".tmp-{}", uuid::Uuid::new_v4().simple()));
    extract(&mut archive, &tmp)?;
    let plugin_dir = plugins_dir.join(&manifest.id);
    let version_dir = plugin_dir.join(&manifest.version);
    // The table does not have this version, so a folder here was left by an install that did
    // not finish.
    if version_dir.exists() {
        fs::remove_dir_all(&version_dir)?;
    }
    fs::create_dir_all(&plugin_dir)?;
    fs::rename(&tmp, &version_dir)?;

    repo::save(
        pool,
        Plugin {
            id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            enabled: true,
            permissions: approved,
            ui_location: manifest.ui_location,
            runs_in_background: manifest.runs_in_background,
            installed_at: utc_now_rfc3339(),
        },
    )
    .await?;

    // The table now points to the new version, so the install has succeeded. If the old version
    // cannot be deleted, do not return an error; the startup cleanup deletes it.
    if let Some(old) = old {
        if let Err(e) = fs::remove_dir_all(plugin_dir.join(&old.version)) {
            log::warn!(
                "plugin {}: cannot delete old version {}: {e}",
                old.id,
                old.version
            );
        }
    }
    Ok(())
}

fn open(zip_path: &Path) -> Result<ZipArchive<File>> {
    ZipArchive::new(File::open(zip_path)?).map_err(|e| Error::PluginPackage(e.to_string()))
}

/// Reads the manifest and checks it.
///
/// Checks that the fields follow the rules and that the entry files the manifest asks for are in
/// the zip.
fn read_checked_manifest(archive: &mut ZipArchive<File>) -> Result<Manifest> {
    let mut json = Vec::new();
    // Read manifest.json directly from the archive.
    archive
        .by_name("manifest.json")
        .map_err(|e| Error::PluginPackage(format!("manifest.json: {e}")))?
        .read_to_end(&mut json)?;
    let manifest = manifest::parse(&json)?;
    let has = |name: &str| archive.file_names().any(|n| n == name);
    // If the plugin runs in the background or has a page, its entry file must be in the zip:
    // the app starts `background.js` and opens `ui/index.html`. Other pages and scripts are
    // loaded from these two.
    if manifest.runs_in_background && !has("background.js") {
        return Err(Error::PluginPackage("no background.js".into()));
    }
    if manifest.ui_location != "none" && !has("ui/index.html") {
        return Err(Error::PluginPackage("no ui/index.html".into()));
    }
    Ok(manifest)
}

/// Unpacks the zip into `dest`.
fn extract(archive: &mut ZipArchive<File>, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| Error::PluginPackage(e.to_string()))?;
        // Stops a symlink from letting the plugin file reader read files outside the plugin
        // library, such as the main database.
        if entry.is_symlink() {
            return Err(Error::PluginPackage(format!("symlink {}", entry.name())));
        }
        // Stops harmful paths such as `../../hindsight.db` and `/Users/x/.zshrc`.
        // `enclosed_name` returns None for absolute paths and for paths that climb out with `..`.
        let Some(relative) = entry.enclosed_name() else {
            return Err(Error::PluginPackage(format!(
                "unsafe path {}",
                entry.name()
            )));
        };
        let path = dest.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&path)?;
        } else {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            io::copy(&mut entry, &mut File::create(&path)?)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    use crate::repo::test_util::fresh_test_pool;

    /// 一个测试一个空文件夹，放 zip 和插件库，测完删掉。
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "hindsight-plugins-{}",
                uuid::Uuid::new_v4().simple()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn plugins(&self) -> PathBuf {
            self.0.join("plugins")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn manifest_json(version: &str) -> String {
        serde_json::json!({
            "id": "hindsight-focus",
            "name": "Focus",
            "version": version,
            "permissions": ["stats.read", "focus.changed", "focus.category"],
            "ui_location": "none",
            "runs_in_background": true
        })
        .to_string()
    }

    fn write_zip(path: &Path, files: &[(&str, &str)]) {
        let mut zip = ZipWriter::new(File::create(path).unwrap());
        for (name, body) in files {
            zip.start_file(*name, SimpleFileOptions::default()).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    /// 专注插件的 zip，background.js 的内容就是版本号，方便看装的是哪一版。
    fn focus_zip(dir: &TempDir, version: &str) -> PathBuf {
        let path = dir.0.join(format!("focus-{version}.zip"));
        let manifest = manifest_json(version);
        write_zip(
            &path,
            &[("manifest.json", &manifest), ("background.js", version)],
        );
        path
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// 装上以后：文件在 <id>/<version>/ 下，临时文件夹已经改名；表里存的是用户同意的权限，不是清单申请的。
    #[tokio::test]
    async fn install_puts_files_then_row() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        let zip = focus_zip(&dir, "0.1.0");

        install(&pool, &dir.plugins(), &zip, vec!["stats.read".into()])
            .await
            .unwrap();

        let script = dir
            .plugins()
            .join("hindsight-focus")
            .join("0.1.0")
            .join("background.js");
        assert_eq!(fs::read_to_string(script).unwrap(), "0.1.0");
        assert_eq!(names(&dir.plugins()), ["hindsight-focus"]);
        let row = repo::get(&pool, "hindsight-focus").await.unwrap().unwrap();
        assert_eq!(row.version, "0.1.0");
        assert_eq!(row.permissions, ["stats.read"]);
    }

    /// 同一个版本再装一次：报「已经装了 0.1.0」。
    #[tokio::test]
    async fn same_version_is_rejected() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        let zip = focus_zip(&dir, "0.1.0");
        install(&pool, &dir.plugins(), &zip, vec![]).await.unwrap();

        let again = install(&pool, &dir.plugins(), &zip, vec![]).await;
        assert!(
            matches!(&again, Err(Error::PluginAlreadyInstalled(v)) if v == "0.1.0"),
            "{again:?}"
        );
    }

    /// 更新到 0.2.0：表换成新版本，旧版本的文件夹删掉。再装回 0.1.0（降级）也一样。
    #[tokio::test]
    async fn update_and_downgrade_switch_version() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        let focus_dir = dir.plugins().join("hindsight-focus");
        install(&pool, &dir.plugins(), &focus_zip(&dir, "0.1.0"), vec![])
            .await
            .unwrap();

        install(&pool, &dir.plugins(), &focus_zip(&dir, "0.2.0"), vec![])
            .await
            .unwrap();
        assert_eq!(names(&focus_dir), ["0.2.0"]);
        let row = repo::get(&pool, "hindsight-focus").await.unwrap().unwrap();
        assert_eq!(row.version, "0.2.0");

        install(&pool, &dir.plugins(), &focus_zip(&dir, "0.1.0"), vec![])
            .await
            .unwrap();
        assert_eq!(names(&focus_dir), ["0.1.0"]);
        let row = repo::get(&pool, "hindsight-focus").await.unwrap().unwrap();
        assert_eq!(row.version, "0.1.0");
    }

    /// 坏的包：缺清单、缺后台脚本、路径跑出去、符号链接。
    /// 都报包错误，且是因为各自的原因；表里没有这一行，插件库里也没有这个插件的文件夹。
    #[tokio::test]
    async fn bad_packages_install_nothing() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        let manifest = manifest_json("0.1.0");
        let cases: [(&str, &[(&str, &str)]); 3] = [
            ("manifest.json", &[("background.js", "x")]),
            ("background.js", &[("manifest.json", &manifest)]),
            (
                "unsafe path",
                &[
                    ("manifest.json", &manifest),
                    ("background.js", "x"),
                    ("../evil.js", "x"),
                ],
            ),
        ];
        let mut zips = Vec::new();
        for (i, (reason, files)) in cases.into_iter().enumerate() {
            let path = dir.0.join(format!("bad-{i}.zip"));
            write_zip(&path, files);
            zips.push((reason, path));
        }
        let symlink = dir.0.join("symlink.zip");
        let mut zip = ZipWriter::new(File::create(&symlink).unwrap());
        let opts = SimpleFileOptions::default();
        zip.start_file("manifest.json", opts).unwrap();
        zip.write_all(manifest.as_bytes()).unwrap();
        zip.start_file("background.js", opts).unwrap();
        zip.add_symlink("data.js", "../../../hindsight.db", opts)
            .unwrap();
        zip.finish().unwrap();
        zips.push(("symlink", symlink));

        for (reason, path) in zips {
            let result = install(&pool, &dir.plugins(), &path, vec![]).await;
            assert!(
                matches!(&result, Err(Error::PluginPackage(m)) if m.contains(reason)),
                "{reason}: {result:?}"
            );
            assert!(repo::get(&pool, "hindsight-focus").await.unwrap().is_none());
            assert!(!dir.plugins().join("hindsight-focus").exists(), "{reason}");
        }
    }
}
