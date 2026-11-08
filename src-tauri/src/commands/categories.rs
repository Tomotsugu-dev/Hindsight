//! 应用分类相关 Tauri 命令——给前端「分类」页面用。
//!
//! 命令薄壳：参数 / 返回值校验由 [`crate::repo::categories`] 层做，本文件只把
//! crate Error 适配成 String 给前端。

use std::collections::HashMap;
use std::sync::Arc;

use tauri::State;

use crate::capture::CaptureService;
use crate::commands::screen_memory::MemoryState;
use crate::repo::categories::{self, Category, CategoryInput, CategoryPatch, UnclassifiedApp};
use crate::repo::purge::{self, PurgeReport, PurgeTarget};
use crate::repo::settings;
use crate::storage::{migrations, DbPool};
use crate::sync::engine::SyncEngine;

/// Deletes all data of the apps and websites now in the `private` category, and returns how
/// much was deleted. When an app or a website is set as private, the category may be saved
/// while the deletion fails. The frontend then calls this to try again.
///
/// Sync is paused first: the deletion writes outbox rows and deletes from tables that push
/// reads. Then capture drops the activity row it is recording, because that row may be one of
/// the rows to delete.
#[tauri::command]
pub async fn purge_private_data(
    pool: State<'_, DbPool>,
    mem: State<'_, MemoryState>,
    svc: State<'_, Arc<CaptureService>>,
    engine: State<'_, Arc<SyncEngine>>,
) -> Result<PurgeReport, String> {
    let cfg = settings::load(&pool).await.map_err(String::from)?;
    let root = std::path::PathBuf::from(&cfg.screenshot_path);
    let _paused = engine.pause_flushes().await;
    svc.run_with_session_cleared(|| async {
        let self_id = engine.self_id();
        purge::purge(&pool, mem.0.as_ref(), &root, self_id, &PurgeTarget::Private)
            .await
            .map_err(String::from)
    })
    .await
}

/// 拉所有分类（包括内置和用户自建），按 sort_order 升序。
#[tauri::command]
pub async fn list_categories(pool: State<'_, DbPool>) -> Result<Vec<Category>, String> {
    categories::list(&pool).await.map_err(Into::into)
}

/// 新建一个分类。`input.id` 应为前端生成的 short id（如 "work" / "play"）。
/// 命名冲突 / 内置 id 占用等校验在 repo 层做，错误透传给前端 toast。
#[tauri::command]
pub async fn create_category(
    pool: State<'_, DbPool>,
    input: CategoryInput,
) -> Result<Category, String> {
    categories::create(&pool, input).await.map_err(Into::into)
}

/// 更新分类的 name / icon / color / sort_order（按 patch 中非空字段更新）。
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

/// 删除一个分类。已分配到该分类的 app_groups 会被退到 'other'；
/// 内置分类拒绝删除（repo 层 `Error::InvalidInput` 抛出）。
#[tauri::command]
pub async fn delete_category(pool: State<'_, DbPool>, id: String) -> Result<(), String> {
    categories::delete(&pool, &id).await.map_err(Into::into)
}

/// 把分类列表按给定的 id 顺序重排（写 sort_order 字段）。前端拖拽改顺序后调一次。
#[tauri::command]
pub async fn reorder_categories(
    pool: State<'_, DbPool>,
    ordered_ids: Vec<String>,
) -> Result<(), String> {
    categories::reorder(&pool, ordered_ids)
        .await
        .map_err(Into::into)
}

/// 把单个 process_name 归类到某 category。
/// 实际写入 app_groups.category_id（process_name 通过 ensure_group 找到对应组）。
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

/// 把某 process_name 的分类清空（退回 'other'）。
#[tauri::command]
pub async fn unassign_app(pool: State<'_, DbPool>, process_name: String) -> Result<(), String> {
    categories::unassign_app(&pool, &process_name)
        .await
        .map_err(Into::into)
}

/// 列最近 N 天里出现过、但还没被分类（或归到 'other'）的应用。
/// 给「分类」页面的"待归类"卡片用，方便用户批量归类。
/// `days_back` 默认 7。
#[tauri::command]
pub async fn list_unclassified_apps(
    pool: State<'_, DbPool>,
    days_back: Option<u32>,
) -> Result<Vec<UnclassifiedApp>, String> {
    categories::list_unclassified(&pool, days_back.unwrap_or(7))
        .await
        .map_err(Into::into)
}

/// Translates the default categories and super-categories that still have their original
/// Chinese names into the UI language.
///
/// See [`migrations::translate_default_names`].
#[tauri::command]
pub async fn translate_default_category_names(
    pool: State<'_, DbPool>,
    categories: HashMap<String, String>,
    super_categories: HashMap<String, String>,
) -> Result<(), String> {
    migrations::translate_default_names(&pool, categories, super_categories)
        .await
        .map_err(Into::into)
}
