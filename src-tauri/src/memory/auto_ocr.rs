//! OCR 按需模式:平时零占用,积压攒够且机器有余力时起引擎清一批,完了释放。
//!
//! 与常驻模式([`super::resident`])的差别在生命周期:常驻把引擎挂在内存里
//! (~400MB)换准实时;按需只在"值得干且不打扰人"的窗口里干,内存只在批内占用,
//! 代价是索引晚一些。设置里三态互斥:关闭 / 自动(本模块)/ 常驻。
//!
//! 两档触发,每 [`TICK_SECS`] 评估一次:
//! - 常规档:积压 ≥ [`BACKLOG_NORMAL`]——人在用电脑,但积压太大也该清了;
//! - 空闲档:积压 ≥ [`BACKLOG_IDLE`],且(锁屏/息屏 或 键鼠空闲 ≥
//!   [`IDLE_SECS`])——人不在,门槛减半提前干。
//!
//! 两档共同的资源门:接通电源、前台不是游戏/影音分类(OCR 走 DirectML,
//! 会抢显卡)、CPU < 50%、可用内存 > 3GB。资源探测一律 fail-open(探测失败
//! 按通过)——让路是优化,探测异常不能卡死消化,与电源探测同原则
//! ([`crate::platform::on_ac_power`])。
//!
//! 批进行中另起 watchdog 每 [`WATCHDOG_SECS`] 复查:电源/前台/内存是硬信号,
//! 违反即置停止标志,消化循环帧间(~1s)让路;CPU 是尖峰型信号,连续
//! [`CPU_STRIKES_TO_STOP`] 次超限(≈15 秒)才停,打开个软件的瞬时冲高不误伤。
//! "判断失误开了工"的代价由此归零。空闲档的空闲条件不复查:人回来后
//! CPU/前台门自然接管,轻度使用时让批把尾巴干完。
//!
//! **批内阈值与开工阈值必须分开**:整机采样分不出负载来源,批内读数含 OCR
//! 自己。沿用开工线会让引擎把自己判死——停批、卸引擎、下轮开工门(读数已
//! 不含自己)又放行,退化成每分钟白装卸一次 400MB。内存靠把引擎占用加回读数
//! 对齐口径,CPU 靠单独的批内上限,见常量区。
//!
//! **暂时只在 Windows 启用**([`AutoOcr::sync`] 收口,前端同步隐藏该选项):
//! - Linux:[`crate::platform`] 的 `idle_secs` / `screen_unavailable` /
//!   `on_ac_power` 全是 stub(恒 0 / false / true),空闲档永远不会触发、
//!   电源门形同虚设,笔记本拔电照跑;
//! - macOS:OCR 走系统 Vision(ANE),没有 ~400MB 的常驻引擎,
//!   [`ENGINE_MEM_MB`] 的迟滞补偿与"避开游戏防抢显卡"的前提都不成立,
//!   阈值要重新标定才谈得上合适。
//!
//! 两个平台都是"参数与前提要重新标定"而非"实现不了";补齐后去掉门控即可。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::task::JoinHandle;

use super::{digest, frames, MemoryDb};
use crate::storage::DbPool;

/// 调度评估间隔。与常驻模式同节奏,足够及时且开销可忽略。
const TICK_SECS: u64 = 60;
/// 常规档积压阈值(帧)。
const BACKLOG_NORMAL: u64 = 800;
/// 空闲档积压阈值(帧)。
const BACKLOG_IDLE: u64 = 400;
/// 空闲档的键鼠空闲判定(秒)。
const IDLE_SECS: u64 = 15 * 60;
/// CPU 占用上限(百分比):开工前判"除 OCR 外别人忙不忙"。
const CPU_MAX_PERCENT: f32 = 50.0;
/// 批进行中的 CPU 上限。必须比开工线宽:整机采样分不出负载来源,批内读数
/// 含 OCR 自己(DirectML 下推理在 GPU,CPU 侧仍有单线程的解码/缩放/CTC,
/// 四核机上约抬高 25 个百分点)。沿用开工线等于让引擎把自己判死——停批、
/// 卸引擎、下轮开工门(读数已不含自己)又放行,退化成每分钟白装卸一次。
const CPU_MAX_PERCENT_IN_BATCH: f32 = CPU_MAX_PERCENT + 30.0;
/// 可用内存下限(MB)。口径是"**除 OCR 引擎之外**系统还剩多少":开工前引擎
/// 未加载,量的就是它;批中引擎已占着 [`ENGINE_MEM_MB`],读数要加回去再比,
/// 两处才是同一件事。否则引擎自身就能把门顶破(理由同上一条)。
const MEM_MIN_MB: u64 = 3 * 1024;
/// OCR 引擎加载后的常驻内存量(MB),批中复查内存时加回这一份。
const ENGINE_MEM_MB: u64 = 400;
// 批内阈值必须比开工阈值宽松,否则引擎会把自己判死(见模块头注释)。
// 编译期钉死,防以后有人把两组数调回同一条线。
const _: () = assert!(CPU_MAX_PERCENT_IN_BATCH > CPU_MAX_PERCENT);
const _: () = assert!(ENGINE_MEM_MB > 0 && ENGINE_MEM_MB < MEM_MIN_MB);

