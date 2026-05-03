use tauri::State;

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
