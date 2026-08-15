//! 自动总结调度:开关(`ai.auto_summary`)打开时,每天到设定时刻自动生成
//! **当天的日报**(并补齐缺失的前一天)与**上一个完整周的周报**,
//! 用户不再需要手动点「开始总结」。
//!
//! 设计要点:
//! - 启动后延迟首查(让采集/引擎先安顿),此后每 [`CHECK_GAP_SECS`] 查一轮;
//! - 只补"从未生成过"的目标:跑过但失败的不自动重试——失败通常是配置问题
//!   (坏 key / 模型缺失),半小时一次的盲目重试只会烧钱刷日志,留给用户手动;
//! - 每个目标在本次进程运行期至多尝试一次(attempted 集合);
//! - 与手动运行共用 [`RunLock`]:抢不到锁(用户正在手动跑)本轮让路,
//!   不标记已尝试,下轮再来;
//! - 月报:生成器尚未实现(MonthlyTab 为占位页),落地后在此接入。

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Local, NaiveDate};
use tauri::{AppHandle, Manager};

use crate::ai::summary::{precheck_week, DaySummaryRunner, WeekSummaryRunner, WEEKLY_SOURCE};
use crate::commands::ai_summary::{RunLock, SummaryCancel};
use crate::error::Result;
use crate::repo::reports::DeviceFilter;
use crate::repo::{ai_summaries, settings};
use crate::storage::{DbPool, SqliteResultExt};

/// 启动后首查延迟(秒):避开启动期的采集初始化/引擎自检。
const FIRST_CHECK_DELAY_SECS: u64 = 120;
/// 常规检查间隔(秒)。目标是"日/周结束后不久自动补上",半小时粒度足够。
const CHECK_GAP_SECS: u64 = 30 * 60;
/// 设定时间点过后多久醒来检查(秒)。醒得太贴点,同分钟内 now >= t
/// 可能差几秒;90s 稳过点,又不至于让用户觉得"到点没动静"。
const POINT_WAKE_BUFFER_SECS: u64 = 90;

/// 后台调度任务。app 退出时随进程终止,无需显式停止。
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(FIRST_CHECK_DELAY_SECS)).await;
        let mut attempted: HashSet<String> = HashSet::new();
        loop {
            if let Err(e) = check_once(&app, &mut attempted).await {
                log::debug!("自动总结本轮跳过: {e}");
            }
            let gap = next_gap_secs(&app).await;
            tokio::time::sleep(std::time::Duration::from_secs(gap)).await;
        }
    });
}

/// 下一轮检查前的睡眠秒数。常规 30 分钟;但若某个设定时间点更近,就睡到
/// 该点过后 [`POINT_WAKE_BUFFER_SECS`] 再查——纯 30 分钟盲睡叠加每轮生成
/// 耗时的漂移,实测能把 23:00 的设定点拖到 23:30 才触发(用户 23:19 来问
/// "怎么还没动")。读设置失败或尽快模式(无设定点)回落常规间隔。
async fn next_gap_secs(app: &AppHandle) -> u64 {
    let pool = app.state::<DbPool>();
    let Ok(cfg) = settings::load(&pool).await else {
        return CHECK_GAP_SECS;
    };
    if !cfg.ai.auto_summary {
        return CHECK_GAP_SECS;
    }
    match secs_until_next_point(Local::now(), &cfg.ai.effective_auto_summary_times()) {
        Some(d) => (d + POINT_WAKE_BUFFER_SECS).clamp(60, CHECK_GAP_SECS),
        None => CHECK_GAP_SECS,
    }
}

/// 距下一个设定时间点的秒数;点在今天已过则算到明天同一点。
/// 空表 / 全坏格式返回 None(尽快模式)。
fn secs_until_next_point(now: chrono::DateTime<Local>, times: &[String]) -> Option<u64> {
    times
        .iter()
        .filter_map(|raw| chrono::NaiveTime::parse_from_str(raw.trim(), "%H:%M").ok())
        .map(|t| {
            let mut delta = (now.date_naive().and_time(t) - now.naive_local()).num_seconds();
            if delta <= 0 {
                delta += 86_400;
            }
            delta as u64
        })
        .min()
}

