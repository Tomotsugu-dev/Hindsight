//! The theme library: `themes/` in the data folder. Each theme has its own folder, named by the
//! theme's id.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use super::manifest::{self, Theme};
use crate::error::{Error, Result};

/// The theme.json in a theme's folder.
const THEME_JSON: &str = "theme.json";

/// The theme.css in a theme's folder. A theme may have none.
const THEME_CSS: &str = "theme.css";

/// Returns the absolute path of the theme library: `themes/` in the data folder.
pub fn themes_dir() -> PathBuf {
    crate::bootstrap::data_root().join("themes")
}

/// A folder in the theme library. If the theme can be read, `theme` is `Some`. If not, `error` is
/// `Some` and holds the reason.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledTheme {
    /// The folder name, not a path.
    pub folder_name: String,
    pub theme: Option<Theme>,
    pub error: Option<String>,
}

/// Lists the themes in the theme library, sorted by folder name.
///
/// Broken themes are listed too, with the reason in `error`.
pub fn list(dir: &Path) -> Result<Vec<InstalledTheme>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut themes = Vec::new();
    for entry in entries {
        let entry = entry?;
        // A folder whose name is not UTF-8 cannot be a theme: an id has only lowercase letters,
        // digits and `-`
        let Ok(folder_name) = entry.file_name().into_string() else {
            continue;
        };
        if folder_name.starts_with('.') || !entry.file_type()?.is_dir() {
            continue;
        }
        let (theme, error) = match read(&entry.path(), &folder_name) {
            Ok(theme) => (Some(theme), None),
            Err(e) => (None, Some(e.to_string())),
        };
        themes.push(InstalledTheme {
            folder_name,
            theme,
            error,
        });
    }
    themes.sort_by(|a, b| a.folder_name.cmp(&b.folder_name));
    Ok(themes)
}

/// Reads the theme in the folder `path` and checks that its id is the same as `folder_name`.
fn read(path: &Path, folder_name: &str) -> Result<Theme> {
    let Some(json) = read_if_exists(&path.join(THEME_JSON))? else {
        return Err(Error::ThemeInvalid(format!("no {THEME_JSON}")));
    };
    let css = read_if_exists(&path.join(THEME_CSS))?;
    let theme = manifest::parse(&json, css.as_deref())?;
    // The id must be the folder name. If not, the frontend cannot find the theme's CSS by its id
    if theme.id != folder_name {
        return Err(Error::ThemeInvalid(format!(
            "id {:?} is not the folder name {folder_name:?}",
            theme.id
        )));
    }
    Ok(theme)
}

/// Checks a theme, writes it into `<id>/` in the theme library, and returns it.
///
/// A theme with the same id is replaced. If the new theme has no theme.css, the old theme.css is
/// deleted too. The theme library is created if it does not exist.
///
/// # Errors
///
/// Returns [`Error::ThemeInvalid`] and writes nothing if the theme breaks a rule.
pub fn install(dir: &Path, json: &[u8], css: Option<&[u8]>) -> Result<Theme> {
    let theme = manifest::parse(json, css)?;
    let folder = dir.join(&theme.id);
    fs::create_dir_all(&folder)?;
    fs::write(folder.join(THEME_JSON), json)?;
    match css {
        Some(css) => fs::write(folder.join(THEME_CSS), css)?,
        None => remove_if_exists(&folder.join(THEME_CSS))?,
    }
    Ok(theme)
}

/// Imports `theme_json`, the theme.json that the user picked. If the same folder has a theme.css,
/// it is imported too. See [`install`].
pub fn import(dir: &Path, theme_json: &Path) -> Result<Theme> {
    let json = fs::read(theme_json)?;
    let css = match theme_json.parent() {
        Some(parent) => read_if_exists(&parent.join(THEME_CSS))?,
        None => None,
    };
    // Both files are already in memory. If the user picked the files in the theme library, the
    // same contents are written back
    install(dir, &json, css.as_deref())
}

/// Returns the theme.css in the folder `folder_name` of the theme library, or `None` if there is
/// none.
///
/// # Errors
///
/// Returns [`Error::ThemeInvalid`] if `folder_name` breaks the rules (see [`folder_path`]) or
/// theme.css is not UTF-8.
pub fn css(dir: &Path, folder_name: &str) -> Result<Option<String>> {
    let Some(bytes) = read_if_exists(&folder_path(dir, folder_name)?.join(THEME_CSS))? else {
        return Ok(None);
    };
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| Error::ThemeInvalid(format!("{THEME_CSS} is not UTF-8")))
}

