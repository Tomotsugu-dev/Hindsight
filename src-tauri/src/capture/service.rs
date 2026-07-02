use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Duration, Local, Timelike};
use serde::Serialize;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::capture::{browser_url, privacy, screenshot, window};
use crate::error::Result;
use crate::repo::settings::TimeRange;
use crate::repo::{activities, app_groups, process_paths};
use crate::storage::DbPool;

const POLL_INTERVAL_SECS: u64 = 5;

/// 采集服务对前端的运行时状态快照。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    /// 后台采集 task 是否在跑
    pub running: bool,
    /// 今日 activities 表行数
    pub today_count: u32,
    /// 最近一次采集成功的 RFC3339 时间；从未采集 None
    pub last_capture_at: Option<String>,
    /// 最近一次 tick 失败的错误描述；成功后清空
    pub last_error: Option<String>,
}

#[derive(Default, Clone)]
struct WorkHoursState {
    enabled: bool,
    ranges: Vec<TimeRange>,
}

#[derive(Clone)]
struct ScreenshotConfig {
    enabled: bool,
    dir: String,
    target_width: u32,
    target_height: u32,
    jpeg_quality: u8,
}

impl Default for ScreenshotConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dir: String::new(),
            target_width: 1280,
            target_height: 720,
            jpeg_quality: 80,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct FocusState {
    app_name: String,
    /// 仅浏览器场景填值；非浏览器为 None。
    /// title 不参与 focus 比较，避免光标位置 / 未保存标记等带来的频繁抖动。
    url: Option<String>,
}

struct CurrentSession {
    id: i64,
    focus: FocusState,
    last_extend_at: Instant,
}

/// CaptureService 的内部状态。
///
/// **锁粒度：每个字段一把独立的 `tokio::sync::Mutex`**——故意没合并成单一 Inner Mutex。
/// 原因：`tick()` 单次执行可能花数百毫秒（截图 + DB 写 + 浏览器 URL 抓取），
/// 合并成一把锁会让所有 setter（设置页改完调用的 `set_*`）和 `status()`
/// 在 tick 期间都阻塞；分开锁后 setter 跟 tick 各自只在用到相关字段时短暂相遇。
///
/// **锁顺序约定：`work_hours / idle_threshold_secs` 互斥使用，再到 `current` 再到
/// `interval_secs`**（参见 `tick()` 顺序）。`set_*` 方法仅取单把锁不嵌套，因此也安全。
/// 任何新增字段或新增持锁路径需保持顺序，否则有死锁风险。
///
/// 临界区都极短（克隆配置 / 单赋值 / 取出 Option），不会因细粒度锁产生抖动。
struct Inner {
    pool: DbPool,
    interval_secs: Mutex<u32>,
    handle: Mutex<Option<JoinHandle<()>>>,
    last_capture_at: Mutex<Option<String>>,
    last_error: Mutex<Option<String>>,
    work_hours: Mutex<WorkHoursState>,
    screenshot: Mutex<ScreenshotConfig>,
    /// 浏览器 URL 关键词；地址栏 URL 命中其中一条即跳过截图
    privacy_url_keywords: Mutex<Vec<String>>,
    /// 应用 / 标题关键词；app_name 或 window title 命中其中一条即跳过截图
    privacy_app_keywords: Mutex<Vec<String>>,
    /// 用户多久不动鼠键就算"挂机"，超过这个秒数 tick 就 seal 当前会话不再延续。
    /// 0 = 关闭挂机检测（永远算在用，回到 idle 检测之前的行为）。
    idle_threshold_secs: Mutex<u32>,
    current: Mutex<Option<CurrentSession>>,
    /// 上一次 tick 的墙钟时刻——睡眠 gap 检测用。系统睡眠期间 tick 循环不跑
    /// （`Instant` 在 macOS 还会暂停计时），若两次 tick 的墙钟间隔远超轮询周期，
    /// 说明机器睡过去了：当前会话必须按"最后已知活跃时刻"（= 上次 tick）封口，
    /// 否则整段睡眠会被算成最后聚焦 app 的使用时长。
    /// 锁顺序：独立短锁，取值即放，不与其它锁嵌套。
    last_tick_at: Mutex<Option<DateTime<Local>>>,
}

