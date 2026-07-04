//! 消化 worker:把登记在案的帧走完 L2(OCR)→ L3(折叠)管线。
//!
//! 生存纪律(screen-memory.md §6):单实例互斥;单帧失败标记重试(上限 3 次)后
//! 跳过,绝不让整批消化卡死在一帧;重跑幂等(已消化帧不重复处理)。
//!
//! 当前形态:进程内任务(由命令/定时触发)。独立子进程化(`--digest-worker`)时
//! 把 [`RUNNING`] 换成文件锁即可,消化逻辑本身不变。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::clusters::{self, ClusterBook};
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

/// L4 视觉编码器模型(MobileCLIP2-S2,ONNX 外部权重两件套,须同目录)。
const VISUAL_SOURCES: [(&str, &str); 2] = [
    (
        "visual.onnx",
        "https://huggingface.co/RuteNL/MobileCLIP2-S2-OpenCLIP-ONNX/resolve/main/visual.onnx",
    ),
    (
        "visual.onnx.data",
        "https://huggingface.co/RuteNL/MobileCLIP2-S2-OpenCLIP-ONNX/resolve/main/visual.onnx.data",
    ),
];

/// OCR 模型三件套:缺哪个下哪个。幂等。
pub async fn ensure_models() -> Result<()> {
    download_missing(&ocr::model_dir(), &MODEL_SOURCES).await
}

/// L4 视觉模型两件套(~140MB)。幂等;只在首个视觉帧出现时被调用。
pub async fn ensure_visual_models() -> Result<()> {
    download_missing(&crate::ai::visual::model_dir(), &VISUAL_SOURCES).await
}

async fn download_missing(dir: &std::path::Path, sources: &[(&str, &str)]) -> Result<()> {
    tokio::fs::create_dir_all(dir).await.map_err(Error::Io)?;
    for (name, url) in sources {
        let dest = dir.join(name);
        if tokio::fs::try_exists(&dest).await.map_err(Error::Io)? {
            continue;
        }
        log::info!("下载模型 {name} ...");
        let bytes = reqwest::get(*url)
            .await?
            .error_for_status()
            .map_err(|e| Error::Ocr(format!("下载 {name} 失败: {e}")))?
            .bytes()
            .await?;
        if *name == "dict.txt" {
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
        log::info!("模型 {name} 就绪 ({} bytes)", bytes.len());
    }
    Ok(())
}

/// 视觉主导帧判据:OCR 有效字符总量低于此值 → 该帧由文字解释不了,
/// 进 L4 视觉支路(嵌入 + 聚簇)。**待真实视觉日标定**(与 L1 的 f 同法)。
const VISUAL_CHAR_MAX: usize = 60;

/// 消化管线的运行态:OCR 引擎 + 懒加载的视觉引擎 + 折叠器 + 当日簇册。
/// 批量模式一次 run 一个;常驻模式跨 tick 持有(会话与簇册连续)。
pub struct Pipeline {
    ocr: Arc<OcrEngine>,
    /// L4 视觉编码器:开发日几乎不触发,首个视觉帧出现才加载(~140MB 下载 + ~250MB 内存)
    visual: Option<Arc<crate::ai::visual::VisualEngine>>,
    folder: Folder,
    clusters: Option<ClusterBook>,
}

impl Pipeline {
    /// 加载 OCR 引擎(模型缺失自动下载;设 ORT_DYLIB_PATH 定位 onnxruntime)。
    pub async fn new() -> Result<Self> {
        ensure_models().await?;
        // ort 的 load-dynamic 靠 ORT_DYLIB_PATH 定位 onnxruntime(与嵌入模型同一份)
        if let Ok(p) = crate::ai::embedding_runtime::dylib_path() {
            if p.exists() {
                std::env::set_var("ORT_DYLIB_PATH", &p);
            }
        }
        let ocr = Arc::new(
            tokio::task::spawn_blocking(OcrEngine::load)
                .await
                .map_err(|e| Error::Ocr(format!("spawn_blocking: {e}")))??,
        );
        Ok(Self {
            ocr,
            visual: None,
            folder: Folder::default(),
            clusters: None,
        })
    }
}

/// 消化积压(批量模式):加载引擎 → 清空登记簿 → 引擎随返回释放。
///
/// 已在跑时直接返回错误(单实例);任何单帧错误只降级(标失败重试),
/// 只有引擎级错误(模型加载失败等)才中断整批。
pub async fn run(mem: &MemoryDb) -> Result<DigestReport> {
    let mut pipe = Pipeline::new().await?;
    let never_stop = AtomicBool::new(false);
    drain(mem, &mut pipe, &never_stop).await
}

/// 消化核心:取待处理帧 → OCR → L3 折叠 → (视觉帧)L4 聚簇 → 记账,
/// 直到登记簿清空或 `stop` 置位。批量与常驻共用——差别只在 [`Pipeline`]
/// 的生命周期归谁管。`stop` 在帧间检查:停止请求最多等一帧(~1s)即生效,
/// 且不会留下半消化状态。
pub async fn drain(mem: &MemoryDb, pipe: &mut Pipeline, stop: &AtomicBool) -> Result<DigestReport> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Err(Error::InvalidInput("消化任务已在运行"));
    }
    let result = drain_inner(mem, pipe, stop).await;
    RUNNING.store(false, Ordering::SeqCst);
    result
}

