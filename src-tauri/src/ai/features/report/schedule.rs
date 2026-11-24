//! 自动生成日报与周报的后台定时调度。
//!
//! 开启 `ai.auto_summary` 后，后台定期检查，补这三样：
//! - 昨天的日报：缺失或有失败段时补跑，已成功的段不重跑；
//! - 当天的日报：只在设了时间点时生成，每个已到的时间点生成或刷新一次；
//! - 上一个完整周的周报：缺失且该周至少有一份日报时生成。
//!
//! 设了时间点时，要有一个点已到，这一轮才会做上面的事。
//!
//! 每个目标在本次进程运行期内至多尝试一次（`attempted` 集合），避免配置错误导致反复重试；
//! 当 [`RunLock`] 或本机引擎正忙时让路，不记为已尝试，下轮再试。

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Local, NaiveDate};
use tauri::{AppHandle, Manager};

use crate::ai::features::report::{
    precheck_week, DaySummaryRunner, WeekSummaryRunner, DAILY_SOURCE, WEEKLY_SOURCE,
};
use crate::ai::policy::{ProfileChoice, Report};
use crate::commands::ai_summary::{RunLock, SummaryCancel};
use crate::error::{Error, Result};
use crate::repo::reports::DeviceFilter;
use crate::repo::{ai_summaries, settings};
use crate::storage::{DbPool, SqliteResultExt};

/// 应用启动后首次检查前的延迟（秒），避开启动期初始化。
const FIRST_CHECK_DELAY_SECS: u64 = 120;
/// 常规轮询检查间隔（秒）。
const CHECK_GAP_SECS: u64 = 30 * 60;
/// 设定时间点过后附加的缓冲（秒），确保唤醒时稳定跨过目标分钟。
const SCHEDULE_WAKE_BUFFER_SECS: u64 = 90;

/// 启动后台调度任务，随应用进程退出自动结束。
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

/// 计算距离下一次检查的等待秒数：默认 [`CHECK_GAP_SECS`]，若有更近的设定时间点则提前在该点后唤醒。
async fn next_gap_secs(app: &AppHandle) -> u64 {
    let pool = app.state::<DbPool>();
    let Ok(cfg) = settings::load(&pool).await else {
        return CHECK_GAP_SECS;
    };
    if !cfg.ai.auto_summary {
        return CHECK_GAP_SECS;
    }
    match secs_until_next_scheduled_time(Local::now(), &cfg.ai.auto_summary_times) {
        Some(d) => (d + SCHEDULE_WAKE_BUFFER_SECS).clamp(60, CHECK_GAP_SECS),
        None => CHECK_GAP_SECS,
    }
}

/// 返回距离最近一个有效 `"HH:MM"` 设定时间点的秒数（今天已过则滚到明天）；无有效配置返回 `None`。
fn secs_until_next_scheduled_time(now: chrono::DateTime<Local>, times: &[String]) -> Option<u64> {
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
    // Skip silently when no daily model is configured yet.
    if !cfg.ai.is_ready_for(crate::ai::config::Task::Daily) {
        log::debug!("自动总结:AI 未配置,跳过");
        return Ok(());
    }

    let now = Local::now();
    let today = now.date_naive();
    let yesterday = today - ChronoDuration::days(1);
    let times = &cfg.ai.auto_summary_times;

    // 定时模式下至少有一个设定时间点已到才放行本轮
    if !times.is_empty() && !any_scheduled_time_due(now.time(), times) {
        log::debug!("自动总结:未到任何设定时间({}),本轮跳过", times.join("/"));
        return Ok(());
    }

    // 昨天日报：缺失或残留失败段时补跑一次（force = false，只重跑未 ok 的段）
    let d_key = format!("d:{yesterday}");
    if !attempted.contains(&d_key)
        && (daily_absent(&pool, yesterday).await?
            || ai_summaries::has_error_segment(&pool, DAILY_SOURCE, &yesterday.to_string()).await?)
        && has_activity(&pool, yesterday).await?
    {
        match try_run_daily(app, yesterday, false).await {
            RunOutcome::Ran => {
                attempted.insert(d_key);
            }
            RunOutcome::Busy => log::debug!("自动总结:别的 AI 任务进行中,日报让路"),
        }
    }

    // 当天日报（仅定时模式）：按已到点且未满足的时间点逐个生成或刷新
    if !times.is_empty() && has_activity(&pool, today).await? {
        // 有失败段时不看生成时间，每个已到的时间点都再跑一次
        let has_err =
            ai_summaries::has_error_segment(&pool, DAILY_SOURCE, &today.to_string()).await?;
        let last_gen = if has_err {
            None
        } else {
            ai_summaries::latest_generated_at(&pool, DAILY_SOURCE, &today.to_string()).await?
        };
        for t in due_unrun_times(now, times, last_gen.as_deref()) {
            let key = format!("d:{today}@{t}");
            if attempted.contains(&key) {
                continue;
            }
            // 有失败段时 force = false，已成功的段不重跑；否则整天重跑
            let force = !has_err && !daily_absent(&pool, today).await?;
            match try_run_daily(app, today, force).await {
                RunOutcome::Ran => {
                    attempted.insert(key);
                }
                RunOutcome::Busy => {
                    log::debug!("自动总结:别的 AI 任务进行中,日报让路");
                    break;
                }
            }
            // 每轮唤醒最多处理一个时间点，后续时间点留到下一轮检查
            break;
        }
    }

    // 上周周报：仅在缺失且上周至少有一天日报时自动生成
    let last_monday = align_to_monday(today) - ChronoDuration::days(7);
    let w_key = format!("w:{last_monday}");
    if !attempted.contains(&w_key) && weekly_absent(&pool, last_monday).await? {
        let pre = precheck_week(&pool, last_monday, &ProfileChoice::Auto(Report::Weekly)).await?;
        if pre.days_with_daily > 0 {
            match try_run_weekly(app, last_monday, pre.days_with_daily < 7).await {
                RunOutcome::Ran => {
                    attempted.insert(w_key);
                }
                RunOutcome::Busy => log::debug!("自动总结:别的 AI 任务进行中,周报让路"),
            }
        }
    }
    Ok(())
}

