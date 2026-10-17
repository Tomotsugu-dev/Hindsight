//! Plugins: installing and running third-party plugins. For the design, see docs/design/plugins/.

pub mod install;
pub mod manifest;

use std::path::PathBuf;

/// The plugin library: one folder per plugin, inside the data folder.
pub fn plugins_dir() -> PathBuf {
    crate::bootstrap::data_root().join("plugins")
}
