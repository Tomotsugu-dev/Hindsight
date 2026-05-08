//! 报表层：按日 / 周 / 月聚合 activities 表，输出给前端 dashboard 的查询函数。
//!
//! 每个 `<scope>_<dim>` 函数（`day_hours` / `day_apps` / `week_days` / ...）输出固定
//! 形状的 Vec，前端拿到直接渲染。所有查询走 [`DeviceFilter`] 控制单设备 vs 全设备聚合。

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Timelike};
use rusqlite::ToSql;
use serde::Serialize;

use crate::error::Result;
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

/// 一小时内某分类的累计分钟数。是 [`HourSlot`] / [`DaySummary`] 的 segment 元素。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourSegment {
    /// 分类 ID（'other' / 用户自定义 ID 等）
    pub category_id: String,
    /// 该分类在该小时累计分钟数；多设备聚合时可超 60
    pub minutes: u32,
}

/// 单小时的分类时长分布（一个 [`HourSlot`] 对应 24 小时柱状图的一根柱子）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourSlot {
    /// 0..=23
    pub hour: u8,
    /// 该小时按分类切分的分钟数（按 minutes 降序），空 segments 表示该小时无活动
    pub segments: Vec<HourSegment>,
}

/// 单日的分类时长分布。给「周 / 月」页面的逐日热力图用。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaySummary {
    /// 日期 `YYYY-MM-DD`
    pub date: String,
    /// 该日按分类切分的分钟数
    pub segments: Vec<HourSegment>,
}

/// 单个应用的累计使用情况（top apps 列表的一行）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUsage {
    /// 显示名：组的 display_name（如 "Visual Studio Code"），同组的成员合并成一行
    pub process: String,
    pub category_id: String,
    pub minutes: u32,
    /// AppIcon 用来查图标的代表 process_name —— 在合并组里取一个稳定的成员名，
    /// 让前端拿组里任一个 process_name 都能查到（图标已跨设备同步）。
    pub icon_process: String,
}

/// 报表层的设备维度：All=多设备聚合，Only(id)=只看某一台
#[derive(Debug, Clone)]
pub enum DeviceFilter {
    All,
    Only(String),
}

impl DeviceFilter {
    /// 给 SQL 拼上设备过滤条件（如果有的话）
    pub(crate) fn sql_clause(&self) -> &'static str {
        match self {
            DeviceFilter::All => "",
            DeviceFilter::Only(_) => " AND a.device_id = ? ",
        }
    }

    /// 配合 [`DeviceFilter::sql_clause`] 给 prepared statement 提供额外参数（如有）。
    pub(crate) fn extra_param(&self) -> Option<&String> {
        match self {
            DeviceFilter::All => None,
            DeviceFilter::Only(id) => Some(id),
        }
    }
}

/// 把 Tauri 命令传过来的 `Option<String>` 设备过滤参数规整成 [`DeviceFilter`]。
/// `None` / 空串 / 全空白 → All；非空字符串 → Only。
pub fn device_filter_from_option(id: Option<String>) -> DeviceFilter {
    match id {
        None => DeviceFilter::All,
        Some(s) if s.trim().is_empty() => DeviceFilter::All,
        Some(s) => DeviceFilter::Only(s),
    }
}

