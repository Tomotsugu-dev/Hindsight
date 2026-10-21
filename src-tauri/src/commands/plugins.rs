//! Tauri commands for plugins: opening the plugin market, installing, uninstalling, and toggling plugins.

use std::collections::HashMap;
use std::path::PathBuf;

use tauri::{AppHandle, State};
use tauri_plugin_notification::NotificationExt;

use crate::error::Error;
use crate::island::content::Content;
use crate::plugins::api;
use crate::plugins::background_window::{self, BackgroundPlugin};
use crate::plugins::icon;
use crate::plugins::market::{self, Market};
use crate::plugins::pages::{self, PluginPage};
use crate::plugins::{self, install, island, manifest, plugin_zips_dir, plugins_dir};
use crate::repo::plugins as repo;
use crate::storage::DbPool;

/// Lists what the plugin market page shows.
///
/// First deletes what a crash in the middle left behind. If this fails, it is only logged and no
/// error is returned.
///
/// See [`market::list`], [`install::remove_leftovers`] and [`market::remove_partial_downloads`].
#[tauri::command]
pub async fn plugin_open_market(pool: State<'_, DbPool>) -> Result<Market, String> {
    if let Err(e) = install::remove_leftovers(&pool, &plugins_dir()).await {
        log::warn!("plugins: cannot remove leftovers: {e}");
    }
    if let Err(e) = market::remove_partial_downloads(&plugin_zips_dir()) {
        log::warn!("plugins: cannot remove partial downloads: {e}");
    }
    let online = market::fetch_online(market::PLUGINS_JSON_URL).await;
    market::list(&pool, &plugin_zips_dir(), online)
        .await
        .map_err(Into::into)
}

/// Installs or updates a plugin from the zip `file_name` in the plugin zip folder.
///
/// See [`install::install`] and [`plugins::refresh`].
#[tauri::command]
pub async fn plugin_install(
    app: AppHandle,
    pool: State<'_, DbPool>,
    file_name: String,
    approved: Vec<String>,
    storage_mb: Option<u32>,
) -> Result<(), String> {
    let zip = market::zip_path(&plugin_zips_dir(), &file_name)?;
    install::install(&pool, &plugins_dir(), &zip, approved, storage_mb).await?;
    plugins::refresh(&app, &pool).await;
    Ok(())
}

/// Downloads `version` of plugin `id` from the plugin market, then installs it.
///
/// Takes only the id and version. The download URL and SHA-256 are read again from the online
/// list: the check only works if both come from the same list.
/// See [`market::download`], [`install::install`] and [`plugins::refresh`].
#[tauri::command]
pub async fn plugin_install_online(
    app: AppHandle,
    pool: State<'_, DbPool>,
    id: String,
    version: String,
    approved: Vec<String>,
    storage_mb: Option<u32>,
) -> Result<(), String> {
    let online = market::fetch_online(market::PLUGINS_JSON_URL).await?;
    let target = market::find_version(&online, &id, &version)?;
    let zip = market::download(&plugin_zips_dir(), &id, target).await?;
    install::install(&pool, &plugins_dir(), &zip, approved, storage_mb).await?;
    plugins::refresh(&app, &pool).await;
    Ok(())
}

/// Uninstalls a plugin.
///
/// See [`install::uninstall`] and [`plugins::refresh`].
#[tauri::command]
pub async fn plugin_uninstall(
    app: AppHandle,
    pool: State<'_, DbPool>,
    id: String,
    delete_data: bool,
) -> Result<(), String> {
    install::uninstall(&pool, &plugins_dir(), &id, delete_data).await?;
    plugins::refresh(&app, &pool).await;
    Ok(())
}

/// Enables or disables a plugin.
///
/// See [`plugins::refresh`].
#[tauri::command]
pub async fn plugin_set_enabled(
    app: AppHandle,
    pool: State<'_, DbPool>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    repo::set_enabled(&pool, &id, enabled).await?;
    plugins::refresh(&app, &pool).await;
    Ok(())
}

