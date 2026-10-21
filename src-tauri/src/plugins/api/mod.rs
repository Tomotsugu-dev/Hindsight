//! Plugin API: a plugin calls `hindsight.call()`, the forwarder adds the plugin id and calls
//! `plugin_call`, and the call ends up here. For the design, see docs/design/plugins/en/api.md.

pub(super) mod categories;
mod db;
mod island;
mod notifications;
mod stats;

use std::path::Path;

use serde_json::value::{to_raw_value, RawValue};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::island::content::Content;
use crate::repo::plugins::{self as repo, Plugin};
use crate::storage::DbPool;
use categories::{CategoryRef, SuperCategoryRef};

/// What a call needs from the app. It is the same for every call.
pub struct Host<'a, N, I> {
    pub pool: &'a DbPool,
    /// The plugin library. Each plugin's own database is in it too
    pub plugins_dir: &'a Path,
    /// The device id of this machine
    pub self_id: &'a str,
    /// Shows a system notification. The parameters are the title and the body
    pub notify: N,
    /// 把这个插件的内容放到岛上（`Some`），或者撤下（`None`）
    pub island: I,
}

/// Runs one call from plugin `plugin_id`.
///
/// - `device`: the device selected in the main window, which the forwarder reads from
///   localStorage. It is a device id or `"all"`, and `None` if nothing was ever selected.
///
/// # Errors
///
/// - The plugin is not installed, or it is turned off: [`Error::PluginNotEnabled`]
/// - There is no such method: [`Error::PluginUnknownMethod`]
/// - The plugin does not have the permission the method needs: [`Error::PluginNoPermission`]
/// - The parameters are wrong: [`Error::InvalidInput`]
/// - 岛的内容不合规则：[`Error::InvalidInputDyn`]
/// - The plugin's SQL has an error, is denied, runs too long, or its database is full:
///   [`Error::Sqlite`]
pub async fn call(
    host: Host<'_, impl FnOnce(&str, &str) -> Result<()>, impl FnOnce(Option<Content>)>,
    plugin_id: &str,
    method: &str,
    params: Value,
    device: Option<String>,
) -> Result<Box<RawValue>> {
    let Host {
        pool,
        plugins_dir,
        self_id,
        notify,
        island,
    } = host;
    let plugin = repo::get(pool, plugin_id)
        .await?
        .filter(|p| p.enabled)
        .ok_or_else(|| Error::PluginNotEnabled(plugin_id.into()))?;
    let method = Method::parse(method)?;
    if let Some(permission) = method.permission() {
        check_granted(&plugin, permission)?;
    }
    let value = match method {
        Method::DayByCategory => {
            stats::day_by_category(pool, &plugin, params, device, self_id).await?
        }
        Method::ListCategories => serde_json::to_value(CategoryRef::list(pool).await?)?,
        Method::ListSuperCategories => serde_json::to_value(SuperCategoryRef::list(pool).await?)?,
        Method::SendNotification => notifications::send(&plugin, params, notify)?,
        Method::ShowIsland => island::show(&plugin, params, island)?,
        Method::HideIsland => island::hide(island)?,
        Method::DbExecute => db::execute(plugins_dir, &plugin, params).await?,
        // The rows are written as JSON text while they are read, so the size can be limited in
        // bytes
        Method::DbQuery => return db::query(plugins_dir, &plugin, params).await,
    };
    Ok(to_raw_value(&value)?)
}

/// The methods a plugin can call.
enum Method {
    /// `stats.day_by_category`
    DayByCategory,
    /// `categories.list`
    ListCategories,
    /// `super_categories.list`
    ListSuperCategories,
    /// `notifications.send`
    SendNotification,
    /// `island.show`
    ShowIsland,
    /// `island.hide`
    HideIsland,
    /// `db.execute`
    DbExecute,
    /// `db.query`
    DbQuery,
}

impl Method {
    /// Finds the method with this name.
    ///
    /// # Errors
    ///
    /// - There is no such method: [`Error::PluginUnknownMethod`]
    fn parse(name: &str) -> Result<Self> {
        match name {
            "stats.day_by_category" => Ok(Self::DayByCategory),
            "categories.list" => Ok(Self::ListCategories),
            "super_categories.list" => Ok(Self::ListSuperCategories),
            "notifications.send" => Ok(Self::SendNotification),
            "island.show" => Ok(Self::ShowIsland),
            "island.hide" => Ok(Self::HideIsland),
            "db.execute" => Ok(Self::DbExecute),
            "db.query" => Ok(Self::DbQuery),
            _ => Err(Error::PluginUnknownMethod(name.into())),
        }
    }

