//! 消化 worker:把登记在案的帧走完 L2(OCR)→ L3(折叠)管线。
//!
//! 生存纪律(screen-memory.md §6):单实例互斥;单帧失败标记重试(上限 3 次)后
//! 跳过,绝不让整批消化卡死在一帧;重跑幂等(已消化帧不重复处理)。
//!
//! 当前形态:进程内任务(由命令/定时触发)。独立子进程化(`--digest-worker`)时
//! 把 [`RUNNING`] 换成文件锁即可,消化逻辑本身不变。

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use super::frames::{self, PendingFrame};
use super::sessions::Folder;
use super::MemoryDb;
use crate::ai::ocr::{self, OcrEngine};
use crate::error::{Error, Result};
use crate::storage::{DbPool, SqliteResultExt};

/// 进程内单实例互斥;子进程化时换文件锁。
static RUNNING: AtomicBool = AtomicBool::new(false);

/// 「当前这一批」的停止请求(`memory_digest_stop` 命令置位)。手动批与常驻批的
/// 当前 drain 都感知它——banner 的停止按钮因此在"后台索引中"态也有效。
/// [`drain`] 结束时清零(批结束/被停后请求即失效):批开始**前**置位的请求
/// 依然有效,覆盖"引擎还在加载、第一帧还没跑"的窗口。
/// 注意语义边界:常驻模式下停的只是当前批,下个周期 tick 仍会继续消化积压;
/// 彻底停常驻走 设置 → 常驻 OCR 开关([`super::resident::ResidentOcr`])。
static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

/// 当前批最近一次「有帧走完」的时刻,单调毫秒(相对进程启动);0 = 本批尚无进度。
/// 存在的理由:`RUNNING` 只是个 bool,批卡死和批正忙在外部看来完全一样。
/// 实测事故里 Apple Vision 死锁后,这个子系统对外表现就是"永远在运行"。
static LAST_PROGRESS_MS: AtomicU64 = AtomicU64::new(0);

/// 单调时钟基准。用 `Instant` 而非墙钟:后者会被系统对时 / 夏令时跳变,
/// 卡死判定绝不能因为调了下时间就误判。
fn mono_base() -> &'static std::time::Instant {
    static BASE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    BASE.get_or_init(std::time::Instant::now)
}

fn mono_now_ms() -> u64 {
    mono_base().elapsed().as_millis() as u64
}

/// 记一次进度。每帧走完(成功/失败/缺图都算)调用一次——只要循环还在推进,
/// 就不该被判卡死。
fn note_progress() {
    LAST_PROGRESS_MS.store(mono_now_ms(), Ordering::Relaxed);
}

/// 当前批已经多久没有推进(毫秒)。`None` = 没有批在跑。
///
/// 判据是**距上一帧完成**的时间,不是批的总时长:三千张积压跑五十分钟是正常的,
/// 按批时长判死会把正常长批误杀。
// 消费方(看门狗判死 + PendingStats 上报)在后续步骤接入;此处先把口径立住,
// 单测已覆盖其语义。
#[allow(dead_code)]
pub fn stalled_ms() -> Option<u64> {
    if !is_running() {
        return None;
    }
    let last = LAST_PROGRESS_MS.load(Ordering::Relaxed);
    Some(mono_now_ms().saturating_sub(last))
}

/// 消化是否正在进行(手动或常驻批任一)。前端 banner/设置页用它在
/// 重新挂载时恢复"后台索引中"的显示,而不是装作无事发生。
pub fn is_running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

/// 批次互斥的 RAII 守卫。
///
/// **为什么必须是 Drop 而不是函数末尾赋值**:旧写法把清零写在 `drain_inner().await`
/// 之后,一旦 `drain_inner` panic 或它的 future 被丢弃(任务取消 / 运行时关停),
/// 清零永远不会执行,`RUNNING` 就永久卡在 true。后果不是"少清一个标志"——
/// 常驻 tick、定时补识别、总结前补识别全部被 `is_running()` 挡在门外,
/// 而 `ocr_catchup` 会每秒空转一次直到天荒地老,日报再也生成不出来。
/// 只有重启才能恢复。
struct BatchGuard;

impl BatchGuard {
    /// 抢占批次;已有批在跑时返回 `None`。
    ///
    /// 被拒绝时**不触碰任何标志**——既有测试
    /// `second_drain_rejected_while_first_running` 断言了这一点:
    /// 被拒的调用若顺手清零,就会把正在跑的那一批的互斥给解开。
    fn acquire() -> Option<Self> {
        if RUNNING.swap(true, Ordering::SeqCst) {
            return None;
        }
        // 批一开始就记一次进度:否则引擎加载那几秒会被算成"无进展"
        note_progress();
        Some(BatchGuard)
    }
}

impl Drop for BatchGuard {
    fn drop(&mut self) {
        // 顺序与旧写法一致:先清停止请求,再放互斥
        STOP_REQUESTED.store(false, Ordering::SeqCst);
        RUNNING.store(false, Ordering::SeqCst);
    }
}

/// 请求停止当前正在进行的消化批(手动批或常驻批的当前轮)。翻标志即返回;
/// 消化循环帧间感知,最多再等一帧(~1s)停下,不留半消化状态。
/// 没有批在跑时置位近似无害——最坏让紧接着开始的下一批空停一轮。
pub fn request_stop() {
    STOP_REQUESTED.store(true, Ordering::SeqCst);
}

/// 每轮从登记簿取的帧数;取完一轮再取,直到无积压。
const BATCH: i64 = 64;

/// 连续多少帧遇设施故障就中止本批。取 3:够区分"零星抖动"与"设施整体不可用",
/// 又能把最坏浪费压到三次超时,而不是拿整批去磨一个已经坏掉的识别引擎。
const INFRA_ABORT_STREAK: u32 = 3;

/// 单帧识别的外层超时(纵深防线)。主防线在 `ai/ocr_supervisor`:90s 无响应
/// 即杀 worker 重建。这里取 120s,恒晚于主防线——只有 supervisor 自身出 bug
/// 卡住时才轮到它,保证消化循环在任何情况下都不可能被一帧钉死。
/// 测试压到 500ms:假识别函数要么立即返回要么故意永久阻塞,无中间态。
const FRAME_TIMEOUT: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_millis(500)
} else {
    std::time::Duration::from_secs(120)
};

/// 连败中止后的冷却时长。没有冷却的话,常驻 tick 每 60s 就会重启一批、
/// 每批再白等 3 × 90s 超时并多泄漏三根阻塞线程——冷却把"引擎持续挂起"
/// 场景的损耗压到每 10 分钟一轮。
const INFRA_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(600);

/// 冷却截止时刻(单调毫秒;0 = 无冷却)。进程级 static:与 RUNNING 同理,
/// 三个自动入口(常驻/定时/总结前)都够不到彼此,只能在这儿会合。
static COOLDOWN_UNTIL_MS: AtomicU64 = AtomicU64::new(0);

/// 冷却剩余秒数;`None` = 未在冷却。定时补识别用它避免把"当天唯一一次机会"
/// 烧在一个注定失败的批上。
pub fn cooldown_remaining_secs() -> Option<u64> {
    let until = COOLDOWN_UNTIL_MS.load(Ordering::Relaxed);
    let now = mono_now_ms();
    (until > now).then(|| (until - now).div_ceil(1000))
}

