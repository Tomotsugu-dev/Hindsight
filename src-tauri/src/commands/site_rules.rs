//! Tauri commands for website rules, used by the website classification page. They only
//! turn repo errors into strings.

use chrono::Local;
use tauri::State;

use crate::repo::site_rules::{self, SiteRow};
use crate::storage::DbPool;

/// Lists all websites, with their time and the category each counts toward.
#[tauri::command]
pub async fn list_sites(pool: State<'_, DbPool>) -> Result<Vec<SiteRow>, String> {
    site_rules::list(&pool, Local::now().date_naive())
        .await
        .map_err(Into::into)
}

/// Assigns a website to a category.
#[tauri::command]
pub async fn set_site_rule(
    pool: State<'_, DbPool>,
    host: String,
    category_id: String,
) -> Result<(), String> {
    site_rules::set(&pool, &host, &category_id)
        .await
        .map_err(Into::into)
}

/// Removes a website's own rule.
#[tauri::command]
pub async fn remove_site_rule(pool: State<'_, DbPool>, host: String) -> Result<(), String> {
    site_rules::remove(&pool, &host).await.map_err(Into::into)
}