/// 拉某日 24 小时的分类时长分布。`day_offset = 0` 今天，-1 昨天。
pub async fn day_hours(
    pool: &DbPool,
    day_offset: i32,
    device: DeviceFilter,
) -> Result<Vec<HourSlot>> {
    let target = Local::now() + Duration::days(day_offset as i64);
    let date = target.format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String, String)> = pool
        .0
        .call(move |conn| {
            // 通过 app_group_members → app_groups 拿分类（group 是 cross-OS 同步的真相），
            // 再 LEFT JOIN active categories 把指向已删分类的归到 'other'。
            let sql = format!(
                "SELECT a.started_at, a.ended_at,
                        COALESCE(c.id, 'other') AS cat
                 FROM activities a
                 LEFT JOIN app_group_members gm
                   ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
                 LEFT JOIN app_groups g
                   ON g.id = gm.group_id AND g.deleted_at IS NULL
                 LEFT JOIN categories c
                   ON c.id = g.category_id AND c.deleted_at IS NULL
                 WHERE a.local_date = ? {}",
                device.sql_clause()
            );
            let mut params: Vec<&dyn ToSql> = Vec::new();
            params.push(&date);
            if let Some(extra) = device.extra_param() {
                params.push(extra);
            }
            let mut stmt = conn.prepare(&sql).db()?;
            let it = stmt
                .query_map(params.as_slice(), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .db()?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.db()?);
            }
            Ok(out)
        })
        .await?;

    let mut buckets: [std::collections::HashMap<String, u64>; 24] =
        std::array::from_fn(|_| std::collections::HashMap::new());

    for (started, ended, cat) in rows {
        let s = parse_local(&started);
        let e = parse_local(&ended);
        if e <= s {
            continue;
        }
        for (hour, secs) in slice_by_hour(s, e) {
            *buckets[hour as usize].entry(cat.clone()).or_insert(0) += secs;
        }
    }

    let slots: Vec<HourSlot> = (0u8..24)
        .map(|h| {
            // 不再 clamp 60 上限：多设备聚合时一个时段总分钟可超 60，前端按
            // 设备数动态调整 Y 轴 limit（max = 60 × deviceCount）
            let mut segs: Vec<HourSegment> = buckets[h as usize]
                .iter()
                .map(|(cat, secs)| HourSegment {
                    category_id: cat.clone(),
                    minutes: ((*secs as f64 / 60.0).round() as u32),
                })
                .filter(|s| s.minutes > 0)
                .collect();
            // 降序排列：sort_by_key 用 Reverse(...) 实现 desc
            segs.sort_by_key(|s| std::cmp::Reverse(s.minutes));
            HourSlot {
                hour: h,
                segments: segs,
            }
        })
        .collect();

    Ok(slots)
}

/// 拉某日的 top 应用列表（按使用时长降序），同组的 process 已合并成一行。
/// `limit` 控制返回行数。
pub async fn day_apps(
    pool: &DbPool,
    day_offset: i32,
    limit: u32,
    device: DeviceFilter,
) -> Result<Vec<AppUsage>> {
    let target = Local::now() + Duration::days(day_offset as i64);
    let date = target.format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String, String, i64)> = pool
        .0
        .call(move |conn| {
            // 按组聚合（不是按 process_name）：同一个组的多个进程名（mac="Code" + win=
            // "Visual Studio Code"）合并成一行，时长相加。display 用组的 display_name；
            // icon_process 取组里 MIN(process_name) 当稳定代表（前端 AppIcon 拿它查
            // app_icons 表，图标已跨设备同步）。
            // 没 group 的进程（理论上 v15 backfill + capture::ensure_group 后不存在）
            // 退化为按 process_name 聚合。
            let sql = format!(
                "SELECT COALESCE(g.display_name, a.process_name)        AS display,
                        COALESCE(c.id, 'other')                         AS cat,
                        MIN(a.process_name)                             AS icon_process,
                        SUM(a.duration_secs)                            AS total
                 FROM activities a
                 LEFT JOIN app_group_members gm
                   ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
                 LEFT JOIN app_groups g
                   ON g.id = gm.group_id AND g.deleted_at IS NULL
                 LEFT JOIN categories c
                   ON c.id = g.category_id AND c.deleted_at IS NULL
                 WHERE a.local_date = ? {}
                 GROUP BY COALESCE(g.id, a.process_name)
                 ORDER BY total DESC
                 LIMIT ?",
                device.sql_clause()
            );
            let mut params: Vec<&dyn ToSql> = Vec::new();
            params.push(&date);
            if let Some(extra) = device.extra_param() {
                params.push(extra);
            }
            params.push(&limit);
            let mut stmt = conn.prepare(&sql).db()?;
            let it = stmt
                .query_map(params.as_slice(), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                })
                .db()?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.db()?);
            }
            Ok(out)
        })
        .await?;

    Ok(rows
        .into_iter()
        .map(|(process, cat, icon_process, secs)| AppUsage {
            process,
            category_id: cat,
            minutes: ((secs as f64 / 60.0).round() as u32),
            icon_process,
        })
        .filter(|a| a.minutes > 0)
        .collect())
}