/// 清除冷却。手动「立即回填」调用:用户主动点了就是明确要求再试一次,
/// 冷却的目的是防自动入口空转,不该拦着人(与被回滚的 auto-ocr 分支同一裁定:
/// 主动要求即撤回异议)。
pub fn clear_cooldown() {
    COOLDOWN_UNTIL_MS.store(0, Ordering::Relaxed);
}

fn enter_cooldown() {
    COOLDOWN_UNTIL_MS.store(
        mono_now_ms() + INFRA_COOLDOWN.as_millis() as u64,
        Ordering::Relaxed,
    );
}

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
    /// 因基础设施故障(识别超时/崩溃、引擎缺失、DB 抖动)被跳过的帧数。
    /// 与 `failed` 分开计:这些帧**没有**消耗重试预算,下次还会再试。
    pub infra_skipped: u64,
}

/// 帧故障 vs 设施故障。
///
/// 判据是"这错误能不能怪这张图":能怪它的才扣该帧的重试预算,怪不到它头上的
/// (识别进程炸了、引擎没装、数据库抖了)一律不扣——否则设施故障期间经手的每一帧
/// 都会被连坐烧穿 `MAX_ATTEMPTS`,而它们本身完全正常。2026-07-17 丢掉的 243 帧
/// 就是这么没的。
fn is_infra_failure(e: &Error) -> bool {
    matches!(
        e,
        Error::OcrInfra(_)
            | Error::EmbeddingRuntimeMissing
            | Error::Io(_)
            | Error::Db(_)
            | Error::Sqlite(_)
    )
}

