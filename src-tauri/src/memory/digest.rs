//! 消化 worker:把登记在案的帧走完 L2(OCR)→ L3(折叠)管线。
//!
//! 生存纪律(screen-memory.md §6):单实例互斥;单帧失败标记重试(上限 3 次)后
//! 跳过,绝不让整批消化卡死在一帧;重跑幂等(已消化帧不重复处理)。
//!
//! 当前形态:进程内任务(由命令/定时触发)。独立子进程化(`--digest-worker`)时
//! 把 [`RUNNING`] 换成文件锁即可,消化逻辑本身不变。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::frames::{self, PendingFrame};
use super::sessions::Folder;
use super::MemoryDb;
use crate::ai::ocr::{self, OcrEngine};
use crate::error::{Error, Result};
use crate::storage::{DbPool, SqliteResultExt};

/// 进程内单实例互斥;子进程化时换文件锁。
static RUNNING: AtomicBool = AtomicBool::new(false);

/// 每轮从登记簿取的帧数;取完一轮再取,直到无积压。
const BATCH: i64 = 64;

/// OCR 模型三件套的下载源(官方 ONNX 发布 + PaddleOCR 官方字典)。
/// 字典条目数与 rec 模型类数强耦合,下载后按 [`DICT_EXPECTED_LINES`] 校验,
/// 上游改版导致不匹配时明确报错而不是解码出乱码。
const MODEL_SOURCES: [(&str, &str); 3] = [
    (
        "det.onnx",
        "https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_det_onnx/resolve/main/inference.onnx",
    ),
    (
        "rec.onnx",
        "https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_rec_onnx/resolve/main/inference.onnx",
    ),
    (
        "dict.txt",
        "https://raw.githubusercontent.com/PaddlePaddle/PaddleOCR/main/ppocr/utils/dict/ppocrv5_dict.txt",
    ),
];
const DICT_EXPECTED_LINES: usize = 18383;

/// 一次消化的结果账单(日志/调试页展示)。
#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DigestReport {
    pub processed: u64,
    pub failed: u64,
    pub skipped_missing_file: u64,
}

/// OCR 模型三件套:缺哪个下哪个。幂等。
pub async fn ensure_models() -> Result<()> {
    let dir = ocr::model_dir();
    tokio::fs::create_dir_all(&dir).await.map_err(Error::Io)?;
    for (name, url) in MODEL_SOURCES {
        let dest = dir.join(name);
        if tokio::fs::try_exists(&dest).await.map_err(Error::Io)? {
            continue;
        }
        log::info!("下载 OCR 模型 {name} ...");
        let bytes = reqwest::get(url)
            .await?
            .error_for_status()
            .map_err(|e| Error::Ocr(format!("下载 {name} 失败: {e}")))?
            .bytes()
            .await?;
        if name == "dict.txt" {
            let lines = std::str::from_utf8(&bytes)
                .map_err(|e| Error::Ocr(format!("字典不是 UTF-8: {e}")))?
                .lines()
                .count();
            if lines != DICT_EXPECTED_LINES {
                return Err(Error::Ocr(format!(
                    "字典条目数 {lines} ≠ 预期 {DICT_EXPECTED_LINES},上游可能改版,拒绝使用"
                )));
            }
        }
        let temp = dir.join(format!("{name}.downloading"));
        tokio::fs::write(&temp, &bytes).await.map_err(Error::Io)?;
        tokio::fs::rename(&temp, &dest).await.map_err(Error::Io)?;
        log::info!("OCR 模型 {name} 就绪 ({} bytes)", bytes.len());
    }
    Ok(())
}

/// 消化积压:取待处理帧 → OCR → L3 折叠 → 记账,直到登记簿清空。
///
/// 已在跑时直接返回错误(单实例);任何单帧错误只降级(标失败重试),
/// 只有引擎级错误(模型加载失败等)才中断整批。
pub async fn run(mem: &MemoryDb) -> Result<DigestReport> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err(Error::InvalidInput("消化任务已在运行"));
    }
    let result = run_inner(mem).await;
    RUNNING.store(false, Ordering::SeqCst);
    result
}

async fn run_inner(mem: &MemoryDb) -> Result<DigestReport> {
    ensure_models().await?;
    // ort 的 load-dynamic 靠 ORT_DYLIB_PATH 定位 onnxruntime(与嵌入模型同一份)
    if let Ok(p) = crate::ai::embedding_runtime::dylib_path() {
        if p.exists() {
            std::env::set_var("ORT_DYLIB_PATH", &p);
        }
    }
    let engine = Arc::new(
        tokio::task::spawn_blocking(OcrEngine::load)
            .await
            .map_err(|e| Error::Ocr(format!("spawn_blocking: {e}")))??,
    );

    let mut report = DigestReport::default();
    let mut folder = Folder::default();
    let started = std::time::Instant::now();

    loop {
        let batch = frames::take_pending(mem, BATCH).await?;
        if batch.is_empty() {
            break;
        }
        for frame in batch {
            match digest_one(mem, &engine, &mut folder, &frame).await {
                Ok(true) => report.processed += 1,
                Ok(false) => {
                    // 图文件已不在(retention 删除/用户清理):按完成记,别无限重试
                    report.skipped_missing_file += 1;
                }
                Err(e) => {
                    log::warn!("帧消化失败 ({}): {e}", frame.path);
                    frames::mark_failed(mem, frame.path.clone()).await?;
                    report.failed += 1;
                }
            }
        }
    }
    log::info!(
        "消化完成: 处理 {} 失败 {} 缺图 {} 用时 {:?}",
        report.processed,
        report.failed,
        report.skipped_missing_file,
        started.elapsed()
    );
    Ok(report)
}

