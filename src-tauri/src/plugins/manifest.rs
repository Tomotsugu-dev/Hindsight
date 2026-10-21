//! manifest.json in a plugin package. It is read once at install, and the checked values are
//! copied into the plugins table.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Permissions this version knows, and the permission each one must be requested with.
/// For all permissions, see docs/design/plugins/en/permissions.md.
const PERMISSIONS: [(&str, Option<&str>); 14] = [
    ("stats.read", None),
    ("stats.today", Some("stats.read")),
    ("stats.week", Some("stats.read")),
    ("stats.month", Some("stats.read")),
    ("stats.history", Some("stats.read")),
    ("devices.read", None),
    ("categories.read", None),
    ("super_categories.read", None),
    ("notifications", None),
    ("island", None),
    ("focus.changed", None),
    ("focus.app", Some("focus.changed")),
    ("focus.category", Some("focus.changed")),
    ("focus.title", Some("focus.changed")),
];

/// Maximum disk space a plugin can request: 10 GB.
pub const MAX_STORAGE_MB: u32 = 10 * 1024;

/// Where the page opens: `main` in the main window, `window` in its own window.
const OPENS_IN: [&str; 2] = ["main", "window"];

/// The window size a plugin can ask for, in logical pixels: each side at least 200, and at most
/// the size of a 2K screen.
const MIN_WINDOW_SIDE: u32 = 200;
const MAX_WINDOW_WIDTH: u32 = 2560;
const MAX_WINDOW_HEIGHT: u32 = 1440;

/// Used when the manifest gives no window size.
pub const DEFAULT_WINDOW: WindowSize = WindowSize {
    width: 900,
    height: 640,
};

/// A window size, in logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

/// The contents of manifest.json.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Permissions the plugin asks for (not always the ones the user approved)
    pub permissions: Vec<String>,
    /// Disk space (MB) the plugin requests for its own database.
    /// If omitted, this is `None` and the plugin cannot use its database.
    pub storage_mb: Option<u32>,
    /// Where the page opens by default, see [`OPENS_IN`]. `None`: not shown in the sidebar, and
    /// opened only from the settings
    pub opens_in: Option<String>,
    /// The window size the first time the page opens in its own window. `None`: install uses
    /// [`DEFAULT_WINDOW`]
    pub window: Option<WindowSize>,
    pub runs_in_background: bool,
}

/// Parses manifest.json and checks that it is valid.
pub fn parse(json: &[u8]) -> Result<Manifest> {
    let manifest: Manifest =
        serde_json::from_slice(json).map_err(|e| Error::PluginManifest(e.to_string()))?;
    check_id(&manifest.id)?;
    check_version(&manifest.version)?;
    if manifest.name.trim().is_empty() {
        return Err(Error::PluginManifest("name is empty".into()));
    }
    if let Some(opens_in) = &manifest.opens_in {
        check_opens_in(opens_in)?;
    }
    if let Some(WindowSize { width, height }) = manifest.window {
        let width_range = MIN_WINDOW_SIDE..=MAX_WINDOW_WIDTH;
        let height_range = MIN_WINDOW_SIDE..=MAX_WINDOW_HEIGHT;
        if !width_range.contains(&width) || !height_range.contains(&height) {
            return Err(Error::PluginManifest(format!(
                "window must be {MIN_WINDOW_SIDE}x{MIN_WINDOW_SIDE} to \
                 {MAX_WINDOW_WIDTH}x{MAX_WINDOW_HEIGHT}, not {width}x{height}"
            )));
        }
    }
    check_permissions(&manifest.permissions)?;
    if !manifest.runs_in_background && manifest.permissions.iter().any(|p| p == "focus.changed") {
        return Err(Error::PluginManifest(
            "focus.changed needs runs_in_background".into(),
        ));
    }
    if let Some(mb) = manifest.storage_mb {
        check_storage_mb(mb)?;
    }
    Ok(manifest)
}

/// Checks that `opens_in` is one of [`OPENS_IN`], whether the author wrote it in the manifest or
/// the user chose it in the settings.
///
/// # Errors
///
/// - It is not one of them: [`Error::PluginManifest`]
pub fn check_opens_in(opens_in: &str) -> Result<()> {
    if !OPENS_IN.contains(&opens_in) {
        return Err(Error::PluginManifest(format!(
            "unknown opens_in {opens_in:?}"
        )));
    }
    Ok(())
}

