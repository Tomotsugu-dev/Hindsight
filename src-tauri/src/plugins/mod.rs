//! Plugins: installing and running third-party plugins. For the design, see docs/design/plugins/.

pub mod api;
pub mod background_window;
pub mod file_reader;
pub mod install;
pub mod manifest;
pub mod market;
#[cfg(test)]
mod test_util;

use std::path::PathBuf;

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