    /// The permission this method needs. Reading and writing the plugin's own database needs
    /// none, so it is `None`.
    fn permission(&self) -> Option<&'static str> {
        match self {
            Self::DayByCategory => Some("stats.read"),
            Self::ListCategories => Some("categories.read"),
            Self::ListSuperCategories => Some("super_categories.read"),
            Self::SendNotification => Some("notifications"),
            Self::ShowIsland | Self::HideIsland => Some("island"),
            Self::DbExecute | Self::DbQuery => None,
        }
    }
}

/// Checks whether the plugin has a permission.
pub(super) fn has_permission(plugin: &Plugin, permission: &str) -> bool {
    plugin.permissions.iter().any(|p| p == permission)
}

/// Checks that the plugin has a permission.
///
/// # Errors
///
/// - The plugin does not have it: [`Error::PluginNoPermission`]
fn check_granted(plugin: &Plugin, permission: &str) -> Result<()> {
    if has_permission(plugin, permission) {
        Ok(())
    } else {
        Err(Error::PluginNoPermission(permission.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::test_util::{plugin_row, plugin_with_permissions, TempDir};
    use crate::repo::test_util::fresh_test_pool;

    /// 主界面没选过设备，通知和岛都什么都不做。
    async fn call_in(
        pool: &DbPool,
        plugins_dir: &Path,
        id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        let host = Host {
            pool,
            plugins_dir,
            self_id: "this-mac",
            notify: |_: &str, _: &str| Ok(()),
            island: |_: Option<Content>| {},
        };
        let raw = call(host, id, method, params, None).await?;
        Ok(serde_json::from_str(raw.get()).unwrap())
    }

    /// 同上，用不到插件自己的库。
    async fn call_simple(pool: &DbPool, id: &str, method: &str, params: Value) -> Result<Value> {
        call_in(pool, Path::new("no-plugins-dir"), id, method, params).await
    }

    /// 没装的、关着的插件调不了；开着的插件调不存在的方法，报不认识这个方法。
    #[tokio::test]
    async fn call_checks_plugin_then_method() {
        let pool = fresh_test_pool().await;
        repo::save(&pool, plugin_row("hindsight-focus", true, true))
            .await
            .unwrap();
        repo::save(&pool, plugin_row("turned-off", false, true))
            .await
            .unwrap();

        for id in ["not-installed", "turned-off"] {
            let err = call_simple(&pool, id, "stats.day_by_category", Value::Null)
                .await
                .unwrap_err();
            assert!(
                matches!(err, Error::PluginNotEnabled(ref got) if got == id),
                "{err}"
            );
        }
        let err = call_simple(&pool, "hindsight-focus", "no.such.method", Value::Null)
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::PluginUnknownMethod(ref got) if got == "no.such.method"),
            "{err}"
        );
    }

    /// 没有方法要的权限就报错，错误里是缺的那个权限。
    #[tokio::test]
    async fn call_needs_the_method_permission() {
        let pool = fresh_test_pool().await;
        repo::save(&pool, plugin_with_permissions("no-permissions", &[]))
            .await
            .unwrap();

        for (method, permission) in [
            ("stats.day_by_category", "stats.read"),
            ("categories.list", "categories.read"),
            ("super_categories.list", "super_categories.read"),
            ("notifications.send", "notifications"),
            ("island.show", "island"),
            ("island.hide", "island"),
        ] {
            let err = call_simple(&pool, "no-permissions", method, Value::Null)
                .await
                .unwrap_err();
            assert!(
                matches!(err, Error::PluginNoPermission(ref got) if got == permission),
                "{method}: {err}"
            );
        }
    }

    /// 没有任何权限、只有磁盘容量的插件也能读写自己的库。
    #[tokio::test]
    async fn db_methods_need_no_permission() {
        let pool = fresh_test_pool().await;
        let dir = TempDir::new();
        std::fs::create_dir_all(dir.plugins().join("no-permissions")).unwrap();
        let mut plugin = plugin_with_permissions("no-permissions", &[]);
        plugin.storage_mb = Some(1);
        repo::save(&pool, plugin).await.unwrap();

        let sql = serde_json::json!({ "sql": "CREATE TABLE t (x)" });
        call_in(&pool, &dir.plugins(), "no-permissions", "db.execute", sql)
            .await
            .unwrap();
        let sql = serde_json::json!({ "sql": "SELECT count(*) AS n FROM t" });
        let rows = call_in(&pool, &dir.plugins(), "no-permissions", "db.query", sql)
            .await
            .unwrap();
        assert_eq!(rows, serde_json::json!([{ "n": 0 }]));
    }
}