async fn check_once(app: &AppHandle, attempted: &mut HashSet<String>) -> Result<()> {
    let pool = app.state::<DbPool>();
    let cfg = settings::load(&pool).await?;
    if !cfg.ai.auto_summary {
        return Ok(());
    }
    // AI 未配置(既无云端也没选本地模型)时静默跳过——开关先于配置打开是合法状态
    if !cfg.ai.summary_use_cloud() && cfg.ai.effective_summary_main().trim().is_empty() {
        log::debug!("自动总结:AI 未配置,跳过");
        return Ok(());
    }

    let now = Local::now();
    let today = now.date_naive();
    let yesterday = today - ChronoDuration::days(1);
    let times = cfg.ai.effective_auto_summary_times();

    // 定时模式:任一时间点到点即放行本轮;全部未到则整体跳过。
    // 未配置任何点 = 尽快模式(旧行为:只补前一天,不动进行中的今天)。
    if !times.is_empty() && !any_point_due(now.time(), &times) {
        log::debug!("自动总结:未到任何设定时间({}),本轮跳过", times.join("/"));
        return Ok(());
    }

    // ── 日报·前一天补漏:只补"从未生成过"的(关机错过设定时刻的场景) ──
    let d_key = format!("d:{yesterday}");
    if !attempted.contains(&d_key)
        && daily_absent(&pool, yesterday).await?
        && has_activity(&pool, yesterday).await?
    {
        match try_run_daily(app, yesterday, false).await {
            RunOutcome::Ran => {
                attempted.insert(d_key);
            }
            RunOutcome::Busy => log::debug!("自动总结:手动任务进行中,日报让路"),
        }
    }

    // ── 日报·当天(仅定时模式):**每个时间点都生成/刷新一次**——
    //    12:00 出半天版、23:00 覆盖成全天版。账本 = 日报自己的 generated_at:
    //    生成时刻 ≥ 时间点即该点已完成,重启不重复、不白烧 LLM。
    //    今天没活动(如凌晨点)自然跳过;失败由 attempted 兜住当天不重试。
    if !times.is_empty() && has_activity(&pool, today).await? {
        let last_gen =
            ai_summaries::latest_generated_at(&pool, "daily", &today.to_string()).await?;
        for t in due_unsatisfied_points(now, &times, last_gen.as_deref()) {
            let key = format!("d:{today}@{t}");
            if attempted.contains(&key) {
                continue;
            }
            // 已有报告则带 force 刷新覆盖;还没有就普通生成
            let force = !daily_absent(&pool, today).await?;
            match try_run_daily(app, today, force).await {
                RunOutcome::Ran => {
                    attempted.insert(key);
                }
                RunOutcome::Busy => {
                    log::debug!("自动总结:手动任务进行中,日报让路");
                    break;
                }
            }
            // 一轮只处理一个点:生成本身要跑几分钟,后续点下轮按账本自然判定
            break;
        }
    }

    // ── 周报:上一个完整周的周一 ───────────────────────
    let last_monday = align_to_monday(today) - ChronoDuration::days(7);
    let w_key = format!("w:{last_monday}");
    if !attempted.contains(&w_key) && weekly_absent(&pool, last_monday).await? {
        let pre = precheck_week(&pool, last_monday).await?;
        // 整周零日报不硬跑:没有叙事材料的周报只是活动统计空壳;
        // 等日报(手动或上面的自动)补上后,同一运行期内下轮再试。
        if pre.days_with_daily > 0 {
            match try_run_weekly(app, last_monday, pre.days_with_daily < 7).await {
                RunOutcome::Ran => {
                    attempted.insert(w_key);
                }
                RunOutcome::Busy => log::debug!("自动总结:手动任务进行中,周报让路"),
            }
        }
    }
    Ok(())
}

enum RunOutcome {
    /// 实际启动过(无论成败——失败也不再自动重试)
    Ran,
    /// RunLock 被手动任务占用,本轮未启动
    Busy,
}

async fn try_run_daily(app: &AppHandle, date: NaiveDate, force_refresh: bool) -> RunOutcome {
    let run_lock = app.state::<RunLock>();
    let Ok(_guard) = run_lock.0.try_lock() else {
        return RunOutcome::Busy;
    };
    log::info!("自动总结:生成 {date} 日报");
    let cancel = app.state::<SummaryCancel>();
    cancel.0.store(false, Ordering::Relaxed);
    // 与手动路径同款:先清 OCR 积压(前端若开着总结页,同样能看到阶段进度)
    let mem = app.state::<crate::commands::screen_memory::MemoryState>();
    crate::ai::ocr_catchup::run(
        &app.state::<DbPool>(),
        mem.0.as_ref(),
        app,
        "daily",
        &date.format("%Y-%m-%d").to_string(),
        &cancel.0,
    )
    .await;
    let runner = DaySummaryRunner::new(
        app.state::<DbPool>().inner().clone(),
        Arc::clone(
            app.state::<Arc<crate::ai::server::EngineSupervisor>>()
                .inner(),
        ),
        app.clone(),
        Arc::clone(&cancel.0),
    );
    if let Err(e) = runner
        .run("daily", date, DeviceFilter::All, force_refresh, None)
        .await
    {
        log::warn!("自动总结:{date} 日报失败(本次运行期不再自动重试): {e}");
    }
    RunOutcome::Ran
}

