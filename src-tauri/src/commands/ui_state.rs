//! The commands for the main window's UI state (ADR-0020).

use std::collections::HashMap;

use tauri::State;

use crate::repo::ui_state;
use crate::storage::DbPool;

/// Reads every key and its value. See [`ui_state::load`].
#[tauri::command]
pub async fn get_ui_state(pool: State<'_, DbPool>) -> Result<HashMap<String, String>, String> {
    ui_state::load(&pool).await.map_err(Into::into)
}

/// Saves the value of `key`, or deletes the key if `value` is `null`. See [`ui_state::set`].
#[tauri::command]
pub async fn set_ui_state(
    pool: State<'_, DbPool>,
    key: String,
    value: Option<String>,
) -> Result<(), String> {
    ui_state::set(&pool, key, value).await.map_err(Into::into)
}

/// Imports the values from localStorage, and returns every key and its value after the import.
/// See [`ui_state::import`].
#[tauri::command]
pub async fn import_ui_state(
    pool: State<'_, DbPool>,
    entries: HashMap<String, String>,
) -> Result<HashMap<String, String>, String> {
    ui_state::import(&pool, entries).await.map_err(Into::into)
}
