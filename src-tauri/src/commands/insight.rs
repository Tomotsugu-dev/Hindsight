//! 云端截图洞察的前端命令面:状态轮询、回填估算/启停、视觉连通性测试。
//! 常驻启停不在这里——由 settings 命令按 `insight_enabled` 同步(同 OCR 常驻模式)。

use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::commands::screen_memory::MemoryState;
use crate::insight::{self, InsightWorker};
use crate::repo::settings;
use crate::storage::DbPool;

fn require_mem(mem: &MemoryState) -> Result<&crate::memory::MemoryDb, String> {
    mem.0.as_ref().ok_or_else(|| "记忆库不可用".to_string())
}

/// 洞察运行状态(设置页轮询)。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InsightStatus {
    /// 今日已分析帧数(按分析时刻计,回填也占额度)
    pub today_done: i64,
    /// 今日额度
    pub daily_cap: u32,
    /// 常驻视角的待处理帧数(水位线之后)
    pub pending: i64,
    /// 回填任务状态;None = 从未跑/已收尾
    pub backfill: Option<BackfillStatus>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackfillStatus {
    pub running: bool,
    pub done: usize,
    pub total: usize,
}

#[tauri::command]
pub async fn insight_status(
    pool: State<'_, DbPool>,
    mem: State<'_, MemoryState>,
    worker: State<'_, Arc<InsightWorker>>,
) -> Result<InsightStatus, String> {
    let mem = require_mem(&mem)?;
    let s = settings::load(&pool).await.map_err(String::from)?;
    let today_done = insight::today_done(mem).await.map_err(String::from)?;
    let pending = match &s.insight_since_ts {
        Some(w) => insight::pending_count(mem, w.clone(), true)
            .await
            .map_err(String::from)?,
        None => 0,
    };
    worker.reap_backfill().await;
    let p = &worker.backfill_progress;
    let total = p.total.load(Ordering::Relaxed);
    let backfill = (total > 0).then(|| BackfillStatus {
        running: p.running.load(Ordering::Relaxed),
        done: p.done.load(Ordering::Relaxed),
        total,
    });
    Ok(InsightStatus {
        today_done,
        daily_cap: s.insight_daily_frame_cap,
        pending,
        backfill,
    })
}

/// 回填估算:水位线之前还有多少存量帧待分析。前端据此显示确认框。
#[tauri::command]
pub async fn insight_backfill_estimate(
    pool: State<'_, DbPool>,
    mem: State<'_, MemoryState>,
) -> Result<i64, String> {
    let mem = require_mem(&mem)?;
    let s = settings::load(&pool).await.map_err(String::from)?;
    match s.insight_since_ts {
        Some(w) => insight::pending_count(mem, w, false)
            .await
            .map_err(String::from),
        None => Ok(0),
    }
}

/// 启动历史回填。false = 已有回填在跑。
#[tauri::command]
pub async fn insight_backfill_start(
    pool: State<'_, DbPool>,
    mem: State<'_, MemoryState>,
    worker: State<'_, Arc<InsightWorker>>,
) -> Result<bool, String> {
    let mem = require_mem(&mem)?.clone();
    worker
        .start_backfill((*pool).clone(), mem)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn insight_backfill_cancel(worker: State<'_, Arc<InsightWorker>>) -> Result<(), String> {
    worker.cancel_backfill().await;
    Ok(())
}

/// 视觉端点连通性测试:发本地合成色块图(零用户数据),返回模型回复原文。
#[tauri::command]
pub async fn test_ai_vision(
    endpoint: String,
    api_key: Option<String>,
    model: String,
) -> Result<String, String> {
    crate::insight::vlm::test_connection(&endpoint, api_key.as_deref().unwrap_or(""), &model)
        .await
        .map_err(String::from)
}