/// Checks that the disk capacity requested by the plugin or approved by the user is between 1 MB and [`MAX_STORAGE_MB`].
///
/// # Errors
///
/// - If the capacity is outside this range: [`Error::PluginManifest`]
pub fn check_storage_mb(mb: u32) -> Result<()> {
    if !(1..=MAX_STORAGE_MB).contains(&mb) {
        return Err(Error::PluginManifest(format!(
            "storage_mb must be 1 to {MAX_STORAGE_MB}, not {mb}"
        )));
    }
    Ok(())
}

/// Checks that the plugin id is valid. The id is a folder name in the plugin library.
///
/// Only lowercase letters, digits and `-` are allowed, and `-` cannot be first or last.
/// `..` and `/` cannot appear. Maximum length is 63 characters.
pub fn check_id(id: &str) -> Result<()> {
    let ok = (1..=63).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-');
    if !ok {
        return Err(Error::PluginManifest(format!("invalid id {id:?}")));
    }
    Ok(())
}

/// Checks that the version is valid: `number.number.number`, for example `0.1.0`.
pub fn check_version(version: &str) -> Result<()> {
    let parts: Vec<&str> = version.split('.').collect();
    let ok = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if !ok {
        return Err(Error::PluginManifest(format!(
            "invalid version {version:?}"
        )));
    }
    Ok(())
}