/// 墙钟 gap 超过多少秒视为"经历了睡眠/进程暂停"。3 个轮询周期：正常 tick 间隔
/// = 5s + tick 自身执行时长（截图/URL 抓取偶尔数百 ms~几秒），15s 留足余量；
/// 误触发也无害——只是把会话在上次 tick 处拆开，少记几秒。
const SLEEP_GAP_SECS: i64 = (POLL_INTERVAL_SECS * 3) as i64;

/// 焦点采集服务的对外句柄。`Arc<Inner>` 让多个 setter / 后台 tick task 共享内部状态。
pub struct CaptureService {
    inner: Arc<Inner>,
}

impl CaptureService {
    /// 创建采集服务。`interval_secs` 是同一焦点持续多少秒后强制补一张截图，
    /// 启动后由 `set_interval` 跟 settings 同步。
    pub fn new(pool: DbPool, interval_secs: u32) -> Self {
        Self {
            inner: Arc::new(Inner {
                pool,
                interval_secs: Mutex::new(interval_secs),
                handle: Mutex::new(None),
                last_capture_at: Mutex::new(None),
                last_error: Mutex::new(None),
                work_hours: Mutex::new(WorkHoursState::default()),
                screenshot: Mutex::new(ScreenshotConfig::default()),
                privacy_url_keywords: Mutex::new(Vec::new()),
                privacy_app_keywords: Mutex::new(Vec::new()),
                // 真实值由 lib.rs 启动时根据 settings 写入；此处给个安全默认（180s = 3min），
                // 万一 set 没调到，行为仍然合理。与 settings::Settings::default 保持一致。
                idle_threshold_secs: Mutex::new(180),
                current: Mutex::new(None),
                last_tick_at: Mutex::new(None),
            }),
        }
    }

    /// 配置截图相关参数：启用 / 目录 / 缩放分辨率 / JPEG 质量。
    /// settings 改完后由 commands 层调一次同步。
    pub async fn set_screenshot_config(
        &self,
        enabled: bool,
        dir: String,
        target_width: u32,
        target_height: u32,
        jpeg_quality: u8,
    ) {
        let mut cfg = self.inner.screenshot.lock().await;
        *cfg = ScreenshotConfig {
            enabled,
            dir,
            target_width,
            target_height,
            jpeg_quality,
        };
    }

