//! Tauri commands for plugins: opening the plugin market, installing, uninstalling, and toggling plugins.

use tauri::State;

use crate::plugins::market::{self, Market};
use crate::plugins::{install, plugin_zips_dir, plugins_dir};
use crate::repo::plugins as repo;
use crate::storage::DbPool;

/// Lists what the plugin market page shows.
///
/// First deletes what a crash in the middle left behind. If this fails, it is only logged and no
/// error is returned.
///
/// See [`market::list`] and [`install::remove_leftovers`].
#[tauri::command]
pub async fn plugin_open_market(pool: State<'_, DbPool>) -> Result<Market, String> {
    if let Err(e) = install::remove_leftovers(&pool, &plugins_dir()).await {
        log::warn!("plugins: cannot remove leftovers: {e}");
    }
    let online = market::fetch_online(market::PLUGINS_JSON_URL).await;
    market::list(&pool, &plugin_zips_dir(), online)
        .await
        .map_err(Into::into)
}

/// Installs or updates a plugin from the zip `file_name` in the plugin zip folder.
///
/// See [`install::install`].
#[tauri::command]
pub async fn plugin_install(
    pool: State<'_, DbPool>,
    file_name: String,
    approved: Vec<String>,
) -> Result<(), String> {
    let zip = market::zip_path(&plugin_zips_dir(), &file_name)?;
    install::install(&pool, &plugins_dir(), &zip, approved)
        .await
        .map_err(Into::into)
}

/// Uninstalls a plugin.
///
/// See [`install::uninstall`].
#[tauri::command]
pub async fn plugin_uninstall(
    pool: State<'_, DbPool>,
    id: String,
    delete_data: bool,
) -> Result<(), String> {
    install::uninstall(&pool, &plugins_dir(), &id, delete_data)
        .await
        .map_err(Into::into)
}

/// Enables or disables a plugin.
#[tauri::command]
pub async fn plugin_set_enabled(
    pool: State<'_, DbPool>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    repo::set_enabled(&pool, &id, enabled)
        .await
        .map_err(Into::into)
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
