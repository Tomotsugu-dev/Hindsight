use std::sync::Arc;

use tauri::State;

use crate::sync::engine::{SyncEngine, SyncStatus};

#[tauri::command]
pub async fn sync_status(engine: State<'_, Arc<SyncEngine>>) -> Result<SyncStatus, String> {
    Ok(engine.status().await)
}

#[tauri::command]
pub async fn sync_now(engine: State<'_, Arc<SyncEngine>>) -> Result<(), String> {
    engine.sync_now().await.map_err(Into::into)
}