/// OCR 模型三件套:缺哪个下哪个。幂等。
/// Vision 后端(macOS 默认)不需要 Paddle 模型,直接跳过——
/// 顺带免掉 onnxruntime 安装引导,macOS 用户零下载可用。
pub async fn ensure_models() -> Result<()> {
    if !OcrEngine::needs_models() {
        return Ok(());
    }
    download_missing(&ocr::model_dir(), &MODEL_SOURCES).await
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

/// 可替换的单帧识别函数:图片路径 → 行文本(阅读序),**异步**。
///
/// 生产实现是对 OCR worker 子进程的一次 IPC(`ai/ocr_supervisor`);测试注入
/// 预设文本。异步不是风格问题:IPC 若包在 `spawn_blocking` 里同步等,就是在
/// 阻塞线程上 block_on——和引擎挂死把线程钉死是同一个形状,超时和取消都会失效。
/// 选闭包而非 trait:digest 只消费"路径→行文本"这一个面,缝开在唯一消费点上。
type RecognizeFut =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<String>>> + Send>>;
type Recognizer = Arc<dyn Fn(std::path::PathBuf) -> RecognizeFut + Send + Sync>;

/// 测试夹具用的同步识别函数(见 [`Pipeline::with_recognizer`])。
#[cfg(test)]
type SyncRecognizer = Arc<dyn Fn(&std::path::Path) -> Result<Vec<String>> + Send + Sync>;

/// 消化管线的运行态:OCR 识别函数 + 折叠器。
/// 批量模式一次 run 一个;常驻模式跨 tick 持有(会话连续)。
pub struct Pipeline {
    recognize: Recognizer,
    folder: Folder,
}

impl Pipeline {
    /// 后台/常驻模式:worker 用保守 OCR 线程数,不打扰前台。
    pub async fn new() -> Result<Self> {
        Self::load(false).await
    }

    /// 手动全速模式:「立即回填」用,worker 线程放开尽快清积压。
    pub async fn new_fast() -> Result<Self> {
        Self::load(true).await
    }

    /// 组装识别管线:模型缺失先下载(留在父进程——代理配置、可视错误都在这边),
    /// 然后预拉起 worker 并完成握手。把 Paddle 的 10-30s 冷启动放在这里,
    /// 是让"引擎起不来"落进「引擎级失败中断整批」的既有语义,
    /// 而不是被算进第一帧的请求超时。
    async fn load(fast: bool) -> Result<Self> {
        ensure_models().await?;
        let sup = Arc::clone(crate::ai::ocr_supervisor::global());
        sup.set_fast(fast).await;
        sup.ensure_ready().await?;
        Ok(Self {
            // 每帧一次 IPC:路径过去、行文本回来(box_norm 消化管线本就不落库)。
            // 超时杀、重建、串行化全在 supervisor 里,这个闭包保持哑。
            recognize: Arc::new(move |path: std::path::PathBuf| {
                let sup = Arc::clone(&sup);
                Box::pin(async move {
                    Ok(sup
                        .recognize(&path)
                        .await?
                        .into_iter()
                        .map(|l| l.text)
                        .collect())
                })
            }),
            folder: Folder::default(),
        })
    }

    /// 测试注入:用假识别函数组装管线,不拉 worker、不加载任何模型。
    /// 测试面保持**同步**闭包(既有夹具全部不动);包装器把它扔进
    /// `spawn_blocking`——与旧生产路径逐字同语义:阻塞型夹具照旧能被
    /// 外层超时打断,panic 照旧折成 `OcrInfra`。
    #[cfg(test)]
    fn with_recognizer(sync: SyncRecognizer) -> Self {
        Self {
            recognize: Arc::new(move |path: std::path::PathBuf| {
                let sync = Arc::clone(&sync);
                Box::pin(async move {
                    tokio::task::spawn_blocking(move || sync(&path))
                        .await
                        .map_err(|e| Error::OcrInfra(format!("识别任务异常终止: {e}")))?
                })
            }),
            folder: Folder::default(),
        }
    }
}

/// 消化积压(批量模式,手动触发):全速引擎 → 清空登记簿 → 引擎随返回释放。
///
/// 已在跑时直接返回错误(单实例);任何单帧错误只降级(标失败重试),
/// 只有引擎级错误(模型加载失败等)才中断整批。
/// 可被 [`request_stop`] 中断:提前停下时正常返回已处理部分的账单。
pub async fn run(mem: &MemoryDb) -> Result<DigestReport> {
    let mut pipe = Pipeline::new_fast().await?;
    let external = AtomicBool::new(false);
    drain(mem, &mut pipe, &external).await
}

/// 消化核心:取待处理帧 → OCR → L3 折叠 → (视觉帧)L4 聚簇 → 记账,
/// 直到登记簿清空、调用方的 `stop` 置位或收到 [`request_stop`]。批量与常驻
/// 共用——差别只在 [`Pipeline`] 的生命周期归谁管。停止都在帧间检查:
/// 最多等一帧(~1s)即生效,且不会留下半消化状态。
/// [`STOP_REQUESTED`] 在批结束时清零:批开始前置位的请求依然有效
/// (覆盖引擎加载窗口),上一批消费过的请求不会殃及下一批。
pub async fn drain(mem: &MemoryDb, pipe: &mut Pipeline, stop: &AtomicBool) -> Result<DigestReport> {
    // 冷却检查在抢批权之前:冷却中连 RUNNING 都不该闪一下
    if let Some(rem) = cooldown_remaining_secs() {
        return Err(Error::OcrInfra(format!(
            "识别引擎多次无响应,冷却中(剩余 {rem}s);手动「立即回填」可立即重试"
        )));
    }
    let Some(_guard) = BatchGuard::acquire() else {
        return Err(Error::InvalidInput("消化任务已在运行"));
    };
    // 标志的清零交给 _guard 的 Drop——panic 与 future 被丢弃时同样生效
    drain_inner(mem, pipe, stop).await
}

async fn drain_inner(
    mem: &MemoryDb,
    pipe: &mut Pipeline,
    stop: &AtomicBool,
) -> Result<DigestReport> {
    let mut report = DigestReport::default();
    let started = std::time::Instant::now();
    // 本批内遇设施故障的帧:它们的 ocr_state 没被改动,外层循环会立刻再取到,
    // 不记下来就是死循环。只在本批内生效,下一批照常重试。
    let mut infra_deferred: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut consecutive_infra: u32 = 0;

    'outer: loop {
        let batch = frames::take_pending(mem, BATCH).await?;
        if batch.is_empty() {
            break;
        }
        // 整批都是本轮已判设施故障的帧 → 再取一次还是它们,收工
        if batch.iter().all(|f| infra_deferred.contains(&f.path)) {
            break;
        }
        for frame in batch {
            if stop.load(Ordering::Relaxed) || STOP_REQUESTED.load(Ordering::Relaxed) {
                break 'outer;
            }
            if infra_deferred.contains(&frame.path) {
                continue;
            }
            // 心跳打在每帧**开始前**:一帧最长也就正常识别时间,而卡死的定义
            // 正是"某一帧再也没走完"——从开始计时才能把它和慢帧区分开
            note_progress();
            match digest_one(mem, pipe, &frame).await {
                Ok(true) => {
                    report.processed += 1;
                    consecutive_infra = 0;
                }
                Ok(false) => {
                    // 图文件已不在(retention 删除/用户清理):按完成记,别无限重试
                    report.skipped_missing_file += 1;
                    consecutive_infra = 0;
                }
                Err(e) if is_infra_failure(&e) => {
                    // 设施故障:**不动 attempts**,帧留在待处理里等下一批。
                    // 这些帧本身没问题,扣它们的重试预算等于替设施故障赔命。
                    log::warn!("帧消化遇设施故障,不计重试 ({}): {e}", frame.path);
                    report.infra_skipped += 1;
                    infra_deferred.insert(frame.path.clone());
                    consecutive_infra += 1;
                    if consecutive_infra >= INFRA_ABORT_STREAK {
                        // 连着坏这么多帧,基本可以断定是设施整体不可用而非零星抖动。
                        // 继续磨下去只是每帧白等一次超时,不如把这批收了、压上冷却
                        // 让自动入口退避——不然常驻 tick 每分钟都会回来白等一轮超时。
                        enter_cooldown();
                        log::error!(
                            "连续 {consecutive_infra} 帧遇设施故障,中止本批(已处理 {} 帧),\
                             冷却 {}s 后自动重试",
                            report.processed,
                            INFRA_COOLDOWN.as_secs()
                        );
                        break 'outer;
                    }
                }
                Err(e) => {
                    log::warn!("帧消化失败 ({}): {e}", frame.path);
                    frames::mark_failed(mem, frame.path.clone()).await?;
                    report.failed += 1;
                    consecutive_infra = 0;
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

/// 单帧管线:读图 → OCR → 折叠 → 标完成。Ok(false) = 图文件缺失(跳过)。
/// 低文字帧不做特殊处理——它们折叠成的"低文字会话"就是 L5 的输入
/// (VLM 描述代表帧,描述文本并入会话进 FTS),不需要消化期分流。
async fn digest_one(mem: &MemoryDb, pipe: &mut Pipeline, frame: &PendingFrame) -> Result<bool> {
    let path = std::path::PathBuf::from(&frame.path);
    if !path.is_file() {
        frames::mark_done(mem, frame.path.clone(), -1).await?;
        return Ok(false);
    }
    // 识别是一次对 worker 子进程的 IPC。挂死的主防线在 supervisor(90s 超时
    // → 杀 worker → 重建);这里的外层超时是纵深:supervisor 自身若有 bug
    // 卡住,消化循环也绝不能跟着一起死——那正是本次事故的形状。
    let fut = (pipe.recognize)(path);
    let lines: Vec<String> = match tokio::time::timeout(FRAME_TIMEOUT, fut).await {
        Err(_elapsed) => {
            return Err(Error::OcrInfra(format!(
                "单帧识别超过 {}s 无响应,已跳过",
                FRAME_TIMEOUT.as_secs()
            )));
        }
        Ok(r) => r?,
    };

    let session_id = pipe.folder.fold_frame(mem, frame, &lines).await?;
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
    use std::collections::HashMap;
    use std::sync::atomic::AtomicUsize;

    use rusqlite::params;

    use super::*;

    // ───────────────────────── 测试基建 ─────────────────────────

    /// [`RUNNING`] / [`STOP_REQUESTED`] 是进程级 static,cargo test 并行跑时
    /// 所有触碰 drain 的测试必须互相串行,否则会看见对方的"已在运行"错误或
    /// 消费掉对方的停止请求。用异步锁:guard 要横跨整个测试体(含 await 点),
    /// std 锁跨 await 会触发 clippy::await_holding_lock;拿锁的测试 panic 时
    /// guard 随栈展开释放,后续测试照常拿锁,无毒化问题。
    async fn drain_lock() -> tokio::sync::MutexGuard<'static, ()> {
        static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let guard = LOCK.lock().await;
        // 冷却是进程级 static:上一个测试触发的熔断会拦住下一个测试的 drain,
        // 拿到锁即清,保证每个测试从无冷却状态起步
        clear_cooldown();
        guard
    }

    /// 每个测试独立的临时目录。帧文件要真实存在:digest_one 先查 is_file,
    /// 内容无所谓(假识别函数从不读它)。
    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hindsight-digest-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 造一个空占位"截图"文件,返回其绝对路径字符串(frames.path 直接当路径用)。
    fn touch(dir: &std::path::Path, name: &str) -> String {
        let p = dir.join(name);
        std::fs::write(&p, b"fake-jpg").unwrap();
        p.to_string_lossy().into_owned()
    }

    async fn reg(mem: &MemoryDb, path: &str, ts: &str, title: &str) {
        frames::register(
            mem,
            path.to_string(),
            ts.to_string(),
            "2026-07-05".to_string(),
            Some("code".to_string()),
            Some(title.to_string()),
        )
        .await
        .unwrap();
    }

    /// 假 OCR:按文件名返回预设行,没预设的路径报识别错;`calls` 记录真实
    /// 调用次数(验证去重帧/缺图帧根本不进识别)。
    fn canned(map: &[(&str, &[&str])], calls: Arc<AtomicUsize>) -> SyncRecognizer {
        let map: HashMap<String, Vec<String>> = map
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
            .collect();
        Arc::new(move |p: &std::path::Path| {
            calls.fetch_add(1, Ordering::SeqCst);
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            map.get(&name)
                .cloned()
                .ok_or_else(|| Error::Ocr(format!("测试假 OCR 无预设: {name}")))
        })
    }

    async fn frame_row(mem: &MemoryDb, path: &str) -> (i64, i64, Option<i64>) {
        let p = path.to_string();
        mem.0
            .call(move |conn| {
                conn.query_row(
                    "SELECT ocr_state, attempts, session_id FROM frames WHERE path = ?1",
                    [p],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .db()
            })
            .await
            .unwrap()
    }

    async fn table_counts(mem: &MemoryDb) -> (i64, i64) {
        mem.0
            .call(|conn| {
                let s = conn
                    .query_row("SELECT COUNT(*) FROM text_sessions", [], |r| r.get(0))
                    .db()?;
                let l = conn
                    .query_row("SELECT COUNT(*) FROM session_lines", [], |r| r.get(0))
                    .db()?;
                Ok((s, l))
            })
            .await
            .unwrap()
    }

    // ───────────────────── 批处理主循环 ─────────────────────

    /// 主路径:登记 3 帧 → 假 OCR → 同标题帧折叠进同一会话、行级并集去重、
    /// 换标题开新会话、FTS 可检索、帧全部记完成并回填会话归属。
    /// 期望值独立推导:a 出 2 行,b 与 a 重叠 1 行再新增 1 行 → 会话一共 3 行;
    /// c 换标题 → 第二个会话 1 行。
    #[tokio::test]
    async fn drain_folds_batch_into_sessions_lines_and_fts() {
        let _g = drain_lock().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = scratch_dir("batch");
        let a = touch(&dir, "a.jpg");
        let b = touch(&dir, "b.jpg");
        let c = touch(&dir, "c.jpg");
        reg(&mem, &a, "2026-07-05T10:00:00+09:00", "阅读笔记").await;
        reg(&mem, &b, "2026-07-05T10:00:30+09:00", "阅读笔记").await;
        reg(&mem, &c, "2026-07-05T10:01:00+09:00", "另一篇").await;

        let calls = Arc::new(AtomicUsize::new(0));
        let mut pipe = Pipeline::with_recognizer(canned(
            &[
                ("a.jpg", &["第一行内容足够长", "第二行内容足够长"]),
                ("b.jpg", &["第二行内容足够长", "第三行内容足够长"]),
                ("c.jpg", &["另一篇的独立一行"]),
            ],
            Arc::clone(&calls),
        ));
        let stop = AtomicBool::new(false);
        let report = drain(&mem, &mut pipe, &stop).await.unwrap();

        assert_eq!(report.processed, 3);
        assert_eq!(report.failed, 0);
        assert_eq!(report.skipped_missing_file, 0);
        assert_eq!(calls.load(Ordering::SeqCst), 3, "每帧恰好识别一次");

        // 帧全部完成,a/b 同会话、c 独立会话
        let (sa, _, sess_a) = frame_row(&mem, &a).await;
        let (sb, _, sess_b) = frame_row(&mem, &b).await;
        let (sc, _, sess_c) = frame_row(&mem, &c).await;
        assert_eq!((sa, sb, sc), (1, 1, 1));
        assert_eq!(sess_a, sess_b, "同标题近时帧折叠进同一会话");
        assert_ne!(sess_a, sess_c, "标题变化开新会话");

        let (sessions, lines) = table_counts(&mem).await;
        assert_eq!(sessions, 2);
        assert_eq!(lines, 3 + 1, "行级并集:重复行只落一次");

        // FTS 现场可搜(b 帧新增的行)
        mem.0
            .call(|conn| {
                let hits: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM text_sessions_fts WHERE text_sessions_fts MATCH '三行内容'",
                        [],
                        |r| r.get(0),
                    )
                    .db()?;
                assert_eq!(hits, 1);
                Ok(())
            })
            .await
            .unwrap();
    }

    /// 重跑幂等:已消化的帧不会被第二次 drain 再碰——识别函数零调用、
    /// 库里会话/行数不变、账单全零。
    #[tokio::test]
    async fn rerun_after_drain_digests_nothing_new() {
        let _g = drain_lock().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = scratch_dir("idem");
        let a = touch(&dir, "a.jpg");
        reg(&mem, &a, "2026-07-05T10:00:00+09:00", "标题").await;

        let calls1 = Arc::new(AtomicUsize::new(0));
        let mut pipe = Pipeline::with_recognizer(canned(
            &[("a.jpg", &["这一行有六个字"])],
            Arc::clone(&calls1),
        ));
        let stop = AtomicBool::new(false);
        assert_eq!(drain(&mem, &mut pipe, &stop).await.unwrap().processed, 1);
        let before = table_counts(&mem).await;

        // 第二轮:换一个会计数的识别函数,必须一次都不被调用
        let calls2 = Arc::new(AtomicUsize::new(0));
        let mut pipe2 = Pipeline::with_recognizer(canned(
            &[("a.jpg", &["不该出现的行不该出现"])],
            Arc::clone(&calls2),
        ));
        let report = drain(&mem, &mut pipe2, &stop).await.unwrap();
        assert_eq!(
            report.processed + report.failed + report.skipped_missing_file,
            0
        );
        assert_eq!(calls2.load(Ordering::SeqCst), 0, "已消化帧不重复识别");
        assert_eq!(table_counts(&mem).await, before, "库内容原封不动");
    }

    // ───────────────────── 停止语义 ─────────────────────

    /// 停止批处理的核心语义:帧间感知停止标志,已处理的部分正常落库,
    /// 未处理的帧保持待消化,下一轮(不置停)能继续清完——停不丢数据。
    #[tokio::test]
    async fn external_stop_between_frames_keeps_finished_part() {
        let _g = drain_lock().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = scratch_dir("stop");
        let a = touch(&dir, "a.jpg");
        let b = touch(&dir, "b.jpg");
        let c = touch(&dir, "c.jpg");
        reg(&mem, &a, "2026-07-05T10:00:00+09:00", "标题").await;
        reg(&mem, &b, "2026-07-05T10:00:30+09:00", "标题").await;
        reg(&mem, &c, "2026-07-05T10:01:00+09:00", "标题").await;

        // 识别第一帧的同时置位停止(模拟用户在批处理中途按停止按钮)
        let stop = Arc::new(AtomicBool::new(false));
        let stop_in_rec = Arc::clone(&stop);
        let mut pipe = Pipeline::with_recognizer(Arc::new(move |_p| {
            stop_in_rec.store(true, Ordering::SeqCst);
            Ok(vec!["第一帧的一行内容".to_string()])
        }));
        let report = drain(&mem, &mut pipe, &stop).await.unwrap();

        // 停止在帧间生效:第 1 帧完整落库,第 2/3 帧原样保持待消化
        assert_eq!(report.processed, 1, "停止前完成的部分照常入账");
        assert_eq!(frame_row(&mem, &a).await.0, 1);
        assert_eq!(frame_row(&mem, &b).await.0, 0, "未处理帧不落半消化状态");
        assert_eq!(frame_row(&mem, &c).await.0, 0);
        let (sessions, lines) = table_counts(&mem).await;
        assert_eq!((sessions, lines), (1, 1), "已处理部分的会话/行完整可查");

        // 停止请求不粘滞:清掉外部标志后新一轮把剩余 2 帧清完
        stop.store(false, Ordering::SeqCst);
        let mut pipe2 =
            Pipeline::with_recognizer(Arc::new(|_p| Ok(vec!["后续帧的一行内容".to_string()])));
        let report2 = drain(&mem, &mut pipe2, &stop).await.unwrap();
        assert_eq!(report2.processed, 2, "停止只作用于当轮,积压下一轮可继续");
    }

    /// 批开始前就按下的停止(引擎还在加载、第一帧还没跑的窗口)同样有效:
    /// 一帧都不处理;且该请求随本轮结束被消费,不殃及下一批。
    #[tokio::test]
    async fn stop_requested_before_batch_is_honored_then_consumed() {
        let _g = drain_lock().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = scratch_dir("prestop");
        let a = touch(&dir, "a.jpg");
        let b = touch(&dir, "b.jpg");
        reg(&mem, &a, "2026-07-05T10:00:00+09:00", "标题").await;
        reg(&mem, &b, "2026-07-05T10:00:30+09:00", "标题").await;

        let calls = Arc::new(AtomicUsize::new(0));
        let recognizer = canned(
            &[
                ("a.jpg", &["这一行有六个字"]),
                ("b.jpg", &["这一行有六个字"]),
            ],
            Arc::clone(&calls),
        );

        request_stop();
        let mut pipe = Pipeline::with_recognizer(Arc::clone(&recognizer));
        let stop = AtomicBool::new(false);
        let report = drain(&mem, &mut pipe, &stop).await.unwrap();
        assert_eq!(report.processed, 0, "批前停止请求让整批空转返回");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "一帧都不进识别");
        assert_eq!(frame_row(&mem, &a).await.0, 0, "帧保持待消化");

        // 请求已被上一轮消费:新一轮正常消化全部积压
        let mut pipe2 = Pipeline::with_recognizer(recognizer);
        let report2 = drain(&mem, &mut pipe2, &stop).await.unwrap();
        assert_eq!(report2.processed, 2, "停止请求不跨批粘滞");
    }

    // ───────────────────── 失败帧处理 ─────────────────────

    /// 顽固失败帧:同轮内被反复重试至上限(3 次)后放弃,既不阻塞其它帧,
    /// 也不把 drain 拖成死循环;且失败帧不切断前后帧的会话折叠。
    #[tokio::test]
    async fn failing_frame_retries_capped_and_does_not_block_others() {
        let _g = drain_lock().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = scratch_dir("fail");
        let good1 = touch(&dir, "good1.jpg");
        let bad = touch(&dir, "bad.jpg");
        let good2 = touch(&dir, "good2.jpg");
        reg(&mem, &good1, "2026-07-05T10:00:00+09:00", "标题").await;
        reg(&mem, &bad, "2026-07-05T10:00:30+09:00", "标题").await;
        reg(&mem, &good2, "2026-07-05T10:01:00+09:00", "标题").await;

        // bad.jpg 无预设 → 假 OCR 永远报错
        let calls = Arc::new(AtomicUsize::new(0));
        let mut pipe = Pipeline::with_recognizer(canned(
            &[
                ("good1.jpg", &["前一帧的一行内容"]),
                ("good2.jpg", &["后一帧的一行内容"]),
            ],
            Arc::clone(&calls),
        ));
        let stop = AtomicBool::new(false);
        let report = drain(&mem, &mut pipe, &stop).await.unwrap();

        assert_eq!(report.processed, 2, "好帧全部消化,不被坏帧卡住");
        // 同一坏帧在本轮内重试:每次尝试都记一笔失败,直到重试上限
        assert_eq!(report.failed, frames::MAX_ATTEMPTS as u64);
        let (state, attempts, _) = frame_row(&mem, &bad).await;
        assert_eq!(
            (state, attempts),
            (2, frames::MAX_ATTEMPTS),
            "坏帧留失败态与完整重试记录"
        );
        // 坏帧識別失败不产生会话残渣;前后好帧同标题近时 → 仍折叠为一个会话
        let (sessions, lines) = table_counts(&mem).await;
        assert_eq!((sessions, lines), (1, 2), "失败帧不切断前后帧的折叠");

        // 上限已到:再跑一轮不会又去啃这帧
        let calls2 = Arc::new(AtomicUsize::new(0));
        let mut pipe2 = Pipeline::with_recognizer(canned(&[], Arc::clone(&calls2)));
        let report2 = drain(&mem, &mut pipe2, &stop).await.unwrap();
        assert_eq!(report2.failed, 0);
        assert_eq!(calls2.load(Ordering::SeqCst), 0, "放弃的帧不再进识别");
    }

    /// 瞬时失败(第一次识别报错、之后恢复):同一轮 drain 内就能重试成功,
    /// 最终帧记完成、文本落库——失败标记是"重试",不是"判死刑"。
    #[tokio::test]
    async fn transient_failure_retried_within_same_drain() {
        let _g = drain_lock().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = scratch_dir("transient");
        let a = touch(&dir, "a.jpg");
        reg(&mem, &a, "2026-07-05T10:00:00+09:00", "标题").await;

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in_rec = Arc::clone(&calls);
        let mut pipe = Pipeline::with_recognizer(Arc::new(move |_p| {
            if calls_in_rec.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(Error::Ocr("第一次识别偶发失败".into()))
            } else {
                Ok(vec!["恢复后识别出的一行".to_string()])
            }
        }));
        let stop = AtomicBool::new(false);
        let report = drain(&mem, &mut pipe, &stop).await.unwrap();

        assert_eq!(report.failed, 1, "首次失败入账");
        assert_eq!(report.processed, 1, "重试成功后正常入账");
        let (state, attempts, session_id) = frame_row(&mem, &a).await;
        assert_eq!(state, 1, "最终是完成态");
        assert_eq!(attempts, 1, "留有一次失败记录");
        assert!(session_id.is_some_and(|s| s > 0));
        assert_eq!(table_counts(&mem).await, (1, 1));
    }

    /// 设施故障绝不消耗帧的重试预算。
    ///
    /// 这条测试是 2026-07-17 事故的回归钉:那天三小时内 243 帧被连续判失败、
    /// `attempts` 全部烧到 3 而永久放弃,截图随后被保留策略删除,文字永久丢失。
    /// 那些截图本身完全正常,坏的是识别设施。
    #[tokio::test]
    async fn infra_failure_never_burns_retry_budget() {
        let _g = drain_lock().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = scratch_dir("infra-budget");
        let a = touch(&dir, "a.jpg");
        reg(&mem, &a, "2026-07-05T10:00:00+09:00", "标题").await;

        let mut pipe =
            Pipeline::with_recognizer(Arc::new(|_p| Err(Error::OcrInfra("识别进程超时".into()))));
        let stop = AtomicBool::new(false);
        let report = drain(&mem, &mut pipe, &stop).await.unwrap();

        assert_eq!(report.infra_skipped, 1, "记在设施故障账上");
        assert_eq!(report.failed, 0, "不计入帧失败");
        let (state, attempts, _) = frame_row(&mem, &a).await;
        assert_eq!(state, 0, "仍是待处理,没被打成失败态");
        assert_eq!(attempts, 0, "重试预算分毫未动 ← 事故的关键点");

        // 设施恢复后这一帧还能正常识别——这才是"没丢"的完整含义
        let mut ok_pipe =
            Pipeline::with_recognizer(Arc::new(|_p| Ok(vec!["设施恢复后识别出来了".to_string()])));
        let report2 = drain(&mem, &mut ok_pipe, &stop).await.unwrap();
        assert_eq!(report2.processed, 1);
        assert_eq!(frame_row(&mem, &a).await.0, 1, "最终正常完成");
    }

    /// DB 抖动同样属设施故障。折叠 / 标记完成里的 DB 错误经 `?` 上抛,
    /// 旧代码一律当帧失败处理——数据库抖一下就够扣掉一批帧的重试预算。
    #[tokio::test]
    async fn db_error_classified_as_infra_not_frame() {
        assert!(is_infra_failure(&Error::Db(
            tokio_rusqlite::Error::ConnectionClosed
        )));
        assert!(is_infra_failure(&Error::OcrInfra("worker 崩溃".into())));
        assert!(is_infra_failure(&Error::EmbeddingRuntimeMissing));
        // 这张图本身的问题:该扣就扣
        assert!(!is_infra_failure(&Error::Ocr("图片解码失败".into())));
    }

    /// 设施整体不可用时,一批不该拿几十帧去磨:连续三帧设施故障即中止本批,
    /// 且这些帧全部保持可重试。
    #[tokio::test]
    async fn consecutive_infra_failures_abort_batch_without_damage() {
        let _g = drain_lock().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = scratch_dir("infra-abort");
        let mut paths = Vec::new();
        for i in 0..10 {
            let p = touch(&dir, &format!("f{i}.jpg"));
            reg(&mem, &p, &format!("2026-07-05T10:00:{i:02}+09:00"), "标题").await;
            paths.push(p);
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in_rec = Arc::clone(&calls);
        let mut pipe = Pipeline::with_recognizer(Arc::new(move |_p| {
            calls_in_rec.fetch_add(1, Ordering::SeqCst);
            Err(Error::OcrInfra("引擎没反应".into()))
        }));
        let stop = AtomicBool::new(false);
        let report = drain(&mem, &mut pipe, &stop).await.unwrap();

        assert_eq!(
            calls.load(Ordering::SeqCst),
            INFRA_ABORT_STREAK as usize,
            "连败到阈值就收手,不把十帧全磨一遍"
        );
        assert_eq!(report.infra_skipped, INFRA_ABORT_STREAK as u64);
        for p in &paths {
            let (state, attempts, _) = frame_row(&mem, p).await;
            assert_eq!((state, attempts), (0, 0), "十帧全部毫发无损");
        }
    }

    /// 识别函数挂起(永不返回)时,单帧超时把它转成设施故障:批继续、帧无损。
    ///
    /// 这是两平台停摆事故的共同解药——macOS Vision 死锁、Windows ORT/DirectML
    /// 挂起,表现都是"这一帧永不结束"。超时前的旧行为:整个消化子系统瘫痪到
    /// 重启为止,停止按钮无效(只在帧间生效)。
    #[tokio::test]
    async fn hung_recognizer_times_out_as_infra_and_frame_unharmed() {
        let _g = drain_lock().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = scratch_dir("hang-timeout");
        let a = touch(&dir, "a.jpg");
        reg(&mem, &a, "2026-07-05T10:00:00+09:00", "标题").await;

        // 永久阻塞的识别函数 = 挂起的引擎。3s 兜底:远大于超时档位(500ms),
        // 又不至于让 tokio 运行时关停时等太久(它会等阻塞任务收尾)
        let mut pipe = Pipeline::with_recognizer(Arc::new(|_p| {
            std::thread::sleep(std::time::Duration::from_secs(3));
            Ok(vec![])
        }));
        let stop = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let report = drain(&mem, &mut pipe, &stop).await.unwrap();

        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "超时必须在测试档位(500ms)附近生效,而不是等挂起的线程自己回来"
        );
        assert_eq!(report.infra_skipped, 1, "挂起记为设施故障");
        assert_eq!(report.failed, 0);
        let (state, attempts, _) = frame_row(&mem, &a).await;
        assert_eq!((state, attempts), (0, 0), "帧无损:不烧重试、仍待处理");
    }

    /// 连败中止 → 进入冷却:冷却期 drain 直接拒绝(自动入口退避),
    /// `clear_cooldown`(手动「立即回填」路径)立即解锁。
    #[tokio::test]
    async fn infra_streak_enters_cooldown_and_manual_clear_unblocks() {
        let _g = drain_lock().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = scratch_dir("cooldown");
        for i in 0..4 {
            let p = touch(&dir, &format!("f{i}.jpg"));
            reg(&mem, &p, &format!("2026-07-05T10:00:{i:02}+09:00"), "标题").await;
        }

        let mut bad =
            Pipeline::with_recognizer(Arc::new(|_p| Err(Error::OcrInfra("引擎没反应".into()))));
        let stop = AtomicBool::new(false);
        let report = drain(&mem, &mut bad, &stop).await.unwrap();
        assert_eq!(report.infra_skipped, INFRA_ABORT_STREAK as u64);
        assert!(
            cooldown_remaining_secs().is_some(),
            "连败中止后必须进入冷却"
        );

        // 冷却期:自动入口的 drain 被拒,且不碰 RUNNING
        let mut ok_pipe = Pipeline::with_recognizer(Arc::new(|_p| Ok(vec!["正常".to_string()])));
        let err = drain(&mem, &mut ok_pipe, &stop).await.unwrap_err();
        assert!(
            err.to_string().contains("冷却"),
            "冷却期拒绝并说明原因: {err}"
        );
        assert!(!is_running(), "被冷却拒绝的调用不得留下运行标志");

        // 手动路径清冷却 → 立即可跑
        clear_cooldown();
        let report2 = drain(&mem, &mut ok_pipe, &stop).await.unwrap();
        assert_eq!(report2.processed, 4, "清冷却后四帧全部正常识别");
    }

    /// panic 展开时批次互斥必须被释放。
    ///
    /// 旧写法把清零放在 `drain_inner().await` 之后,展开会直接跳过它,
    /// `RUNNING` 永久卡 true——常驻 tick、定时补识别、总结前补识别全部被
    /// `is_running()` 挡住,`ocr_catchup` 每秒空转,日报再也生成不出来,
    /// 只有重启能恢复。
    ///
    /// 注:识别函数自身 panic 走不到这里——它在 `digest_one` 内层
    /// `spawn_blocking` 里,JoinError 被接住转成 `Error::Ocr` 当帧故障处理
    /// (顺带烧掉该帧三次重试,这个误分类留给失败分类那一步修)。真正会展开到
    /// `drain` 的是折叠 / DB 层的 panic,所以这里直接对守卫本身取证。
    #[tokio::test]
    async fn panic_unwind_releases_batch_lock() {
        let _g = drain_lock().await;
        assert!(!is_running(), "起点干净");

        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = BatchGuard::acquire().expect("应能抢到批权");
            assert!(is_running(), "持有守卫期间标志为真");
            panic!("模拟折叠/DB 层在批中途炸掉");
        }));

        assert!(caught.is_err(), "确实发生了 panic 展开");
        assert!(!is_running(), "展开后互斥必须已释放");
        assert!(stalled_ms().is_none(), "没有批在跑时不报卡死时长");

        // 关键后果验证:下一批还能正常开工
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = scratch_dir("panic-guard");
        let a = touch(&dir, "a.jpg");
        reg(&mem, &a, "2026-07-05T10:00:00+09:00", "标题").await;
        let mut pipe = Pipeline::with_recognizer(Arc::new(|_p| Ok(vec!["恢复正常".to_string()])));
        let stop = AtomicBool::new(false);
        let report = drain(&mem, &mut pipe, &stop).await.unwrap();
        assert_eq!(report.processed, 1, "panic 之后的批不受影响");
    }

    /// future 被丢弃(任务取消 / 运行时关停)时,批次互斥同样必须被释放。
    /// 这是 panic 之外的第二条泄漏路径:`drain` 的 future 停在某个 await 上被
    /// drop,函数末尾的清零永远等不到执行。
    #[tokio::test]
    async fn cancelled_drain_releases_batch_lock() {
        let _g = drain_lock().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = scratch_dir("cancel-guard");
        let a = touch(&dir, "a.jpg");
        reg(&mem, &a, "2026-07-05T10:00:00+09:00", "标题").await;

        // 识别函数卡住不返回(模拟 Vision 死锁)→ drain 停在这一帧上,
        // 超时后整个 future 被丢弃。用同步阻塞而非 async:识别函数本就是
        // 同步的、跑在 spawn_blocking 线程上,这样才和真实卡死同形。
        // recv_timeout 兜底,免得测试结束后还留个永久线程。
        let (_tx, rx) = std::sync::mpsc::channel::<()>();
        let rx = std::sync::Mutex::new(rx);
        let mut pipe = Pipeline::with_recognizer(Arc::new(move |_p| {
            let _ = rx
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(10));
            Ok(vec![])
        }));
        let stop = AtomicBool::new(false);

        let timed = tokio::time::timeout(
            std::time::Duration::from_millis(300),
            drain(&mem, &mut pipe, &stop),
        )
        .await;
        assert!(timed.is_err(), "drain 确实被超时丢弃");
        assert!(!is_running(), "future 被丢弃后互斥必须已释放");
    }

    /// 心跳语义:批在跑时报告"距上一帧多久",空闲时不报。
    /// 判据是距上一帧而非批总时长——三千张积压跑五十分钟属于正常。
    #[tokio::test]
    async fn stalled_ms_tracks_frame_progress_not_batch_age() {
        let _g = drain_lock().await;
        assert!(stalled_ms().is_none(), "没有批在跑");

        let guard = BatchGuard::acquire().expect("应能抢到批权");
        let at_start = stalled_ms().expect("批在跑时应有读数");
        assert!(at_start < 1_000, "刚开批算作刚有过进度,不是一上来就卡死");

        note_progress();
        assert!(stalled_ms().unwrap() < 1_000, "记一次进度后重新计时");

        drop(guard);
        assert!(stalled_ms().is_none(), "批结束后不再报");
        assert!(!is_running());
    }

    /// 图文件已被清理(retention 删除)的帧:不进识别、按完成记(session_id=-1),
    /// 不产生会话,也绝不留在登记簿里无限重试。
    #[tokio::test]
    async fn missing_file_marked_done_without_recognition() {
        let _g = drain_lock().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = scratch_dir("missing");
        let ghost = dir.join("deleted.jpg").to_string_lossy().into_owned(); // 从未创建
        reg(&mem, &ghost, "2026-07-05T10:00:00+09:00", "标题").await;

        let calls = Arc::new(AtomicUsize::new(0));
        let mut pipe = Pipeline::with_recognizer(canned(&[], Arc::clone(&calls)));
        let stop = AtomicBool::new(false);
        let report = drain(&mem, &mut pipe, &stop).await.unwrap();

        assert_eq!(report.skipped_missing_file, 1);
        assert_eq!(report.processed, 0);
        assert_eq!(report.failed, 0);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "缺图帧不进识别");
        let (state, _, session_id) = frame_row(&mem, &ghost).await;
        assert_eq!(state, 1, "按完成记,不再重试");
        assert_eq!(session_id, Some(-1), "-1 = 无会话归属的哨兵值");
        assert_eq!(table_counts(&mem).await, (0, 0), "不产生空会话");
    }

    // ───────────────────── 单实例互斥 ─────────────────────

    /// "已在运行"互斥:第一轮 drain 卡在识别中时,第二个 drain 立即报错让路,
    /// 且不能把第一轮的运行标志碰掉;第一轮结束后标志复位。
    #[tokio::test]
    async fn second_drain_rejected_while_first_running() {
        let _g = drain_lock().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();
        let dir = scratch_dir("mutex");
        let a = touch(&dir, "a.jpg");
        reg(&mem, &a, "2026-07-05T10:00:00+09:00", "标题").await;

        // 识别函数阻塞在通道上,把第一轮 drain 钉在"正在消化"状态
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let rx = std::sync::Mutex::new(rx);
        let mut pipe = Pipeline::with_recognizer(Arc::new(move |_p| {
            rx.lock().unwrap().recv().unwrap();
            Ok(vec!["放行后识别出的行".to_string()])
        }));
        let mem_bg = mem.clone();
        let stop_bg = Arc::new(AtomicBool::new(false));
        let stop_bg2 = Arc::clone(&stop_bg);
        let first = tokio::spawn(async move { drain(&mem_bg, &mut pipe, &stop_bg2).await });

        // 等第一轮真正进入运行态(拿到 RUNNING)
        let mut running = false;
        for _ in 0..200 {
            if is_running() {
                running = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(running, "第一轮 drain 应进入运行态");

        // 第二个 drain:立即拒绝,不阻塞不排队
        let mut pipe2 = Pipeline::with_recognizer(Arc::new(|_p| Ok(vec![])));
        let stop2 = AtomicBool::new(false);
        let second = drain(&mem, &mut pipe2, &stop2).await;
        assert!(
            matches!(second, Err(Error::InvalidInput(m)) if m.contains("已在运行")),
            "第二个消化请求应被互斥拒绝: {second:?}"
        );
        assert!(is_running(), "被拒的请求不得清掉进行中批次的运行标志");

        // 放行第一轮:正常完成,运行标志复位
        tx.send(()).unwrap();
        let report = first.await.unwrap().unwrap();
        assert_eq!(report.processed, 1, "互斥冲突不影响第一轮的正常完成");
        assert!(!is_running(), "批结束后运行标志复位");
    }

    // ───────────────────── 模型下载 ─────────────────────

    /// 本地假文件服务:按 URL 路径回预设 body 并统计命中次数;未知路径回 404。
    /// 范式照抄 ai/summary_operations.rs 的假 OpenAI 服务(127.0.0.1 canned HTTP)。
    async fn spawn_file_server(files: Vec<(&'static str, Vec<u8>)>) -> (u16, Arc<AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_srv = Arc::clone(&hits);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                // GET 无 body,读到空行即收全请求
                let mut buf: Vec<u8> = Vec::new();
                let mut tmp = [0u8; 2048];
                loop {
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let head = String::from_utf8_lossy(&buf);
                let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
                let resp = match files.iter().find(|(p, _)| *p == path) {
                    Some((_, body)) => {
                        hits_srv.fetch_add(1, Ordering::SeqCst);
                        let mut r = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(body);
                        r
                    }
                    None => {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    }
                };
                let _ = sock.write_all(&resp).await;
                let _ = sock.shutdown().await;
            }
        });
        (port, hits)
    }

    /// 拿一个刚释放的本地端口——连接必然被拒,模拟下载源不可达。
    fn free_local_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        port
    }

    /// 缺哪个下哪个:已存在的文件绝不重下(内容原样),缺的按源取回;
    /// 全部就绪后再跑零请求(幂等),且不留 .downloading 中间文件。
    #[tokio::test]
    async fn download_missing_only_fetches_absent_files() {
        let dir = scratch_dir("dl");
        std::fs::write(dir.join("det.onnx"), b"already-here").unwrap();
        let (port, hits) = spawn_file_server(vec![
            ("/det", b"SHOULD-NOT-BE-FETCHED".to_vec()),
            ("/rec", b"fake-rec-model-bytes".to_vec()),
        ])
        .await;
        let det_url = format!("http://127.0.0.1:{port}/det");
        let rec_url = format!("http://127.0.0.1:{port}/rec");
        let sources = [
            ("det.onnx", det_url.as_str()),
            ("rec.onnx", rec_url.as_str()),
        ];

        download_missing(&dir, &sources).await.unwrap();
        assert_eq!(
            std::fs::read(dir.join("det.onnx")).unwrap(),
            b"already-here",
            "已存在的文件不被覆盖"
        );
        assert_eq!(
            std::fs::read(dir.join("rec.onnx")).unwrap(),
            b"fake-rec-model-bytes",
            "缺的文件按源取回"
        );
        assert_eq!(hits.load(Ordering::SeqCst), 1, "只有缺的那个发起了请求");
        assert!(
            !dir.join("rec.onnx.downloading").exists(),
            "写入走临时文件 + rename,不留中间产物"
        );

        // 幂等:三件套齐了就零网络
        download_missing(&dir, &sources).await.unwrap();
        assert_eq!(hits.load(Ordering::SeqCst), 1, "全部就绪后不再发请求");
    }

    /// 字典完整性门:条目数不等于预期(上游改版)或不是 UTF-8 → 明确报错,
    /// 且坏字典绝不落盘;条目数恰好匹配 → 正常写入。
    #[tokio::test]
    async fn download_missing_dict_line_guard_and_utf8() {
        let good_dict = "字\n".repeat(DICT_EXPECTED_LINES);
        let (port, _) = spawn_file_server(vec![
            ("/bad", "第一行\n第二行\n第三行\n".as_bytes().to_vec()),
            ("/binary", vec![0xff, 0xfe, 0x9f, 0x00]),
            ("/good", good_dict.into_bytes()),
        ])
        .await;

        // 条目数不符 → 拒绝使用,不落盘
        let dir = scratch_dir("dict-bad");
        let url = format!("http://127.0.0.1:{port}/bad");
        let err = download_missing(&dir, &[("dict.txt", url.as_str())])
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains(&DICT_EXPECTED_LINES.to_string()),
            "错误要说清预期条目数: {msg}"
        );
        assert!(!dir.join("dict.txt").exists(), "坏字典不得落盘");
        assert!(!dir.join("dict.txt.downloading").exists());

        // 非 UTF-8 → 同样拒绝
        let dir2 = scratch_dir("dict-bin");
        let url2 = format!("http://127.0.0.1:{port}/binary");
        let err2 = download_missing(&dir2, &[("dict.txt", url2.as_str())])
            .await
            .unwrap_err();
        assert!(
            err2.to_string().contains("UTF-8"),
            "报错点名编码问题: {err2}"
        );
        assert!(!dir2.join("dict.txt").exists());

        // 条目数恰好匹配 → 正常写入
        let dir3 = scratch_dir("dict-good");
        let url3 = format!("http://127.0.0.1:{port}/good");
        download_missing(&dir3, &[("dict.txt", url3.as_str())])
            .await
            .unwrap();
        let saved = std::fs::read_to_string(dir3.join("dict.txt")).unwrap();
        assert_eq!(saved.lines().count(), DICT_EXPECTED_LINES);
    }

    /// 下载源不可达(拒连)与 404:都返回错误且目标文件不产生——
    /// 半截模型文件比没有模型更糟(引擎会加载到坏文件)。
    #[tokio::test]
    async fn download_missing_network_failures_leave_no_file() {
        // 拒连
        let dir = scratch_dir("dl-refused");
        let url = format!("http://127.0.0.1:{}/rec", free_local_port());
        let res = download_missing(&dir, &[("rec.onnx", url.as_str())]).await;
        assert!(res.is_err(), "拒连必须报错: {res:?}");
        assert!(!dir.join("rec.onnx").exists());

        // 404(源在但路径错/上游下架)
        let (port, _) = spawn_file_server(vec![]).await;
        let dir2 = scratch_dir("dl-404");
        let url2 = format!("http://127.0.0.1:{port}/gone");
        let err = download_missing(&dir2, &[("rec.onnx", url2.as_str())])
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("rec.onnx"),
            "错误要点名是哪个文件下载失败: {err}"
        );
        assert!(!dir2.join("rec.onnx").exists());
    }

    // ───────────────────── 历史回填 ─────────────────────

    async fn insert_activity(pool: &DbPool, started: &str, shot: Option<&str>, title: &str) {
        let (started, shot, title) = (
            started.to_string(),
            shot.map(str::to_string),
            title.to_string(),
        );
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(started_at, ended_at, duration_secs, local_date,
                        local_hour, process_name, window_title, category_id, screenshot_path)
                     VALUES (?1, ?1, 30, '2026-07-05', 10, 'code', ?2, 'other', ?3)",
                    params![started, title, shot],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// 历史回填:同一截图的多条活动行归并成一帧(取最早时刻),无截图/空路径
    /// 的行不登记;重跑幂等(INSERT OR IGNORE,登记簿不膨胀)。
    #[tokio::test]
    async fn backfill_registers_each_screenshot_once() {
        let pool = crate::repo::test_util::fresh_test_pool().await;
        let mem = MemoryDb::open_in_memory().await.unwrap();

        // 同一截图两条活动行(乱序插入) + 另一截图 + 两条不该回填的行
        insert_activity(
            &pool,
            "2026-07-05T10:05:00+09:00",
            Some("shots/a.jpg"),
            "main.rs",
        )
        .await;
        insert_activity(
            &pool,
            "2026-07-05T10:00:00+09:00",
            Some("shots/a.jpg"),
            "main.rs",
        )
        .await;
        insert_activity(
            &pool,
            "2026-07-05T10:07:00+09:00",
            Some("shots/b.jpg"),
            "lib.rs",
        )
        .await;
        insert_activity(&pool, "2026-07-05T10:08:00+09:00", None, "无截图").await;
        insert_activity(&pool, "2026-07-05T10:09:00+09:00", Some(""), "空路径").await;

        let n = backfill_from_activities(&pool, &mem).await.unwrap();
        assert_eq!(n, 2, "只有带截图的唯一路径被登记");

        let pending = frames::take_pending(&mem, 10).await.unwrap();
        assert_eq!(pending.len(), 2);
        // 按 ts 升序:a.jpg 取的是该截图两条活动行的最早时刻
        assert_eq!(pending[0].path, "shots/a.jpg");
        assert_eq!(pending[0].ts, "2026-07-05T10:00:00+09:00");
        assert_eq!(pending[0].title.as_deref(), Some("main.rs"));
        assert_eq!(pending[1].path, "shots/b.jpg");
        assert_eq!(pending[1].ts, "2026-07-05T10:07:00+09:00");

        // 幂等:重跑不产生重复登记,已有帧的 ts 不被改写
        backfill_from_activities(&pool, &mem).await.unwrap();
        let again = frames::take_pending(&mem, 10).await.unwrap();
        assert_eq!(again.len(), 2, "重复回填不膨胀登记簿");
        assert_eq!(again[0].ts, "2026-07-05T10:00:00+09:00");
    }

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