/// 拉某日特定小时（local_hour = ?）的 top 应用列表，逻辑同 [`day_apps`]——
/// 多 `AND a.local_hour = ?` 一条过滤。给前端"点小时柱子→排行筛选到该小时"用。
pub async fn day_hour_apps(
    pool: &DbPool,
    day_offset: i32,
    hour: i32,
    limit: u32,
    device: DeviceFilter,
) -> Result<Vec<AppUsage>> {
    let target = Local::now() + Duration::days(day_offset as i64);
    let date = target.format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String, String, i64)> = pool
        .0
        .call(move |conn| {
            // 跟 day_apps 同 SQL，多了 `AND a.local_hour = ?`
            let sql = format!(
                "SELECT COALESCE(g.display_name, a.process_name)        AS display,
                        COALESCE(c.id, 'other')                         AS cat,
                        MIN(a.process_name)                             AS icon_process,
                        SUM(a.duration_secs)                            AS total
                 FROM activities a
                 LEFT JOIN app_group_members gm
                   ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
                 LEFT JOIN app_groups g
                   ON g.id = gm.group_id AND g.deleted_at IS NULL
                 LEFT JOIN categories c
                   ON c.id = g.category_id AND c.deleted_at IS NULL
                 WHERE a.local_date = ? AND a.local_hour = ? {}
                 GROUP BY COALESCE(g.id, a.process_name)
                 ORDER BY total DESC
                 LIMIT ?",
                device.sql_clause()
            );
            let mut params: Vec<&dyn ToSql> = Vec::new();
            params.push(&date);
            params.push(&hour);
            if let Some(extra) = device.extra_param() {
                params.push(extra);
            }
            params.push(&limit);
            let mut stmt = conn.prepare(&sql).db()?;
            let it = stmt
                .query_map(params.as_slice(), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                })
                .db()?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.db()?);
            }
            Ok(out)
        })
        .await?;

    Ok(rows
        .into_iter()
        .map(|(process, cat, icon_process, secs)| AppUsage {
            process,
            category_id: cat,
            minutes: ((secs as f64 / 60.0).round() as u32),
            icon_process,
        })
        .filter(|a| a.minutes > 0)
        .collect())
}

/// 拉某周 7 天每天的分类时长分布。`week_offset = 0` 是本周（周一开始）。
pub async fn week_days(
    pool: &DbPool,
    week_offset: i32,
    device: DeviceFilter,
) -> Result<Vec<DaySummary>> {
    let (monday, sunday) = week_range(week_offset);
    days_in_range(pool, monday, sunday, device).await
}

/// 拉某周的 top 应用聚合（跨 7 天总时长降序），按组合并。
pub async fn week_apps(
    pool: &DbPool,
    week_offset: i32,
    limit: u32,
    device: DeviceFilter,
) -> Result<Vec<AppUsage>> {
    let (monday, sunday) = week_range(week_offset);
    apps_in_range(pool, monday, sunday, limit, device).await
}

/// 拉某月每日的分类时长分布（28~31 行）。`month_offset = 0` 是本月。
pub async fn month_days(
    pool: &DbPool,
    month_offset: i32,
    device: DeviceFilter,
) -> Result<Vec<DaySummary>> {
    let (first, last) = month_range(month_offset);
    days_in_range(pool, first, last, device).await
}

/// 拉某月的 top 应用聚合（跨整月总时长降序），按组合并。
pub async fn month_apps(
    pool: &DbPool,
    month_offset: i32,
    limit: u32,
    device: DeviceFilter,
) -> Result<Vec<AppUsage>> {
    let (first, last) = month_range(month_offset);
    apps_in_range(pool, first, last, limit, device).await
}