async fn try_run_weekly(app: &AppHandle, monday: NaiveDate, allow_missing: bool) -> RunOutcome {
    let run_lock = app.state::<RunLock>();
    let Ok(_guard) = run_lock.0.try_lock() else {
        return RunOutcome::Busy;
    };
    log::info!("自动总结:生成 {monday} 起始周的周报(缺日容忍={allow_missing})");
    let cancel = app.state::<SummaryCancel>();
    cancel.0.store(false, Ordering::Relaxed);
    let mem = app.state::<crate::commands::screen_memory::MemoryState>();
    crate::ai::ocr_catchup::run(
        &app.state::<DbPool>(),
        mem.0.as_ref(),
        app,
        WEEKLY_SOURCE,
        &monday.format("%Y-%m-%d").to_string(),
        &cancel.0,
    )
    .await;
    let runner = WeekSummaryRunner::new(
        app.state::<DbPool>().inner().clone(),
        Arc::clone(
            app.state::<Arc<crate::ai::server::EngineSupervisor>>()
                .inner(),
        ),
        app.clone(),
        Arc::clone(&cancel.0),
    );
    if let Err(e) = runner.run(monday, false, allow_missing).await {
        log::warn!("自动总结:{monday} 周报失败(本次运行期不再自动重试): {e}");
    }
    RunOutcome::Ran
}

/// 该日是否**从未**生成过日报(任何状态的行都算"生成过",失败行留给用户处置)。
async fn daily_absent(pool: &DbPool, date: NaiveDate) -> Result<bool> {
    let rows = ai_summaries::get_day(pool, "daily", &date.format("%Y-%m-%d").to_string()).await?;
    Ok(rows.is_empty())
}

async fn weekly_absent(pool: &DbPool, monday: NaiveDate) -> Result<bool> {
    let rows =
        ai_summaries::get_day(pool, WEEKLY_SOURCE, &monday.format("%Y-%m-%d").to_string()).await?;
    Ok(rows.is_empty())
}

/// 该日主库是否有活动记录(零活动的日子没有可总结的东西;
/// excluded 行不算——全被忽略规则打标的日子同样没东西可总结)。
async fn has_activity(pool: &DbPool, date: NaiveDate) -> Result<bool> {
    let key = date.format("%Y-%m-%d").to_string();
    pool.0
        .call(move |conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM activities WHERE local_date = ?1 AND excluded = 0)",
                    rusqlite::params![key],
                    |r| r.get(0),
                )
                .db()?;
            Ok(n > 0)
        })
        .await
        .map_err(Into::into)
}

/// 任一时间点已到(纯函数):放行本轮检查。坏格式项跳过。
fn any_point_due(now: chrono::NaiveTime, times: &[String]) -> bool {
    times.iter().any(|raw| {
        chrono::NaiveTime::parse_from_str(raw.trim(), "%H:%M")
            .map(|t| now >= t)
            .unwrap_or(false)
    })
}

/// 已到点且**尚未被满足**的时间点(纯函数,配置序):
/// 满足 = 当天日报最近一次 generated_at(本地时刻)≥ 该时间点。
/// last_gen 解析失败按已满足处理(宁可漏刷一版,不无限重烧 LLM)。
fn due_unsatisfied_points(
    now: chrono::DateTime<chrono::Local>,
    times: &[String],
    last_gen: Option<&str>,
) -> Vec<String> {
    let last_local: Option<chrono::NaiveDateTime> = last_gen.map(|raw| {
        chrono::DateTime::parse_from_rfc3339(raw)
            .map(|dt| dt.with_timezone(&chrono::Local).naive_local())
            .unwrap_or(chrono::NaiveDateTime::MAX)
    });
    let today = now.date_naive();
    times
        .iter()
        .filter(|raw| {
            let Ok(t) = chrono::NaiveTime::parse_from_str(raw.trim(), "%H:%M") else {
                return false;
            };
            if now.time() < t {
                return false; // 未到点
            }
            let point_dt = today.and_time(t);
            match last_local {
                Some(gen) => gen < point_dt, // 生成早于该点 → 该点未满足
                None => true,                // 从未生成 → 未满足
            }
        })
        .cloned()
        .collect()
}