enum RunOutcome {
    /// 已尝试执行（成功或非忙碌类报错，本运行期不再自动重试）。
    Ran,
    /// 因 [`RunLock`] 或本机引擎正忙而让路，下轮再试。
    Busy,
}

fn outcome_of(result: Result<()>, what: &str) -> RunOutcome {
    match result {
        Ok(()) => RunOutcome::Ran,
        Err(Error::EngineBusy) => RunOutcome::Busy,
        Err(e) => {
            log::warn!("自动总结:{what}失败(本次运行期不再自动重试): {e}");
            RunOutcome::Ran
        }
    }
}

async fn try_run_daily(app: &AppHandle, date: NaiveDate, force_refresh: bool) -> RunOutcome {
    let run_lock = app.state::<RunLock>();
    let Ok(_guard) = run_lock.0.try_lock() else {
        return RunOutcome::Busy;
    };
    log::info!("自动总结:生成 {date} 日报");
    let cancel = app.state::<SummaryCancel>();
    cancel.0.store(false, Ordering::Relaxed);
    let mem = app.state::<crate::commands::screen_memory::MemoryState>();
    crate::ai::ocr_catchup::run(
        &app.state::<DbPool>(),
        mem.0.as_ref(),
        app,
        DAILY_SOURCE,
        &date.format("%Y-%m-%d").to_string(),
        &cancel.0,
    )
    .await;
    let runner = DaySummaryRunner::new(
        app.state::<DbPool>().inner().clone(),
        Arc::clone(
            app.state::<Arc<crate::ai::engine::EngineSupervisor>>()
                .inner(),
        ),
        app.clone(),
        Arc::clone(&cancel.0),
    );
    let result = runner
        .run(
            date,
            DeviceFilter::All,
            force_refresh,
            &ProfileChoice::Auto(Report::Daily),
        )
        .await;
    outcome_of(result, &format!("{date} 日报"))
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
            app.state::<Arc<crate::ai::engine::EngineSupervisor>>()
                .inner(),
        ),
        app.clone(),
        Arc::clone(&cancel.0),
    );
    let result = runner
        .run(
            monday,
            false,
            allow_missing,
            &ProfileChoice::Auto(Report::Weekly),
        )
        .await;
    outcome_of(result, &format!("{monday} 周报"))
}

async fn daily_absent(pool: &DbPool, date: NaiveDate) -> Result<bool> {
    let rows =
        ai_summaries::get_day(pool, DAILY_SOURCE, &date.format("%Y-%m-%d").to_string()).await?;
    Ok(rows.is_empty())
}

async fn weekly_absent(pool: &DbPool, monday: NaiveDate) -> Result<bool> {
    let rows =
        ai_summaries::get_day(pool, WEEKLY_SOURCE, &monday.format("%Y-%m-%d").to_string()).await?;
    Ok(rows.is_empty())
}

/// 返回 `date` 是否存在至少一条未被忽略规则排除的活动记录。
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

/// 若 `times` 中存在任一有效 `"HH:MM"` 时间点已经到达，则返回 `true`。
fn any_scheduled_time_due(now: chrono::NaiveTime, times: &[String]) -> bool {
    times.iter().any(|raw| {
        chrono::NaiveTime::parse_from_str(raw.trim(), "%H:%M")
            .map(|t| now >= t)
            .unwrap_or(false)
    })
}

