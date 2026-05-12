//! 快速模板总结：纯 SQL 聚合后落到一组结构化指标，前端按 i18n 模板拼成自然语言段落。
//!
//! 跟 [`crate::ai::summary`] 互补——AI 总结要本地大模型 + 截图 + 跑分钟级；本模块只读
//! activities 聚合表，毫秒级返回，没有硬件门槛。两条路径输出形态完全独立，前端可在
//! 「AI 总结 / 快速模板」之间切换。
//!
//! 所有查询复用 [`crate::repo::reports`] 的 SQL，避免重复造轮子；新增的只是"指标抽取"
//! 这一层（peak 小时、活跃天数、时段分桶、占比换算等）。

use chrono::{Datelike, Duration, Local, NaiveDate};
use serde::Serialize;

use crate::error::Result;
use crate::repo::reports::{
    self, AppUsage, DaySummary, DeviceFilter, HourSlot,
};
use crate::storage::DbPool;

/// 单段时长指标（top app / 分类占比共用）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageEntry {
    /// 显示名（应用名或分类 id）
    pub key: String,
    /// 分钟数
    pub minutes: u32,
    /// 占总时长比例（0..=1）；分类用，应用列表里也填，前端可选不渲染
    pub percent: f32,
    /// 仅 top apps 用：分类 id / 图标 process_name；其它场景空串
    pub category_id: String,
    pub icon_process: String,
}

/// 时段分桶：早 / 上午 / 下午 / 晚 四块；通常足够画一个"一天分布"的感觉。
/// 划分按 local_hour：0–5 night、6–11 morning、12–17 afternoon、18–23 evening。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayPart {
    /// "night" / "morning" / "afternoon" / "evening"
    pub key: String,
    pub minutes: u32,
    pub percent: f32,
}

/// 日报维度的快速模板数据。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickDaySummary {
    /// 当日日期 "YYYY-MM-DD"
    pub date: String,
    /// 当日总时长（分钟）
    pub total_minutes: u32,
    /// 有活动的小时数（duration > 0）；0..=24
    pub active_hours: u8,
    /// 时长最高的小时（0..=23）；total_minutes=0 时为 None
    pub peak_hour: Option<u8>,
    /// 该 peak_hour 的时长（分钟）；与 peak_hour 同 None
    pub peak_hour_minutes: u32,
    /// 早午晚分桶
    pub day_parts: Vec<DayPart>,
    /// top N 应用
    pub top_apps: Vec<UsageEntry>,
    /// 分类时长占比
    pub categories: Vec<UsageEntry>,
}

/// 周报维度——逻辑接近 daily 但单位换成"天"。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickWeekSummary {
    /// 周一日期 "YYYY-MM-DD"
    pub week_start: String,
    /// 周日日期 "YYYY-MM-DD"
    pub week_end: String,
    pub total_minutes: u32,
    /// 7 天里有活动的天数
    pub active_days: u8,
    /// 日均（基于 active_days；active_days=0 时为 0）
    pub daily_average_minutes: u32,
    /// 时长最高那天（date + minutes + weekday 0..=6 周一起算）
    pub peak_day: Option<PeakDay>,
    /// 7 天逐日时长（按周一到周日顺序）
    pub daily_series: Vec<DailyPoint>,
    pub top_apps: Vec<UsageEntry>,
    pub categories: Vec<UsageEntry>,
}

