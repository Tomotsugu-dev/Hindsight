//! 云端截图洞察(docs/design/cloud-insight.md)。
//!
//! 管线:帧登记(frames 表)→ 内容粗门 → 应用策略筛帧 → 单图单调用 VLM
//! → frame_insights 帧级落库(无聚合层)。
//!
//! 生命周期与 [`crate::memory::resident::ResidentOcr`] 同构:常驻循环按设置
//! 启停;历史回填是独立的一次性任务,靠 `insight_since_ts` 水位线与常驻分工
//! (常驻只吃水位线之后的新帧,回填显式确认后吃之前的存量)。

pub mod vlm;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use futures_util::StreamExt;
use rusqlite::params;
use tokio::task::JoinHandle;

use crate::error::Result;
use crate::memory::MemoryDb;
use crate::repo::settings::Settings;
use crate::storage::{DbPool, SqliteResultExt};

/// 常驻循环 tick(秒)。与采集 30s 同拍:每 tick 至多 1-2 张新帧。
const TICK_SECS: u64 = 30;
/// 内容粗门:缩略图变化像素占比达到该值才算"新画面"。
/// 与存储级 L1 门(0.10%)不是一回事——那是落盘门,砍不动上传量。
const GATE_FRACTION: f64 = 0.10;
/// 同画面最长隔多久强制刷新一帧(有界丢失)。
const GATE_REFRESH_SECS: i64 = 600;
/// 缩略图规格(与 L1 评测同规格)与单像素灰度差阈值。
const THUMB_W: u32 = 256;
const THUMB_H: u32 = 144;
const PIXEL_TAU: i16 = 12;
/// recommended 档非重点应用的采样间隔(秒)。
const SAMPLE_GAP_SECS: i64 = 300;
/// 分辨率:重点/全量 1280(文字密集底线),非重点采样 960。
const SIDE_FULL: u32 = 1280;
const SIDE_SAMPLED: u32 = 960;
/// 失败重试上限(跨 drain 轮次,attempts 记账)。
const MAX_ATTEMPTS: i64 = 5;
/// 单轮 drain 取的候选上限;积压超过它下一 tick 继续。
const BATCH_LIMIT: i64 = 500;
/// 积压超过该数用并发消化,否则串行(常驻稳态)。
const BURST_THRESHOLD: usize = 50;
/// 积压/回填并发。实测服务端 ~4 帧/s 见顶,16 已到收益平台(设计 §7)。
const BURST_CONCURRENCY: usize = 16;

// ── 策略与粗门(纯函数,单测覆盖)────────────────────────

/// 单帧的处理决定。
#[derive(Debug, PartialEq)]
pub enum Decision {
    /// 不上传(策略未选中/采样间隔内)。
    Skip,
    /// 上传,长边压到指定像素。
    Upload { max_side: u32 },
}

/// 应用策略:scope + 重点应用集合(小写匹配)。
pub struct Strategy {
    scope: Scope,
    focus: HashSet<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Scope {
    Focus,
    Recommended,
    All,
}

impl Strategy {
    pub fn from_settings(s: &Settings) -> Self {
        let scope = match s.insight_scope.as_str() {
            "focus" => Scope::Focus,
            "all" => Scope::All,
            _ => Scope::Recommended,
        };
        Self {
            scope,
            focus: s
                .insight_focus_apps
                .iter()
                .map(|a| a.trim().to_lowercase())
                .filter(|a| !a.is_empty())
                .collect(),
        }
    }

    fn is_focus(&self, app: &str) -> bool {
        let a = app.to_lowercase();
        self.focus.iter().any(|f| a.contains(f))
    }

    /// 决定一帧的去留。`last_app_upload` 是该应用上一次上传的 epoch 秒
    /// (recommended 档非重点应用按间隔采样用)。
    pub fn decide(&self, app: &str, last_app_upload: Option<i64>, now: i64) -> Decision {
        match (self.scope, self.is_focus(app)) {
            (Scope::All, _) | (_, true) => Decision::Upload {
                max_side: SIDE_FULL,
            },
            (Scope::Focus, false) => Decision::Skip,
            (Scope::Recommended, false) => match last_app_upload {
                Some(t) if now - t < SAMPLE_GAP_SECS => Decision::Skip,
                _ => Decision::Upload {
                    max_side: SIDE_SAMPLED,
                },
            },
        }
    }
}

/// 缩略图变化占比:|Δ|>τ 的像素数 / 总像素。长度不一致按 1.0(必上传)。
pub fn changed_fraction(a: &[u8], b: &[u8]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }
    let changed = a
        .iter()
        .zip(b)
        .filter(|(x, y)| (**x as i16 - **y as i16).abs() > PIXEL_TAU)
        .count();
    changed as f64 / a.len() as f64
}