/// Deletes the folder `folder_name` in the theme library. Does nothing if the folder does not
/// exist.
///
/// Deletes by folder name, not by id: the id of a broken theme cannot be read, but the theme must
/// still be deletable.
///
/// # Errors
///
/// Returns [`Error::ThemeInvalid`] if `folder_name` breaks the rules (see [`folder_path`]).
pub fn delete(dir: &Path, folder_name: &str) -> Result<()> {
    match fs::remove_dir_all(folder_path(dir, folder_name)?) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// Returns the path of the folder `folder_name` in the theme library.
///
/// `folder_name` comes from the frontend and must be exactly one folder name: no path, not `..`,
/// and not starting with `.`. Otherwise it could point outside the theme library, or to a folder
/// that [`list`] skips.
fn folder_path(dir: &Path, folder_name: &str) -> Result<PathBuf> {
    let mut parts = Path::new(folder_name).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(_)), None) if !folder_name.starts_with('.') => {
            Ok(dir.join(folder_name))
        }
        _ => Err(Error::ThemeInvalid(format!(
            "invalid folder name {folder_name:?}"
        ))),
    }
}

/// Reads a file that may not exist. Returns `None` if it does not exist.
fn read_if_exists(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Deletes a file that may not exist.
fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用的主题库，用完删掉。
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "hindsight-themes-{}",
                uuid::Uuid::new_v4().simple()
            ));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// 一个合规的 theme.json。
    fn theme_json(id: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "format": 1, "id": id, "name": id, "version": "1.0.0"
        }))
        .unwrap()
    }

    /// 在主题库里放一个文件夹，里面放 `files`（文件名, 内容）。
    fn put(dir: &Path, folder_name: &str, files: &[(&str, &[u8])]) {
        let folder = dir.join(folder_name);
        fs::create_dir_all(&folder).unwrap();
        for (name, body) in files {
            fs::write(folder.join(name), body).unwrap();
        }
    }

    /// 文件夹里的文件名，排好序。
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// 主题库还不存在：空列表，不报错，也不建文件夹。
    #[test]
    fn list_is_empty_without_the_library() {
        let dir = TempDir::new();
        let library = dir.0.join("themes");

        assert!(list(&library).unwrap().is_empty());
        assert!(!library.exists());
    }

    /// 按文件夹名排；带 theme.css 的 has_css 为真。
    #[test]
    fn list_reads_themes_in_folder_order() {
        let dir = TempDir::new();
        put(&dir.0, "nord", &[("theme.json", &theme_json("nord"))]);
        put(
            &dir.0,
            "dracula",
            &[
                ("theme.json", &theme_json("dracula")),
                ("theme.css", b"body {}"),
            ],
        );

        let themes = list(&dir.0).unwrap();

        let folder_names: Vec<&str> = themes.iter().map(|t| t.folder_name.as_str()).collect();
        assert_eq!(folder_names, ["dracula", "nord"]);
        assert!(themes[0].theme.as_ref().unwrap().has_css);
        assert!(!themes[1].theme.as_ref().unwrap().has_css);
        assert!(themes.iter().all(|t| t.error.is_none()));
    }

    /// 坏掉的主题也列出来，带上原因：没有 theme.json、读不出来、id 和文件夹名不同。
    #[test]
    fn list_keeps_broken_themes_with_the_reason() {
        let dir = TempDir::new();
        put(&dir.0, "empty", &[]);
        put(&dir.0, "garbled", &[("theme.json", b"{ not json")]);
        put(&dir.0, "renamed", &[("theme.json", &theme_json("nord"))]);

        let themes = list(&dir.0).unwrap();

        assert_eq!(themes.len(), 3);
        assert!(themes.iter().all(|t| t.theme.is_none()));
        let errors: Vec<&str> = themes.iter().map(|t| t.error.as_deref().unwrap()).collect();
        assert!(errors[0].contains("theme.json"), "{}", errors[0]);
        assert!(errors[1].contains("theme"), "{}", errors[1]);
        assert!(errors[2].contains("renamed"), "{}", errors[2]);
    }

    /// `.` 开头的文件夹和单独的文件跳过。
    #[test]
    fn list_skips_dot_folders_and_files() {
        let dir = TempDir::new();
        put(&dir.0, ".cache", &[("theme.json", &theme_json("cache"))]);
        fs::write(dir.0.join("readme.txt"), b"hi").unwrap();

        assert!(list(&dir.0).unwrap().is_empty());
    }

    /// 安装：主题库不存在就建；两个文件原样写进 <id>/。
    #[test]
    fn install_writes_the_files_into_the_id_folder() {
        let dir = TempDir::new();
        let library = dir.0.join("themes");
        let json = theme_json("nord");

        let theme = install(&library, &json, Some(b"body {}")).unwrap();

        assert_eq!(theme.id, "nord");
        assert_eq!(fs::read(library.join("nord/theme.json")).unwrap(), json);
        assert_eq!(
            fs::read(library.join("nord/theme.css")).unwrap(),
            b"body {}"
        );
    }

    /// 同 id 再装一次：替换旧主题；新主题没有 theme.css，旧的也删掉。
    #[test]
    fn install_replaces_the_theme_with_the_same_id() {
        let dir = TempDir::new();
        install(&dir.0, &theme_json("nord"), Some(b"body {}")).unwrap();
        let newer = serde_json::to_vec(&serde_json::json!({
            "format": 1, "id": "nord", "name": "Nord", "version": "2.0.0"
        }))
        .unwrap();

        install(&dir.0, &newer, None).unwrap();

        assert_eq!(names(&dir.0.join("nord")), ["theme.json"]);
        let themes = list(&dir.0).unwrap();
        assert_eq!(themes[0].theme.as_ref().unwrap().version, "2.0.0");
    }

    /// 校验不过：报错，什么都不写。
    #[test]
    fn install_rejects_an_invalid_theme_and_writes_nothing() {
        let dir = TempDir::new();
        let library = dir.0.join("themes");

        let result = install(&library, b"{ not json", None);

        assert!(matches!(result, Err(Error::ThemeInvalid(_))));
        assert!(!library.exists());
    }

    /// 导入：读选中的 theme.json，同一文件夹里有 theme.css 就一起导入。
    #[test]
    fn import_brings_the_theme_css_next_to_it() {
        let dir = TempDir::new();
        let library = dir.0.join("themes");
        put(
            &dir.0,
            "downloads",
            &[
                ("theme.json", &theme_json("nord")),
                ("theme.css", b"body {}"),
            ],
        );

        import(&library, &dir.0.join("downloads/theme.json")).unwrap();

        assert_eq!(names(&library.join("nord")), ["theme.css", "theme.json"]);
    }

    /// 选中的就是主题库里这个主题自己的 theme.json：文件原样留着。
    #[test]
    fn import_from_the_library_itself_keeps_the_files() {
        let dir = TempDir::new();
        let json = theme_json("nord");
        put(
            &dir.0,
            "nord",
            &[("theme.json", &json), ("theme.css", b"body {}")],
        );

        import(&dir.0, &dir.0.join("nord/theme.json")).unwrap();

        assert_eq!(fs::read(dir.0.join("nord/theme.json")).unwrap(), json);
        assert_eq!(fs::read(dir.0.join("nord/theme.css")).unwrap(), b"body {}");
    }

    /// 读 CSS：有就返回内容，没有返回 None。
    #[test]
    fn css_returns_the_contents_or_none() {
        let dir = TempDir::new();
        put(
            &dir.0,
            "dracula",
            &[
                ("theme.json", &theme_json("dracula")),
                ("theme.css", b"body {}"),
            ],
        );
        put(&dir.0, "nord", &[("theme.json", &theme_json("nord"))]);

        assert_eq!(css(&dir.0, "dracula").unwrap().as_deref(), Some("body {}"));
        assert_eq!(css(&dir.0, "nord").unwrap(), None);
    }

    /// 删除：整个文件夹删掉；本来就没有的不报错。
    #[test]
    fn delete_removes_the_folder_and_ignores_a_missing_one() {
        let dir = TempDir::new();
        put(&dir.0, "nord", &[("theme.json", &theme_json("nord"))]);

        delete(&dir.0, "nord").unwrap();
        delete(&dir.0, "nord").unwrap();

        assert!(names(&dir.0).is_empty());
    }

    /// 坏掉的主题按文件夹名删，文件夹名不是合规的 id 也能删。
    #[test]
    fn delete_removes_a_broken_theme_by_its_folder_name() {
        let dir = TempDir::new();
        put(&dir.0, "My Theme", &[("theme.json", b"{ not json")]);

        delete(&dir.0, "My Theme").unwrap();

        assert!(names(&dir.0).is_empty());
    }

    /// 文件夹名带路径、是 `..`、以 `.` 开头或是空的：报错，主题库外面的东西不动。
    #[test]
    fn delete_and_css_reject_names_that_are_not_one_folder() {
        let dir = TempDir::new();
        let library = dir.0.join("themes");
        put(&library, "nord", &[("theme.json", &theme_json("nord"))]);
        put(&dir.0, "outside", &[("theme.css", b"body {}")]);

        for folder_name in ["../outside", "nord/..", "..", ".", ".hidden", ""] {
            assert!(
                matches!(delete(&library, folder_name), Err(Error::ThemeInvalid(_))),
                "{folder_name:?}"
            );
            assert!(
                matches!(css(&library, folder_name), Err(Error::ThemeInvalid(_))),
                "{folder_name:?}"
            );
        }
        assert_eq!(names(&dir.0), ["outside", "themes"]);
        assert_eq!(names(&library), ["nord"]);
    }
}
