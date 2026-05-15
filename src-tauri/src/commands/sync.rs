use std::sync::Arc;

use tauri::State;

use crate::storage::DbPool;
use crate::storage::SqliteResultExt;
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

/// 「强制完全重同步」的结果摘要，回前端用于 toast 提示。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForceResyncReport {
    /// 是否成功把 pull 游标重置到 epoch（=该行存在时 true；首装从未同步时为 false）
    pub reset_cursor: bool,
    /// 清掉的失败 outbox 行数（attempts > 0 或带 last_error）
    pub cleared_dead_letter: u64,
    /// 新追加的 (local_date) 入队条数；幂等：之前已入队的日期会被 NOT EXISTS 跳过
    pub enqueued_days: u64,
    /// 立刻触发的 push+pull 报错；None 表示这一轮成功
    pub sync_error: Option<String>,
}

/// 「强制完全重同步」：用户在「设备」页点红色按钮后调用。
///
/// 在一个事务里跑三步幂等 SQL：
///   1. 把 `sync_cursor.entity='drive_files'` 的 last_pulled_at 重置回 epoch
///      —— 下次 pull 重新枚举 Drive 上所有文件（按 `(device_id, remote_id)` 幂等 upsert + LWW，重复拉不会重复加）。
///   2. 把所有有过失败痕迹的 outbox 行重置（attempts=0、retry epoch、清 last_error）——
///      包括 dead-letter（>=10）和正在退避的（0<attempts<10）；下个 push tick 立刻可选。
///   3. 重新入队每个 (origin='local', local_date) 一行 outbox；NOT EXISTS 跳过已入队的天 ——
///      连点 N 次返回值 enqueuedDays 第二次起恒为 0。
///
/// 然后触发一次 `engine.sync_now()` 让 push+pull 立即跑，错误不抛上去（state 已经干净，
/// 后台 tick 也会兜底），仅放进 report.syncError 给 UI toast 展示。
///
/// 整体幂等：连点 N 次和点 1 次效果完全一致（除了 push/pull 各自的 idempotent retry）。
#[tauri::command]
pub async fn force_resync(
    pool: State<'_, DbPool>,
    engine: State<'_, Arc<SyncEngine>>,
) -> Result<ForceResyncReport, String> {
    let (reset_cursor, cleared_dead_letter, enqueued_days) = pool
        .0
        .call(|conn| {
            // 1. 重置 pull 游标 —— 行存在才视为「重置成功」
            let reset_n = conn
                .execute(
                    "UPDATE sync_cursor SET last_pulled_at = '1970-01-01T00:00:00Z'
                     WHERE entity = 'drive_files'",
                    [],
                )
                .db()?;

            // 2. 清掉所有失败 outbox 行（包括 dead-letter 和退避中）。
            //    WHERE 限定让二次执行时无命中行 → no-op，SQL 级幂等。
            let cleared = conn
                .execute(
                    "UPDATE sync_outbox
                     SET attempts = 0,
                         next_retry_at = '1970-01-01T00:00:00+00:00',
                         last_error = NULL
                     WHERE attempts > 0 OR last_error IS NOT NULL",
                    [],
                )
                .db()? as u64;

            // 3. 重新入队 (origin='local') 的每个 local_date 一行 outbox。
            //    NOT EXISTS 跳过已经有 activity-entity outbox 行的日期 → SQL 级幂等。
            let enqueued = conn
                .execute(
                    "INSERT INTO sync_outbox(
                         op, entity, entity_pk, payload, created_at, attempts, next_retry_at
                     )
                     SELECT 'upsert', 'activity', CAST(MIN(a.id) AS TEXT),
                            json_object('localDate', a.local_date),
                            '1970-01-01T00:00:00+00:00',
                            0,
                            '1970-01-01T00:00:00+00:00'
                     FROM activities a
                     WHERE a.origin = 'local'
                       AND NOT EXISTS (
                           SELECT 1 FROM sync_outbox so
                           WHERE so.entity = 'activity'
                             AND json_extract(so.payload, '$.localDate') = a.local_date
                       )
                     GROUP BY a.local_date",
                    [],
                )
                .db()? as u64;

            Ok((reset_n > 0, cleared, enqueued))
        })
        .await
        .map_err(|e| e.to_string())?;

    // SQL 状态已经干净；立刻 push+pull 一次让用户尽快看到效果。
    // 失败也不抛上去（state 已清，后台 30s tick 会兜底）—— 只塞进 report 给 UI 展示。
    let sync_error = match engine.sync_now().await {
        Ok(()) => None,
        Err(e) => Some(e.to_string()),
    };

    Ok(ForceResyncReport {
        reset_cursor,
        cleared_dead_letter,
        enqueued_days,
        sync_error,
    })
}
