//! Background window manager: when some plugins run in the background, it opens the hidden plugin
//! background window, and tells the window when the plugin list changes.

use serde::Serialize;
use tauri::utils::config::BackgroundThrottlingPolicy;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::error::Result;
use crate::plugins::file_reader;
use crate::repo::plugins as repo;
use crate::storage::DbPool;

/// Label of the plugin background window. `capabilities/plugin-background.json` uses the same
/// label.
pub(super) const LABEL: &str = "plugin-background";

/// Name of the event sent to the plugin background window when the plugin list changes.
const CHANGED_EVENT: &str = "plugins-changed";

/// A plugin that runs in the plugin background window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundPlugin {
    pub id: String,
    pub launcher_url: String,
}

/// Lists the enabled plugins that run in the background.
pub async fn list(pool: &DbPool) -> Result<Vec<BackgroundPlugin>> {
    let plugins = repo::list(pool).await?;
    Ok(plugins
        .into_iter()
        .filter(|p| p.enabled && p.runs_in_background)
        .map(|p| BackgroundPlugin {
            launcher_url: file_reader::launcher_url(&p.id, &p.version),
            id: p.id,
        })
        .collect())
}

/// Brings the plugin background window up to date with the plugin list: closes the window if no
/// plugin runs in the background, and opens it if some do. If the window is already open, sends
/// it the new list.
///
/// Failures are only logged: when this runs, the plugin has already been installed, uninstalled,
/// enabled or disabled.
pub async fn refresh(app: &AppHandle, pool: &DbPool) {
    let plugins = match list(pool).await {
        Ok(plugins) => plugins,
        Err(e) => {
            log::warn!("plugins: cannot list background plugins: {e}");
            return;
        }
    };
    let done = match (app.get_webview_window(LABEL), plugins.is_empty()) {
        // Window open, no plugin runs in the background → destroy the window
        (Some(window), true) => window.destroy(),
        // Window open, some plugins run in the background → send the new list. The forwarder in
        // the window adds and removes iframes itself
        (Some(_), false) => app.emit_to(LABEL, CHANGED_EVENT, plugins),
        // No window, no plugin runs in the background → nothing to do
        (None, true) => Ok(()),
        // No window, some plugins run in the background → open the window
        (None, false) => open(app),
    };
    if let Err(e) = done {
        log::warn!("plugins: cannot update the background window: {e}");
    }
}

/// Opens the plugin background window. After it loads, the page in the window gets the plugin list
/// itself.
fn open(app: &AppHandle) -> tauri::Result<()> {
    WebviewWindowBuilder::new(
        app,
        LABEL,
        WebviewUrl::App("src/plugin-background/index.html".into()),
    )
    .visible(false)
    .skip_taskbar(true)
    // Otherwise plugin timers stop while the window is hidden
    .background_throttling(BackgroundThrottlingPolicy::Disabled)
    .build()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::test_util::plugin_row;
    use crate::repo::test_util::fresh_test_pool;

    /// 只列出开着、要在后台运行的插件。
    #[tokio::test]
    async fn lists_enabled_background_plugins() {
        let pool = fresh_test_pool().await;
        for (id, enabled, runs_in_background) in [
            ("hindsight-focus", true, true),
            ("turned-off", false, true),
            ("page-only", true, false),
        ] {
            repo::save(&pool, plugin_row(id, enabled, runs_in_background))
                .await
                .unwrap();
        }

        assert_eq!(
            list(&pool).await.unwrap(),
            [BackgroundPlugin {
                id: "hindsight-focus".into(),
                launcher_url: file_reader::launcher_url("hindsight-focus", "0.1.0"),
            }]
        );
    }
}