fn check_permissions(permissions: &[String]) -> Result<()> {
    for p in permissions {
        match PERMISSIONS.iter().find(|(name, _)| name == p) {
            None => return Err(Error::PluginUnknownPermission(p.clone())),
            Some((_, Some(needs))) if !permissions.iter().any(|q| q == needs) => {
                return Err(Error::PluginManifest(format!("{p} needs {needs}")));
            }
            Some(_) => {}
        }
    }
    // On the install page, the user must see how far back the plugin can read. The ranges are
    // the permissions that need stats.read
    let has = |name: &str| permissions.iter().any(|p| p == name);
    let ranges: Vec<&str> = PERMISSIONS
        .iter()
        .filter(|(_, needs)| *needs == Some("stats.read"))
        .map(|(name, _)| *name)
        .collect();
    if has("stats.read") && !ranges.iter().any(|r| has(r)) {
        return Err(Error::PluginManifest(format!(
            "stats.read needs one of {}",
            ranges.join(", ")
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn focus() -> Value {
        json!({
            "id": "hindsight-focus",
            "name": "Focus",
            "version": "0.1.0",
            "permissions": ["stats.read", "stats.today", "focus.changed", "focus.category"],
            "runs_in_background": true
        })
    }

    fn parse_value(v: &Value) -> Result<Manifest> {
        parse(&serde_json::to_vec(v).unwrap())
    }

    /// 合法的清单原样读出来；认不出的字段忽略。
    #[test]
    fn parses_valid_manifest() {
        let mut v = focus();
        v["homepage"] = json!("https://example.com");
        let m = parse_value(&v).unwrap();
        assert_eq!(m.id, "hindsight-focus");
        assert_eq!(m.name, "Focus");
        assert_eq!(m.version, "0.1.0");
        assert_eq!(json!(m.permissions), focus()["permissions"]);
        assert!(m.runs_in_background);
    }

    /// 不是 JSON、缺字段、名字为空：都报清单错误。
    #[test]
    fn rejects_broken_manifest() {
        assert!(matches!(
            parse(b"{ not json"),
            Err(Error::PluginManifest(_))
        ));

        let mut v = focus();
        v.as_object_mut().unwrap().remove("name");
        assert!(matches!(parse_value(&v), Err(Error::PluginManifest(_))));

        let mut v = focus();
        v["name"] = json!("  ");
        assert!(matches!(parse_value(&v), Err(Error::PluginManifest(_))));
    }

    /// id：能跑出插件库的、大写的、`-` 开头结尾的、超过 63 个字符的都拒绝。
    #[test]
    fn id_rules() {
        let longest = "a".repeat(63);
        let too_long = "a".repeat(64);
        for ok in ["hindsight-focus", "a", "focus2", &longest] {
            assert!(check_id(ok).is_ok(), "{ok} 应该通过");
        }
        for bad in [
            "",
            "..",
            "a/b",
            "a\\b",
            "Focus",
            "-focus",
            "focus-",
            "focus_timer",
            "专注",
            &too_long,
        ] {
            assert!(check_id(bad).is_err(), "{bad:?} 应该被拒绝");
        }
    }

    /// 版本号：只认三段数字。
    #[test]
    fn version_rules() {
        for ok in ["0.1.0", "10.20.30"] {
            assert!(check_version(ok).is_ok(), "{ok} 应该通过");
        }
        for bad in [
            "",
            "..",
            "0.1",
            "0.1.0.1",
            "0.1.a",
            "1..0",
            "v0.1.0",
            "0.1.0-beta",
        ] {
            assert!(check_version(bad).is_err(), "{bad:?} 应该被拒绝");
        }
    }

    /// 不认识的权限单独报错，带上权限名：用户要看到「需要更新 Hindsight」，不是「插件坏了」。
    #[test]
    fn unknown_permission_names_it() {
        let mut v = focus();
        v["permissions"] = json!(["stats.read", "screen.images"]);
        assert!(matches!(
            parse_value(&v),
            Err(Error::PluginUnknownPermission(p)) if p == "screen.images"
        ));
    }

    /// focus.app 这类加字段的权限，没有 focus.changed 就拒绝；一起申请就通过。
    #[test]
    fn focus_fields_need_focus_changed() {
        let mut v = focus();
        v["permissions"] = json!(["focus.app"]);
        assert!(matches!(parse_value(&v), Err(Error::PluginManifest(_))));

        v["permissions"] = json!(["focus.changed", "focus.app", "focus.title"]);
        assert!(parse_value(&v).is_ok());
    }

    /// 不在后台运行的插件收不到事件，申请 focus.changed 就拒绝。
    #[test]
    fn focus_changed_needs_a_background_script() {
        let mut v = focus();
        v["runs_in_background"] = json!(false);
        assert!(
            matches!(parse_value(&v), Err(Error::PluginManifest(m)) if m.contains("runs_in_background"))
        );

        v["permissions"] = json!(["stats.read", "stats.today"]);
        assert!(parse_value(&v).is_ok());
    }

    /// 磁盘容量：不写是 None；1 MB 到 10 GB 之间通过；0、超过 10 GB、负数、小数都拒绝。
    #[test]
    fn storage_mb_rules() {
        assert_eq!(parse_value(&focus()).unwrap().storage_mb, None);
        for ok in [1, 200, MAX_STORAGE_MB] {
            let mut v = focus();
            v["storage_mb"] = json!(ok);
            assert_eq!(parse_value(&v).unwrap().storage_mb, Some(ok));
        }
        for bad in [json!(0), json!(MAX_STORAGE_MB + 1), json!(-1), json!(1.5)] {
            let mut v = focus();
            v["storage_mb"] = bad.clone();
            assert!(
                matches!(parse_value(&v), Err(Error::PluginManifest(_))),
                "{bad} 应该被拒绝"
            );
        }
    }

    /// 打开方式：不写是 None；main、window 通过；别的都拒绝。
    #[test]
    fn opens_in_rules() {
        assert_eq!(parse_value(&focus()).unwrap().opens_in, None);
        for ok in ["main", "window"] {
            let mut v = focus();
            v["opens_in"] = json!(ok);
            assert_eq!(parse_value(&v).unwrap().opens_in.as_deref(), Some(ok));
        }
        for bad in ["sidebar", "hidden", ""] {
            let mut v = focus();
            v["opens_in"] = json!(bad);
            assert!(
                matches!(parse_value(&v), Err(Error::PluginManifest(_))),
                "{bad:?} 应该被拒绝"
            );
        }
    }

    /// 窗口大小：不写是 None；200×200 到 2560×1440 之间通过；哪条边短于 200、宽过 2560、
    /// 高过 1440、缺一条边都拒绝。
    #[test]
    fn window_rules() {
        assert_eq!(parse_value(&focus()).unwrap().window, None);
        for (width, height) in [(200, 200), (360, 480), (2560, 1440)] {
            let mut v = focus();
            v["window"] = json!({ "width": width, "height": height });
            assert_eq!(
                parse_value(&v).unwrap().window,
                Some(WindowSize { width, height })
            );
        }
        for bad in [
            json!({ "width": 199, "height": 480 }),
            json!({ "width": 360, "height": 199 }),
            json!({ "width": 2561, "height": 480 }),
            json!({ "width": 360, "height": 1441 }),
            json!({ "width": 360 }),
        ] {
            let mut v = focus();
            v["window"] = bad.clone();
            assert!(
                matches!(parse_value(&v), Err(Error::PluginManifest(_))),
                "{bad} 应该被拒绝"
            );
        }
    }

    /// stats.read 要配一档；档位要和 stats.read 一起申请。
    #[test]
    fn stats_read_needs_a_range() {
        let mut v = focus();
        for bad in [json!(["stats.read"]), json!(["stats.week"])] {
            v["permissions"] = bad;
            assert!(matches!(parse_value(&v), Err(Error::PluginManifest(_))));
        }
        v["permissions"] = json!(["stats.read", "stats.week"]);
        assert!(parse_value(&v).is_ok());
    }
}
