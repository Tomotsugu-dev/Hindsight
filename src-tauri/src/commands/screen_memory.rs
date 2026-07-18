//! 屏幕记忆(L2/L3)命令:回填 + 手动触发消化 + 未入索引统计。
//!
//! 定时触发与调试页观测后续接。

use tauri::State;

use crate::memory::{digest, MemoryDb};
use crate::storage::{DbPool, SqliteResultExt};

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

/// 请求停止正在进行的手动消化批(banner 的停止按钮)。翻标志即返回,
/// 循环帧间感知、最多一帧(~1s)后停;`memory_digest_now` 随即正常 resolve
/// 已处理部分的账单。没有批在跑时调用也静默成功(幂等)。
/// 常驻批不受影响——它的停止走 设置 → 常驻 OCR 开关。
#[tauri::command]
pub fn memory_digest_stop() {
    digest::request_stop();
}

/// 未入索引统计:主库截图全集 vs 记忆库登记/完成情况的两库对账。
/// 近似值——文件可能已被保留策略删除(消化时会计入 skipped),可接受。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingStats {
    /// 主库有截图但 frames 未登记(需回填)
    pub unregistered: u64,
    /// frames 已登记但 OCR 未完成(待处理 + 可重试的失败)
    pub pending_ocr: u64,
    /// 两者之和,前端 banner 的 N
    pub total: u64,
    /// 消化(手动/常驻批)是否正在进行——前端据此在挂载时直接进入
    /// "后台索引中"态,而不是显示带按钮的初始态
    pub digest_running: bool,
}

/// 两库对账走 Rust 侧集合差,不用 ATTACH:主库是应用唯一写连接,
/// ATTACH 状态残留与异常路径的 DETACH 都是额外锁面;路径全集最坏
/// 十万级 ≈ 几 MB、毫秒级,简单无风险。
#[tauri::command]
pub async fn memory_pending_stats(
    pool: State<'_, DbPool>,
    mem: State<'_, MemoryState>,
) -> Result<PendingStats, String> {
    let db = require(&mem)?;

    // 主库截图全集(与 digest::backfill_from_activities 同一口径)
    let all_paths: Vec<String> = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT screenshot_path FROM activities
                     WHERE screenshot_path IS NOT NULL AND screenshot_path != ''",
                )
                .db()?;
            let out = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(out)
        })
        .await
        .map_err(|e| e.to_string())?;

    // 只算滞留超过 10 分钟的待处理帧:常驻模式 60s 一个 tick,队列里永远有
    // 一两张"刚拍完还没轮到"的在途帧,把它们当积压展示是误报。
    // cutoff 与 frames.ts 同为本地时区 RFC3339,字典序可比。
    let cutoff = (chrono::Local::now() - chrono::Duration::minutes(10)).to_rfc3339();
    let (registered, pending_ocr): (std::collections::HashSet<String>, u64) =
        db.0.call(move |conn| {
            let mut stmt = conn.prepare("SELECT path FROM frames").db()?;
            let paths = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .db()?
                .collect::<rusqlite::Result<std::collections::HashSet<_>>>()
                .db()?;
            let pending: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM frames
                     WHERE (ocr_state = 0 OR (ocr_state = 2 AND attempts < 3))
                       AND ts < ?1",
                    rusqlite::params![cutoff],
                    |r| r.get(0),
                )
                .db()?;
            Ok((paths, pending as u64))
        })
        .await
        .map_err(|e| e.to_string())?;

    let unregistered = all_paths
        .iter()
        .filter(|p| !registered.contains(*p))
        .count() as u64;
    Ok(PendingStats {
        unregistered,
        pending_ocr,
        total: unregistered + pending_ocr,
        digest_running: digest::is_running(),
    })
}
