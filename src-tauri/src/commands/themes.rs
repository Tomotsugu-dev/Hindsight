//! Tauri commands for custom themes: listing themes, reading a theme's CSS, importing, deleting,
//! and opening the theme library.

use std::path::PathBuf;

use crate::error::Result;
use crate::themes::plugin::library::{self, themes_dir, InstalledTheme};
use crate::themes::plugin::manifest::Theme;

/// Lists the themes in the theme library. See [`library::list`].
#[tauri::command]
pub async fn theme_list() -> std::result::Result<Vec<InstalledTheme>, String> {
    blocking(|| library::list(&themes_dir())).await
}

/// Returns the theme.css of theme `id`, or `None` if there is none. See [`library::css`].
#[tauri::command]
pub async fn theme_css(id: String) -> std::result::Result<Option<String>, String> {
    blocking(move || library::css(&themes_dir(), &id)).await
}

/// Imports the theme.json that the user picked, and returns the imported theme. See
/// [`library::import`].
#[tauri::command]
pub async fn theme_import(path: String) -> std::result::Result<Theme, String> {
    blocking(move || library::import(&themes_dir(), &PathBuf::from(path))).await
}

/// Deletes the folder `folder_name` in the theme library. See [`library::delete`].
#[tauri::command]
pub async fn theme_delete(folder_name: String) -> std::result::Result<(), String> {
    blocking(move || library::delete(&themes_dir(), &folder_name)).await
}

/// Opens the theme library in the system's file manager. Creates it first if it does not exist.
#[tauri::command]
pub async fn theme_open_folder() -> std::result::Result<(), String> {
    blocking(|| {
        let dir = themes_dir();
        std::fs::create_dir_all(&dir)?;
        crate::platform::open_in_file_manager(&dir)?;
        Ok(())
    })
    .await
}

/// Runs `f` on the blocking thread pool. Reading and writing files and opening the file manager
/// hold the thread, so they must not run on a Tokio worker thread.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T> + Send + 'static,
) -> std::result::Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())?
        .map_err(Into::into)
}
