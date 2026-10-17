//! Installs, updates and uninstalls plugins, and cleans up what a crash in the middle left behind.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use zip::ZipArchive;

use crate::error::{Error, Result};
use crate::plugins::manifest::{self, Manifest};
use crate::repo::plugins::{self as repo, Plugin};
use crate::storage::{utc_now_rfc3339, DbPool};

/// Install, uninstall and leftover removal must not run at the same time, to avoid race
/// conditions.
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Returns the checked manifest from the zip without unpacking it. Used to show the plugin's
/// details in the frontend before it is installed.
///
/// # Errors
///
/// - The zip or its manifest.json is not valid: [`Error::PluginPackage`],
///   [`Error::PluginManifest`], [`Error::PluginUnknownPermission`]
pub fn read_manifest(zip_path: &Path) -> Result<Manifest> {
    read_checked_manifest(&mut open(zip_path)?)
}

/// Installs a plugin from a zip. If another version is installed, this is an update, and a
/// downgrade counts too. `approved` is the permissions the user approved; they are stored in the
/// table as they are.
///
/// Deletes the zip after a successful install. If the install fails, the zip is kept.
///
/// # Errors
///
/// - The same version is already installed: [`Error::PluginAlreadyInstalled`]
/// - The zip or its manifest.json is not valid: [`Error::PluginPackage`],
///   [`Error::PluginManifest`], [`Error::PluginUnknownPermission`]
pub async fn install(
    pool: &DbPool,
    plugins_dir: &Path,
    zip_path: &Path,
    approved: Vec<String>,
) -> Result<()> {
    let _lock = LOCK.lock().await;
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
    // cannot be deleted, do not return an error; it is removed the next time the plugin market
    // opens.
    if let Some(old) = old {
        if let Err(e) = fs::remove_dir_all(plugin_dir.join(&old.version)) {
            log::warn!(
                "plugin {}: cannot delete old version {}: {e}",
                old.id,
                old.version
            );
        }
    }
    // If the zip cannot be deleted, only log it.
    if let Err(e) = fs::remove_file(zip_path) {
        log::warn!("cannot delete installed zip {}: {e}", zip_path.display());
    }
    Ok(())
}

/// Uninstalls a plugin.
///
/// `delete_data`: whether to delete the plugin's own data files.
///
/// Files that cannot be deleted are only logged, not reported as errors.
pub async fn uninstall(
    pool: &DbPool,
    plugins_dir: &Path,
    id: &str,
    delete_data: bool,
) -> Result<()> {
    let _lock = LOCK.lock().await;
    // The folder name is the id from the table. Its format was checked at install, so it cannot
    // be a name like `..` that reaches outside the plugin library.
    let Some(plugin) = repo::get(pool, id).await? else {
        return Ok(());
    };
    repo::remove(pool, &plugin.id).await?;
    let plugin_dir = plugins_dir.join(&plugin.id);
    let removed = if delete_data {
        fs::remove_dir_all(&plugin_dir)
    } else {
        remove_versions(&plugin_dir, None)
    };
    // The row is gone, so the uninstall has succeeded. Version folders that cannot be deleted are
    // removed the next time the plugin market opens.
    if let Err(e) = removed {
        log::warn!("plugin {}: cannot delete its files: {e}", plugin.id);
    }
    Ok(())
}

/// Deletes what a crash in the middle of an install, update or uninstall left behind.
///
/// The `plugins` table in the database is the truth, and the plugin library is tidied to match
/// it: temp folders are deleted whole; each plugin keeps only the version in the table, and
/// plugins not in the table are deleted; each plugin's own data is kept.
pub async fn remove_leftovers(pool: &DbPool, plugins_dir: &Path) -> Result<()> {
    let _lock = LOCK.lock().await;
    let installed = repo::list(pool).await?;
    let entries = match fs::read_dir(plugins_dir) {
        Ok(entries) => entries,
        // No plugin has been installed yet
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        // A file at the top of the plugin library is most likely Finder's `.DS_Store`
        if !entry.file_type()?.is_dir() {
            fs::remove_file(entry.path())?;
            continue;
        }
        // A folder starting with `.tmp-` is a temp folder left by a crash during unpacking.
        // Delete all of it.
        if name.to_string_lossy().starts_with(".tmp-") {
            fs::remove_dir_all(entry.path())?;
            continue;
        }
        let keep = installed
            .iter()
            .find(|p| name == p.id.as_str())
            .map(|p| p.version.as_str());
        remove_versions(&entry.path(), keep)?;
    }
    Ok(())
}

