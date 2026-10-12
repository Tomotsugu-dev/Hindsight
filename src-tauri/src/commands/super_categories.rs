//! Tauri commands for super-categories: CRUD, drag-to-reorder, and assigning categories to them.
//!
//! Commands only call the repo layer; the repo enqueues the outbox rows in the same
//! transaction, so changes sync to the other devices.

use tauri::State;

use crate::repo::super_categories::{self, SuperCategory, SuperCategoryInput, SuperCategoryPatch};
use crate::storage::DbPool;

#[tauri::command]
pub async fn list_super_categories(pool: State<'_, DbPool>) -> Result<Vec<SuperCategory>, String> {
    super_categories::list(&pool).await.map_err(Into::into)
}

#[tauri::command]
pub async fn create_super_category(
    pool: State<'_, DbPool>,
    input: SuperCategoryInput,
) -> Result<SuperCategory, String> {
    super_categories::create(&pool, input)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn update_super_category(
    pool: State<'_, DbPool>,
    id: String,
    patch: SuperCategoryPatch,
) -> Result<(), String> {
    super_categories::update(&pool, &id, patch)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn reorder_super_categories(
    pool: State<'_, DbPool>,
    ordered_ids: Vec<String>,
) -> Result<(), String> {
    super_categories::reorder(&pool, ordered_ids)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_super_category(pool: State<'_, DbPool>, id: String) -> Result<(), String> {
    super_categories::delete(&pool, &id)
        .await
        .map_err(Into::into)
}

/// Assigns a category to a super-category.
///
/// A `super_id` of `None` or an empty string removes it, back to "unassigned".
#[tauri::command]
pub async fn assign_category_to_super(
    pool: State<'_, DbPool>,
    category_id: String,
    super_id: Option<String>,
) -> Result<(), String> {
    let normalized = super_id.and_then(|s| {
        let t = s.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    });
    super_categories::assign_category(&pool, &category_id, normalized)
        .await
        .map_err(Into::into)
}
