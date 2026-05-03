//! 跨设备应用配对的 Tauri 命令。

use tauri::State;

use crate::repo::app_groups::{self, AppGroup};
use crate::storage::DbPool;

#[tauri::command]
pub async fn list_app_groups(pool: State<'_, DbPool>) -> Result<Vec<AppGroup>, String> {
    app_groups::list_groups(&pool).await.map_err(Into::into)
}

#[tauri::command]
pub async fn merge_app_group(
    pool: State<'_, DbPool>,
    process_name: String,
    target_group_id: String,
) -> Result<(), String> {
    app_groups::merge(&pool, &process_name, &target_group_id)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn unmerge_app_group(
    pool: State<'_, DbPool>,
    process_name: String,
) -> Result<(), String> {
    app_groups::unmerge(&pool, &process_name)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn rename_app_group(
    pool: State<'_, DbPool>,
    group_id: String,
    display_name: String,
) -> Result<(), String> {
    app_groups::rename(&pool, &group_id, &display_name)
        .await
        .map_err(Into::into)
}

/// 给组指派分类。category_id = None / 空字符串 → 取消分类。
#[tauri::command]
pub async fn assign_app_group_category(
    pool: State<'_, DbPool>,
    group_id: String,
    category_id: Option<String>,
) -> Result<(), String> {
    let cat = category_id.and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    app_groups::assign_category(&pool, &group_id, cat)
        .await
        .map_err(Into::into)
}
