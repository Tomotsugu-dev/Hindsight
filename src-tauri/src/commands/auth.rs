use tauri::{AppHandle, State};

use crate::storage::DbPool;
use crate::sync::auth::{self, AuthState};

#[tauri::command]
pub async fn auth_status(pool: State<'_, DbPool>) -> Result<AuthState, String> {
    auth::current_state(&pool).await.map_err(Into::into)
}

#[tauri::command]
pub async fn sign_in_with_google(pool: State<'_, DbPool>) -> Result<AuthState, String> {
    auth::sign_in_with_google(&pool).await.map_err(Into::into)
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