/// Lists the enabled plugins that have a page, for the sidebar and the plugin pages.
///
/// See [`pages::list`].
#[tauri::command]
pub async fn plugin_pages(pool: State<'_, DbPool>) -> Result<Vec<PluginPage>, String> {
    pages::list(&pool).await.map_err(Into::into)
}

/// Returns the icon path of each installed plugin, by plugin id. The frontend turns each path
/// into an asset URL with `convertFileSrc`, the same way as app icons.
///
/// See [`icon::paths`].
#[tauri::command]
pub async fn plugin_icons(pool: State<'_, DbPool>) -> Result<HashMap<String, PathBuf>, String> {
    let installed = repo::list(&pool).await?;
    Ok(icon::paths(&plugins_dir(), &installed))
}

/// Changes where a plugin page opens: `main`, `window`, or `None` (not shown in the sidebar).
///
/// See [`manifest::check_opens_in`] and [`pages::refresh`].
#[tauri::command]
pub async fn plugin_set_opens_in(
    app: AppHandle,
    pool: State<'_, DbPool>,
    id: String,
    opens_in: Option<String>,
) -> Result<(), String> {
    if let Some(opens_in) = &opens_in {
        manifest::check_opens_in(opens_in)?;
    }
    repo::set_opens_in(&pool, &id, opens_in).await?;
    pages::refresh(&app, &pool).await;
    Ok(())
}

/// Opens a plugin page in its own window.
///
/// See [`pages::open_window`].
#[tauri::command]
pub async fn plugin_open_window(
    app: AppHandle,
    pool: State<'_, DbPool>,
    id: String,
) -> Result<(), String> {
    pages::open_window(&app, &pool, &id)
        .await
        .map_err(Into::into)
}

/// Lists the plugins the plugin background window runs.
///
/// See [`background_window::list`].
#[tauri::command]
pub async fn plugin_background_list(
    pool: State<'_, DbPool>,
) -> Result<Vec<BackgroundPlugin>, String> {
    background_window::list(&pool).await.map_err(Into::into)
}

/// The only entry to the plugin API: plugin `plugin_id` calls `method`.
///
/// `plugin_id` and `device` (the device selected in the main window) are filled in by
/// Hindsight's forwarder. The plugin does not report them itself.
///
/// See [`api::call`].
#[tauri::command]
pub async fn plugin_call(
    app: AppHandle,
    pool: State<'_, DbPool>,
    plugin_id: String,
    method: String,
    params: serde_json::Value,
    device: Option<String>,
) -> Result<Box<serde_json::value::RawValue>, String> {
    log::debug!("plugins: {plugin_id} calls {method}");
    let notify = |title: &str, body: &str| {
        app.notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|e| Error::Other(format!("notification: {e}")))
    };
    let island = |content: Option<Content>| match content {
        Some(content) => island::show(&app, &plugin_id, content),
        None => island::hide(&app, &plugin_id),
    };
    let plugins_dir = plugins_dir();
    let host = api::Host {
        pool: &pool,
        plugins_dir: &plugins_dir,
        self_id: crate::device::self_id()?,
        notify,
        island,
    };
    api::call(host, &plugin_id, &method, params, device)
        .await
        .map_err(Into::into)
}

/// 插件 `plugin_id` 的后台被启动器停下了：连着两次 ping 都没回。撤下它放在岛上的内容。
#[tauri::command]
pub fn plugin_stopped(app: AppHandle, plugin_id: String) {
    log::warn!("plugins: {plugin_id} was stopped: no reply to two pings");
    island::hide(&app, &plugin_id);
}

/// Opens the local plugin zip folder in the file manager, creating it if it does not exist.
#[tauri::command]
pub async fn plugin_open_zips_folder() -> Result<(), String> {
    let dir = plugin_zips_dir();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| e.to_string())?;
    // Opening the system file manager may block, so run it on the blocking thread pool
    // to avoid occupying a Tokio worker thread.
    tokio::task::spawn_blocking(move || {
        crate::platform::open_in_file_manager(&dir).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