    /// 启动后台采集 task。已在跑时是 no-op。
    pub async fn start(&self) {
        let mut h = self.inner.handle.lock().await;
        if h.is_some() {
            return;
        }

        // 启动期清扫：上次进程退出（quit / crash / 重启）时遗留的 unsealed 活跃
        // session 永远拿不回 ended_at —— current_lock 是 in-memory 的，进程一退就丢，
        // 留在 DB 里的 duration_secs=0 + ended_at=started_at 的孤儿行就这么僵着。
        // 这里 spawn 后台 tick 之前先一刀切删掉所有这种孤儿，避免：
        //   1. day_apps / day_hours 的 SUM 不变（孤儿贡献本来就是 0），但 PairingSection
        //      之类按行展示的 UI 会看到一堆 dur=0 的诡异历史
        //   2. push 把它们当今天/历史日的有效行上传 Drive，对端 mirror 进 DB 占空间
        //
        // 注意时序：start() 才刚开始跑，self.inner.current 一定是 None
        // （只有 tick() 才会 set 它），所以无差别 DELETE 不会冲掉"正在 capture 的当前
        // session" —— 当前还没创建呢。
        if let Err(e) = activities::purge_orphan_sessions(&self.inner.pool).await {
            log::warn!("启动期孤儿 session 清理失败: {e}");
        }

        let inner = Arc::clone(&self.inner);
        *h = Some(tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)).await;
                if let Err(e) = tick(&inner).await {
                    log::warn!("采集 tick 失败: {e}");
                    let mut le = inner.last_error.lock().await;
                    *le = Some(e.to_string());
                }
            }
        }));
    }

    /// 停止后台采集 task。同时 seal 当前会话写入 outbox，避免数据丢失。
    pub async fn stop(&self) {
        let mut h = self.inner.handle.lock().await;
        if let Some(handle) = h.take() {
            handle.abort();
        }
        // seal 当前会话（如果有），让它进入 outbox 在下次同步推送。
        // 结束时刻做 gap 钳制：机器刚从睡眠醒来、还没跑过一次 tick 用户就退出的话，
        // 按"上次 tick"封口而不是 now——不把整段睡眠算进最后那个 app。
        let end = {
            let lt = self.inner.last_tick_at.lock().await;
            match *lt {
                Some(prev) if (Local::now() - prev).num_seconds() > SLEEP_GAP_SECS => prev,
                _ => Local::now(),
            }
        };
        let mut cur_lock = self.inner.current.lock().await;
        if let Some(prev) = cur_lock.take() {
            drop(cur_lock);
            if let Err(e) = activities::seal_session(&self.inner.pool, prev.id, end).await {
                log::warn!("stop: seal_session 失败 (id={}): {e}", prev.id);
            }
        }
    }

    /// 在**持有会话锁**的前提下执行清库类操作。
    ///
    /// `purge_activities` 这类"DELETE 全表"若与 tick 并发：tick 可能在 DELETE 之后
    /// 插入新行、又被随后的 `reset_session` 清掉指针——留下一条永远不会被 seal 的
    /// dur=0 孤儿行（用户刚清完库就多出一条脏数据）。tick 的插入/延长路径都必须先
    /// 拿 `current` 锁（见 [`tick`]），所以整个闭包期间持锁即可完全互斥。
    /// 进入时直接丢弃当前会话（不 seal——它所在的行马上就要被删了）。
    pub async fn run_with_session_cleared<T, F, Fut>(&self, f: F) -> T
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        let mut cur = self.inner.current.lock().await;
        *cur = None;
        let out = f().await;
        drop(cur);
        out
    }

    /// 后台采集 task 是否在跑。
    pub async fn is_running(&self) -> bool {
        self.inner.handle.lock().await.is_some()
    }

    /// 更新同焦点强制截图间隔（秒）；clamp 到 1..=600。
    pub async fn set_interval(&self, secs: u32) {
        let secs = secs.clamp(1, 600);
        *self.inner.interval_secs.lock().await = secs;
    }

    /// 更新工作时段：`enabled=false` 表示 24 小时全采集，`true + ranges` 限时段内采集。
    pub async fn set_work_hours(&self, enabled: bool, ranges: Vec<TimeRange>) {
        let mut state = self.inner.work_hours.lock().await;
        *state = WorkHoursState { enabled, ranges };
    }

    /// 更新挂机阈值（秒）。0 = 关闭检测。设置页改完由命令层调一次。
    pub async fn set_idle_threshold(&self, secs: u32) {
        *self.inner.idle_threshold_secs.lock().await = secs.min(3600);
    }

    /// 更新隐私关键词。设置页改完后由命令层调一次。
    pub async fn set_privacy_keywords(&self, url_keywords: Vec<String>, app_keywords: Vec<String>) {
        *self.inner.privacy_url_keywords.lock().await = url_keywords;
        *self.inner.privacy_app_keywords.lock().await = app_keywords;
    }

    /// 拉当前运行时状态（today_count 实时查 DB；其它字段从内存读）。
    pub async fn status(&self) -> CaptureStatus {
        let today_count = activities::today_count(&self.inner.pool)
            .await
            .unwrap_or_else(|e| {
                log::warn!("today_count 查询失败: {e}");
                0
            });
        CaptureStatus {
            running: self.is_running().await,
            today_count,
            last_capture_at: self.inner.last_capture_at.lock().await.clone(),
            last_error: self.inner.last_error.lock().await.clone(),
        }
    }
}

