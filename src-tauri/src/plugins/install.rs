//! Installs, updates and uninstalls plugins, and cleans up what a crash in the middle left behind.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use zip::ZipArchive;

use crate::error::{Error, Result};
use crate::plugins::icon;
use crate::plugins::manifest::{self, Manifest};
use crate::repo::plugins::{self as repo, Plugin, UiSettings};
use crate::storage::{utc_now_rfc3339, DbPool};

/// Install, uninstall and leftover removal must not run at the same time, to avoid race
/// conditions.
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The largest manifest.json allowed, in bytes.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// The largest total size of all files after unpacking, in bytes.
const MAX_UNPACKED_BYTES: u64 = 100 * 1024 * 1024;

/// The most entries a package may have. Folders count too.
const MAX_ENTRIES: usize = 1000;

/// Returns the checked manifest from the zip without unpacking it. Used to show the plugin's
/// details in the frontend before it is installed.
///
/// # Errors
///
/// - The zip or its manifest.json is not valid: [`Error::PluginPackage`],
///   [`Error::PluginManifest`], [`Error::PluginUnknownPermission`]
/// - manifest.json is larger than 64 KB: [`Error::PluginPackage`]
/// - icon.png is larger than 256 KB or is not a PNG file: [`Error::PluginPackage`]
pub fn read_manifest(zip_path: &Path) -> Result<Manifest> {
    read_checked_manifest(&mut open(zip_path)?)
}

