//! 总结一条龙的第一棒:开跑日报/周报前,把**所有**未识别的截图先 OCR 掉。
//!
//! 语义(产品定稿,2026-07-30 修订):
//! - **跟随「常驻 OCR」意愿**——该开关是用户"要不要自动识别"的唯一信号:
//!   关着 = 用户不用 OCR(或只手动回填),总结**永不**触碰 OCR,本函数零成本
//!   直接返回;开着 = 总结前把常驻攒下的积压(电池纪律/刚开机的漏网帧)
//!   清零,保证日报时刻索引齐。它是常驻模式的收尾保障,不是独立 OCR 入口。
//! - **尽力而为**——OCR 引擎不可用(模型没下载/加载失败)不阻断总结:
//!   记日志、跳过本阶段,LLM 阶段照常。总结的材料是活动记录,不依赖 OCR。
//! - **进度可见**——通过既有的 [`SUMMARY_PROGRESS_EVENT`] 流发两个新 phase
//!   (`ocr_engine_starting` / `ocr_running`),前端总结页把"加载 OCR 模型 →
//!   识别中 x/y → 加载 LLM → 生成中"整条流水线摆给用户看。
//!
//! 并发纪律:消化核心复用 [`digest`](crate::memory::digest) 的全局单批互斥
//! (`RUNNING`)。常驻批正在跑时不抢——两边清的是同一本登记簿,本阶段只
//! 轮询计数发进度;需要时才自己起批。取消总结(SummaryCancel)会顺带
//! [`digest::request_stop`] 停掉当前批,与屏幕记忆页停止按钮同一语义。

use std::sync::atomic::{AtomicBool, Ordering};

use crate::ai::summary_progress::SummaryProgress;
use crate::ai::summary_runner::ProgressSink;
use crate::memory::{digest, frames, MemoryDb};
use crate::storage::DbPool;

/// 进度轮询间隔。1s:与常驻 tick(60s)/单帧耗时(~1s)相称,事件流不刷屏。
const POLL_MS: u64 = 1000;

/// 本阶段的硬上限。超过就放弃清积压、让总结照常进行——总结的材料是活动记录,
/// 本就不依赖 OCR,没理由被它无限期扣住。测试下压到毫秒级。
const MAX_WAIT: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_millis(600)
} else {
    std::time::Duration::from_secs(900)
};

/// 软上限:积压在这段时间内一帧都没少,说明没人在真的消化(或消化方卡住了)。
const NO_PROGRESS_WAIT: std::time::Duration = if cfg!(test) {
    std::time::Duration::from_millis(300)
} else {
    std::time::Duration::from_secs(180)
};