/// 读帧并出 256×144 灰度缩略图(粗门比对用)。
async fn thumbnail(path: &std::path::Path) -> Result<Vec<u8>> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let img =
            image::open(&path).map_err(|e| crate::error::Error::Ocr(format!("读帧失败: {e}")))?;
        let g = image::imageops::grayscale(&img.resize_exact(
            THUMB_W,
            THUMB_H,
            image::imageops::FilterType::Triangle,
        ));
        Ok(g.into_raw())
    })
    .await
    .map_err(|e| crate::error::Error::Ocr(format!("spawn_blocking: {e}")))?
}

fn epoch_of(ts: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|d| d.timestamp())
        .unwrap_or(0)
}

// ── 落库 ──────────────────────────────────────────────

/// 待处理帧的元数据(来自 frames 登记表)。
#[derive(Debug, Clone)]
pub struct Candidate {
    pub path: String,
    pub ts: String,
    pub local_date: String,
    pub app: String,
    pub title: String,
    /// true = 此前上传失败的重试帧(跳过粗门,直接按策略上传)
    pub retry: bool,
}

/// 取待处理帧:新帧(未有 insight 行)+ 可重试的失败帧。
/// `after=true` 取水位线之后(常驻),false 取之前(回填)。
async fn pending(
    mem: &MemoryDb,
    watermark: String,
    after: bool,
    limit: i64,
) -> Result<Vec<Candidate>> {
    let cmp = if after { ">" } else { "<=" };
    mem.0
        .call(move |conn| {
            let sql = format!(
                "SELECT f.path, f.ts, f.local_date,
                        COALESCE(f.app_id,''), COALESCE(f.title,''),
                        fi.path IS NOT NULL
                 FROM frames f
                 LEFT JOIN frame_insights fi ON fi.path = f.path
                 WHERE (fi.path IS NULL OR (fi.state = 2 AND fi.attempts < ?2))
                   AND f.ts {cmp} ?1
                 ORDER BY f.ts LIMIT ?3"
            );
            let mut stmt = conn.prepare(&sql).db()?;
            let rows = stmt
                .query_map(params![watermark, MAX_ATTEMPTS, limit], |r| {
                    Ok(Candidate {
                        path: r.get(0)?,
                        ts: r.get(1)?,
                        local_date: r.get(2)?,
                        app: r.get(3)?,
                        title: r.get(4)?,
                        retry: r.get(5)?,
                    })
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(rows)
        })
        .await
        .map_err(Into::into)
}

/// 写结果行(UPSERT:重试帧覆盖旧失败行)。state: 1完/2失败/3跳过。
async fn mark(
    mem: &MemoryDb,
    c: &Candidate,
    state: i64,
    insight: Option<String>,
    entities: Option<String>,
) -> Result<()> {
    let (path, ts, date, app, title) = (
        c.path.clone(),
        c.ts.clone(),
        c.local_date.clone(),
        c.app.clone(),
        c.title.clone(),
    );
    let now = chrono::Local::now().to_rfc3339();
    mem.0
        .call(move |conn| {
            conn.execute(
                "INSERT INTO frame_insights
                     (path, ts, local_date, app, title, insight, entities, state, attempts, done_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                         CASE WHEN ?8 = 2 THEN 1 ELSE 0 END, ?9)
                 ON CONFLICT(path) DO UPDATE SET
                     insight = excluded.insight, entities = excluded.entities,
                     state = excluded.state, done_at = excluded.done_at,
                     attempts = attempts + CASE WHEN excluded.state = 2 THEN 1 ELSE 0 END",
                params![path, ts, date, app, title, insight, entities, state, now],
            )
            .db()?;
            Ok(())
        })
        .await
        .map_err(Into::into)
}

/// 今日已分析帧数(按 done_at 日期计——回填也占当日额度)。
pub async fn today_done(mem: &MemoryDb) -> Result<i64> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    mem.0
        .call(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM frame_insights
                 WHERE state = 1 AND substr(done_at, 1, 10) = ?1",
                params![today],
                |r| r.get(0),
            )
            .db()
        })
        .await
        .map_err(Into::into)
}

