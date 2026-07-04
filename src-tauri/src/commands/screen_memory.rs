//! 屏幕记忆(L2/L3)命令:回填 + 手动触发消化。
//!
//! 定时触发与调试页观测后续接;当前两条命令足够让功能可用:
//! 首次启用点回填,之后由消化拉平积压。

use tauri::State;

use crate::memory::{digest, MemoryDb};
use crate::storage::DbPool;

/// 记忆库句柄的 managed state。None = 启动时打开失败(帧登记同样停用),
/// 命令层对 None 返回明确错误而不是 panic。
pub struct MemoryState(pub Option<MemoryDb>);

fn require(mem: &MemoryState) -> Result<&MemoryDb, String> {
    mem.0
        .as_ref()
        .ok_or_else(|| "屏幕记忆库不可用(启动时打开失败,详见日志)".to_string())
}

/// 历史回填:把主库已有截图的活动行登记为待消化帧。幂等,重复调用无副作用。
/// 返回登记(含已存在跳过)的行数。
#[tauri::command]
pub async fn memory_backfill(
    pool: State<'_, DbPool>,
    mem: State<'_, MemoryState>,
) -> Result<u64, String> {
    let db = require(&mem)?;
    digest::backfill_from_activities(&pool, db)
        .await
        .map_err(String::from)
}

/// 手动触发一次消化(OCR → 折叠 → FTS)。已在跑时返回错误。
/// 首次调用会自动下载 OCR 模型(约 21MB)。
#[tauri::command]
pub async fn memory_digest_now(
    mem: State<'_, MemoryState>,
) -> Result<digest::DigestReport, String> {
    let db = require(&mem)?;
    digest::run(db).await.map_err(String::from)
}