async fn drain_inner(
    mem: &MemoryDb,
    pipe: &mut Pipeline,
    stop: &AtomicBool,
) -> Result<DigestReport> {
    let mut report = DigestReport::default();
    let started = std::time::Instant::now();

    'outer: loop {
        let batch = frames::take_pending(mem, BATCH).await?;
        if batch.is_empty() {
            break;
        }
        for frame in batch {
            if stop.load(Ordering::Relaxed) {
                break 'outer;
            }
            match digest_one(mem, pipe, &frame).await {
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
    if report.processed + report.failed + report.skipped_missing_file > 0 {
        log::info!(
            "消化完成: 处理 {} 失败 {} 缺图 {} 用时 {:?}",
            report.processed,
            report.failed,
            report.skipped_missing_file,
            started.elapsed()
        );
    }
    Ok(report)
}

/// 单帧管线:读图 → OCR → 折叠 → 标完成 → 视觉主导帧走 L4(嵌入+聚簇)。
/// Ok(false) = 图文件缺失(跳过)。L4 是尽力而为的富集:失败只告警,
/// 不影响帧的完成态(文字部分已入库)。
async fn digest_one(mem: &MemoryDb, pipe: &mut Pipeline, frame: &PendingFrame) -> Result<bool> {
    let path = std::path::PathBuf::from(&frame.path);
    if !path.is_file() {
        frames::mark_done(mem, frame.path.clone(), -1).await?;
        return Ok(false);
    }
    let eng = Arc::clone(&pipe.ocr);
    let p = path.clone();
    let lines: Vec<String> = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
        let img = image::open(&p).map_err(|e| Error::Ocr(format!("读图失败: {e}")))?;
        Ok(eng.recognize(&img)?.into_iter().map(|l| l.text).collect())
    })
    .await
    .map_err(|e| Error::Ocr(format!("spawn_blocking: {e}")))??;

    let session_id = pipe.folder.fold_frame(mem, frame, &lines).await?;
    frames::mark_done(mem, frame.path.clone(), session_id).await?;

    // L4 视觉支路:文字解释不了的帧(字符量低于阈值)才嵌入+聚簇
    let char_count: usize = lines
        .iter()
        .map(|l| l.chars().filter(|c| !c.is_whitespace()).count())
        .sum();
    if char_count < VISUAL_CHAR_MAX {
        if let Err(e) = visual_branch(mem, pipe, frame, &path).await {
            log::warn!("视觉支路失败 ({}),文字部分不受影响: {e}", frame.path);
        }
    }
    Ok(true)
}

/// L4:懒加载视觉引擎 → 嵌入 → 当日簇册归属 → 留痕。
async fn visual_branch(
    mem: &MemoryDb,
    pipe: &mut Pipeline,
    frame: &PendingFrame,
    path: &std::path::Path,
) -> Result<()> {
    if pipe.visual.is_none() {
        ensure_visual_models().await?;
        pipe.visual = Some(Arc::new(
            tokio::task::spawn_blocking(crate::ai::visual::VisualEngine::load)
                .await
                .map_err(|e| Error::Ocr(format!("spawn_blocking: {e}")))??,
        ));
    }
    let engine = Arc::clone(pipe.visual.as_ref().expect("上面刚保证过已加载"));
    let p = path.to_path_buf();
    let embedding: Vec<f32> = tokio::task::spawn_blocking(move || -> Result<Vec<f32>> {
        let img = image::open(&p).map_err(|e| Error::Ocr(format!("读图失败: {e}")))?;
        engine.embed(&img)
    })
    .await
    .map_err(|e| Error::Ocr(format!("spawn_blocking: {e}")))??;

    // 簇册按日:跨日的第一个视觉帧触发换册
    if pipe.clusters.as_ref().map(|b| b.date()) != Some(frame.local_date.as_str()) {
        pipe.clusters = Some(ClusterBook::load(mem, &frame.local_date).await?);
    }
    let book = pipe.clusters.as_mut().expect("上面刚保证过已加载");
    let title = crate::memory::sessions::normalize_title(frame.title.as_deref().unwrap_or(""));
    let cluster_id = book
        .assign(mem, &frame.path, &title, embedding.clone())
        .await?;
    clusters::record_frame(mem, &frame.path, cluster_id, &embedding).await?;
    Ok(())
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
