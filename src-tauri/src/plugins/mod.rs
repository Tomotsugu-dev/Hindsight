//! Plugins: installing and running third-party plugins. For the design, see docs/design/plugins/.

pub mod api;
pub mod background_window;
pub mod events;
pub mod file_reader;
pub mod icon;
pub mod install;
pub mod manifest;
pub mod market;
pub mod pages;
#[cfg(test)]
mod test_util;

use std::path::PathBuf;

use tauri::AppHandle;

use crate::storage::DbPool;

/// Call after a plugin is installed, uninstalled, enabled or disabled, so that the plugin
/// background window and the page windows follow the new plugin list.
///
/// See [`background_window::refresh`] and [`pages::refresh`].
pub async fn refresh(app: &AppHandle, pool: &DbPool) {
    background_window::refresh(app, pool).await;
    pages::refresh(app, pool).await;
}

/// Returns the plugin library: one folder per plugin, inside the data folder.
pub fn plugins_dir() -> PathBuf {
    crate::bootstrap::data_root().join("plugins")
}

/// Returns the path to the plugin ZIP folder.
///
/// ZIP files for plugins to be installed should be placed here.
pub fn plugin_zips_dir() -> PathBuf {
    crate::bootstrap::data_root().join("plugin-zips")
}