/// 返回今天已到点但尚未被 `last_gen` 满足的 `"HH:MM"` 时间点列表。
///
/// 当 `last_gen`（转本地时间后）不早于某个时间点时视为已满足；时间戳解析失败按已满足处理，避免死循环重跑。
fn due_unrun_times(
    now: chrono::DateTime<Local>,
    times: &[String],
    last_gen: Option<&str>,
) -> Vec<String> {
    let last_local: Option<chrono::NaiveDateTime> = last_gen.map(|raw| {
        chrono::DateTime::parse_from_rfc3339(raw)
            .map(|dt| dt.with_timezone(&Local).naive_local())
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
                return false;
            }
            let slot_dt = today.and_time(t);
            match last_local {
                Some(gen) => gen < slot_dt,
                None => true,
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

    /// 本机引擎被别的任务占着时不算跑过，下轮再试；别的失败算跑过，不再重试。
    #[test]
    fn a_busy_engine_does_not_count_as_a_run() {
        assert!(matches!(
            outcome_of(Err(Error::EngineBusy), "日报"),
            RunOutcome::Busy
        ));
        assert!(matches!(
            outcome_of(Err(Error::InvalidInput("坏配置")), "日报"),
            RunOutcome::Ran
        ));
        assert!(matches!(outcome_of(Ok(()), "日报"), RunOutcome::Ran));
    }

    /// any_scheduled_time_due: 任一到点即放行; 坏格式跳过; 空表不放行。
    #[test]
    fn any_scheduled_time_due_truth_table() {
        let t = |h, m| chrono::NaiveTime::from_hms_opt(h, m, 0).unwrap();
        let v = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(!any_scheduled_time_due(t(23, 59), &[]));
        assert!(any_scheduled_time_due(t(12, 0), &v(&["12:00", "23:00"])));
        assert!(!any_scheduled_time_due(t(11, 59), &v(&["12:00", "23:00"])));
        assert!(any_scheduled_time_due(t(23, 0), &v(&["12:00", "23:00"])));
        assert!(!any_scheduled_time_due(t(12, 0), &v(&["25:99"])));
        assert!(any_scheduled_time_due(t(9, 30), &v(&[" 09:30 "])));
    }

    /// secs_until_next_scheduled_time: 点前给到点差值，点后滚到明天，多点取最近，空表/坏格式回 None。
    #[test]
    fn secs_until_next_scheduled_time_math() {
        use chrono::TimeZone;
        let v = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let at = |h, m| chrono::Local.with_ymd_and_hms(2026, 8, 8, h, m, 0).unwrap();

        // 22:30 → 23:00 点差 30 分钟
        assert_eq!(
            secs_until_next_scheduled_time(at(22, 30), &v(&["23:00"])),
            Some(1800)
        );
        // 23:10 → 今天的点已过,滚到明天 23:00
        assert_eq!(
            secs_until_next_scheduled_time(at(23, 10), &v(&["23:00"])),
            Some(24 * 3600 - 600)
        );
        // 多点取最近的
        assert_eq!(
            secs_until_next_scheduled_time(at(11, 0), &v(&["12:00", "23:00"])),
            Some(3600)
        );
        // 空表 / 坏格式 = 尽快模式
        assert_eq!(secs_until_next_scheduled_time(at(11, 0), &[]), None);
        assert_eq!(
            secs_until_next_scheduled_time(at(11, 0), &v(&["25:99"])),
            None
        );
    }

    /// due_unrun_times: 账本(当天日报 generated_at)判定——
    /// 生成时刻 ≥ 时间点 = 该点已完成; 从未生成全未满足; 解析失败视为满足。
    #[test]
    fn due_unrun_times_ledger() {
        use chrono::TimeZone;
        let v = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let now = chrono::Local
            .with_ymd_and_hms(2026, 8, 2, 23, 10, 0)
            .unwrap();
        let times = v(&["12:00", "23:00"]);

        // 从未生成:两点都到期且未满足
        assert_eq!(due_unrun_times(now, &times, None), times);

        // 12:05 生成过:12:00 点已满足,23:00 点未满足
        let gen_1205 = chrono::Local
            .with_ymd_and_hms(2026, 8, 2, 12, 5, 0)
            .unwrap()
            .to_rfc3339();
        assert_eq!(due_unrun_times(now, &times, Some(&gen_1205)), v(&["23:00"]));

        // 23:05 生成过:全部满足
        let gen_2305 = chrono::Local
            .with_ymd_and_hms(2026, 8, 2, 23, 5, 0)
            .unwrap()
            .to_rfc3339();
        assert!(due_unrun_times(now, &times, Some(&gen_2305)).is_empty());

        // 未到点的不入列:12:30 时 23:00 还没到
        let noon = chrono::Local
            .with_ymd_and_hms(2026, 8, 2, 12, 30, 0)
            .unwrap();
        assert_eq!(due_unrun_times(noon, &times, None), v(&["12:00"]));

        // 昨天生成的不算满足今天的点
        let gen_yday = chrono::Local
            .with_ymd_and_hms(2026, 8, 1, 23, 5, 0)
            .unwrap()
            .to_rfc3339();
        assert_eq!(due_unrun_times(now, &times, Some(&gen_yday)), times);

        // 时间戳解析失败:视为满足(不无限重烧 LLM)
        assert!(due_unrun_times(now, &times, Some("垃圾时间戳")).is_empty());

        // 坏格式时间点跳过
        assert!(due_unrun_times(now, &v(&["25:99"]), None).is_empty());
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
