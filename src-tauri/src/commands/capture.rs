use std::sync::Arc;
use tauri::State;

use crate::capture::{CaptureService, CaptureStatus};

/// 启动焦点采集后台循环。已在跑时是 no-op。
#[tauri::command]
pub async fn start_capture(svc: State<'_, Arc<CaptureService>>) -> Result<(), String> {
    svc.start().await;
    Ok(())
}

/// 停止焦点采集后台循环 + seal 当前会话写入 outbox。
#[tauri::command]
pub async fn stop_capture(svc: State<'_, Arc<CaptureService>>) -> Result<(), String> {
    svc.stop().await;
    Ok(())
}

/// 拉采集服务运行时状态：是否在跑、今日 activities 行数、最近一次采集时间、最近错误。
/// 前端在 dashboard 顶部"采集中 / 暂停"指示器读这条。
#[tauri::command]
pub async fn get_capture_status(
    svc: State<'_, Arc<CaptureService>>,
) -> Result<CaptureStatus, String> {
    Ok(svc.status().await)
}
