//! Repository layer for the plugins table: manages installation, updates, uninstallation, and toggle state.

use rusqlite::OptionalExtension;
use serde::Serialize;

use crate::error::Result;
use crate::storage::{DbPool, SqliteResultExt};

/// One row of the plugins table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
    /// Permissions the user approved, such as `stats.read`
    pub permissions: Vec<String>,
    /// The disk space the user approved, in MB. `None` if the plugin asked for none; then it
    /// cannot use its own database
    pub storage_mb: Option<u32>,
    /// Where the plugin page shows: `sidebar`, `window` or `none`
    pub ui_location: String,
    pub runs_in_background: bool,
    pub installed_at: String,
}

const COLUMNS: &str = "id, name, version, enabled, permissions, ui_location, runs_in_background, \
                       installed_at, storage_mb";

/// Inserts a new plugin or updates an existing plugin's metadata.
///
/// Preserves the user's toggle state (`enabled`) and original `installed_at`
/// timestamp when updating an already installed plugin.
pub async fn save(pool: &DbPool, plugin: Plugin) -> Result<()> {
    let permissions = serde_json::to_string(&plugin.permissions)?;
    pool.0
        .call(move |conn| {
            conn.execute(
                &format!(
                    "INSERT INTO plugins({COLUMNS})
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                     ON CONFLICT(id) DO UPDATE
                        SET name = excluded.name,
                            version = excluded.version,
                            permissions = excluded.permissions,
                            ui_location = excluded.ui_location,
                            runs_in_background = excluded.runs_in_background,
                            storage_mb = excluded.storage_mb"
                ),
                rusqlite::params![
                    plugin.id,
                    plugin.name,
                    plugin.version,
                    plugin.enabled,
                    permissions,
                    plugin.ui_location,
                    plugin.runs_in_background,
                    plugin.installed_at,
                    plugin.storage_mb
                ],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Retrieves a plugin by its ID, returning `None` if it is not installed.
pub async fn get(pool: &DbPool, id: &str) -> Result<Option<Plugin>> {
    let id = id.to_string();
    let plugin = pool
        .0
        .call(move |conn| {
            conn.query_row(
                &format!("SELECT {COLUMNS} FROM plugins WHERE id = ?1"),
                [&id],
                from_row,
            )
            .optional()
            .db()
        })
        .await?;
    Ok(plugin)
}

/// Lists all installed plugins, sorted by ID.
pub async fn list(pool: &DbPool) -> Result<Vec<Plugin>> {
    let plugins = pool
        .0
        .call(|conn| {
            let plugins = conn
                .prepare(&format!("SELECT {COLUMNS} FROM plugins ORDER BY id"))
                .db()?
                .query_map([], from_row)
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(plugins)
        })
        .await?;
    Ok(plugins)
}

/// Enables or disables a plugin by ID.
pub async fn set_enabled(pool: &DbPool, id: &str, enabled: bool) -> Result<()> {
    let id = id.to_string();
    pool.0
        .call(move |conn| {
            conn.execute(
                "UPDATE plugins SET enabled = ?2 WHERE id = ?1",
                rusqlite::params![id, enabled],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Removes a plugin record from the database.
///
/// The database row is deleted before removing the plugin directory from disk: if a crash occurs
/// midway, the orphaned directory will have no matching database record and can be cleaned up the
/// next time the plugin market opens.
pub async fn remove(pool: &DbPool, id: &str) -> Result<()> {
    let id = id.to_string();
    pool.0
        .call(move |conn| {
            conn.execute("DELETE FROM plugins WHERE id = ?1", [&id])
                .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Plugin> {
    let permissions: String = r.get(4)?;
    Ok(Plugin {
        id: r.get(0)?,
        name: r.get(1)?,
        version: r.get(2)?,
        enabled: r.get(3)?,
        permissions: serde_json::from_str(&permissions).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
        })?,
        ui_location: r.get(5)?,
        runs_in_background: r.get(6)?,
        installed_at: r.get(7)?,
        storage_mb: r.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::fresh_test_pool;

    fn focus(version: &str) -> Plugin {
        Plugin {
            id: "hindsight-focus".into(),
            name: "Focus".into(),
            version: version.into(),
            enabled: true,
            permissions: vec!["stats.read".into(), "focus.changed".into()],
            storage_mb: Some(10),
            ui_location: "sidebar".into(),
            runs_in_background: true,
            installed_at: "2026-10-17T00:00:00+00:00".into(),
        }
    }

    async fn ids(pool: &DbPool) -> Vec<String> {
        list(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect()
    }

    /// 装上以后原样读得回来；没装的读到 None。
    #[tokio::test]
    async fn save_then_get() {
        let pool = fresh_test_pool().await;
        save(&pool, focus("0.1.0")).await.unwrap();

        assert_eq!(
            get(&pool, "hindsight-focus").await.unwrap(),
            Some(focus("0.1.0"))
        );
        assert_eq!(get(&pool, "not-installed").await.unwrap(), None);
    }

    /// 更新：名字、版本、权限、容量、页面位置、后台运行换成新的；用户关掉的开关和安装时间不变。
    #[tokio::test]
    async fn update_keeps_switch_and_install_time() {
        let pool = fresh_test_pool().await;
        save(&pool, focus("0.1.0")).await.unwrap();
        set_enabled(&pool, "hindsight-focus", false).await.unwrap();

        let mut newer = focus("0.2.0");
        newer.name = "Focus Timer".into();
        newer.permissions.push("notifications".into());
        newer.storage_mb = Some(200);
        newer.ui_location = "window".into();
        newer.runs_in_background = false;
        newer.installed_at = "2026-10-20T00:00:00+00:00".into();
        save(&pool, newer.clone()).await.unwrap();

        let got = get(&pool, "hindsight-focus").await.unwrap().unwrap();
        assert_eq!(got.name, "Focus Timer");
        assert_eq!(got.version, "0.2.0");
        assert_eq!(got.permissions, newer.permissions);
        assert_eq!(got.storage_mb, Some(200));
        assert_eq!(got.ui_location, "window");
        assert!(!got.runs_in_background);
        assert!(!got.enabled, "更新不能把用户关掉的插件重新打开");
        assert_eq!(got.installed_at, "2026-10-17T00:00:00+00:00");
    }

    /// 按 id 排序列出；卸载后不再出现，别的插件不受影响。
    #[tokio::test]
    async fn list_and_remove() {
        let pool = fresh_test_pool().await;
        let mut weekly = focus("1.0.0");
        weekly.id = "a-weekly-report".into();
        save(&pool, focus("0.1.0")).await.unwrap();
        save(&pool, weekly).await.unwrap();
        assert_eq!(ids(&pool).await, ["a-weekly-report", "hindsight-focus"]);

        remove(&pool, "hindsight-focus").await.unwrap();
        assert_eq!(ids(&pool).await, ["a-weekly-report"]);
    }
}
