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
    pub ui: UiSettings,
    pub runs_in_background: bool,
    pub installed_at: String,
}

/// How the plugin page shows. `has_page` comes from the package and is replaced on update.
/// The other fields are the user's settings and are kept on update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiSettings {
    /// Whether the package has `ui/index.html`
    pub has_page: bool,
    /// Where the page opens: `main` in the main window, `window` in its own window. `None`: not
    /// shown in the sidebar
    pub opens_in: Option<String>,
    /// Where the page's own window was last, in logical pixels. `None` if it was never opened
    pub window_x: Option<i32>,
    pub window_y: Option<i32>,
    /// The size of the page's own window, in logical pixels. The manifest's size at install, and
    /// the user's size once the user resizes it
    pub window_width: u32,
    pub window_height: u32,
}

const COLUMNS: &str = "id, name, version, enabled, permissions, has_page, runs_in_background, \
                       installed_at, storage_mb, opens_in, window_x, window_y, window_width, \
                       window_height";

/// Installs a plugin, or updates one that is already installed.
///
/// An update keeps what the user set: the on/off switch, the fields of [`UiSettings`] other than
/// `has_page`, and the install time.
pub async fn save(pool: &DbPool, plugin: Plugin) -> Result<()> {
    let permissions = serde_json::to_string(&plugin.permissions)?;
    pool.0
        .call(move |conn| {
            conn.execute(
                &format!(
                    "INSERT INTO plugins({COLUMNS})
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                     ON CONFLICT(id) DO UPDATE
                        SET name = excluded.name,
                            version = excluded.version,
                            permissions = excluded.permissions,
                            has_page = excluded.has_page,
                            runs_in_background = excluded.runs_in_background,
                            storage_mb = excluded.storage_mb"
                ),
                rusqlite::params![
                    plugin.id,
                    plugin.name,
                    plugin.version,
                    plugin.enabled,
                    permissions,
                    plugin.ui.has_page,
                    plugin.runs_in_background,
                    plugin.installed_at,
                    plugin.storage_mb,
                    plugin.ui.opens_in,
                    plugin.ui.window_x,
                    plugin.ui.window_y,
                    plugin.ui.window_width,
                    plugin.ui.window_height
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

/// Changes where the plugin page opens.
pub async fn set_opens_in(pool: &DbPool, id: &str, opens_in: Option<String>) -> Result<()> {
    let id = id.to_string();
    pool.0
        .call(move |conn| {
            conn.execute(
                "UPDATE plugins SET opens_in = ?2 WHERE id = ?1",
                rusqlite::params![id, opens_in],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Saves the position and size of the page's own window, in logical pixels.
///
/// Changes only these four columns, so this and [`set_opens_in`] cannot overwrite each other
/// when they write close together.
pub async fn set_window_bounds(
    pool: &DbPool,
    id: &str,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> Result<()> {
    let id = id.to_string();
    pool.0
        .call(move |conn| {
            conn.execute(
                "UPDATE plugins SET window_x = ?2, window_y = ?3, window_width = ?4,
                                    window_height = ?5
                  WHERE id = ?1",
                rusqlite::params![id, x, y, width, height],
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
        runs_in_background: r.get(6)?,
        installed_at: r.get(7)?,
        storage_mb: r.get(8)?,
        ui: UiSettings {
            has_page: r.get(5)?,
            opens_in: r.get(9)?,
            window_x: r.get(10)?,
            window_y: r.get(11)?,
            window_width: r.get(12)?,
            window_height: r.get(13)?,
        },
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
            ui: UiSettings {
                has_page: true,
                opens_in: Some("main".into()),
                window_x: None,
                window_y: None,
                window_width: 900,
                window_height: 640,
            },
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

    /// 更新：名字、版本、权限、容量、有没有页面、后台运行换成新的；用户设的开关、
    /// 页面显示的其余几项，以及安装时间不变。
    #[tokio::test]
    async fn update_keeps_what_the_user_set() {
        let pool = fresh_test_pool().await;
        let mut older = focus("0.1.0");
        older.enabled = false;
        older.ui = UiSettings {
            has_page: true,
            opens_in: Some("window".into()),
            window_x: Some(-1200),
            window_y: Some(80),
            window_width: 400,
            window_height: 700,
        };
        save(&pool, older.clone()).await.unwrap();

        let mut newer = focus("0.2.0");
        newer.name = "Focus Timer".into();
        newer.permissions.push("notifications".into());
        newer.storage_mb = Some(200);
        newer.ui.has_page = false;
        newer.runs_in_background = false;
        newer.installed_at = "2026-10-20T00:00:00+00:00".into();
        save(&pool, newer.clone()).await.unwrap();

        let got = get(&pool, "hindsight-focus").await.unwrap().unwrap();
        assert_eq!(
            got,
            Plugin {
                name: newer.name,
                version: newer.version,
                permissions: newer.permissions,
                storage_mb: newer.storage_mb,
                ui: UiSettings {
                    has_page: newer.ui.has_page,
                    ..older.ui
                },
                runs_in_background: newer.runs_in_background,
                ..older
            }
        );
    }

    /// 改打开方式、存窗口：只动自己那几列，别的列不变；改成 None 就是侧边栏里不显示。
    #[tokio::test]
    async fn set_opens_in_and_window_bounds() {
        let pool = fresh_test_pool().await;
        save(&pool, focus("0.1.0")).await.unwrap();

        set_window_bounds(&pool, "hindsight-focus", -1200, 80, 400, 700)
            .await
            .unwrap();
        set_opens_in(&pool, "hindsight-focus", None).await.unwrap();

        let got = get(&pool, "hindsight-focus").await.unwrap().unwrap();
        assert_eq!(
            got.ui,
            UiSettings {
                opens_in: None,
                window_x: Some(-1200),
                window_y: Some(80),
                window_width: 400,
                window_height: 700,
                ..focus("0.1.0").ui
            }
        );
        assert_eq!(
            Plugin {
                ui: focus("0.1.0").ui,
                ..got
            },
            focus("0.1.0")
        );
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
