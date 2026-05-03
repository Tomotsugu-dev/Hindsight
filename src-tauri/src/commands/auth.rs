use tauri::State;

use crate::repo::settings;
use crate::storage::DbPool;
use crate::sync::auth::{self, AuthState};

#[tauri::command]
pub async fn auth_status(pool: State<'_, DbPool>) -> Result<AuthState, String> {
    let s = settings::load(&pool).await.map_err(|e| e.to_string())?;
    auth::current_state(&pool, &s).await.map_err(Into::into)
}

#[tauri::command]
pub async fn sign_in_with_google(pool: State<'_, DbPool>) -> Result<AuthState, String> {
    let s = settings::load(&pool).await.map_err(|e| e.to_string())?;
    auth::sign_in_with_google(&pool, &s)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn sign_out(pool: State<'_, DbPool>) -> Result<(), String> {
    auth::sign_out(&pool).await.map_err(Into::into)
}