/// 清积压主入口。`mem = None`(屏幕记忆库不可用)时直接返回。
/// 永不返回错误——本阶段对总结而言是尽力而为的前置。
pub async fn run<S: ProgressSink>(
    pool: &DbPool,
    mem: Option<&MemoryDb>,
    sink: &S,
    source: &str,
    date: &str,
    cancel: &AtomicBool,
) {
    let Some(mem) = mem else { return };

    // 意愿 gate:常驻 OCR 关 = 用户不要自动识别,直接退——支持"截图但
    // 不用 OCR"的用户,点日报绝不被强制 OCR(手动回填不受影响)。
    match crate::repo::settings::load(pool).await {
        Ok(cfg) if cfg.memory_ocr_resident => {}
        Ok(_) => return,
        Err(e) => {
            log::warn!("OCR 清积压:设置读取失败,跳过本阶段: {e}");
            return;
        }
    }

    // 幂等回填:主库里有截图、登记簿还没登的行先补上——"全部未识别的图"
    // 以主库为准,不能只看登记簿存量。
    if let Err(e) = digest::backfill_from_activities(pool, mem).await {
        log::warn!("OCR 清积压:登记簿回填失败,跳过本阶段: {e}");
        return;
    }
    let total = match frames::count_pending(mem).await {
        Ok(0) => return, // 没积压,一个事件都不发
        Ok(n) => n,
        Err(e) => {
            log::warn!("OCR 清积压:积压计数失败,跳过本阶段: {e}");
            return;
        }
    };

    let mut p = SummaryProgress::base(
        source.to_string(),
        date.to_string(),
        "ocr_engine_starting",
        0,
    );
    p.images_total = Some(total.min(u32::MAX as u64) as u32);
    sink.emit_progress(p);
    log::info!("OCR 清积压:{total} 帧待识别,总结前先行处理");

    // 自己起的批;常驻批在跑时为 None(共同消化同一登记簿,不抢锁)
    let mut task: Option<tokio::task::JoinHandle<()>> = None;
    // 有界等待的两个计时器。没有它们时,只要 digest 的运行标志卡住(实测发生过:
    // Apple Vision 死锁 + 标志泄漏),这个循环的三个退出条件全部失效——
    // 别人的批"永远在跑"所以自己不起批,`own_batch_finished` 恒假,pending 只增不减,
    // 于是每秒轮询、每秒发一个进度事件,日报永远卡在第一阶段出不来。
    let waiting_since = std::time::Instant::now();
    let mut last_pending = u64::MAX;
    let mut last_progress_at = std::time::Instant::now();

    loop {
        if cancel.load(Ordering::Relaxed) {
            // 用户取消总结:当前批一并停(与屏幕记忆页停止按钮同语义)。
            // 只在确有批在跑时才置停止请求——digest 的停止请求会存活到
            // 下一批开头(设计如此,覆盖引擎加载窗口),无批时置位会白停
            // 之后的合法批(常驻下一轮 tick / 用户手动回填)。
            // runner 随后自会感知 cancel 并发 cancelled 事件,这里静默退。
            let own_running = task.as_ref().map(|t| !t.is_finished()).unwrap_or(false);
            if own_running || digest::is_running() {
                digest::request_stop();
            }
            break;
        }

        let pending = match frames::count_pending(mem).await {
            Ok(n) => n,
            Err(e) => {
                log::warn!("OCR 清积压:计数失败,提前结束: {e}");
                break;
            }
        };
        let done = total.saturating_sub(pending);
        let mut p = SummaryProgress::base(source.to_string(), date.to_string(), "ocr_running", 0);
        p.images_total = Some(total.min(u32::MAX as u64) as u32);
        p.image_index = Some(done.min(u32::MAX as u64) as u32);
        sink.emit_progress(p);

        if pending == 0 {
            break;
        }

        // 硬上限:无论外面发生什么,总结不能被这一阶段无限期扣住
        if waiting_since.elapsed() > MAX_WAIT {
            log::warn!(
                "OCR 清积压:等待超过 {:?} 仍剩 {pending} 帧,放弃本阶段,总结继续",
                MAX_WAIT
            );
            break;
        }
        // 软上限:积压完全不动 = 有人卡着或压根没人在消化,别陪着空转
        if pending < last_pending {
            last_pending = pending;
            last_progress_at = std::time::Instant::now();
        } else if last_progress_at.elapsed() > NO_PROGRESS_WAIT {
            log::warn!(
                "OCR 清积压:{:?} 内积压毫无进展(剩 {pending} 帧),放弃本阶段,总结继续",
                NO_PROGRESS_WAIT
            );
            break;
        }

        let own_batch_finished = task.as_ref().map(|t| t.is_finished()).unwrap_or(false);
        if own_batch_finished {
            // 自己的批结束了但积压还在:要么引擎级失败(模型缺失),要么被
            // 外部停止——两种都不该无脑重启,放弃本阶段,总结照常。
            log::warn!("OCR 清积压:消化批提前结束,剩 {pending} 帧未识别,总结继续");
            break;
        }
        if task.is_none() && !digest::is_running() {
            let m = mem.clone();
            task = Some(tokio::spawn(async move {
                match digest::run(&m).await {
                    Ok(report) => log::info!("OCR 清积压批完成: {report:?}"),
                    Err(e) => log::warn!("OCR 清积压批失败: {e}"),
                }
            }));
        }

        tokio::time::sleep(std::time::Duration::from_millis(POLL_MS)).await;
    }
    // 不 join task:pending==0 时批会因登记簿清空自行收尾并释放引擎;
    // cancel 时 request_stop 已置位,批最多一帧(~1s)后停。
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::SqliteResultExt;
    use std::sync::Mutex;

    /// 录事件的假 sink:单测断言事件序列,不碰 Tauri。
    struct RecordingSink(Mutex<Vec<SummaryProgress>>);
    impl ProgressSink for RecordingSink {
        fn emit_progress(&self, payload: SummaryProgress) {
            self.0.lock().unwrap().push(payload);
        }
    }

    async fn fixture() -> (DbPool, MemoryDb) {
        let pool = crate::repo::test_util::fresh_test_pool().await;
        let mem = crate::memory::MemoryDb::open_in_memory().await.unwrap();
        (pool, mem)
    }

    /// 打开常驻 OCR 意愿开关(gate 之后的行为都以此为前提)。
    async fn enable_resident(pool: &DbPool) {
        let mut cfg = crate::repo::settings::load(pool).await.unwrap();
        cfg.memory_ocr_resident = true;
        crate::repo::settings::save(pool, &cfg).await.unwrap();
    }

    /// 意愿 gate:常驻 OCR 关(默认)时,即使登记簿有积压也零事件零动作——
    /// "截图但不用 OCR"的用户点日报绝不被强制 OCR(本次产品修订的核心)。
    #[tokio::test]
    async fn resident_off_skips_even_with_backlog() {
        let (pool, mem) = fixture().await;
        mem.0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO frames(path, ts, local_date, ocr_state, attempts)
                     VALUES('/tmp/fake2.png', '2026-07-30T10:00:00+08:00', '2026-07-30', 0, 0)",
                    [],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
        let sink = RecordingSink(Mutex::new(Vec::new()));
        let cancel = AtomicBool::new(false);
        run(&pool, Some(&mem), &sink, "daily", "2026-07-30", &cancel).await;
        assert!(
            sink.0.lock().unwrap().is_empty(),
            "常驻关时有积压也不得发事件/起批"
        );
    }

    /// 别人的批"永远在跑"时,本阶段必须自己退出来,不能把总结无限期扣住。
    ///
    /// 复现的是实测事故的后半段:digest 的运行标志因 Vision 死锁而卡在 true 之后,
    /// 这个循环的三个退出条件同时失效(别人在跑所以自己不起批 → 自己的批恒为空 →
    /// 积压只增不减),于是每秒空转、每秒发一个进度事件,日报永远出不来。
    /// 现在由硬/软两个上限兜底,与外面的标志是否卡住无关。
    #[tokio::test]
    async fn bounded_wait_when_backlog_never_drains() {
        let (pool, mem) = fixture().await;
        enable_resident(&pool).await;
        // 一帧积压,且没有任何人会去消化它(测试环境没有识别引擎)
        mem.0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO frames(path, ts, local_date, ocr_state, attempts)
                     VALUES('/tmp/never-drains.png', '2026-07-30T10:00:00+08:00', '2026-07-30', 0, 0)",
                    [],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        let sink = RecordingSink(Mutex::new(Vec::new()));
        let cancel = AtomicBool::new(false);
        let started = std::time::Instant::now();
        run(&pool, Some(&mem), &sink, "daily", "2026-07-30", &cancel).await;

        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "必须在上限内主动退出,而不是无限等下去"
        );
        let events = sink.0.lock().unwrap().len();
        assert!(
            events < 30,
            "事件数受上限约束(实际 {events}),不会每秒刷一条到天荒地老"
        );
    }

    /// 零积压:一个事件都不发,立即返回——常驻开着的用户总结体验零变化。
    #[tokio::test]
    async fn no_backlog_emits_nothing() {
        let (pool, mem) = fixture().await;
        enable_resident(&pool).await;
        let sink = RecordingSink(Mutex::new(Vec::new()));
        let cancel = AtomicBool::new(false);
        run(&pool, Some(&mem), &sink, "daily", "2026-07-30", &cancel).await;
        assert!(sink.0.lock().unwrap().is_empty(), "零积压不应发任何事件");
    }

    /// mem 不可用(屏幕记忆没开):静默跳过。
    #[tokio::test]
    async fn missing_memory_db_is_noop() {
        let (pool, _mem) = fixture().await;
        let sink = RecordingSink(Mutex::new(Vec::new()));
        let cancel = AtomicBool::new(false);
        run(&pool, None, &sink, "daily", "2026-07-30", &cancel).await;
        assert!(sink.0.lock().unwrap().is_empty());
    }

    /// 有积压 + 立即取消:发 ocr_engine_starting 后在首轮循环感知 cancel 退出,
    /// 不会卡住总结取消链路。此时尚无消化批在跑,不得置全局停止请求——
    /// 否则会毒害同进程随后的合法批(曾令 digest 的 drain 测试随机挂)。
    #[tokio::test]
    async fn cancel_exits_promptly_after_starting_event() {
        let (pool, mem) = fixture().await;
        enable_resident(&pool).await;
        // 直接向登记簿塞一帧待识别(绕开真实截图文件)
        mem.0
            .call(|conn| {
                conn.execute(
                    "INSERT INTO frames(path, ts, local_date, ocr_state, attempts)
                     VALUES('/tmp/fake.png', '2026-07-30T10:00:00+08:00', '2026-07-30', 0, 0)",
                    [],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        let sink = RecordingSink(Mutex::new(Vec::new()));
        let cancel = AtomicBool::new(true); // 进循环即取消
        run(&pool, Some(&mem), &sink, "daily", "2026-07-30", &cancel).await;

        let events = sink.0.lock().unwrap();
        assert_eq!(events.len(), 1, "只应发 ocr_engine_starting 即退出");
        assert_eq!(events[0].phase, "ocr_engine_starting");
        assert_eq!(events[0].images_total, Some(1));
        assert_eq!(events[0].source, "daily");
    }
}