/// 单帧管线:读图 → OCR → 折叠 → 标完成。Ok(false) = 图文件缺失(跳过)。
async fn digest_one(
    mem: &MemoryDb,
    engine: &Arc<OcrEngine>,
    folder: &mut Folder,
    frame: &PendingFrame,
) -> Result<bool> {
    let path = std::path::PathBuf::from(&frame.path);
    if !path.is_file() {
        frames::mark_done(mem, frame.path.clone(), -1).await?;
        return Ok(false);
    }
    let eng = Arc::clone(engine);
    let lines: Vec<String> = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
        let img = image::open(&path).map_err(|e| Error::Ocr(format!("读图失败: {e}")))?;
        Ok(eng.recognize(&img)?.into_iter().map(|l| l.text).collect())
    })
    .await
    .map_err(|e| Error::Ocr(format!("spawn_blocking: {e}")))??;

    let session_id = folder.fold_frame(mem, frame, &lines).await?;
    frames::mark_done(mem, frame.path.clone(), session_id).await?;
    Ok(true)
}

/// 历史回填:把主库 activities 里已有截图的活动行派生成帧登记(一次性,幂等)。
/// 只回填 retention 窗口内仍存在的档案;调用方决定何时触发(首次启用/设置页按钮)。
pub async fn backfill_from_activities(pool: &DbPool, mem: &MemoryDb) -> Result<u64> {
    let rows: Vec<(String, String, String, String, String)> = pool
        .0
        .call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT screenshot_path, MIN(started_at), MIN(local_date),
                            process_name, window_title
                     FROM activities
                     WHERE screenshot_path IS NOT NULL AND screenshot_path != ''
                     GROUP BY screenshot_path",
                )
                .db()?;
            let out = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(out)
        })
        .await?;

    let mut n = 0u64;
    for (path, started_at, local_date, app, title) in rows {
        frames::register(mem, path, started_at, local_date, Some(app), Some(title)).await?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 端到端:真实主库回填 → 真模型消化 → FTS 检索。
    /// 跑法(release,debug 下 OCR 太慢):
    ///   `E2E_DATE=2026-07-05 E2E_QUERY=屏幕记忆 cargo test --release --lib digest::tests::e2e -- --ignored --nocapture`
    /// 写入的是 scratch 记忆库(系统临时目录),不碰真实 memory.sqlite。
    #[tokio::test]
    #[ignore]
    async fn e2e_real_archive_to_fts() {
        let date = std::env::var("E2E_DATE").expect("设 E2E_DATE=YYYY-MM-DD");
        let query = std::env::var("E2E_QUERY").expect("设 E2E_QUERY=要搜的词");

        // scratch 记忆库
        let tmp = std::env::temp_dir().join(format!("hindsight-e2e-{date}.sqlite"));
        let _ = std::fs::remove_file(&tmp);
        let mem = MemoryDb::open_at(&tmp).await.unwrap();

        // 真实主库(只读用途;WAL 下与运行中的 app 并存)
        let main = crate::storage::db_path().unwrap();
        let pool = DbPool::open(&main).await.unwrap();

        let n = backfill_from_activities(&pool, &mem).await.unwrap();
        // 只消化指定日期,控制时长
        mem.0
            .call({
                let date = date.clone();
                move |conn| {
                    conn.execute("DELETE FROM frames WHERE local_date != ?1", [date])
                        .db()?;
                    Ok(())
                }
            })
            .await
            .unwrap();
        println!("回填 {n} 帧,保留 {date} 的部分");

        let report = run(&mem).await.unwrap();
        println!("消化账单: {report:?}");

        let (sessions, lines, hits): (i64, i64, i64) = mem
            .0
            .call(move |conn| {
                let s = conn
                    .query_row("SELECT COUNT(*) FROM text_sessions", [], |r| r.get(0))
                    .db()?;
                let l = conn
                    .query_row("SELECT COUNT(*) FROM session_lines", [], |r| r.get(0))
                    .db()?;
                let h = conn
                    .query_row(
                        "SELECT COUNT(*) FROM text_sessions_fts WHERE text_sessions_fts MATCH ?1",
                        [query],
                        |r| r.get(0),
                    )
                    .db()?;
                Ok((s, l, h))
            })
            .await
            .unwrap();
        println!("会话 {sessions} | 唯一行 {lines} | 命中会话 {hits}");
        assert!(report.processed > 0, "至少消化了一帧");
        assert!(sessions > 0 && lines > 0);
        assert!(hits > 0, "今天屏幕上出现过的词应能搜到");
    }
}
