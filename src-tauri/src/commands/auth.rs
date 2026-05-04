use std::sync::Arc;
use tauri::{AppHandle, State};

use crate::storage::DbPool;
use crate::sync::auth::{self, AuthState};
use crate::sync::engine::SyncEngine;

#[tauri::command]
pub async fn auth_status(pool: State<'_, DbPool>) -> Result<AuthState, String> {
    auth::current_state(&pool).await.map_err(Into::into)
}

#[tauri::command]
pub async fn sign_in_with_google(
    pool: State<'_, DbPool>,
    engine: State<'_, Arc<SyncEngine>>,
) -> Result<AuthState, String> {
    let next = auth::sign_in_with_google(&pool).await.map_err(|e| e.to_string())?;
    // 登录成功 = 拿到新 token，旧的 "登录凭证失效" 错误立刻作废
    engine.clear_last_error().await;
    Ok(next)
}

#[tauri::command]
pub async fn sign_out(pool: State<'_, DbPool>) -> Result<(), String> {
    auth::sign_out(&pool).await.map_err(Into::into)
}

/// 重启 app —— 切账号后用：active_user.json 已指向新 uid，重启后 db_path() 自动切到新 DB。
#[tauri::command]
pub fn restart_app(app: AppHandle) {
    app.restart();
}
