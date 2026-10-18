//! Plugin API: a plugin calls `hindsight.call()`, the forwarder adds the plugin id and calls
//! `plugin_call`, and the call ends up here. For the design, see docs/design/plugins/en/api.md.

mod notifications;
mod stats;

use serde_json::Value;

use crate::error::{Error, Result};
use crate::repo::plugins::{self as repo, Plugin};
use crate::storage::DbPool;

/// Runs one call from plugin `plugin_id`.
///
/// - `device`: the device selected in the main window, which the forwarder reads from
///   localStorage. It is a device id or `"all"`, and `None` if nothing was ever selected.
/// - `self_id`: this computer's device id.
/// - `notify`: shows a system notification, given a title and a body.
///
/// # Errors
///
/// - The plugin is not installed, or it is turned off: [`Error::PluginNotEnabled`]
/// - There is no such method: [`Error::PluginUnknownMethod`]
/// - The plugin does not have the permission the method needs: [`Error::PluginNoPermission`]
/// - The parameters are wrong: [`Error::InvalidInput`]
pub async fn call(
    pool: &DbPool,
    plugin_id: &str,
    method: &str,
    params: Value,
    device: Option<String>,
    self_id: &str,
    notify: impl FnOnce(&str, &str) -> Result<()>,
) -> Result<Value> {
    let plugin = repo::get(pool, plugin_id)
        .await?
        .filter(|p| p.enabled)
        .ok_or_else(|| Error::PluginNotEnabled(plugin_id.into()))?;
    let method = Method::parse(method)?;
    check_granted(&plugin, method.permission())?;
    match method {
        Method::DayByCategory => {
            stats::day_by_category(pool, &plugin, params, device, self_id).await
        }
        Method::SendNotification => notifications::send(&plugin, params, notify),
    }
}

/// The methods a plugin can call.
enum Method {
    /// `stats.day_by_category`
    DayByCategory,
    /// `notifications.send`
    SendNotification,
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
            "notifications.send" => Ok(Self::SendNotification),
            _ => Err(Error::PluginUnknownMethod(name.into())),
        }
    }

    /// Returns the permission needed to call this method.
    fn permission(&self) -> &'static str {
        match self {
            Self::DayByCategory => "stats.read",
            Self::SendNotification => "notifications",
        }
    }
}

/// Checks whether the plugin has a permission.
fn has_permission(plugin: &Plugin, permission: &str) -> bool {
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
    use crate::plugins::test_util::{plugin_row, plugin_with_permissions};
    use crate::repo::test_util::fresh_test_pool;

    /// 主界面没选过设备，通知什么都不做。
    async fn call_simple(pool: &DbPool, id: &str, method: &str, params: Value) -> Result<Value> {
        call(pool, id, method, params, None, "this-mac", |_, _| Ok(())).await
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
            ("notifications.send", "notifications"),
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
}