fn align_to_monday(d: NaiveDate) -> NaiveDate {
    use chrono::Datelike;
    d - ChronoDuration::days(d.weekday().num_days_from_monday() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// any_point_due:任一到点即放行;坏格式跳过;空表不放行(尽快模式走别的分支)。
    #[test]
    fn any_point_due_truth_table() {
        let t = |h, m| chrono::NaiveTime::from_hms_opt(h, m, 0).unwrap();
        let v = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!any_point_due(t(23, 59), &[]));
        assert!(any_point_due(t(12, 0), &v(&["12:00", "23:00"])));
        assert!(!any_point_due(t(11, 59), &v(&["12:00", "23:00"])));
        assert!(any_point_due(t(23, 0), &v(&["12:00", "23:00"])));
        assert!(!any_point_due(t(12, 0), &v(&["25:99"])));
        assert!(any_point_due(t(9, 30), &v(&[" 09:30 "])));
    }

    /// secs_until_next_point:点前给到点差值(轮询据此收紧),点后滚到
    /// 明天,多点取最近,空表/坏格式回 None(尽快模式保持 30 分钟常规间隔)。
    #[test]
    fn secs_until_next_point_math() {
        use chrono::TimeZone;
        let v = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let at = |h, m| chrono::Local.with_ymd_and_hms(2026, 8, 8, h, m, 0).unwrap();

        // 22:30 → 23:00 点差 30 分钟
        assert_eq!(
            secs_until_next_point(at(22, 30), &v(&["23:00"])),
            Some(1800)
        );
        // 23:10 → 今天的点已过,滚到明天 23:00
        assert_eq!(
            secs_until_next_point(at(23, 10), &v(&["23:00"])),
            Some(24 * 3600 - 600)
        );
        // 多点取最近的
        assert_eq!(
            secs_until_next_point(at(11, 0), &v(&["12:00", "23:00"])),
            Some(3600)
        );
        // 空表 / 坏格式 = 尽快模式
        assert_eq!(secs_until_next_point(at(11, 0), &[]), None);
        assert_eq!(secs_until_next_point(at(11, 0), &v(&["25:99"])), None);
    }

    /// due_unsatisfied_points:账本(当天日报 generated_at)判定——
    /// 生成时刻 ≥ 时间点 = 该点已完成;从未生成全未满足;解析失败视为满足。
    #[test]
    fn due_unsatisfied_points_ledger() {
        use chrono::TimeZone;
        let v = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let now = chrono::Local
            .with_ymd_and_hms(2026, 8, 2, 23, 10, 0)
            .unwrap();
        let times = v(&["12:00", "23:00"]);

        // 从未生成:两点都到期且未满足
        assert_eq!(due_unsatisfied_points(now, &times, None), times);

        // 12:05 生成过:12:00 点已满足,23:00 点未满足
        let gen_1205 = chrono::Local
            .with_ymd_and_hms(2026, 8, 2, 12, 5, 0)
            .unwrap()
            .to_rfc3339();
        assert_eq!(
            due_unsatisfied_points(now, &times, Some(&gen_1205)),
            v(&["23:00"])
        );

        // 23:05 生成过:全部满足
        let gen_2305 = chrono::Local
            .with_ymd_and_hms(2026, 8, 2, 23, 5, 0)
            .unwrap()
            .to_rfc3339();
        assert!(due_unsatisfied_points(now, &times, Some(&gen_2305)).is_empty());

        // 未到点的不入列:12:30 时 23:00 还没到
        let noon = chrono::Local
            .with_ymd_and_hms(2026, 8, 2, 12, 30, 0)
            .unwrap();
        assert_eq!(due_unsatisfied_points(noon, &times, None), v(&["12:00"]));

        // 昨天生成的不算满足今天的点
        let gen_yday = chrono::Local
            .with_ymd_and_hms(2026, 8, 1, 23, 5, 0)
            .unwrap()
            .to_rfc3339();
        assert_eq!(due_unsatisfied_points(now, &times, Some(&gen_yday)), times);

        // 时间戳解析失败:视为满足(不无限重烧 LLM)
        assert!(due_unsatisfied_points(now, &times, Some("垃圾时间戳")).is_empty());

        // 坏格式时间点跳过
        assert!(due_unsatisfied_points(now, &v(&["25:99"]), None).is_empty());
    }

    #[test]
    fn align_to_monday_covers_week() {
        let mon = NaiveDate::from_ymd_opt(2026, 7, 20).unwrap(); // 周一
        for off in 0..7 {
            assert_eq!(align_to_monday(mon + ChronoDuration::days(off)), mon);
        }
        assert_eq!(
            align_to_monday(mon - ChronoDuration::days(1)),
            NaiveDate::from_ymd_opt(2026, 7, 13).unwrap()
        );
    }
}
