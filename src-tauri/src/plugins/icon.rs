//! The plugin icon: `icon.png` at the root of the package. A plugin without one gets a default
//! icon.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use zip::result::ZipError;
use zip::ZipArchive;

use crate::error::{Error, Result};
use crate::repo::plugins::Plugin;

/// The file name of the icon in the package.
pub const FILE: &str = "icon.png";

/// The size limit of the icon file.
const MAX_BYTES: u64 = 256 * 1024;

/// Every PNG file starts with these 8 bytes.
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Checks the icon in the package. A package without an icon is not an error.
///
/// # Errors
///
/// `icon.png` is larger than 256 KB or is not a PNG file: [`Error::PluginPackage`]
pub fn check(archive: &mut ZipArchive<File>) -> Result<()> {
    let icon = match archive.by_name(FILE) {
        Ok(icon) => icon,
        Err(ZipError::FileNotFound) => return Ok(()),
        Err(e) => return Err(Error::PluginPackage(format!("{FILE}: {e}"))),
    };
    let mut bytes = Vec::new();
    // Reading 1 byte past the limit shows whether the file is over it
    icon.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(Error::PluginPackage(format!(
            "{FILE} is larger than 256 KB"
        )));
    }
    if !bytes.starts_with(PNG_SIGNATURE) {
        return Err(Error::PluginPackage(format!("{FILE} is not a PNG file")));
    }
    Ok(())
}

/// Returns the icon path of each installed plugin, by plugin id. Plugins without an icon are
/// left out.
pub fn paths(plugins_dir: &Path, installed: &[Plugin]) -> HashMap<String, PathBuf> {
    installed
        .iter()
        .filter_map(|p| {
            let path = plugins_dir.join(&p.id).join(&p.version).join(FILE);
            path.is_file().then(|| (p.id.clone(), path))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use crate::plugins::install;
    use crate::plugins::test_util::{
        install_focus, manifest_json, plugin_row, write_zip_bytes, TempDir,
    };
    use crate::repo::test_util::fresh_test_pool;

    /// 带 PNG 图标的包能装上，路径指向表里那个版本的 icon.png；没带图标的插件不在结果里。
    #[tokio::test]
    async fn paths_point_to_installed_icons() {
        let dir = TempDir::new();
        let pool = fresh_test_pool().await;
        install_focus(&pool, &dir, "0.1.0").await;
        let mut manifest: serde_json::Value =
            serde_json::from_str(&manifest_json("0.2.0")).unwrap();
        manifest["id"] = "pomodoro".into();
        let manifest = manifest.to_string();
        let png = [PNG_SIGNATURE, b"rest of the image"].concat();
        let zip = dir.0.join("pomodoro.zip");
        write_zip_bytes(
            &zip,
            &[
                ("manifest.json", manifest.as_bytes()),
                ("background.js", b"x"),
                (FILE, &png),
            ],
        );
        install::install(&pool, &dir.plugins(), &zip, vec![], None)
            .await
            .unwrap();

        let installed = crate::repo::plugins::list(&pool).await.unwrap();
        let icons = paths(&dir.plugins(), &installed);

        let expected = dir.plugins().join("pomodoro").join("0.2.0").join(FILE);
        assert_eq!(icons, HashMap::from([("pomodoro".to_string(), expected)]));
    }

    /// 不是表里那个版本的文件夹里的图标不算：更新时留下的旧版本不该被显示出来。
    #[test]
    fn paths_skip_icons_of_other_versions() {
        let dir = TempDir::new();
        let old = dir.plugins().join("pomodoro").join("0.0.9");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join(FILE), PNG_SIGNATURE).unwrap();

        let icons = paths(&dir.plugins(), &[plugin_row("pomodoro", true, false)]);

        assert!(icons.is_empty(), "{icons:?}");
    }
}
