use tauri::State;

use crate::repo::categories::{
    self, Category, CategoryInput, CategoryPatch, UnclassifiedApp,
};
use crate::storage::DbPool;

#[tauri::command]
pub async fn list_categories(pool: State<'_, DbPool>) -> Result<Vec<Category>, String> {
    categories::list(&pool).await.map_err(Into::into)
}

#[tauri::command]
pub async fn create_category(
    pool: State<'_, DbPool>,
    input: CategoryInput,
) -> Result<Category, String> {
    categories::create(&pool, input).await.map_err(Into::into)
}

#[tauri::command]
pub async fn update_category(
    pool: State<'_, DbPool>,
    id: String,
    patch: CategoryPatch,
) -> Result<(), String> {
    categories::update(&pool, &id, patch)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn delete_category(pool: State<'_, DbPool>, id: String) -> Result<(), String> {
    categories::delete(&pool, &id).await.map_err(Into::into)
}

#[tauri::command]
pub async fn assign_app_to_category(
    pool: State<'_, DbPool>,
    process_name: String,
    category_id: String,
) -> Result<(), String> {
    categories::assign_app(&pool, &process_name, &category_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn unassign_app(pool: State<'_, DbPool>, process_name: String) -> Result<(), String> {
    categories::unassign_app(&pool, &process_name)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn list_unclassified_apps(
    pool: State<'_, DbPool>,
    days_back: Option<u32>,
) -> Result<Vec<UnclassifiedApp>, String> {
    categories::list_unclassified(&pool, days_back.unwrap_or(7))
        .await
        .map_err(Into::into)
}
