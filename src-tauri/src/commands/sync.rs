use std::sync::Arc;

use tauri::State;

use crate::sync::engine::{SyncEngine, SyncStatus};

/// 拉同步引擎当前状态（最近一次成功 / 失败时间、下次计划、是否在 push/pull 中）。
/// 前端「设备」页面用来展示同步指示器。
#[tauri::command]
pub async fn sync_status(engine: State<'_, Arc<SyncEngine>>) -> Result<SyncStatus, String> {
    Ok(engine.status().await)
}

/// 立刻触发一次 push + pull（用户在「设备」页点"立刻同步"）。
/// 未登录时为 no-op；正常引擎背景循环也会推，本命令只是给用户一个手动钩子。
#[tauri::command]
pub async fn sync_now(engine: State<'_, Arc<SyncEngine>>) -> Result<(), String> {
    engine.sync_now().await.map_err(Into::into)
}
