//! 跨设备应用配对的 Tauri 命令。

use tauri::State;

use crate::repo::app_groups::{self, AppGroup};
use crate::storage::DbPool;

/// 拉所有应用组（含 display_name + members + category_id）。
#[tauri::command]
pub async fn list_app_groups(pool: State<'_, DbPool>) -> Result<Vec<AppGroup>, String> {
    app_groups::list_groups(&pool).await.map_err(Into::into)
}

/// 新建一个空应用组（仅有 display_name，无成员）。返回新组 group_id。
/// 用户在「分类」页可手动建组再合并 process_names 进去。
#[tauri::command]
pub async fn create_app_group(
    pool: State<'_, DbPool>,
    display_name: String,
) -> Result<String, String> {
    app_groups::create(&pool, &display_name)
        .await
        .map_err(Into::into)
}

/// 删除应用组。组内 process 退回各自单成员组（不丢数据）。
#[tauri::command]
pub async fn delete_app_group(pool: State<'_, DbPool>, group_id: String) -> Result<(), String> {
    app_groups::delete(&pool, &group_id)
        .await
        .map_err(Into::into)
}

/// 强力删除应用组：组 + 所有 member 一起软删。给 UI 上「行视觉为空」（成员存在但
/// 全部近 7 天无活动）场景用。详见 [`app_groups::purge_with_members`]。
#[tauri::command]
pub async fn purge_app_group(pool: State<'_, DbPool>, group_id: String) -> Result<(), String> {
    app_groups::purge_with_members(&pool, &group_id)
        .await
        .map_err(Into::into)
}

/// 把某 process_name 合并到目标组（让两个 process 在统计上算同一应用）。
/// 例如把 `chrome.exe` 和 `Google Chrome` 合并。
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

/// 把某 process_name 从所在组拆出来变成单成员独立组。
#[tauri::command]
pub async fn unmerge_app_group(
    pool: State<'_, DbPool>,
    process_name: String,
) -> Result<(), String> {
    app_groups::unmerge(&pool, &process_name)
        .await
        .map_err(Into::into)
}

/// 改组的展示名（不影响成员关系，也不影响 category）。
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
