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
            // `g.category_id IS NOT 'hidden'`：SQLite NULL-safe 标量比较，未分组的活动
            // (g.category_id 为 NULL) 仍通过，仅显式指派到 hidden 的被剔除。
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
                 WHERE a.local_date = ? {}
                   AND g.category_id IS NOT 'hidden'",
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
                   AND g.category_id IS NOT 'hidden'
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
            // 跟 day_apps 同 SQL，多了 `AND a.local_hour = ?` + hidden 过滤
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
                   AND g.category_id IS NOT 'hidden'
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
            // 同 day_hours：通过 group → category 拿分类，过滤已删分类 + hidden 分类
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
                   AND g.category_id IS NOT 'hidden'
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
            // hidden 分类的活动整段排除（不计入 top apps）
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
                   AND g.category_id IS NOT 'hidden'
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo::test_util::{fresh_test_pool, TEST_SELF_ID};
    use crate::storage::SqliteResultExt;

    /// 测 [`day_apps`] 跨设备 SUM：
    /// - `DeviceFilter::All` 合并两端时长到 1 行
    /// - `DeviceFilter::Only(...)` 只算指定设备
    ///
    /// 钉死「今日总览」上方设备 chip 切换的数字一致性。
    #[tokio::test]
    async fn day_apps_aggregates_correctly_across_devices() {
        let pool = fresh_test_pool().await;
        let today = Local::now().format("%Y-%m-%d").to_string();

        // 同一进程 "Code" 在 self（5 分钟）和 device-win（3 分钟）各贡献时长
        insert_activity(&pool, TEST_SELF_ID, &today, "Code", 300).await;
        insert_activity(&pool, "device-win", &today, "Code", 180).await;
        // 简单 1:1 组：组 id = process_name = "Code"，category=code
        seed_solo_group(&pool, "Code", "code").await;

        // All: 5 + 3 = 8 分钟
        let all = day_apps(&pool, 0, 50, DeviceFilter::All).await.unwrap();
        assert_eq!(all.len(), 1, "All 视角应只有一行");
        assert_eq!(all[0].process, "Code");
        assert_eq!(all[0].minutes, 8);
        assert_eq!(all[0].category_id, "code");

        // Only self: 只 5 分钟
        let only_self = day_apps(&pool, 0, 50, DeviceFilter::Only(TEST_SELF_ID.into()))
            .await
            .unwrap();
        assert_eq!(only_self.len(), 1);
        assert_eq!(only_self[0].minutes, 5);

        // Only win: 只 3 分钟
        let only_win = day_apps(&pool, 0, 50, DeviceFilter::Only("device-win".into()))
            .await
            .unwrap();
        assert_eq!(only_win.len(), 1);
        assert_eq!(only_win[0].minutes, 3);
    }

    /// 测 [`day_apps`] 跨 OS 别名合并：mac="Code" + Win="Code.exe" 共享
    /// canonical 组 "Visual Studio Code" → All 视角下应合并成 1 行。
    ///
    /// 钉死："两台机器各显示 5min / 3min" 而不是合并的 "8min" 这条 bug 重现。
    #[tokio::test]
    async fn day_apps_merges_cross_os_aliases_into_one_row() {
        let pool = fresh_test_pool().await;
        let today = Local::now().format("%Y-%m-%d").to_string();

        // mac 视角的 "Code" 5 分钟 + Win 视角的 "Code.exe" 3 分钟
        insert_activity(&pool, TEST_SELF_ID, &today, "Code", 300).await;
        insert_activity(&pool, "device-win", &today, "Code.exe", 180).await;

        // 一个 canonical 组，两个成员都指向它
        pool.0
            .call(|conn| {
                let now = "2026-05-15T10:00:00Z";
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES('Visual Studio Code', 'Visual Studio Code', 'code', ?1, NULL)",
                    rusqlite::params![now],
                )
                .db()?;
                for name in ["Code", "Code.exe"] {
                    conn.execute(
                        "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                         VALUES(?1, 'Visual Studio Code', ?2, NULL)",
                        rusqlite::params![name, now],
                    )
                    .db()?;
                }
                Ok(())
            })
            .await
            .unwrap();

        let rows = day_apps(&pool, 0, 50, DeviceFilter::All).await.unwrap();
        assert_eq!(rows.len(), 1, "cross-OS 别名应合并成一行，不是两行");
        assert_eq!(rows[0].process, "Visual Studio Code");
        assert_eq!(rows[0].minutes, 8);
        assert_eq!(rows[0].category_id, "code");
        // icon_process 是 MIN(process_name)，二选一即可
        assert!(
            rows[0].icon_process == "Code" || rows[0].icon_process == "Code.exe",
            "icon_process 应是组内某个真实成员名: got {}",
            rows[0].icon_process
        );
    }

    async fn insert_activity(
        pool: &DbPool,
        device_id: &str,
        local_date: &str,
        process_name: &str,
        duration_secs: i64,
    ) {
        let device_id = device_id.to_string();
        let local_date = local_date.to_string();
        let process_name = process_name.to_string();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(
                        started_at, ended_at, duration_secs, local_date, local_hour,
                        process_name, window_title, category_id, device_id, updated_at, origin
                     ) VALUES(
                        ?1 || 'T10:00:00Z', ?1 || 'T10:00:30Z', ?2, ?1, 10,
                        ?3, '', 'other', ?4, ?1 || 'T10:00:30Z', 'local'
                     )",
                    rusqlite::params![local_date, duration_secs, process_name, device_id],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// 测 [`day_hours`]：跨两个小时的 session 应按时钟分桶到对应 HourSlot。
    /// 10:30 → 11:30 的 1 小时 session：hour=10 / hour=11 各 30 分钟。
    #[tokio::test]
    async fn day_hours_buckets_correctly() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();
        let started = Local
            .from_local_datetime(&today.and_hms_opt(10, 30, 0).unwrap())
            .single()
            .unwrap();
        let ended = Local
            .from_local_datetime(&today.and_hms_opt(11, 30, 0).unwrap())
            .single()
            .unwrap();
        insert_session_with_times(&pool, TEST_SELF_ID, &today_str, "Code", started, ended).await;
        seed_solo_group(&pool, "Code", "code").await;

        let slots = day_hours(&pool, 0, DeviceFilter::All).await.unwrap();
        assert_eq!(slots.len(), 24);

        let h10 = slots.iter().find(|s| s.hour == 10).unwrap();
        let h10_code: u32 = h10
            .segments
            .iter()
            .filter(|s| s.category_id == "code")
            .map(|s| s.minutes)
            .sum();
        assert_eq!(h10_code, 30, "10 点应有 30 分钟 code");

        let h11 = slots.iter().find(|s| s.hour == 11).unwrap();
        let h11_code: u32 = h11
            .segments
            .iter()
            .filter(|s| s.category_id == "code")
            .map(|s| s.minutes)
            .sum();
        assert_eq!(h11_code, 30, "11 点应有 30 分钟 code");

        // 其它小时不该出现 code 段
        for h in [9u8, 12, 13] {
            let slot = slots.iter().find(|s| s.hour == h).unwrap();
            assert!(
                slot.segments.iter().all(|s| s.category_id != "code"),
                "{h} 点不该出现 code 段"
            );
        }
    }

    /// 测 [`day_hour_apps`]：local_hour 过滤后只返该小时内的应用。
    #[tokio::test]
    async fn day_hour_apps_filters_by_hour() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();

        // 10 点 30 分钟 Code
        let s10 = Local
            .from_local_datetime(&today.and_hms_opt(10, 0, 0).unwrap())
            .single()
            .unwrap();
        let e10 = s10 + Duration::minutes(30);
        insert_session_with_times(&pool, TEST_SELF_ID, &today_str, "Code", s10, e10).await;

        // 11 点 30 分钟 Chrome
        let s11 = Local
            .from_local_datetime(&today.and_hms_opt(11, 0, 0).unwrap())
            .single()
            .unwrap();
        let e11 = s11 + Duration::minutes(30);
        insert_session_with_times(&pool, TEST_SELF_ID, &today_str, "Chrome", s11, e11).await;

        seed_solo_group(&pool, "Code", "code").await;
        seed_solo_group(&pool, "Chrome", "browse").await;

        let h10 = day_hour_apps(&pool, 0, 10, 50, DeviceFilter::All).await.unwrap();
        assert_eq!(h10.len(), 1, "hour=10 只应有 Code");
        assert_eq!(h10[0].process, "Code");

        let h11 = day_hour_apps(&pool, 0, 11, 50, DeviceFilter::All).await.unwrap();
        assert_eq!(h11.len(), 1, "hour=11 只应有 Chrome");
        assert_eq!(h11[0].process, "Chrome");
    }

    /// 测 [`week_days`]：今天的 DaySummary 应 SUM 多设备 (All) 或单设备 (Only) 时长。
    #[tokio::test]
    async fn week_days_aggregates_cross_device() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();

        insert_activity(&pool, TEST_SELF_ID, &today_str, "Code", 300).await; // 5 min self
        insert_activity(&pool, "device-win", &today_str, "Code", 180).await; // 3 min win
        seed_solo_group(&pool, "Code", "code").await;

        let all = week_days(&pool, 0, DeviceFilter::All).await.unwrap();
        let today_all = all.iter().find(|d| d.date == today_str).unwrap();
        let code_all: u32 = today_all
            .segments
            .iter()
            .filter(|s| s.category_id == "code")
            .map(|s| s.minutes)
            .sum();
        assert_eq!(code_all, 8, "All 视角 today 应 5+3 = 8 分钟 code");

        let only_self = week_days(&pool, 0, DeviceFilter::Only(TEST_SELF_ID.into()))
            .await
            .unwrap();
        let today_self = only_self.iter().find(|d| d.date == today_str).unwrap();
        let code_self: u32 = today_self
            .segments
            .iter()
            .filter(|s| s.category_id == "code")
            .map(|s| s.minutes)
            .sum();
        assert_eq!(code_self, 5, "Only self 视角 today 应 5 分钟");
    }

    /// 测 [`month_apps`]：top N 按总时长降序。
    #[tokio::test]
    async fn month_apps_top_n_correct() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();

        insert_activity(&pool, TEST_SELF_ID, &today_str, "Code", 300).await; // 5 min
        insert_activity(&pool, TEST_SELF_ID, &today_str, "Chrome", 180).await; // 3 min
        insert_activity(&pool, TEST_SELF_ID, &today_str, "Slack", 60).await; // 1 min
        seed_solo_group(&pool, "Code", "code").await;
        seed_solo_group(&pool, "Chrome", "browse").await;
        seed_solo_group(&pool, "Slack", "talk").await;

        let apps = month_apps(&pool, 0, 5, DeviceFilter::All).await.unwrap();
        assert!(apps.len() >= 3, "应至少 3 行");
        // 降序：Code (5) > Chrome (3) > Slack (1)
        assert_eq!(apps[0].process, "Code");
        assert_eq!(apps[0].minutes, 5);
        assert_eq!(apps[1].process, "Chrome");
        assert_eq!(apps[1].minutes, 3);
        assert_eq!(apps[2].process, "Slack");
        assert_eq!(apps[2].minutes, 1);

        // limit 钉死
        let top_2 = month_apps(&pool, 0, 2, DeviceFilter::All).await.unwrap();
        assert_eq!(top_2.len(), 2);
        assert_eq!(top_2[0].process, "Code");
        assert_eq!(top_2[1].process, "Chrome");
    }

    /// 给 day_hours / day_hour_apps 测试用：插一行 sealed activity 但用真实的 local 时区
    /// started_at / ended_at（不再用固定的 'T10:00:00Z' UTC 串）。
    async fn insert_session_with_times(
        pool: &DbPool,
        device_id: &str,
        local_date: &str,
        process_name: &str,
        started: DateTime<Local>,
        ended: DateTime<Local>,
    ) {
        let device_id = device_id.to_string();
        let local_date = local_date.to_string();
        let process_name = process_name.to_string();
        let dur = (ended - started).num_seconds().max(0);
        let local_hour = started.hour() as i64;
        let started_str = started.to_rfc3339();
        let ended_str = ended.to_rfc3339();
        pool.0
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activities(
                        started_at, ended_at, duration_secs, local_date, local_hour,
                        process_name, window_title, category_id, device_id, updated_at, origin
                     ) VALUES(?, ?, ?, ?, ?, ?, '', 'other', ?, ?, 'local')",
                    rusqlite::params![
                        started_str,
                        ended_str,
                        dur,
                        local_date,
                        local_hour,
                        process_name,
                        device_id,
                        ended_str,
                    ],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn seed_solo_group(pool: &DbPool, name: &str, category_id: &str) {
        let name = name.to_string();
        let category_id = category_id.to_string();
        pool.0
            .call(move |conn| {
                let now = "2026-05-15T10:00:00Z";
                conn.execute(
                    "INSERT INTO app_groups(id, display_name, category_id, updated_at, deleted_at)
                     VALUES(?1, ?1, ?2, ?3, NULL)",
                    rusqlite::params![name, category_id, now],
                )
                .db()?;
                conn.execute(
                    "INSERT INTO app_group_members(process_name, group_id, updated_at, deleted_at)
                     VALUES(?1, ?1, ?2, NULL)",
                    rusqlite::params![name, now],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();
    }
}
