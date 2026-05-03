use std::sync::Arc;
use tauri::State;

use crate::capture::{CaptureService, CaptureStatus};

#[tauri::command]
pub async fn start_capture(svc: State<'_, Arc<CaptureService>>) -> Result<(), String> {
    svc.start().await;
    Ok(())
}

#[tauri::command]
pub async fn stop_capture(svc: State<'_, Arc<CaptureService>>) -> Result<(), String> {
    svc.stop().await;
    Ok(())
}

#[tauri::command]
pub async fn get_capture_status(
    svc: State<'_, Arc<CaptureService>>,
) -> Result<CaptureStatus, String> {
    Ok(svc.status().await)
}
