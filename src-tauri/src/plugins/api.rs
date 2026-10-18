//! Plugin API: a plugin calls `hindsight.call()`, the forwarder adds the plugin id and calls
//! `plugin_call`, and the call ends up here. For the design, see docs/design/plugins/en/api.md.

use serde_json::Value;

use crate::error::{Error, Result};
use crate::repo::plugins as repo;
use crate::storage::DbPool;

/// Runs one call from plugin `plugin_id`.
///
/// # Errors
///
/// - The plugin is not installed or is turned off: [`Error::PluginNotEnabled`]
/// - There is no such method: [`Error::PluginUnknownMethod`]
pub async fn call(pool: &DbPool, plugin_id: &str, method: &str, _params: Value) -> Result<Value> {
    let enabled = repo::get(pool, plugin_id).await?.is_some_and(|p| p.enabled);
    if !enabled {
        return Err(Error::PluginNotEnabled(plugin_id.into()));
    }
    Err(Error::PluginUnknownMethod(method.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::test_util::plugin_row;
    use crate::repo::test_util::fresh_test_pool;

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
            let err = call(&pool, id, "stats.today_by_category", Value::Null)
                .await
                .unwrap_err();
            assert!(
                matches!(err, Error::PluginNotEnabled(ref got) if got == id),
                "{err}"
            );
        }
        let err = call(&pool, "hindsight-focus", "no.such.method", Value::Null)
            .await
            .unwrap_err();
        assert!(
            matches!(err, Error::PluginUnknownMethod(ref got) if got == "no.such.method"),
            "{err}"
        );
    }
}
