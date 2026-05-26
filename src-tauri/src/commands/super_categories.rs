//! 大类（super-category）的 Tauri 命令。CRUD + 拖拽排序 + 给分类指派大类。
//!
//! v1：所有写都是单设备本地写，不上 outbox。多设备同步是 TODO。

use tauri::State;

use crate::repo::super_categories::{
    self, SuperCategory, SuperCategoryInput, SuperCategoryPatch,
};
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

/// 把某分类归到某大类。`super_id = None` / 空字符串 = 移出大类回到「未归入」。
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
