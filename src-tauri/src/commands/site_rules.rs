//! Tauri commands for website rules, used by the website classification page.

use std::sync::Arc;

use chrono::Local;
use tauri::State;

use crate::capture::CaptureService;
use crate::commands::screen_memory::MemoryState;
use crate::privacy::purge_private_after_change;
use crate::repo::categories::PRIVATE_CATEGORY_ID;
use crate::repo::site_rules::{self, SiteRow};
use crate::storage::DbPool;
use crate::sync::engine::SyncEngine;

/// Lists all websites, with their time and the category each counts toward.
#[tauri::command]
pub async fn list_sites(pool: State<'_, DbPool>) -> Result<Vec<SiteRow>, String> {
    site_rules::list(&pool, Local::now().date_naive())
        .await
        .map_err(Into::into)
}

/// Assigns a website to a category.
///
/// When set to private, deletes private data after saving.
#[tauri::command]
pub async fn set_site_rule(
    pool: State<'_, DbPool>,
    mem: State<'_, MemoryState>,
    svc: State<'_, Arc<CaptureService>>,
    engine: State<'_, Arc<SyncEngine>>,
    host: String,
    category_id: String,
) -> Result<(), String> {
    site_rules::set(&pool, &host, &category_id)
        .await
        .map_err(String::from)?;
    if category_id == PRIVATE_CATEGORY_ID {
        purge_private_after_change(&pool, mem.0.as_ref(), &svc, &engine).await?;
    }
    Ok(())
}

/// Removes a website's own rule.
///
/// Deletes private data after saving.
#[tauri::command]
pub async fn remove_site_rule(
    pool: State<'_, DbPool>,
    mem: State<'_, MemoryState>,
    svc: State<'_, Arc<CaptureService>>,
    engine: State<'_, Arc<SyncEngine>>,
    host: String,
) -> Result<(), String> {
    site_rules::remove(&pool, &host)
        .await
        .map_err(String::from)?;
    purge_private_after_change(&pool, mem.0.as_ref(), &svc, &engine).await
}