/// 待处理帧数(常驻视角:水位线之后)。
pub async fn pending_count(mem: &MemoryDb, watermark: String, after: bool) -> Result<i64> {
    let cmp = if after { ">" } else { "<=" };
    mem.0
        .call(move |conn| {
            conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM frames f
                     LEFT JOIN frame_insights fi ON fi.path = f.path
                     WHERE (fi.path IS NULL OR (fi.state = 2 AND fi.attempts < ?2))
                       AND f.ts {cmp} ?1"
                ),
                params![watermark, MAX_ATTEMPTS],
                |r| r.get(0),
            )
            .db()
        })
        .await
        .map_err(Into::into)
}

/// 段总结注入:某日某小时段的帧洞察行 `(HH:MM, "app | 洞察 | 实体")`。
/// 独立打只读连接(总结管线没有记忆库句柄);任何失败返回空——
/// 洞察是总结的增强材料,绝不阻塞总结本身。行数超预算按步长抽稀。
pub async fn segment_insights(date: &str, start_hour: u8, end_hour: u8) -> Vec<(String, String)> {
    const MAX_LINES: usize = 60;
    let Ok(path) = crate::memory::memory_db_path() else {
        return Vec::new();
    };
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let Ok(conn) = tokio_rusqlite::Connection::open_with_flags(path, flags).await else {
        return Vec::new();
    };
    let (date, sh, eh) = (date.to_string(), start_hour as i64, end_hour as i64);
    let rows: Vec<(String, String)> = conn
        .call(move |c| {
            let mut stmt = c.prepare(
                "SELECT substr(ts, 12, 5),
                        COALESCE(app,''), COALESCE(insight,''), COALESCE(entities,'')
                 FROM frame_insights
                 WHERE local_date = ?1 AND state = 1
                   AND CAST(substr(ts, 12, 2) AS INTEGER) >= ?2
                   AND CAST(substr(ts, 12, 2) AS INTEGER) < ?3
                 ORDER BY ts",
            )?;
            let out = stmt
                .query_map(params![date, sh, eh], |r| {
                    let t: String = r.get(0)?;
                    let app: String = r.get(1)?;
                    let insight: String = r.get(2)?;
                    let entities: String = r.get(3)?;
                    let text = if entities.is_empty() {
                        format!("{app} | {insight}")
                    } else {
                        format!("{app} | {insight} | {entities}")
                    };
                    Ok((t, text))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(out)
        })
        .await
        .unwrap_or_default();
    if rows.len() <= MAX_LINES {
        return rows;
    }
    let step = rows.len() as f64 / MAX_LINES as f64;
    (0..MAX_LINES)
        .map(|i| rows[(i as f64 * step) as usize].clone())
        .collect()
}

// ── drain 核心(常驻与回填共用)────────────────────────

struct DrainCtx {
    client: reqwest::Client,
    endpoint: String,
    api_key: String,
    model: String,
    lang: String,
}

/// 处理一批候选帧。返回本轮实际上传数。
/// 粗门/策略判定串行(本地毫秒级,且锚点有顺序依赖);VLM 调用并发。
#[allow(clippy::too_many_arguments)]
async fn drain_batch(
    mem: &MemoryDb,
    ctx: &DrainCtx,
    settings: &Settings,
    batch: Vec<Candidate>,
    anchor: &mut Option<(Vec<u8>, i64)>,
    last_app_upload: &mut HashMap<String, i64>,
    budget: usize,
    stop: &AtomicBool,
) -> Result<usize> {
    let strategy = Strategy::from_settings(settings);
    let mut uploads: Vec<(Candidate, u32)> = Vec::new();

    for c in batch {
        if stop.load(Ordering::Relaxed) || uploads.len() >= budget {
            break;
        }
        // 上传侧隐私复核:采集端命中就不会有截图,这里防的是"名单是后来才加的"
        if crate::capture::privacy::should_skip_screenshot(
            &c.app,
            &c.title,
            None,
            &settings.privacy_url_keywords,
            &settings.privacy_app_keywords,
        ) {
            mark(mem, &c, 3, None, None).await?;
            continue;
        }
        let decision = strategy.decide(
            &c.app,
            last_app_upload.get(&c.app).copied(),
            epoch_of(&c.ts),
        );
        let max_side = match decision {
            Decision::Skip => {
                mark(mem, &c, 3, None, None).await?;
                continue;
            }
            Decision::Upload { max_side } => max_side,
        };
        // 内容粗门(重试帧免门:此前已判定过要上传)
        if !c.retry {
            let thumb = match thumbnail(std::path::Path::new(&c.path)).await {
                Ok(t) => t,
                Err(e) => {
                    log::warn!("洞察缩略图失败,跳过 {}: {e}", c.path);
                    mark(mem, &c, 3, None, None).await?;
                    continue;
                }
            };
            let ts = epoch_of(&c.ts);
            if let Some((prev, prev_ts)) = anchor.as_ref() {
                if changed_fraction(prev, &thumb) < GATE_FRACTION
                    && ts - prev_ts < GATE_REFRESH_SECS
                {
                    mark(mem, &c, 3, None, None).await?;
                    continue;
                }
            }
            *anchor = Some((thumb, ts));
        }
        last_app_upload.insert(c.app.clone(), epoch_of(&c.ts));
        uploads.push((c, max_side));
    }

    if uploads.is_empty() {
        return Ok(0);
    }
    let concurrency = if uploads.len() > BURST_THRESHOLD {
        BURST_CONCURRENCY
    } else {
        1
    };
    let done = futures_util::stream::iter(uploads.into_iter().map(|(c, side)| {
        let mem = mem.clone();
        let ctx_ref = ctx;
        async move {
            if stop.load(Ordering::Relaxed) {
                return 0usize;
            }
            let jpeg = match vlm::downscale_jpeg(std::path::Path::new(&c.path), side).await {
                Ok(j) => j,
                Err(e) => {
                    log::warn!("洞察降采样失败 {}: {e}", c.path);
                    let _ = mark(&mem, &c, 3, None, None).await;
                    return 0;
                }
            };
            match vlm::describe(
                &ctx_ref.client,
                &ctx_ref.endpoint,
                &ctx_ref.api_key,
                &ctx_ref.model,
                &jpeg,
                &ctx_ref.lang,
            )
            .await
            {
                Ok((insight, entities)) => {
                    let _ = mark(&mem, &c, 1, Some(insight), Some(entities)).await;
                    1
                }
                Err(e) => {
                    log::warn!("洞察调用失败 {}: {e}", c.path);
                    let _ = mark(&mem, &c, 2, None, None).await;
                    0
                }
            }
        }
    }))
    .buffer_unordered(concurrency)
    .fold(0usize, |acc, n| async move { acc + n })
    .await;
    Ok(done)
}

fn drain_ctx(settings: &Settings) -> Option<DrainCtx> {
    let (endpoint, api_key, model) = settings.ai.vision_conn()?;
    Some(DrainCtx {
        client: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .ok()?,
        endpoint,
        api_key,
        model,
        lang: settings.ai.prompt_language.clone(),
    })
}

// ── 常驻 worker + 回填任务 ────────────────────────────

struct Running {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

/// 回填进度(前端轮询)。
#[derive(Default)]
pub struct BackfillProgress {
    pub running: AtomicBool,
    pub done: AtomicUsize,
    pub total: AtomicUsize,
}

/// 洞察控制器——tauri managed state。常驻循环 + 一次性回填任务。
#[derive(Default)]
pub struct InsightWorker {
    resident: tokio::sync::Mutex<Option<Running>>,
    backfill: tokio::sync::Mutex<Option<Running>>,
    pub backfill_progress: Arc<BackfillProgress>,
}

impl InsightWorker {
    /// 按设置同步常驻启停(启动期与设置保存时调用)。
    pub async fn sync(&self, enabled: bool, pool: Option<DbPool>, mem: Option<MemoryDb>) {
        match (enabled, pool, mem) {
            (true, Some(pool), Some(mem)) => self.start(pool, mem).await,
            _ => self.stop().await,
        }
    }

    async fn start(&self, pool: DbPool, mem: MemoryDb) {
        let mut guard = self.resident.lock().await;
        if guard.is_some() {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop_task = Arc::clone(&stop);
        let handle = tokio::spawn(async move {
            log::info!("云端截图洞察常驻启动");
            let mut anchor: Option<(Vec<u8>, i64)> = None;
            let mut last_app: HashMap<String, i64> = HashMap::new();
            loop {
                for _ in 0..TICK_SECS {
                    if stop_task.load(Ordering::Relaxed) {
                        log::info!("云端截图洞察常驻停止");
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                let settings = match crate::repo::settings::load(&pool).await {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!("洞察读取设置失败: {e}");
                        continue;
                    }
                };
                if !settings.insight_enabled {
                    continue;
                }
                let Some(watermark) = settings.insight_since_ts.clone() else {
                    continue; // 前端开启时必打水位线;缺失宁可不动存量
                };
                let Some(ctx) = drain_ctx(&settings) else {
                    continue; // 视觉连接未配齐
                };
                let done_today = match today_done(&mem).await {
                    Ok(n) => n,
                    Err(e) => {
                        log::warn!("洞察日额度查询失败: {e}");
                        continue;
                    }
                };
                let budget = (settings.insight_daily_frame_cap as i64 - done_today).max(0) as usize;
                if budget == 0 {
                    continue;
                }
                match pending(&mem, watermark, true, BATCH_LIMIT).await {
                    Ok(batch) if !batch.is_empty() => {
                        if let Err(e) = drain_batch(
                            &mem,
                            &ctx,
                            &settings,
                            batch,
                            &mut anchor,
                            &mut last_app,
                            budget,
                            &stop_task,
                        )
                        .await
                        {
                            log::warn!("洞察本轮消化失败: {e}");
                        }
                    }
                    Ok(_) => {}
                    Err(e) => log::warn!("洞察候选查询失败: {e}"),
                }
            }
        });
        *guard = Some(Running { stop, handle });
    }

    async fn stop(&self) {
        let mut guard = self.resident.lock().await;
        if let Some(running) = guard.take() {
            running.stop.store(true, Ordering::Relaxed);
            let _ = running.handle.await;
        }
    }

    /// 启动历史回填(水位线之前的存量帧)。已在跑则 no-op 返回 false。
    pub async fn start_backfill(&self, pool: DbPool, mem: MemoryDb) -> Result<bool> {
        let mut guard = self.backfill.lock().await;
        if guard.is_some() {
            return Ok(false);
        }
        let settings = crate::repo::settings::load(&pool).await?;
        let Some(watermark) = settings.insight_since_ts.clone() else {
            return Err(crate::error::Error::InvalidInput("洞察未开启"));
        };
        if drain_ctx(&settings).is_none() {
            return Err(crate::error::Error::InvalidInput("视觉模型未配置"));
        }
        let total = pending_count(&mem, watermark.clone(), false).await? as usize;
        let progress = Arc::clone(&self.backfill_progress);
        progress.total.store(total, Ordering::Relaxed);
        progress.done.store(0, Ordering::Relaxed);
        progress.running.store(true, Ordering::Relaxed);

        let stop = Arc::new(AtomicBool::new(false));
        let stop_task = Arc::clone(&stop);
        let handle = tokio::spawn(async move {
            log::info!("洞察历史回填启动:{total} 帧");
            let mut anchor: Option<(Vec<u8>, i64)> = None;
            let mut last_app: HashMap<String, i64> = HashMap::new();
            loop {
                if stop_task.load(Ordering::Relaxed) {
                    break;
                }
                // 每轮重读设置:回填中途改策略/限额/关功能都即时生效
                let Ok(settings) = crate::repo::settings::load(&pool).await else {
                    break;
                };
                if !settings.insight_enabled {
                    break;
                }
                let Some(ctx) = drain_ctx(&settings) else {
                    break;
                };
                let done_today = today_done(&mem).await.unwrap_or(0);
                let budget = (settings.insight_daily_frame_cap as i64 - done_today).max(0) as usize;
                if budget == 0 {
                    log::info!("洞察回填触到日限额,暂停(次日常驻不接管,需重新点回填)");
                    break;
                }
                let batch = match pending(&mem, watermark.clone(), false, BATCH_LIMIT).await {
                    Ok(b) => b,
                    Err(e) => {
                        log::warn!("回填候选查询失败: {e}");
                        break;
                    }
                };
                if batch.is_empty() {
                    break;
                }
                let before = pending_count(&mem, watermark.clone(), false)
                    .await
                    .unwrap_or(0);
                if let Err(e) = drain_batch(
                    &mem,
                    &ctx,
                    &settings,
                    batch,
                    &mut anchor,
                    &mut last_app,
                    budget,
                    &stop_task,
                )
                .await
                {
                    log::warn!("回填批次失败: {e}");
                    break;
                }
                let after = pending_count(&mem, watermark.clone(), false)
                    .await
                    .unwrap_or(0);
                let progressed = (before - after).max(0) as usize;
                progress.done.fetch_add(progressed, Ordering::Relaxed);
                if progressed == 0 {
                    break; // 防御:批次没有任何推进(全失败)就别空转
                }
            }
            progress.running.store(false, Ordering::Relaxed);
            log::info!("洞察历史回填结束");
        });
        *guard = Some(Running { stop, handle });
        Ok(true)
    }

    /// 取消回填(已完成的帧保留,重进从断点续)。
    pub async fn cancel_backfill(&self) {
        let mut guard = self.backfill.lock().await;
        if let Some(running) = guard.take() {
            running.stop.store(true, Ordering::Relaxed);
            let _ = running.handle.await;
        }
        self.backfill_progress
            .running
            .store(false, Ordering::Relaxed);
    }

    /// 回填任务收尾清理(结束后调用方查询时顺手把句柄收掉)。
    pub async fn reap_backfill(&self) {
        let mut guard = self.backfill.lock().await;
        if let Some(r) = guard.as_ref() {
            if r.handle.is_finished() {
                guard.take();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_with(scope: &str, focus: &[&str]) -> Settings {
        Settings {
            insight_scope: scope.to_string(),
            insight_focus_apps: focus.iter().map(|s| s.to_string()).collect(),
            ..Settings::default()
        }
    }

    #[test]
    fn strategy_focus_only_uploads_focus_apps() {
        let s = Strategy::from_settings(&settings_with("focus", &["WeChat"]));
        assert_eq!(
            s.decide("WeChat", None, 1000),
            Decision::Upload { max_side: 1280 }
        );
        assert_eq!(s.decide("Code", None, 1000), Decision::Skip);
    }

    #[test]
    fn strategy_recommended_samples_non_focus_by_gap() {
        let s = Strategy::from_settings(&settings_with("recommended", &["WeChat"]));
        // 重点应用永远全帧
        assert_eq!(
            s.decide("WeChat", Some(999), 1000),
            Decision::Upload { max_side: 1280 }
        );
        // 非重点:5 分钟内已传过 → 跳
        assert_eq!(s.decide("Code", Some(800), 1000), Decision::Skip);
        // 超过间隔 → 采样上传(960)
        assert_eq!(
            s.decide("Code", Some(400), 1000),
            Decision::Upload { max_side: 960 }
        );
        // 首帧 → 上传
        assert_eq!(
            s.decide("Code", None, 1000),
            Decision::Upload { max_side: 960 }
        );
    }

    #[test]
    fn strategy_all_uploads_everything_full_res() {
        let s = Strategy::from_settings(&settings_with("all", &[]));
        assert_eq!(
            s.decide("Anything", Some(999), 1000),
            Decision::Upload { max_side: 1280 }
        );
    }

    #[test]
    fn strategy_focus_match_is_case_insensitive_substring() {
        let s = Strategy::from_settings(&settings_with("focus", &["wechat"]));
        assert_eq!(
            s.decide("WeChat.exe", None, 0),
            Decision::Upload { max_side: 1280 }
        );
    }

    #[test]
    fn changed_fraction_thresholds() {
        let a = vec![100u8; 100];
        // 全同 → 0
        assert_eq!(changed_fraction(&a, &a), 0.0);
        // 一半像素变化超阈 → 0.5
        let mut b = a.clone();
        for p in b.iter_mut().take(50) {
            *p = 130;
        }
        assert!((changed_fraction(&a, &b) - 0.5).abs() < 1e-9);
        // τ 以内的抖动不算变化
        let c = vec![110u8; 100];
        assert_eq!(changed_fraction(&a, &c), 0.0);
        // 长度不一致 fail-open
        assert_eq!(changed_fraction(&a, &[1, 2, 3]), 1.0);
    }
}