async fn days_in_range(
    pool: &DbPool,
    from: NaiveDate,
    to: NaiveDate,
    device: DeviceFilter,
) -> Result<Vec<DaySummary>> {
    let from_str = from.format("%Y-%m-%d").to_string();
    let to_str = to.format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String, i64)> = pool
        .0
        .call(move |conn| {
            // 同 day_hours：通过 group → category 拿分类，过滤已删分类
            let sql = format!(
                "SELECT a.local_date,
                        COALESCE(c.id, 'other') AS cat,
                        SUM(a.duration_secs) AS total
                 FROM activities a
                 LEFT JOIN app_group_members gm
                   ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
                 LEFT JOIN app_groups g
                   ON g.id = gm.group_id AND g.deleted_at IS NULL
                 LEFT JOIN categories c
                   ON c.id = g.category_id AND c.deleted_at IS NULL
                 WHERE a.local_date >= ? AND a.local_date <= ? {}
                 GROUP BY a.local_date, cat",
                device.sql_clause()
            );
            let mut params: Vec<&dyn ToSql> = Vec::new();
            params.push(&from_str);
            params.push(&to_str);
            if let Some(extra) = device.extra_param() {
                params.push(extra);
            }
            let mut stmt = conn.prepare(&sql).db()?;
            let it = stmt
                .query_map(params.as_slice(), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                })
                .db()?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.db()?);
            }
            Ok(out)
        })
        .await?;

    let mut buckets: std::collections::HashMap<String, std::collections::HashMap<String, u32>> =
        std::collections::HashMap::new();
    for (date, cat, secs) in rows {
        let minutes = (secs as f64 / 60.0).round() as u32;
        if minutes == 0 {
            continue;
        }
        buckets.entry(date).or_default().insert(cat, minutes);
    }

    let mut out = Vec::new();
    let mut cur = from;
    while cur <= to {
        let key = cur.format("%Y-%m-%d").to_string();
        let mut segs: Vec<HourSegment> = buckets
            .remove(&key)
            .unwrap_or_default()
            .into_iter()
            .map(|(category_id, minutes)| HourSegment {
                category_id,
                minutes,
            })
            .collect();
        // 降序：见上面同模式注释
        segs.sort_by_key(|s| std::cmp::Reverse(s.minutes));
        out.push(DaySummary {
            date: key,
            segments: segs,
        });
        cur += Duration::days(1);
    }

    Ok(out)
}

async fn apps_in_range(
    pool: &DbPool,
    from: NaiveDate,
    to: NaiveDate,
    limit: u32,
    device: DeviceFilter,
) -> Result<Vec<AppUsage>> {
    let from_str = from.format("%Y-%m-%d").to_string();
    let to_str = to.format("%Y-%m-%d").to_string();

    let rows: Vec<(String, String, String, i64)> = pool
        .0
        .call(move |conn| {
            // 同 day_apps：按组聚合，display = display_name，icon_process = MIN(process_name)
            let sql = format!(
                "SELECT COALESCE(g.display_name, a.process_name)        AS display,
                        COALESCE(c.id, 'other')                         AS cat,
                        MIN(a.process_name)                             AS icon_process,
                        SUM(a.duration_secs)                            AS total
                 FROM activities a
                 LEFT JOIN app_group_members gm
                   ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
                 LEFT JOIN app_groups g
                   ON g.id = gm.group_id AND g.deleted_at IS NULL
                 LEFT JOIN categories c
                   ON c.id = g.category_id AND c.deleted_at IS NULL
                 WHERE a.local_date >= ? AND a.local_date <= ? {}
                 GROUP BY COALESCE(g.id, a.process_name)
                 ORDER BY total DESC
                 LIMIT ?",
                device.sql_clause()
            );
            let mut params: Vec<&dyn ToSql> = Vec::new();
            params.push(&from_str);
            params.push(&to_str);
            if let Some(extra) = device.extra_param() {
                params.push(extra);
            }
            params.push(&limit);
            let mut stmt = conn.prepare(&sql).db()?;
            let it = stmt
                .query_map(params.as_slice(), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                })
                .db()?;
            let mut out = Vec::new();
            for r in it {
                out.push(r.db()?);
            }
            Ok(out)
        })
        .await?;

    Ok(rows
        .into_iter()
        .map(|(process, cat, icon_process, secs)| AppUsage {
            process,
            category_id: cat,
            minutes: (secs as f64 / 60.0).round() as u32,
            icon_process,
        })
        .filter(|a| a.minutes > 0)
        .collect())
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
    // month 经上面 while 循环钳到 1..=12，year+1 / month+1 也都在 chrono 接受范围内
    // 用 expect 而非 unwrap：将来若改循环边界，panic 信息能直接指明哪条违反了哪条不变量
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

fn parse_local(s: &str) -> DateTime<Local> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Local))
        .unwrap_or_else(|_| Local::now())
}

fn slice_by_hour(start: DateTime<Local>, end: DateTime<Local>) -> Vec<(u8, u64)> {
    let mut out = Vec::new();
    let mut cur = start;
    while cur < end {
        let hour = cur.hour() as u8;
        let next_hour = Local
            .with_ymd_and_hms(cur.year(), cur.month(), cur.day(), cur.hour(), 0, 0)
            .single()
            .map(|t| t + Duration::hours(1))
            .unwrap_or(end);
        let chunk_end = if next_hour < end { next_hour } else { end };
        let secs = (chunk_end - cur).num_seconds().max(0) as u64;
        if secs > 0 {
            out.push((hour, secs));
        }
        cur = chunk_end;
    }
    out
}
