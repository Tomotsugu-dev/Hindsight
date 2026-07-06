//! OCR 常驻模式:引擎驻留内存,新登记的帧准实时消化。
//!
//! 与批量模式([`super::digest::run`])共用同一个消化核心,差别在生命周期:
//! - 常驻:引擎加载一次挂在循环里(~400MB),每 [`TICK_SECS`] 看一眼登记簿,
//!   有积压就消化;折叠器跨 tick 存活,阅读会话不会被 tick 边界切碎。
//! - 停止:置停止标志,循环在帧间退出(最多等一帧 ~1s),引擎随任务释放。
//! - **电源纪律**(docs/design/screen-memory.md §6"插电常驻,拔电即退"):
//!   电池供电的 tick 不消化并释放引擎;接回电源后下个 tick 懒加载自动恢复、
//!   补消化积压。探测 fail-open(见 [`crate::platform::on_ac_power`])。
//!
//! 由 设置 → 是否常驻 OCR 开关控制,启动期与设置保存时同步启停。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::task::JoinHandle;

use super::digest;
use super::MemoryDb;

/// 常驻循环检查登记簿的间隔。60s:比采集间隔(30s)略缓,一个 tick 通常
/// 消化 1-2 帧,准实时且开销平滑。
const TICK_SECS: u64 = 60;

/// 常驻消化控制器——tauri managed state。start/stop 幂等。
#[derive(Default)]
pub struct ResidentOcr {
    inner: tokio::sync::Mutex<Option<Running>>,
}

struct Running {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

impl ResidentOcr {
    /// 启动常驻循环。已在跑则 no-op。引擎在首个 tick 里懒加载,
    /// 加载失败(模型下载失败/运行时缺失)只告警并在下个 tick 重试。
    pub async fn start(&self, mem: MemoryDb) {
        let mut guard = self.inner.lock().await;
        if guard.is_some() {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_task = Arc::clone(&stop);
        let handle = tokio::spawn(async move {
            let mut pipe = None;
            let mut was_on_ac = true;
            log::info!("OCR 常驻模式启动");
            loop {
                for _ in 0..TICK_SECS {
                    if stop_for_task.load(Ordering::Relaxed) {
                        log::info!("OCR 常驻模式停止,引擎释放");
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                // 电源纪律:电池供电时本 tick 不消化,释放引擎省 ~400MB;
                // 只在状态翻转时打日志,避免每分钟刷屏。
                if !crate::platform::on_ac_power() {
                    if was_on_ac {
                        log::info!("电池供电,常驻 OCR 暂停(插电后自动恢复)");
                        was_on_ac = false;
                    }
                    pipe = None;
                    continue;
                }
                if !was_on_ac {
                    log::info!("接通电源,常驻 OCR 恢复");
                    was_on_ac = true;
                }
                if pipe.is_none() {
                    match digest::Pipeline::new().await {
                        Ok(p) => pipe = Some(p),
                        Err(err) => {
                            log::warn!("常驻 OCR 引擎加载失败,下个周期重试: {err}");
                            continue;
                        }
                    }
                }
                let p = pipe.as_mut().expect("上面刚保证过已加载");
                match digest::drain(&mem, p, &stop_for_task).await {
                    Ok(_) => {}
                    // "已在运行" = 手动消化正在跑,让路即可
                    Err(e) => log::debug!("常驻消化本轮跳过: {e}"),
                }
            }
        });
        *guard = Some(Running { stop, handle });
    }

    /// 停止常驻循环并释放引擎。未在跑则 no-op。
    pub async fn stop(&self) {
        let mut guard = self.inner.lock().await;
        if let Some(running) = guard.take() {
            running.stop.store(true, Ordering::Relaxed);
            // 循环 1s 内看到标志退出;消化中最多再等一帧。不 abort——
            // 帧间退出保证不留半消化状态。
            let _ = running.handle.await;
        }
    }

    /// 按设置同步启停(启动期与设置保存时调用)。
    pub async fn sync(&self, enabled: bool, mem: Option<MemoryDb>) {
        match (enabled, mem) {
            (true, Some(db)) => self.start(db).await,
            _ => self.stop().await,
        }
    }
}
