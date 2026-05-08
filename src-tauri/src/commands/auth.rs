use std::sync::Arc;
use tauri::{AppHandle, State};

use crate::storage::DbPool;
use crate::sync::auth::{self, AuthState};
use crate::sync::engine::SyncEngine;

/// 拉当前 OAuth 登录状态（已登录 uid + email + 最后刷新时间，或 NotSignedIn）。
#[tauri::command]
pub async fn auth_status(pool: State<'_, DbPool>) -> Result<AuthState, String> {
    auth::current_state(&pool).await.map_err(Into::into)
}

/// 触发 Google OAuth 登录（弹本地 listener + 浏览器同意页）。
/// 成功后清掉同步引擎里残留的"凭证失效"错误，让 status 立刻刷新成正常。
#[tauri::command]
pub async fn sign_in_with_google(
    pool: State<'_, DbPool>,
    engine: State<'_, Arc<SyncEngine>>,
) -> Result<AuthState, String> {
    let next = auth::sign_in_with_google(&pool)
        .await
        .map_err(String::from)?;
    // 登录成功 = 拿到新 token，旧的 "登录凭证失效" 错误立刻作废
    engine.clear_last_error().await;
    Ok(next)
}

/// 退出登录：删 DB 里的 auth_state + 清 keyring 里的 refresh_token。
/// 多账号场景：同时清掉 active_user.json 让下次启动不再绑定该 uid。
#[tauri::command]
pub async fn sign_out(pool: State<'_, DbPool>) -> Result<(), String> {
    auth::sign_out(&pool).await.map_err(Into::into)
}

/// 重启 app —— 切账号后用：active_user.json 已指向新 uid，重启后 db_path() 自动切到新 DB。
#[tauri::command]
pub fn restart_app(app: AppHandle) {
    app.restart();
}
