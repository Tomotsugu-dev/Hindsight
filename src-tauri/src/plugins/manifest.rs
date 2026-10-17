//! manifest.json in a plugin package. It is read once at install, and the checked values are
//! copied into the plugins table.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Permissions this version knows, and the permission each one must be requested with.
/// For all permissions, see docs/design/plugins/en/permissions.md.
const PERMISSIONS: [(&str, Option<&str>); 6] = [
    ("stats.read", None),
    ("notifications", None),
    ("focus.changed", None),
    ("focus.app", Some("focus.changed")),
    ("focus.category", Some("focus.changed")),
    ("focus.title", Some("focus.changed")),
];

const UI_LOCATIONS: [&str; 3] = ["sidebar", "window", "none"];

/// The contents of manifest.json.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all(serialize = "camelCase"))]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Permissions the plugin asks for (not always the ones the user approved)
    pub permissions: Vec<String>,
    pub ui_location: String,
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
    if !UI_LOCATIONS.contains(&manifest.ui_location.as_str()) {
        return Err(Error::PluginManifest(format!(
            "unknown ui_location {:?}",
            manifest.ui_location
        )));
    }
    check_permissions(&manifest.permissions)?;
    Ok(manifest)
}

/// Checks that the plugin id is valid. The id is a folder name in the plugin library.
///
/// Only lowercase letters, digits and `-` are allowed, and `-` cannot be first or last.
/// At most 64 characters. `..` and `/` cannot appear.
fn check_id(id: &str) -> Result<()> {
    let ok = (1..=64).contains(&id.len())
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
            "permissions": ["stats.read", "focus.changed", "focus.category"],
            "ui_location": "sidebar",
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
        assert_eq!(
            m.permissions,
            ["stats.read", "focus.changed", "focus.category"]
        );
        assert_eq!(m.ui_location, "sidebar");
        assert!(m.runs_in_background);
    }

    /// 不是 JSON、缺字段、名字为空、不认识的页面位置：都报清单错误。
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

        let mut v = focus();
        v["ui_location"] = json!("dynamic-island");
        assert!(matches!(parse_value(&v), Err(Error::PluginManifest(_))));
    }

    /// id：能跑出插件库的、大写的、`-` 开头结尾的、超过 64 个字符的都拒绝。
    #[test]
    fn id_rules() {
        let longest = "a".repeat(64);
        let too_long = "a".repeat(65);
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
}