async fn tick(inner: &Inner) -> Result<()> {
    // ── 睡眠 gap 检测（最先跑，先于工作时段/挂机分支）──
    // 两次 tick 的墙钟间隔远超轮询周期 = 机器睡过 / 进程被暂停过。当前会话按
    // "最后已知活跃时刻"（上次 tick）封口——不封的话整段睡眠会算进该 app：
    //   合盖 9 小时 → 醒来第一个 tick 若不处理，昨晚的 Chrome 会话吸走 9h。
    // `last_extend_at` 的 `Instant` 在 macOS(Intel) 睡眠期间不走，靠它兜不住。
    let now_tick = Local::now();
    let prev_tick = {
        let mut lt = inner.last_tick_at.lock().await;
        lt.replace(now_tick)
    };
    if let Some(prev) = prev_tick {
        if (now_tick - prev).num_seconds() > SLEEP_GAP_SECS {
            let mut cur_lock = inner.current.lock().await;
            if let Some(prev_sess) = cur_lock.take() {
                drop(cur_lock);
                log::info!(
                    "tick gap {}s（睡眠/暂停），会话 {} 按上次 tick 时刻封口",
                    (now_tick - prev).num_seconds(),
                    prev_sess.id
                );
                if let Err(e) = activities::seal_session(&inner.pool, prev_sess.id, prev).await {
                    log::warn!("seal_session 失败 (睡眠 gap, id={}): {e}", prev_sess.id);
                }
            }
        }
    }

    {
        let wh = inner.work_hours.lock().await;
        if wh.enabled && !wh.ranges.is_empty() {
            let now = Local::now();
            let now_minutes = now.hour() as i32 * 60 + now.minute() as i32;
            let in_range = wh.ranges.iter().any(|r| {
                let start = parse_hm(&r.start);
                let end = parse_hm(&r.end);
                if start <= end {
                    now_minutes >= start && now_minutes < end
                } else {
                    now_minutes >= start || now_minutes < end
                }
            });
            if !in_range {
                log::debug!("跳过本次采集：当前不在工作时段");
                // 离开工作时段：seal 当前会话（推到云端） + 清空 current
                let mut cur_lock = inner.current.lock().await;
                if let Some(prev) = cur_lock.take() {
                    drop(cur_lock);
                    if let Err(e) =
                        activities::seal_session(&inner.pool, prev.id, Local::now()).await
                    {
                        log::warn!("seal_session 失败 (离开工作时段, id={}): {e}", prev.id);
                    }
                }
                return Ok(());
            }
        }
    }

    // 挂机检测：用户多久没动鼠键。超过阈值就 seal 当前会话并 return，让"挂机时段"
    // 不计入使用时长。ended_at 用"用户最后一次活动时间" = now - idle，duration 才准。
    // 用户回来动鼠键 → 下次 tick 看到 current=None → 自然开新会话。
    // idle 读一次供两处用：本分支的阈值判断 + 下面 interval-roll 的抑制。
    let idle_now = crate::platform::idle_secs();
    let idle_threshold = *inner.idle_threshold_secs.lock().await;
    {
        let threshold = idle_threshold;
        if threshold > 0 && idle_now as u32 >= threshold {
            let mut cur_lock = inner.current.lock().await;
            if let Some(prev) = cur_lock.take() {
                drop(cur_lock);
                let real_end = Local::now() - Duration::seconds(idle_now as i64);
                if let Err(e) = activities::seal_session(&inner.pool, prev.id, real_end).await {
                    log::warn!(
                        "seal_session 失败 (用户挂机 {idle_now}s, id={}): {e}",
                        prev.id
                    );
                }
            }
            return Ok(());
        }
    }

    let info = match window::current_window() {
        Ok(i) => i,
        Err(e) => {
            log::debug!("跳过本次采集：{e}");
            return Ok(());
        }
    };

    if info.app_name.is_empty() || info.app_name == "Unknown" {
        return Ok(());
    }

    // 调试字符串残片（如 "8607797 pid=58750 ]"）：进程启动/退出瞬间 AppKit/xcap
    // 偶尔给出的垃圾名。写进 activities 会在前端出现无图标的幽灵应用行，跳过本 tick。
    if window::is_garbage_window_name(&info.app_name) {
        log::debug!("跳过本次采集：app 名疑似调试残片 ({})", info.app_name);
        return Ok(());
    }

    // 系统占位进程（锁屏 loginwindow / 屏保 ScreenSaverEngine 等）：用户明显挂机但
    // platform::idle_secs() 在 macOS 锁屏后有时不增加 → 上面 idle 分支漏判 → 17 分钟
    // 锁屏被记成 17 分钟使用。这里基于"前台是谁"再做一次 force-idle 兜底。
    // 行为跟 idle 分支一致：seal 当前会话 + return，下一 tick 用户回来动鼠键自然开新会话。
    if window::is_system_idle_proxy(&info.app_name) {
        let mut cur_lock = inner.current.lock().await;
        if let Some(prev) = cur_lock.take() {
            drop(cur_lock);
            if let Err(e) = activities::seal_session(&inner.pool, prev.id, Local::now()).await {
                log::warn!(
                    "seal_session 失败 (系统占位进程 {}, id={}): {e}",
                    info.app_name,
                    prev.id
                );
            }
        }
        return Ok(());
    }

    let now = Local::now();

    // 浏览器场景：每次 tick 都抓 URL，因为 URL 是 focus 的一部分（切 URL 要立即触发新会话）。
    // 平台调用阻塞 + 偶尔卡几百 ms，扔到 spawn_blocking 不堵 async runtime。
    // 非浏览器跳过这步，省 50–300ms。
    let is_browser = browser_url::is_browser_app(&info.app_name);
    let fetched_url = if is_browser {
        let app_name_for_url = info.app_name.clone();
        tokio::task::spawn_blocking(move || {
            browser_url::try_get_foreground_browser_url(&app_name_for_url)
        })
        .await
        .ok()
        .flatten()
    } else {
        None
    };

    let mut current_lock = inner.current.lock().await;

    // URL 抓取的瞬时失败（浏览器还在前台但这次没拿到地址栏）不能当成"URL 变了"：
    // Some -> None 会被 focus 比较判成切换 → 强制开新会话 + 在"恰好没法做 URL 隐私
    // 过滤"的这一刻截图。同 app 且上个会话有已知 URL 时继承之，视为焦点未变。
    // `url_inherited` 记住"这是继承的旧值"：继承只用于焦点连续性判断，**不能**
    // 拿去做隐私判断——抓取持续失败（如中途撤销自动化权限）时旧 URL 会被无限继承，
    // 屏幕上实际可能已是命中关键词的隐私页面，按旧 URL 判会照常截图。
    let (url, url_inherited) = match (&fetched_url, current_lock.as_ref()) {
        (None, Some(cur))
            if is_browser && cur.focus.app_name == info.app_name && cur.focus.url.is_some() =>
        {
            (cur.focus.url.clone(), true)
        }
        _ => (fetched_url, false),
    };

    let new_focus = FocusState {
        app_name: info.app_name.clone(),
        url: url.clone(),
    };

    let interval_secs = *inner.interval_secs.lock().await;
    let need_new = match current_lock.as_ref() {
        None => true,
        // 触发新会话的两种情况：
        //   1) 焦点切换（不同 app / 不同 url）—— 立即截图
        //   2) 同一焦点停留满 interval_secs —— 周期性补一张图，让长时间停留也有时间序列。
        //      但用户已经手离鼠键一阵（idle 超过两个轮询周期）时**不 roll**：roll 出来的
        //      会话按墙钟封口，会把阈值前的挂机秒数（默认最多 180s-30s=150s/次）全记成
        //      使用。焦点没变又没人在操作 → 让当前会话原地等着，等 idle 分支/焦点切换
        //      给它正确的结束时刻。
        //      例外：idle_threshold == 0 = 用户明确关闭挂机检测（"永远算在用"）——
        //      此时不能再按 idle 抑制 roll，否则看视频/阅读几分钟不碰键鼠就没有
        //      周期截图和时间序列了，违背"关闭"的语义。
        Some(cur) => {
            cur.focus != new_focus
                || (cur.last_extend_at.elapsed().as_secs() as u32 >= interval_secs
                    && (idle_threshold == 0 || idle_now < POLL_INTERVAL_SECS * 2))
        }
    };

    if need_new {
        // 焦点切换 / 间隔到点：先把旧会话钉死（推 outbox），再开新会话
        if let Some(prev) = current_lock.take() {
            if let Err(e) = activities::seal_session(&inner.pool, prev.id, now).await {
                log::warn!("seal_session 失败 (开新会话, id={}): {e}", prev.id);
            }
        }
        // 隐私过滤：标题或 URL（如果有）命中关键词 → 不截图，但活动行照常落库。
        // 继承来的 URL 按 None 处理（fail-closed）：它只证明"焦点大概率没变"，
        // 不能证明"当前页面安全"。
        let privacy_url = if url_inherited { None } else { url.as_deref() };
        let skip = should_skip_for_privacy(inner, &info, privacy_url, is_browser).await;
        if skip {
            log::info!(
                "隐私过滤命中，跳过截图 app={} title={:?} url={:?} (inherited={url_inherited})",
                info.app_name,
                info.title,
                url
            );
        }
        let shot = if skip {
            None
        } else {
            match take_screenshot(inner, info.pid).await {
                Some(path) => {
                    // TOCTOU 复核：隐私判断用的是 tick 开始时的标题/URL，截图在几百 ms
                    // 后才拍；同一浏览器进程内切 tab 不换 PID，pre-capture 的 PID 校验
                    // 挡不住。拍完按"现在"的焦点再判一次，命中就丢图（活动行照常落库）。
                    if recheck_privacy_after_shot(inner, &info, is_browser).await {
                        log::info!("隐私复核命中，丢弃截图 app={}", info.app_name);
                        let _ = tokio::fs::remove_file(&path).await;
                        None
                    } else {
                        Some(path)
                    }
                }
                None => None,
            }
        };
        let id = activities::insert_new(&inner.pool, &info, now, shot).await?;
        // 保证这个 process_name 有对应的 app_group / member（首次见到的应用建单成员组）
        if let Err(e) = app_groups::ensure_group(&inner.pool, &info.app_name).await {
            log::warn!("ensure_group 失败 ({}): {e}", info.app_name);
        }
        *current_lock = Some(CurrentSession {
            id,
            focus: new_focus,
            last_extend_at: Instant::now(),
        });
    }
    drop(current_lock);

    if let Some(path) = info.app_path.as_ref() {
        if !path.is_empty() {
            let _ = process_paths::upsert(&inner.pool, &info.app_name, path).await;
        }
    }

    let mut last_at = inner.last_capture_at.lock().await;
    *last_at = Some(now.to_rfc3339());
    let mut last_err = inner.last_error.lock().await;
    *last_err = None;

    Ok(())
}