/// 月报维度。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickMonthSummary {
    /// 月份的第一天 "YYYY-MM-DD"
    pub month_start: String,
    /// 月份的最后一天 "YYYY-MM-DD"
    pub month_end: String,
    /// 月内总天数（28-31）
    pub total_days: u8,
    pub total_minutes: u32,
    /// 月内有活动的天数
    pub active_days: u8,
    /// 日均（基于 active_days；active_days=0 时为 0）
    pub daily_average_minutes: u32,
    /// 时长最高那天
    pub peak_day: Option<PeakDay>,
    /// 时长最低那天（仅在 active_days >= 2 时返回，避免只有一天活动时退化等于 peak）
    pub quiet_day: Option<PeakDay>,
    /// 月内逐日时长（按日期升序）
    pub daily_series: Vec<DailyPoint>,
    pub top_apps: Vec<UsageEntry>,
    pub categories: Vec<UsageEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeakDay {
    pub date: String,
    pub minutes: u32,
    /// 0..=6 周一起算（跟 chrono::Weekday::num_days_from_monday 对齐）
    pub weekday: u8,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyPoint {
    pub date: String,
    pub minutes: u32,
}

const TOP_APPS_LIMIT: u32 = 5;

/// 计算某天的快速总结。`day_offset = 0` 今天，-1 昨天。
pub async fn compute_day(
    pool: &DbPool,
    day_offset: i32,
    device: DeviceFilter,
) -> Result<QuickDaySummary> {
    let date = (Local::now() + Duration::days(day_offset as i64))
        .format("%Y-%m-%d")
        .to_string();

    let hours = reports::day_hours(pool, day_offset, device.clone()).await?;
    let apps = reports::day_apps(pool, day_offset, TOP_APPS_LIMIT, device.clone()).await?;

    let (total_minutes, active_hours, peak_hour, peak_hour_minutes, day_parts, categories) =
        digest_day_hours(&hours);
    let top_apps = top_apps_from_rows(&apps, total_minutes);

    Ok(QuickDaySummary {
        date,
        total_minutes,
        active_hours,
        peak_hour,
        peak_hour_minutes,
        day_parts,
        top_apps,
        categories,
    })
}

/// 计算某周的快速总结。`week_offset = 0` 本周，-1 上周。
pub async fn compute_week(
    pool: &DbPool,
    week_offset: i32,
    device: DeviceFilter,
) -> Result<QuickWeekSummary> {
    let (monday, sunday) = week_range(week_offset);
    let days = reports::week_days(pool, week_offset, device.clone()).await?;
    let apps = reports::week_apps(pool, week_offset, TOP_APPS_LIMIT, device.clone()).await?;

    let agg = digest_days_range(&days, /* expect_quiet = */ false);
    let top_apps = top_apps_from_rows(&apps, agg.total_minutes);

    Ok(QuickWeekSummary {
        week_start: monday.format("%Y-%m-%d").to_string(),
        week_end: sunday.format("%Y-%m-%d").to_string(),
        total_minutes: agg.total_minutes,
        active_days: agg.active_days,
        daily_average_minutes: agg.daily_average,
        peak_day: agg.peak_day,
        daily_series: agg.series,
        top_apps,
        categories: agg.categories,
    })
}

/// 计算某月的快速总结。`month_offset = 0` 本月，-1 上月。
pub async fn compute_month(
    pool: &DbPool,
    month_offset: i32,
    device: DeviceFilter,
) -> Result<QuickMonthSummary> {
    let (first, last) = month_range(month_offset);
    let total_days = last.day() as u8; // first.day() == 1，last.day() = 当月最大天
    let days = reports::month_days(pool, month_offset, device.clone()).await?;
    let apps =
        reports::month_apps(pool, month_offset, TOP_APPS_LIMIT, device.clone()).await?;

    let agg = digest_days_range(&days, /* expect_quiet = */ true);
    let top_apps = top_apps_from_rows(&apps, agg.total_minutes);

    Ok(QuickMonthSummary {
        month_start: first.format("%Y-%m-%d").to_string(),
        month_end: last.format("%Y-%m-%d").to_string(),
        total_days,
        total_minutes: agg.total_minutes,
        active_days: agg.active_days,
        daily_average_minutes: agg.daily_average,
        peak_day: agg.peak_day,
        quiet_day: agg.quiet_day,
        daily_series: agg.series,
        top_apps,
        categories: agg.categories,
    })
}

// ───────────────────────── 内部辅助 ─────────────────────────

/// 把 24 小时分布折叠成 (总分钟, 活跃小时, peak, peak_minutes, day_parts, categories)。
/// 命中的分类按 minutes 降序，0 分钟项被过滤掉。
fn digest_day_hours(
    hours: &[HourSlot],
) -> (u32, u8, Option<u8>, u32, Vec<DayPart>, Vec<UsageEntry>) {
    let mut total: u32 = 0;
    let mut active_hours: u8 = 0;
    let mut peak: Option<(u8, u32)> = None;
    // 时段分桶：用 u32 累计避免 24h×60 = 1440 上限附近溢出风险
    let mut part_buckets: [u32; 4] = [0; 4]; // night/morning/afternoon/evening
    let mut cat_totals: std::collections::HashMap<String, u32> = std::collections::HashMap::new();

    for slot in hours {
        let hour_total: u32 = slot.segments.iter().map(|s| s.minutes).sum();
        if hour_total > 0 {
            active_hours += 1;
        }
        total = total.saturating_add(hour_total);
        if hour_total > 0 && peak.map(|(_, v)| hour_total > v).unwrap_or(true) {
            peak = Some((slot.hour, hour_total));
        }
        // 时段分桶
        let bucket = match slot.hour {
            0..=5 => 0,
            6..=11 => 1,
            12..=17 => 2,
            _ => 3,
        };
        part_buckets[bucket] = part_buckets[bucket].saturating_add(hour_total);
        // 分类累计
        for seg in &slot.segments {
            *cat_totals.entry(seg.category_id.clone()).or_insert(0) += seg.minutes;
        }
    }

    let day_parts: Vec<DayPart> = ["night", "morning", "afternoon", "evening"]
        .iter()
        .enumerate()
        .map(|(i, k)| DayPart {
            key: (*k).to_string(),
            minutes: part_buckets[i],
            percent: pct(part_buckets[i], total),
        })
        .collect();

    let categories = build_categories(&cat_totals, total);

    let (peak_hour, peak_minutes) = match peak {
        Some((h, m)) => (Some(h), m),
        None => (None, 0),
    };

    (total, active_hours, peak_hour, peak_minutes, day_parts, categories)
}

struct DaysAgg {
    total_minutes: u32,
    active_days: u8,
    daily_average: u32,
    peak_day: Option<PeakDay>,
    quiet_day: Option<PeakDay>,
    series: Vec<DailyPoint>,
    categories: Vec<UsageEntry>,
}

/// 把一个日期范围（7 天或一个月）的逐日分类分布聚成 DaysAgg。
/// `expect_quiet` = true 时尝试找最低活跃日（active_days >= 2 才返回）。
fn digest_days_range(days: &[DaySummary], expect_quiet: bool) -> DaysAgg {
    let mut total: u32 = 0;
    let mut active_days: u8 = 0;
    let mut peak: Option<PeakDay> = None;
    let mut quiet: Option<PeakDay> = None;
    let mut series: Vec<DailyPoint> = Vec::with_capacity(days.len());
    let mut cat_totals: std::collections::HashMap<String, u32> = std::collections::HashMap::new();

    for d in days {
        let day_total: u32 = d.segments.iter().map(|s| s.minutes).sum();
        if day_total > 0 {
            active_days += 1;
        }
        total = total.saturating_add(day_total);

        let weekday = weekday_from_iso(&d.date);
        if day_total > 0 && peak.as_ref().map(|p| day_total > p.minutes).unwrap_or(true) {
            peak = Some(PeakDay {
                date: d.date.clone(),
                minutes: day_total,
                weekday,
            });
        }
        if expect_quiet
            && day_total > 0
            && quiet
                .as_ref()
                .map(|p| day_total < p.minutes)
                .unwrap_or(true)
        {
            quiet = Some(PeakDay {
                date: d.date.clone(),
                minutes: day_total,
                weekday,
            });
        }
        series.push(DailyPoint {
            date: d.date.clone(),
            minutes: day_total,
        });
        for seg in &d.segments {
            *cat_totals.entry(seg.category_id.clone()).or_insert(0) += seg.minutes;
        }
    }

    // quiet 仅当 active_days >= 2 且跟 peak 的分钟数不同才有意义。
    // 按 minutes 而不是 date 判断重合——两天时长完全相等时 quiet/peak 等价，没必要再说一句。
    let quiet_day = if expect_quiet && active_days >= 2 {
        quiet.filter(|q| peak.as_ref().map(|p| p.minutes != q.minutes).unwrap_or(true))
    } else {
        None
    };

    let daily_average = if active_days == 0 {
        0
    } else {
        total / active_days as u32
    };

    let categories = build_categories(&cat_totals, total);

    DaysAgg {
        total_minutes: total,
        active_days,
        daily_average,
        peak_day: peak,
        quiet_day,
        series,
        categories,
    }
}

fn build_categories(
    cat_totals: &std::collections::HashMap<String, u32>,
    total: u32,
) -> Vec<UsageEntry> {
    let mut entries: Vec<UsageEntry> = cat_totals
        .iter()
        .filter(|(_, m)| **m > 0)
        .map(|(id, m)| UsageEntry {
            key: id.clone(),
            minutes: *m,
            percent: pct(*m, total),
            category_id: String::new(),
            icon_process: String::new(),
        })
        .collect();
    entries.sort_by(|a, b| b.minutes.cmp(&a.minutes).then(a.key.cmp(&b.key)));
    entries
}

fn top_apps_from_rows(apps: &[AppUsage], total: u32) -> Vec<UsageEntry> {
    apps.iter()
        .map(|a| UsageEntry {
            key: a.process.clone(),
            minutes: a.minutes,
            percent: pct(a.minutes, total),
            category_id: a.category_id.clone(),
            icon_process: a.icon_process.clone(),
        })
        .collect()
}

fn pct(part: u32, total: u32) -> f32 {
    if total == 0 {
        0.0
    } else {
        (part as f32) / (total as f32)
    }
}

fn weekday_from_iso(date: &str) -> u8 {
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|d| d.weekday().num_days_from_monday() as u8)
        .unwrap_or(0)
}

fn week_range(week_offset: i32) -> (NaiveDate, NaiveDate) {
    let today = Local::now().date_naive();
    let dow = today.weekday().num_days_from_monday() as i64;
    let monday = today - Duration::days(dow) + Duration::days(week_offset as i64 * 7);
    let sunday = monday + Duration::days(6);
    (monday, sunday)
}

fn month_range(month_offset: i32) -> (NaiveDate, NaiveDate) {
    let today = Local::now().date_naive();
    let mut year = today.year();
    let mut month = today.month() as i32 + month_offset;
    while month <= 0 {
        month += 12;
        year -= 1;
    }
    while month > 12 {
        month -= 12;
        year += 1;
    }
    let first = NaiveDate::from_ymd_opt(year, month as u32, 1)
        .expect("month_range: year/month 应在 chrono 合法范围");
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1).expect("month_range: 跨年到 1 月")
    } else {
        NaiveDate::from_ymd_opt(year, (month + 1) as u32, 1)
            .expect("month_range: month+1 应在 1..=12")
    };
    let last = next - Duration::days(1);
    (first, last)
}