/// 批进行中资源门复查间隔(秒)。
const WATCHDOG_SECS: u64 = 5;
/// 批进行中 CPU 连续超限这么多次(× WATCHDOG_SECS ≈ 15 秒)才停批:
/// CPU 是尖峰型信号,打开个软件就能瞬间冲高、两秒回落,单次采样命中尖峰
/// 就停批的代价是引擎白白卸载重载;持续高负载才是真要让路的对象。
/// 电源/前台/内存不设连击——它们不是抖动型信号,违反即停。
const CPU_STRIKES_TO_STOP: u32 = 3;

/// 按需消化控制器——tauri managed state。start/stop 幂等。
#[derive(Default)]
pub struct AutoOcr {
    inner: tokio::sync::Mutex<Option<Running>>,
}

struct Running {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

/// 前台应用是否属于游戏/影音分类——识别抢显卡的主冲突面。
/// 走 组→分类 链:分类 id 是 game/video、v31 前的遗留娱乐分类 fun(内置规则
/// 至今仍把游戏与播放器归到它,如 Steam),或挂在「娱乐」大类(play)下的自定义
/// 分类,都算。未分组/未分类/探测失败按"不是"处理(fail-open)。
async fn foreground_is_av(pool: &DbPool) -> bool {
    let Ok(win) = crate::capture::window::current_window() else {
        return false;
    };
    let name = win.app_name;
    pool.0
        .call(move |conn| {
            let hit: i64 = conn.query_row(
                "SELECT COUNT(*) FROM app_group_members gm
                 JOIN app_groups g ON g.id = gm.group_id AND g.deleted_at IS NULL
                 LEFT JOIN categories c ON c.id = g.category_id AND c.deleted_at IS NULL
                 WHERE gm.process_name = ?1 AND gm.deleted_at IS NULL
                   AND (g.category_id IN ('game','video','fun')
                        OR c.super_category_id = 'play')",
                rusqlite::params![name],
                |r| r.get(0),
            )?;
            Ok(hit > 0)
        })
        .await
        .unwrap_or(false)
}

/// 硬信号门:电源/前台分类/内存——非抖动型,单次判定即生效。
/// `engine_loaded_mb` = 调用时引擎已占的内存(开工前 0、批中 [`ENGINE_MEM_MB`]),
/// 加回读数后两处比的都是"除引擎外还剩多少"(见 [`MEM_MIN_MB`])。
/// CPU 单独走 [`cpu_busy`](阈值分开,批中还有连击缓冲)。
async fn hard_gates_pass(pool: &DbPool, engine_loaded_mb: u64) -> bool {
    if !crate::platform::on_ac_power() {
        return false;
    }
    if foreground_is_av(pool).await {
        return false;
    }
    if let Some(mem_mb) = crate::platform::available_memory_mb() {
        if mem_mb + engine_loaded_mb <= MEM_MIN_MB {
            return false;
        }
    }
    true
}

/// CPU 是否超过给定上限(采样窗见 [`crate::platform::cpu_usage_percent`];
/// 探测失败按不超限,fail-open)。
async fn cpu_busy(limit: f32) -> bool {
    matches!(
        crate::platform::cpu_usage_percent().await,
        Some(cpu) if cpu >= limit
    )
}

/// 空闲档的"人不在"判定:锁屏/息屏是硬信号,键鼠空闲是软信号。
fn user_away() -> bool {
    crate::platform::screen_unavailable() || crate::platform::idle_secs() >= IDLE_SECS
}

/// 本 tick 是否应该开工(纯判定,便于单测):两档阈值 × 是否空闲。
fn backlog_fires(pending: u64, away: bool) -> bool {
    pending >= BACKLOG_NORMAL || (away && pending >= BACKLOG_IDLE)
}

impl AutoOcr {
    /// 启动调度循环。已在跑则 no-op。引擎只在触发时加载,批后随作用域释放。
    pub async fn start(&self, mem: MemoryDb, pool: DbPool) {
        let mut guard = self.inner.lock().await;
        if guard.is_some() {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_task = Arc::clone(&stop);
        let handle = tokio::spawn(async move {
            log::info!("OCR 按需模式启动");
            loop {
                for _ in 0..TICK_SECS {
                    if stop_for_task.load(Ordering::Relaxed) {
                        log::info!("OCR 按需模式停止");
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                let pending = match frames::pending_count(&mem).await {
                    Ok(n) => n,
                    Err(e) => {
                        log::warn!("按需 OCR 查积压失败,下个周期重试: {e}");
                        continue;
                    }
                };
                let away = user_away();
                log::debug!("按需 OCR 本轮评估:积压 {pending} 帧,空闲 {away}");
                // 手动批/常驻批持锁时先让路:drain 的单实例互斥要等引擎装完才撞上,
                // 白装一次 ~400MB 不划算。这是优化性短路不是互斥,残余竞态由
                // drain 自己的 RUNNING 兜底(下面的 Err 分支)。
                // 开工前 CPU 单次判定即可:误判尖峰的代价只是推迟到下一轮评估
                if !backlog_fires(pending, away)
                    || digest::is_running()
                    || !hard_gates_pass(&pool, 0).await
                    || cpu_busy(CPU_MAX_PERCENT).await
                {
                    continue;
                }
                log::info!("按需 OCR 触发:积压 {pending} 帧,加载引擎开始消化");
                let mut pipe = match digest::Pipeline::new().await {
                    Ok(p) => p,
                    Err(e) => {
                        log::warn!("按需 OCR 引擎加载失败,下个周期重试: {e}");
                        continue;
                    }
                };
                // 批内断流两路合一:外部 stop(设置切换/退出)与 watchdog(资源门
                // 破坏)都置 batch_stop,drain 帧间感知、干净收尾。
                let batch_stop = Arc::new(AtomicBool::new(false));
                let watchdog = tokio::spawn({
                    let batch_stop = Arc::clone(&batch_stop);
                    let outer_stop = Arc::clone(&stop_for_task);
                    let pool = pool.clone();
                    async move {
                        // CPU 连击计数:连续超限达到停止线才让路,单次尖峰清零重来
                        let mut cpu_strikes = 0u32;
                        loop {
                            tokio::time::sleep(std::time::Duration::from_secs(WATCHDOG_SECS)).await;
                            if batch_stop.load(Ordering::Relaxed) {
                                return; // 批已收尾,watchdog 退场
                            }
                            // 批中读数含引擎自己,内存把它加回去比(见 MEM_MIN_MB)
                            if outer_stop.load(Ordering::Relaxed)
                                || !hard_gates_pass(&pool, ENGINE_MEM_MB).await
                            {
                                log::info!("按需 OCR 让路:运行条件不再满足,停止本批");
                                batch_stop.store(true, Ordering::Relaxed);
                                return;
                            }
                            cpu_strikes = if cpu_busy(CPU_MAX_PERCENT_IN_BATCH).await {
                                cpu_strikes + 1
                            } else {
                                0
                            };
                            if cpu_strikes >= CPU_STRIKES_TO_STOP {
                                log::info!(
                                    "按需 OCR 让路:CPU 持续超过 {CPU_MAX_PERCENT_IN_BATCH}% 约 {} 秒,停止本批",
                                    CPU_STRIKES_TO_STOP as u64 * WATCHDOG_SECS
                                );
                                batch_stop.store(true, Ordering::Relaxed);
                                return;
                            }
                        }
                    }
                });
                match digest::drain(&mem, &mut pipe, &batch_stop).await {
                    Ok(r) => log::info!(
                        "按需 OCR 本批结束:处理 {} 帧,失败 {},缺文件 {}",
                        r.processed,
                        r.failed,
                        r.skipped_missing_file
                    ),
                    // "已在运行" = 手动/常驻批持锁,让路即可
                    Err(e) => log::debug!("按需消化本轮跳过: {e}"),
                }
                batch_stop.store(true, Ordering::Relaxed);
                let _ = watchdog.await;
                // pipe 在此 drop——引擎随批释放,回到零占用
            }
        });
        *guard = Some(Running { stop, handle });
    }

    /// 停止调度循环。空转时 ~1s 退出;批进行中要等 watchdog 下次复查发现
    /// 外部停止(≤5s)再加一帧收尾,最坏约 6-7s。未在跑则 no-op。
    pub async fn stop(&self) {
        let mut guard = self.inner.lock().await;
        if let Some(running) = guard.take() {
            running.stop.store(true, Ordering::Relaxed);
            let _ = running.handle.await;
        }
    }

    /// 按设置同步启停(启动期与设置保存时调用)。常驻模式优先:两者都开时
    /// 由调用方传 enabled=false 关掉本模块。
    pub async fn sync(&self, enabled: bool, mem: Option<MemoryDb>, pool: DbPool) {
        // 非 Windows 一律不启(理由见模块头注释)。收口在这一处:启动期与设置
        // 保存两条路径都经过它。前端已隐藏该选项,这里兜住"设置从别处被写开"
        // (手动改库/拷贝数据目录)的情况。
        let enabled = enabled && cfg!(target_os = "windows");
        match (enabled, mem) {
            (true, Some(db)) => self.start(db, pool).await,
            _ => self.stop().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backlog_two_tiers() {
        // 常规档:人在,800 起步
        assert!(!backlog_fires(799, false));
        assert!(backlog_fires(800, false));
        // 空闲档:人不在,400 起步
        assert!(!backlog_fires(399, true));
        assert!(backlog_fires(400, true));
        // 空闲档不影响常规档下限
        assert!(!backlog_fires(400, false));
    }
}