fn parse_hm(s: &str) -> i32 {
    let mut parts = s.split(':');
    let h: i32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let m: i32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    h * 60 + m
}

/// 隐私关键词是否命中本次焦点。
///
/// fail-closed：浏览器在前台、配置了 URL 关键词、但这次没拿到 URL（连继承都没有，
/// 比如浏览器刚启动的第一个 tick）→ 视为命中跳过截图。没法判定就不拍。
async fn should_skip_for_privacy(
    inner: &Inner,
    info: &window::WindowInfo,
    url: Option<&str>,
    is_browser: bool,
) -> bool {
    let url_kw = inner.privacy_url_keywords.lock().await.clone();
    let app_kw = inner.privacy_app_keywords.lock().await.clone();
    if url_kw.is_empty() && app_kw.is_empty() {
        return false;
    }
    if is_browser && url.is_none() && !url_kw.is_empty() {
        return true;
    }
    privacy::should_skip_screenshot(&info.app_name, &info.title, url, &url_kw, &app_kw)
}

/// 截图后的隐私复核：按"现在"的窗口标题（浏览器再抓一次 URL）重跑同一套隐私规则。
/// 返回 true = 应丢弃这张图。判定不了（窗口解析失败 / 前台进程已换）也返回 true——
/// 无法证明安全就不留。没配任何关键词时直接 false，零开销。
async fn recheck_privacy_after_shot(
    inner: &Inner,
    expected: &window::WindowInfo,
    is_browser: bool,
) -> bool {
    let url_kw = inner.privacy_url_keywords.lock().await.clone();
    let app_kw = inner.privacy_app_keywords.lock().await.clone();
    if url_kw.is_empty() && app_kw.is_empty() {
        return false;
    }
    let now_info = match window::current_window() {
        Ok(i) => i,
        Err(_) => return true,
    };
    if now_info.pid != expected.pid || now_info.app_name != expected.app_name {
        // 前台已换进程：这张图的归属都不确定了，丢弃
        return true;
    }
    let url = if is_browser && !url_kw.is_empty() {
        let app_name = now_info.app_name.clone();
        tokio::task::spawn_blocking(move || browser_url::try_get_foreground_browser_url(&app_name))
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    should_skip_for_privacy(inner, &now_info, url.as_deref(), is_browser).await
}

async fn take_screenshot(inner: &Inner, expected_pid: u32) -> Option<String> {
    let cfg = inner.screenshot.lock().await.clone();
    if !cfg.enabled || cfg.dir.trim().is_empty() {
        return None;
    }
    let path = std::path::PathBuf::from(&cfg.dir);
    match screenshot::capture_active_window(
        path,
        cfg.target_width,
        cfg.target_height,
        cfg.jpeg_quality,
        expected_pid,
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            log::warn!("截图失败: {e}");
            None
        }
    }
}