/// Installs a plugin from a zip. If another version is installed, this is an update, and a
/// downgrade counts too.
///
/// - `approved`: the permissions the user approved
/// - `storage_mb`: the disk space the user approved, in MB. `None` if the user approved none
///
/// Both are stored in the table as they are. They are not compared with what the manifest asks
/// for.
///
/// Deletes the zip after a successful install. If the install fails, the zip is kept.
///
/// # Errors
///
/// - The same version is already installed: [`Error::PluginAlreadyInstalled`]
/// - The zip or its manifest.json is not valid: [`Error::PluginPackage`],
///   [`Error::PluginManifest`], [`Error::PluginUnknownPermission`]
/// - manifest.json is larger than 64 KB, the zip has more than 1000 entries, or the files are
///   larger than 100 MB unpacked: [`Error::PluginPackage`]
/// - icon.png is larger than 256 KB or is not a PNG file: [`Error::PluginPackage`]
/// - `storage_mb` is not between 1 MB and 10 GB: [`Error::PluginManifest`]
pub async fn install(
    pool: &DbPool,
    plugins_dir: &Path,
    zip_path: &Path,
    approved: Vec<String>,
    storage_mb: Option<u32>,
) -> Result<()> {
    // The approved disk space comes from the frontend.
    if let Some(mb) = storage_mb {
        manifest::check_storage_mb(mb)?;
    }
    let _lock = LOCK.lock().await;
    let mut archive = open(zip_path)?;
    let manifest = read_checked_manifest(&mut archive)?;
    let has_page = archive.file_names().any(|n| n == "ui/index.html");
    let old = repo::get(pool, &manifest.id).await?;
    if old.as_ref().is_some_and(|p| p.version == manifest.version) {
        return Err(Error::PluginAlreadyInstalled(manifest.version));
    }

    // Unpack into a temp folder, then rename it. On the same disk a rename happens in one step,
    // so the plugin library never has a half-unpacked version.
    let tmp = plugins_dir.join(format!(".tmp-{}", uuid::Uuid::new_v4().simple()));
    if let Err(e) = extract(&mut archive, &tmp) {
        // The half-unpacked files can be as large as 100 MB, so delete them now instead of
        // waiting until the plugin market is opened next time
        if let Err(e) = fs::remove_dir_all(&tmp) {
            log::warn!("cannot delete temp folder {}: {e}", tmp.display());
        }
        return Err(e);
    }
    let plugin_dir = plugins_dir.join(&manifest.id);
    let version_dir = plugin_dir.join(&manifest.version);
    // The table does not have this version, so a folder here was left by an install that did
    // not finish.
    if version_dir.exists() {
        fs::remove_dir_all(&version_dir)?;
    }
    fs::create_dir_all(&plugin_dir)?;
    fs::rename(&tmp, &version_dir)?;

    let window = manifest.window.unwrap_or(manifest::DEFAULT_WINDOW);
    repo::save(
        pool,
        Plugin {
            id: manifest.id,
            name: manifest.name,
            version: manifest.version,
            enabled: true,
            permissions: approved,
            storage_mb,
            ui: UiSettings {
                has_page,
                opens_in: manifest.opens_in,
                window_x: None,
                window_y: None,
                window_width: window.width,
                window_height: window.height,
            },
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
/// the zip. Also checks the icon if the package has one, see [`icon::check`].
fn read_checked_manifest(archive: &mut ZipArchive<File>) -> Result<Manifest> {
    let mut json = Vec::new();
    // Read manifest.json directly from the archive. Reading 1 byte past the limit shows whether
    // the file is over it
    archive
        .by_name("manifest.json")
        .map_err(|e| Error::PluginPackage(format!("manifest.json: {e}")))?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut json)?;
    if json.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(Error::PluginPackage(
            "manifest.json is larger than 64 KB".into(),
        ));
    }
    let manifest = manifest::parse(&json)?;
    // A plugin that runs in the background must have `background.js` in the package.
    if manifest.runs_in_background && !archive.file_names().any(|n| n == "background.js") {
        return Err(Error::PluginPackage("no background.js".into()));
    }
    icon::check(archive)?;
    Ok(manifest)
}

/// Unpacks the zip into `dest`.
fn extract(archive: &mut ZipArchive<File>, dest: &Path) -> Result<()> {
    if archive.len() > MAX_ENTRIES {
        return Err(Error::PluginPackage(format!(
            "more than {MAX_ENTRIES} entries"
        )));
    }
    fs::create_dir_all(dest)?;
    let mut left = MAX_UNPACKED_BYTES;
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
            // Count the bytes actually written. Reading 1 byte past what is left shows whether
            // the files are over the limit
            let copied = io::copy(
                &mut entry.by_ref().take(left + 1),
                &mut File::create(&path)?,
            )?;
            if copied > left {
                return Err(Error::PluginPackage("larger than 100 MB unpacked".into()));
            }
            left -= copied;
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

    use crate::plugins::test_util::{
        focus_zip, install_focus, manifest_json, names, write_zip, write_zip_bytes, TempDir,
    };
    use crate::repo::test_util::fresh_test_pool;

    /// 装上以后：文件在 <id>/<version>/ 下，临时文件夹已经改名；表里存的是用户同意的权限和容量，
    /// 不是清单申请的；zip 删掉了。
    #[tokio::test]
    async fn install_puts_files_then_row() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        let zip = focus_zip(&dir, "0.1.0");

        install(
            &pool,
            &dir.plugins(),
            &zip,
            vec!["stats.read".into()],
            Some(200),
        )
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
        assert_eq!(row.storage_mb, Some(200));
    }

    /// 有没有页面看包里有没有 ui/index.html，更新时按新包重写。打开方式和窗口大小用清单里的；
    /// 清单没写，打开方式是 None（侧边栏里不显示），窗口是 900×640。
    #[tokio::test]
    async fn ui_settings_come_from_the_package() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        install_focus(&pool, &dir, "0.1.0").await;
        let row = repo::get(&pool, "hindsight-focus").await.unwrap().unwrap();
        assert!(!row.ui.has_page);
        assert_eq!(row.ui.opens_in, None);
        assert_eq!((row.ui.window_width, row.ui.window_height), (900, 640));

        // 0.2.0 加了页面
        let zip = dir.0.join("focus-0.2.0.zip");
        write_zip(
            &zip,
            &[
                ("manifest.json", &manifest_json("0.2.0")),
                ("background.js", "x"),
                ("ui/index.html", "x"),
            ],
        );
        install(&pool, &dir.plugins(), &zip, vec![], None)
            .await
            .unwrap();
        let row = repo::get(&pool, "hindsight-focus").await.unwrap().unwrap();
        assert!(row.ui.has_page);

        let mut manifest: serde_json::Value =
            serde_json::from_str(&manifest_json("0.1.0")).unwrap();
        manifest["id"] = "pomodoro".into();
        manifest["opens_in"] = "window".into();
        manifest["window"] = serde_json::json!({ "width": 360, "height": 480 });
        let zip = dir.0.join("pomodoro.zip");
        write_zip(
            &zip,
            &[
                ("manifest.json", &manifest.to_string()),
                ("background.js", "x"),
            ],
        );
        install(&pool, &dir.plugins(), &zip, vec![], None)
            .await
            .unwrap();
        let row = repo::get(&pool, "pomodoro").await.unwrap().unwrap();
        assert_eq!(row.ui.opens_in.as_deref(), Some("window"));
        assert_eq!((row.ui.window_width, row.ui.window_height), (360, 480));
    }

    /// 容量是 0 或超过 10 GB：报清单错误，什么都不装，zip 不删。
    #[tokio::test]
    async fn bad_storage_installs_nothing() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        let zip = focus_zip(&dir, "0.1.0");

        for mb in [0, manifest::MAX_STORAGE_MB + 1] {
            let result = install(&pool, &dir.plugins(), &zip, vec![], Some(mb)).await;
            assert!(
                matches!(&result, Err(Error::PluginManifest(_))),
                "{mb}: {result:?}"
            );
            assert!(repo::get(&pool, "hindsight-focus").await.unwrap().is_none());
            assert!(!dir.plugins().exists());
            assert!(zip.exists());
        }
    }

    /// 同一个版本再装一次：报「已经装了 0.1.0」，zip 不删。
    #[tokio::test]
    async fn same_version_is_rejected() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        install_focus(&pool, &dir, "0.1.0").await;

        let zip = focus_zip(&dir, "0.1.0");
        let again = install(&pool, &dir.plugins(), &zip, vec![], None).await;
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
        install_focus(&pool, &dir, "0.1.0").await;

        install_focus(&pool, &dir, "0.2.0").await;
        assert_eq!(names(&focus_dir), ["0.2.0"]);
        let row = repo::get(&pool, "hindsight-focus").await.unwrap().unwrap();
        assert_eq!(row.version, "0.2.0");

        install_focus(&pool, &dir, "0.1.0").await;
        assert_eq!(names(&focus_dir), ["0.1.0"]);
        let row = repo::get(&pool, "hindsight-focus").await.unwrap().unwrap();
        assert_eq!(row.version, "0.1.0");
    }

    /// 坏的包：缺清单、缺后台脚本、路径跑出去、符号链接、清单太大、条目太多、解压后太大、
    /// 图标不是 PNG、图标太大。
    /// 都报包错误，且是因为各自的原因；表里没有这一行，插件库里也没有这个插件的文件夹，
    /// 解了一半的临时文件夹也删掉了；zip 不删。
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

        // 清单超过 64 KB。不认识的字段本来会被忽略，但先被大小挡住
        let mut big = serde_json::from_str::<serde_json::Value>(&manifest).unwrap();
        big["padding"] = "x".repeat(64 * 1024).into();
        let path = dir.0.join("big-manifest.zip");
        write_zip(
            &path,
            &[("manifest.json", &big.to_string()), ("background.js", "x")],
        );
        zips.push(("64 KB", path));

        // 1002 个条目
        let extra: Vec<String> = (0..1000).map(|i| format!("f{i}.js")).collect();
        let mut files = vec![("manifest.json", manifest.as_str()), ("background.js", "x")];
        files.extend(extra.iter().map(|name| (name.as_str(), "x")));
        let path = dir.0.join("many-entries.zip");
        write_zip(&path, &files);
        zips.push(("entries", path));

        // 每个文件都不到 100 MB，加起来超过
        let half = "0".repeat((MAX_UNPACKED_BYTES / 2 + 1) as usize);
        let path = dir.0.join("too-large.zip");
        write_zip(
            &path,
            &[
                ("manifest.json", &manifest),
                ("background.js", "x"),
                ("a.js", &half),
                ("b.js", &half),
            ],
        );
        zips.push(("100 MB", path));

        // 图标是 SVG 改了个名字；图标开头是 PNG，但超过 256 KB
        let too_big_png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[0; 256 * 1024]].concat();
        for (reason, icon) in [
            ("not a PNG", b"<svg></svg>".as_slice()),
            ("256 KB", &too_big_png),
        ] {
            let path = dir.0.join(format!("bad-icon-{}.zip", icon.len()));
            write_zip_bytes(
                &path,
                &[
                    ("manifest.json", manifest.as_bytes()),
                    ("background.js", b"x"),
                    ("icon.png", icon),
                ],
            );
            zips.push((reason, path));
        }

        for (reason, path) in zips {
            let result = install(&pool, &dir.plugins(), &path, vec![], None).await;
            assert!(
                matches!(&result, Err(Error::PluginPackage(m)) if m.contains(reason)),
                "{reason}: {result:?}"
            );
            assert!(repo::get(&pool, "hindsight-focus").await.unwrap().is_none());
            assert!(!dir.plugins().join("hindsight-focus").exists(), "{reason}");
            assert!(
                !dir.plugins().exists() || names(&dir.plugins()).is_empty(),
                "{reason}"
            );
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
        install_focus(&pool, &dir, "0.1.0").await;
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
        install_focus(&pool, &dir, "0.1.0").await;
        fs::write(focus_dir.join("data.sqlite"), "goal: 6h").unwrap();

        uninstall(&pool, &dir.plugins(), "hindsight-focus", false)
            .await
            .unwrap();
        assert!(repo::get(&pool, "hindsight-focus").await.unwrap().is_none());
        assert_eq!(names(&focus_dir), ["data.sqlite"]);

        install_focus(&pool, &dir, "0.1.0").await;
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
        install_focus(&pool, &dir, "0.2.0").await;
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
