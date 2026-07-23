//! 自动总结调度:开关(`ai.auto_summary`)打开时,后台周期性补齐
//! **昨天的日报**与**上一个完整周的周报**,用户不再需要手动点「开始总结」。
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

/// 后台调度任务。app 退出时随进程终止,无需显式停止。
pub fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(FIRST_CHECK_DELAY_SECS)).await;
        let mut attempted: HashSet<String> = HashSet::new();
        loop {
            if let Err(e) = check_once(&app, &mut attempted).await {
                log::debug!("自动总结本轮跳过: {e}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(CHECK_GAP_SECS)).await;
        }
    });
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

    let today = Local::now().date_naive();

    // ── 日报:昨天(今天还在进行中,不自动跑)──────────────
    let yesterday = today - ChronoDuration::days(1);
    let d_key = format!("d:{yesterday}");
    if !attempted.contains(&d_key)
        && daily_absent(&pool, yesterday).await?
        && has_activity(&pool, yesterday).await?
    {
        match try_run_daily(app, yesterday).await {
            RunOutcome::Ran => {
                attempted.insert(d_key);
            }
            RunOutcome::Busy => log::debug!("自动总结:手动任务进行中,日报让路"),
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

async fn try_run_daily(app: &AppHandle, date: NaiveDate) -> RunOutcome {
    let run_lock = app.state::<RunLock>();
    let Ok(_guard) = run_lock.0.try_lock() else {
        return RunOutcome::Busy;
    };
    log::info!("自动总结:生成 {date} 日报");
    let cancel = app.state::<SummaryCancel>();
    cancel.0.store(false, Ordering::Relaxed);
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
        .run("daily", date, DeviceFilter::All, false, None)
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

/// 该日主库是否有活动记录(零活动的日子没有可总结的东西)。
async fn has_activity(pool: &DbPool, date: NaiveDate) -> Result<bool> {
    let key = date.format("%Y-%m-%d").to_string();
    pool.0
        .call(move |conn| {
            let n: i64 = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM activities WHERE local_date = ?1)",
                    rusqlite::params![key],
                    |r| r.get(0),
                )
                .db()?;
            Ok(n > 0)
        })
        .await
        .map_err(Into::into)
}

fn align_to_monday(d: NaiveDate) -> NaiveDate {
    use chrono::Datelike;
    d - ChronoDuration::days(d.weekday().num_days_from_monday() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

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