/// Deletes the version folders in a plugin folder and keeps the plugin's database file.
/// `keep`: the version to keep; if it is `None`, all versions are deleted.
/// If the plugin folder is empty afterwards, it is deleted too.
fn remove_versions(plugin_dir: &Path, keep: Option<&str>) -> io::Result<()> {
    for version in fs::read_dir(plugin_dir)? {
        let version = version?;
        let kept = keep.is_some_and(|v| version.file_name() == v);
        if version.file_type()?.is_dir() && !kept {
            fs::remove_dir_all(version.path())?;
        }
    }
    if fs::read_dir(plugin_dir)?.next().is_none() {
        fs::remove_dir(plugin_dir)?;
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

    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    use crate::plugins::test_util::{focus_zip, manifest_json, names, write_zip, TempDir};
    use crate::repo::test_util::fresh_test_pool;

    /// 装上以后：文件在 <id>/<version>/ 下，临时文件夹已经改名；表里存的是用户同意的权限，不是清单申请的；
    /// zip 删掉了。
    #[tokio::test]
    async fn install_puts_files_then_row() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        let zip = focus_zip(&dir, "0.1.0");

        install(&pool, &dir.plugins(), &zip, vec!["stats.read".into()])
            .await
            .unwrap();

        assert!(!zip.exists());

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

    /// 同一个版本再装一次：报「已经装了 0.1.0」，zip 不删。
    #[tokio::test]
    async fn same_version_is_rejected() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        install(&pool, &dir.plugins(), &focus_zip(&dir, "0.1.0"), vec![])
            .await
            .unwrap();

        let zip = focus_zip(&dir, "0.1.0");
        let again = install(&pool, &dir.plugins(), &zip, vec![]).await;
        assert!(
            matches!(&again, Err(Error::PluginAlreadyInstalled(v)) if v == "0.1.0"),
            "{again:?}"
        );
        assert!(zip.exists());
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
    /// 都报包错误，且是因为各自的原因；表里没有这一行，插件库里也没有这个插件的文件夹；zip 不删。
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
            assert!(path.exists(), "{reason}");
        }
    }

    /// 卸载并删数据：表里的行和整个插件文件夹（包括插件自己的数据库）都删掉；
    /// 没装的插件再卸载也不报错。
    #[tokio::test]
    async fn uninstall_and_delete_data() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        let focus_dir = dir.plugins().join("hindsight-focus");
        install(&pool, &dir.plugins(), &focus_zip(&dir, "0.1.0"), vec![])
            .await
            .unwrap();
        fs::write(focus_dir.join("data.sqlite"), "").unwrap();

        uninstall(&pool, &dir.plugins(), "hindsight-focus", true)
            .await
            .unwrap();
        assert!(repo::get(&pool, "hindsight-focus").await.unwrap().is_none());
        assert!(!focus_dir.exists());

        uninstall(&pool, &dir.plugins(), "hindsight-focus", true)
            .await
            .unwrap();
    }

    /// 卸载但保留数据：版本文件夹删掉，data.sqlite 留着；重装同一个插件，数据还在。
    #[tokio::test]
    async fn uninstall_and_keep_data() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        let focus_dir = dir.plugins().join("hindsight-focus");
        install(&pool, &dir.plugins(), &focus_zip(&dir, "0.1.0"), vec![])
            .await
            .unwrap();
        fs::write(focus_dir.join("data.sqlite"), "goal: 6h").unwrap();

        uninstall(&pool, &dir.plugins(), "hindsight-focus", false)
            .await
            .unwrap();
        assert!(repo::get(&pool, "hindsight-focus").await.unwrap().is_none());
        assert_eq!(names(&focus_dir), ["data.sqlite"]);

        install(&pool, &dir.plugins(), &focus_zip(&dir, "0.1.0"), vec![])
            .await
            .unwrap();
        assert_eq!(names(&focus_dir), ["0.1.0", "data.sqlite"]);
        assert_eq!(
            fs::read_to_string(focus_dir.join("data.sqlite")).unwrap(),
            "goal: 6h"
        );
    }

    /// 清理残留：临时文件夹、根目录下的文件删掉；每个插件只留表里那个版本；
    /// 表里没有的插件版本全删，留下的数据不动，删空了的插件文件夹也删掉。
    #[tokio::test]
    async fn remove_leftovers_keeps_only_what_the_table_says() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        let plugins = dir.plugins();
        let focus_dir = plugins.join("hindsight-focus");
        let kept_dir = plugins.join("uninstalled-kept-data");
        install(&pool, &plugins, &focus_zip(&dir, "0.2.0"), vec![])
            .await
            .unwrap();
        fs::create_dir_all(focus_dir.join("0.1.0")).unwrap();
        fs::write(focus_dir.join("data.sqlite"), "").unwrap();
        fs::create_dir_all(kept_dir.join("0.1.0")).unwrap();
        fs::write(kept_dir.join("data.sqlite"), "").unwrap();
        fs::create_dir_all(plugins.join("half-installed").join("0.1.0")).unwrap();
        fs::create_dir_all(plugins.join(".tmp-abc")).unwrap();
        fs::write(plugins.join(".tmp-abc").join("manifest.json"), "").unwrap();
        fs::write(plugins.join(".DS_Store"), "").unwrap();

        remove_leftovers(&pool, &plugins).await.unwrap();

        assert_eq!(
            names(&plugins),
            ["hindsight-focus", "uninstalled-kept-data"]
        );
        assert_eq!(names(&focus_dir), ["0.2.0", "data.sqlite"]);
        assert_eq!(names(&kept_dir), ["data.sqlite"]);
    }

    /// 还没装过插件，插件库文件夹都不存在：什么也不做，不报错。
    #[tokio::test]
    async fn remove_leftovers_without_plugins_folder() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        remove_leftovers(&pool, &dir.plugins()).await.unwrap();
    }
}
