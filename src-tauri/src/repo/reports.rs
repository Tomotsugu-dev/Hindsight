//! 报表层：按日 / 周 / 月聚合 activities 表，输出给前端 dashboard 的查询函数。
//!
//! 每个 `<scope>_<dim>` 函数（`day_hours` / `day_apps` / `week_days` / ...）输出固定
//! 形状的 Vec，前端拿到直接渲染。所有查询走 [`DeviceFilter`] 控制单设备 vs 全设备聚合。

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, TimeZone, Timelike};
use rusqlite::{OptionalExtension, ToSql};
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
    /// 该分类在该小时累计分钟数；多设备聚合时可超 60。**仅供柱图显示**——
    /// 求总和请用 `secs`：每桶各自四舍五入的分钟数相加会系统性偏离真实总量
    /// （碎片使用越多偏得越多，如每小时 40s×12h：round 后 12min，实际 8min）。
    pub minutes: u32,
    /// 该分类在该桶的累计秒数（未取整原值）。前端所有"总时长/日均/占比"
    /// 计算都应从 secs 累加、最后一步再换算分钟，保证与 top-apps 的
    /// "先加总后取整"口径一致。
    pub secs: u64,
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

/// 「点应用 → 详情抽屉」的聚合数据：一串时间柱 + 窗口标题用时排行。
/// 日 / 周 / 月共用——各自的日期范围在后端聚合好再下发（避免把一个月上万条
/// 原始 session 全传给前端）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppDetail {
    /// 时间柱：小时粒度=24 根(key="0".."23")，天粒度=范围内每天一根(key="YYYY-MM-DD")。
    /// 已按时间排好、含 0 值空桶，前端直接渲染。
    pub buckets: Vec<DetailBucket>,
    /// 按窗口标题聚合的用时（降序）。原始标题，前端再剥 app 名后缀 + 合并。
    pub titles: Vec<TitleUsage>,
    /// 代表进程是否浏览器——前端据此决定要不要把标题列表按网站分组展示。
    pub is_browser: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailBucket {
    /// 小时粒度："0".."23"；天粒度："YYYY-MM-DD"
    pub key: String,
    pub secs: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleUsage {
    pub title: String,
    pub secs: u32,
    /// 浏览器会话的网站域名（如 github.com）；非浏览器 / 老记录 / 读不到地址栏
    /// 时为 None。前端按它把页面行分到「按网站」的各组里。
    pub host: Option<String>,
}

/// 详情时间柱的聚合粒度：日报按小时，周 / 月报按天。
#[derive(Debug, Clone, Copy)]
enum BucketBy {
    Hour,
    Day,
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
                   AND g.category_id IS NOT 'hidden'
                   AND a.excluded = 0",
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
                    secs: *secs,
                })
                // 按 secs 过滤而不是 minutes：<30s 的片段 minutes 四舍五入成 0，
                // 但 secs 仍计入前端总量（HourSegment.secs 契约），丢掉会和 top-apps 对不上
                .filter(|s| s.secs > 0)
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
            // 按「显示名 + 分类」聚合：同一个组的多个进程名（mac="Code" + win=
            // "Visual Studio Code"）合并成一行，时长相加；display 用组的 display_name，
            // icon_process 取 MIN(process_name) 当稳定代表（前端 AppIcon 拿它查
            // app_icons 表，图标已跨设备同步）。
            // 不能按 g.id 聚合：两个实体显示名相同时（mac 进程名"QQ音乐" + win 组
            // display_name"QQ音乐"）会出两行同名——对用户就是同一个应用，必须合并。
            // 显示名相同但分类不同的仍分开列（语义如此）。
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
                   AND a.excluded = 0
                 GROUP BY COALESCE(g.display_name, a.process_name), COALESCE(c.id, 'other')
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

/// 拉某日特定小时的 top 应用列表。给前端"点小时柱子→排行筛选到该小时"用。
///
/// **口径必须与柱子一致**：[`day_hours`] 的柱子是把每条会话按真实时钟切片进小时桶
/// （跨小时的会话两头各计各的）；而 `activities.local_hour` 是**会话开始时刻**的
/// 小时、seal 时不更新——按它过滤会出现"柱子有量、点开明细为空/对不上"（采集
/// 间隔调大时跨界会话可达 10 分钟）。所以这里同样取整天的行、在 Rust 里
/// [`slice_by_hour`] 切片后只留目标小时的秒数再聚合。
pub async fn day_hour_apps(
    pool: &DbPool,
    day_offset: i32,
    hour: i32,
    limit: u32,
    device: DeviceFilter,
) -> Result<Vec<AppUsage>> {
    let target = Local::now() + Duration::days(day_offset as i64);
    let date = target.format("%Y-%m-%d").to_string();

    // (display, cat, icon_process, started, ended) —— 聚合放到切片之后做
    let rows: Vec<(String, String, String, String, String)> = pool
        .0
        .call(move |conn| {
            let sql = format!(
                "SELECT COALESCE(g.display_name, a.process_name)        AS display,
                        COALESCE(c.id, 'other')                         AS cat,
                        a.process_name                                  AS icon_process,
                        a.started_at, a.ended_at
                 FROM activities a
                 LEFT JOIN app_group_members gm
                   ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
                 LEFT JOIN app_groups g
                   ON g.id = gm.group_id AND g.deleted_at IS NULL
                 LEFT JOIN categories c
                   ON c.id = g.category_id AND c.deleted_at IS NULL
                 WHERE a.local_date = ? {}
                   AND g.category_id IS NOT 'hidden'
                   AND a.excluded = 0",
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
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
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

    // 按 display 聚合目标小时内的切片秒数；icon_process 取该组见到的第一个成员名
    let mut agg: std::collections::HashMap<String, (String, String, u64)> =
        std::collections::HashMap::new();
    for (display, cat, icon_process, started, ended) in rows {
        let s = parse_local(&started);
        let e = parse_local(&ended);
        if e <= s {
            continue;
        }
        let hour_secs: u64 = slice_by_hour(s, e)
            .into_iter()
            .filter(|(h, _)| *h as i32 == hour)
            .map(|(_, secs)| secs)
            .sum();
        if hour_secs == 0 {
            continue;
        }
        let entry = agg.entry(display).or_insert((cat, icon_process, 0));
        entry.2 += hour_secs;
    }

    let mut list: Vec<AppUsage> = agg
        .into_iter()
        .map(|(process, (category_id, icon_process, secs))| AppUsage {
            process,
            category_id,
            minutes: ((secs as f64 / 60.0).round() as u32),
            icon_process,
        })
        .filter(|a| a.minutes > 0)
        .collect();
    list.sort_by_key(|a| std::cmp::Reverse(a.minutes));
    list.truncate(limit as usize);
    Ok(list)
}

/// 「点应用 → 详情抽屉」核心：按 `[from, to]` 日期范围 + 粒度，聚合出时间柱(buckets)
/// 与窗口标题用时(titles)。先解析 icon_process 的 group key（与 [`day_apps`] 的
/// `GROUP BY COALESCE(g.display_name, a.process_name)` 同口径），再对同组活动按粒度聚合。
async fn app_range_detail(
    pool: &DbPool,
    from: NaiveDate,
    to: NaiveDate,
    icon_process: String,
    device: DeviceFilter,
    bucket_by: BucketBy,
) -> Result<AppDetail> {
    let from_str = from.format("%Y-%m-%d").to_string();
    let to_str = to.format("%Y-%m-%d").to_string();
    let name_is_browser = crate::capture::browser_url::is_browser_app(&icon_process);

    let (raw_buckets, titles): (std::collections::HashMap<String, u64>, Vec<TitleUsage>) = pool
        .0
        .call(move |conn| {
            // 1) 解析 group key：有组取 group_id，无组退化为 process_name 本身
            let group_key: String = conn
                .query_row(
                    "SELECT group_id FROM app_group_members
                     WHERE process_name = ?1 AND deleted_at IS NULL",
                    rusqlite::params![icon_process],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .db()?
                .unwrap_or_else(|| icon_process.clone());

            // 2) 时间柱。小时粒度不能按 local_hour 分组：它是会话**开始时刻**的小时、
            //    seal 时不更新（见 day_hour_apps 的口径注释），跨小时会话会整段挤进
            //    开始桶，和 day_hours 的柱子对不上。这里同样取行后在 Rust 里按真实
            //    时钟 slice_by_hour 切片。天粒度仍按 local_date SQL 求和。
            let mut raw: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
            match bucket_by {
                BucketBy::Hour => {
                    let bsql = format!(
                        "SELECT a.started_at, a.ended_at
                         FROM activities a
                         LEFT JOIN app_group_members gm
                           ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
                         LEFT JOIN app_groups g
                           ON g.id = gm.group_id AND g.deleted_at IS NULL
                         WHERE a.local_date >= ? AND a.local_date <= ?
                           AND COALESCE(g.id, a.process_name) = ?
                           AND a.excluded = 0
                           {}",
                        device.sql_clause()
                    );
                    let mut bparams: Vec<&dyn ToSql> = Vec::new();
                    bparams.push(&from_str);
                    bparams.push(&to_str);
                    bparams.push(&group_key);
                    if let Some(extra) = device.extra_param() {
                        bparams.push(extra);
                    }
                    let mut bstmt = conn.prepare(&bsql).db()?;
                    let bit = bstmt
                        .query_map(bparams.as_slice(), |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                        })
                        .db()?;
                    for row in bit {
                        let (started, ended) = row.db()?;
                        let s = parse_local(&started);
                        let e = parse_local(&ended);
                        if e <= s {
                            continue;
                        }
                        for (h, secs) in slice_by_hour(s, e) {
                            *raw.entry(h.to_string()).or_insert(0) += secs;
                        }
                    }
                }
                BucketBy::Day => {
                    let bsql = format!(
                        "SELECT a.local_date AS k, SUM(a.duration_secs) AS total
                         FROM activities a
                         LEFT JOIN app_group_members gm
                           ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
                         LEFT JOIN app_groups g
                           ON g.id = gm.group_id AND g.deleted_at IS NULL
                         WHERE a.local_date >= ? AND a.local_date <= ?
                           AND COALESCE(g.id, a.process_name) = ?
                           AND a.excluded = 0
                           {}
                         GROUP BY k",
                        device.sql_clause()
                    );
                    let mut bparams: Vec<&dyn ToSql> = Vec::new();
                    bparams.push(&from_str);
                    bparams.push(&to_str);
                    bparams.push(&group_key);
                    if let Some(extra) = device.extra_param() {
                        bparams.push(extra);
                    }
                    let mut bstmt = conn.prepare(&bsql).db()?;
                    let bit = bstmt
                        .query_map(bparams.as_slice(), |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64))
                        })
                        .db()?;
                    for row in bit {
                        let (k, secs) = row.db()?;
                        raw.insert(k, secs);
                    }
                }
            }

            // 3) 窗口标题用时：按 (window_title, url_host) 聚合（空标题归一为空串），降序。
            //    同一标题在不同域名下（如"首页"）分开计——它们本就是不同网站的页面。
            let tsql = format!(
                "SELECT COALESCE(a.window_title, '') AS t, a.url_host AS h,
                        SUM(a.duration_secs) AS total
                 FROM activities a
                 LEFT JOIN app_group_members gm
                   ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
                 LEFT JOIN app_groups g
                   ON g.id = gm.group_id AND g.deleted_at IS NULL
                 WHERE a.local_date >= ? AND a.local_date <= ?
                   AND COALESCE(g.id, a.process_name) = ?
                   AND a.excluded = 0
                   {}
                 GROUP BY t, h
                 ORDER BY total DESC",
                device.sql_clause()
            );
            let mut tparams: Vec<&dyn ToSql> = Vec::new();
            tparams.push(&from_str);
            tparams.push(&to_str);
            tparams.push(&group_key);
            if let Some(extra) = device.extra_param() {
                tparams.push(extra);
            }
            let mut tstmt = conn.prepare(&tsql).db()?;
            let tit = tstmt
                .query_map(tparams.as_slice(), |r| {
                    Ok(TitleUsage {
                        title: r.get::<_, String>(0)?,
                        host: r.get::<_, Option<String>>(1)?,
                        secs: r.get::<_, i64>(2)?.max(0) as u32,
                    })
                })
                .db()?;
            let mut titles = Vec::new();
            for row in tit {
                titles.push(row.db()?);
            }

            Ok((raw, titles))
        })
        .await?;

    // 4) 把稀疏聚合铺成"完整有序、含 0 空桶"的柱序列，前端直接渲染
    let buckets = match bucket_by {
        BucketBy::Hour => (0u8..24)
            .map(|h| {
                let key = h.to_string();
                let secs = raw_buckets.get(&key).copied().unwrap_or(0) as u32;
                DetailBucket { key, secs }
            })
            .collect(),
        BucketBy::Day => {
            let mut out = Vec::new();
            let mut cur = from;
            while cur <= to {
                let key = cur.format("%Y-%m-%d").to_string();
                let secs = raw_buckets.get(&key).copied().unwrap_or(0) as u32;
                out.push(DetailBucket { key, secs });
                cur += Duration::days(1);
            }
            out
        }
    };

    // 数据优先：有域名的行说明采集侧已把它当浏览器（合并组的代表进程是
    // MIN(process_name)，可能是组里的非浏览器成员名）；没数据再按名字判——
    // 给"是浏览器但一个域名都没有"的提示用。
    let is_browser = name_is_browser || titles.iter().any(|t| t.host.is_some());
    Ok(AppDetail {
        buckets,
        titles,
        is_browser,
    })
}

/// 日报详情：当天按小时聚合（24 桶）。`day_offset = 0` 今天。
pub async fn app_day_detail(
    pool: &DbPool,
    day_offset: i32,
    icon_process: String,
    device: DeviceFilter,
) -> Result<AppDetail> {
    let date = (Local::now() + Duration::days(day_offset as i64)).date_naive();
    app_range_detail(pool, date, date, icon_process, device, BucketBy::Hour).await
}

/// 周报详情：本周(周一~周日)按天聚合（7 桶）。`week_offset = 0` 本周。
pub async fn app_week_detail(
    pool: &DbPool,
    week_offset: i32,
    icon_process: String,
    device: DeviceFilter,
) -> Result<AppDetail> {
    let (monday, sunday) = week_range(week_offset);
    app_range_detail(pool, monday, sunday, icon_process, device, BucketBy::Day).await
}

/// 月报详情：当月每天聚合（28~31 桶）。`month_offset = 0` 本月。
pub async fn app_month_detail(
    pool: &DbPool,
    month_offset: i32,
    icon_process: String,
    device: DeviceFilter,
) -> Result<AppDetail> {
    let (first, last) = month_range(month_offset);
    app_range_detail(pool, first, last, icon_process, device, BucketBy::Day).await
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
                   AND a.excluded = 0
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

    let mut buckets: std::collections::HashMap<String, std::collections::HashMap<String, u64>> =
        std::collections::HashMap::new();
    for (date, cat, secs) in rows {
        // 只丢 secs == 0 的行；<30s 的片段 minutes 会取整成 0，但 secs 必须保留，
        // 否则前端按 secs 求和的总量 / 日均和 top-apps 的 SQL SUM 对不上
        if secs <= 0 {
            continue;
        }
        buckets.entry(date).or_default().insert(cat, secs as u64);
    }

    let mut out = Vec::new();
    let mut cur = from;
    while cur <= to {
        let key = cur.format("%Y-%m-%d").to_string();
        let mut segs: Vec<HourSegment> = buckets
            .remove(&key)
            .unwrap_or_default()
            .into_iter()
            .map(|(category_id, secs)| HourSegment {
                category_id,
                minutes: (secs as f64 / 60.0).round() as u32,
                secs,
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
            // 同 day_apps：按「显示名 + 分类」聚合（同名实体合并），icon_process = MIN(process_name)
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
                   AND a.excluded = 0
                 GROUP BY COALESCE(g.display_name, a.process_name), COALESCE(c.id, 'other')
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
        // DST 回拨时整点是 Ambiguous，.single() 会返回 None 把剩余全塞进当前桶；
        // 用 .latest()（第二次出现的那个整点）：重复时段墙钟仍显示同一小时，归入
        // 当前桶本来就正确，且 +1h 后严格大于 cur，循环保证前进
        let next_hour = Local
            .with_ymd_and_hms(cur.year(), cur.month(), cur.day(), cur.hour(), 0, 0)
            .latest()
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

    /// 忽略规则打标的行（excluded=1）不进报表口径——day_apps 该只剩没打标的行。
    #[tokio::test]
    async fn day_apps_skips_excluded_rows() {
        let pool = fresh_test_pool().await;
        let today = Local::now().format("%Y-%m-%d").to_string();
        insert_activity(&pool, "dev-a", &today, "Downloader", 600).await;
        insert_activity(&pool, "dev-a", &today, "Editor", 600).await;
        pool.0
            .call(|conn| {
                conn.execute(
                    "UPDATE activities SET excluded = 1 WHERE process_name = 'Downloader'",
                    [],
                )
                .db()?;
                Ok(())
            })
            .await
            .unwrap();

        let rows = day_apps(&pool, 0, 50, DeviceFilter::All).await.unwrap();
        assert_eq!(rows.len(), 1, "excluded 行不该出现在 day_apps");
        assert_eq!(rows[0].process, "Editor");
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

        let h10 = day_hour_apps(&pool, 0, 10, 50, DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(h10.len(), 1, "hour=10 只应有 Code");
        assert_eq!(h10[0].process, "Code");

        let h11 = day_hour_apps(&pool, 0, 11, 50, DeviceFilter::All)
            .await
            .unwrap();
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

    /// 测 [`device_filter_from_option`]：None / 空串 / 全空白 → All；非空 → Only。
    /// 这是所有报表 Tauri 命令入口的参数规整口径，错了会把"全设备"误当成某台设备查。
    #[test]
    fn device_filter_from_option_normalizes() {
        assert!(matches!(device_filter_from_option(None), DeviceFilter::All));
        assert!(matches!(
            device_filter_from_option(Some(String::new())),
            DeviceFilter::All
        ));
        // 全空白（含 tab）也归 All——前端 select 未选中时可能传占位空白串
        assert!(matches!(
            device_filter_from_option(Some("  \t ".into())),
            DeviceFilter::All
        ));
        match device_filter_from_option(Some("device-win".into())) {
            DeviceFilter::Only(id) => assert_eq!(id, "device-win"),
            DeviceFilter::All => panic!("非空 id 应得到 Only，不是 All"),
        }
        // 两端带空白但中间非空 → 保留原串的 Only（当前契约：只用 trim 判空，不改写 id）
        match device_filter_from_option(Some(" dev ".into())) {
            DeviceFilter::Only(id) => assert_eq!(id, " dev ", "id 原样保留，不做 trim 改写"),
            DeviceFilter::All => panic!("含非空白字符的串不该归 All"),
        }
    }

    /// 测 [`app_day_detail`]（Hour 粒度）：
    /// - 固定 24 桶、key 按 "0".."23" 有序、无活动小时补 0
    /// - 跨小时会话按真实时钟切片分摊（10:30→11:30 应各给 10/11 点 1800s），
    ///   而不是按 local_hour 把整段挤进开始桶
    #[tokio::test]
    async fn app_day_detail_hour_buckets_split_and_zero_fill() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();

        // 9:15→9:20 = 300s（单小时内）
        let s1 = Local
            .from_local_datetime(&today.and_hms_opt(9, 15, 0).unwrap())
            .single()
            .unwrap();
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            &today_str,
            "Code",
            s1,
            s1 + Duration::minutes(5),
        )
        .await;
        // 10:30→11:30 = 跨小时，两头各 1800s
        let s2 = Local
            .from_local_datetime(&today.and_hms_opt(10, 30, 0).unwrap())
            .single()
            .unwrap();
        insert_session_with_times(
            &pool,
            TEST_SELF_ID,
            &today_str,
            "Code",
            s2,
            s2 + Duration::hours(1),
        )
        .await;
        seed_solo_group(&pool, "Code", "code").await;

        let detail = app_day_detail(&pool, 0, "Code".into(), DeviceFilter::All)
            .await
            .unwrap();

        assert_eq!(detail.buckets.len(), 24, "小时粒度固定 24 桶");
        let keys: Vec<&str> = detail.buckets.iter().map(|b| b.key.as_str()).collect();
        let expect_keys: Vec<String> = (0u8..24).map(|h| h.to_string()).collect();
        assert_eq!(
            keys,
            expect_keys.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "key 应为有序的 \"0\"..\"23\""
        );
        for b in &detail.buckets {
            let expect = match b.key.as_str() {
                "9" => 300,
                "10" | "11" => 1800,
                _ => 0,
            };
            assert_eq!(b.secs, expect, "hour={} 的 secs 不符", b.key);
        }
    }

    /// 「按网站」分组的原料：titles 行带 url_host，同标题不同域名分开计
    /// （"首页"在 github 和 youtube 各算各的），无域名的老行照常返回且 host=None；
    /// 代表进程是浏览器时 is_browser=true，否则 false——前端据此决定是否分组。
    #[tokio::test]
    async fn app_day_detail_titles_carry_url_host_and_browser_flag() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive().format("%Y-%m-%d").to_string();
        let d = today.clone();
        pool.0
            .call(move |conn| {
                let rows: [(&str, &str, Option<&str>, i64); 5] = [
                    (
                        "Google Chrome",
                        "Hindsight - GitHub",
                        Some("github.com"),
                        120,
                    ),
                    ("Google Chrome", "首页", Some("github.com"), 30),
                    ("Google Chrome", "首页", Some("youtube.com"), 45),
                    ("Google Chrome", "旧记录", None, 60),
                    ("Code", "main.rs", None, 50),
                ];
                for (i, (p, t, h, secs)) in rows.iter().enumerate() {
                    let at = format!("{d}T10:0{i}:00Z");
                    conn.execute(
                        "INSERT INTO activities(
                            started_at, ended_at, duration_secs, local_date, local_hour,
                            process_name, window_title, category_id, device_id,
                            updated_at, origin, url_host
                         ) VALUES(?1, ?1, ?2, ?3, 10, ?4, ?5, 'other', 'dev-a', ?1, 'local', ?6)",
                        rusqlite::params![at, secs, d, p, t, h],
                    )
                    .db()?;
                }
                Ok(())
            })
            .await
            .unwrap();

        let chrome = app_day_detail(&pool, 0, "Google Chrome".into(), DeviceFilter::All)
            .await
            .unwrap();
        assert!(chrome.is_browser, "Google Chrome 是浏览器");
        let secs_of = |t: &str, h: Option<&str>| {
            chrome
                .titles
                .iter()
                .find(|x| x.title == t && x.host.as_deref() == h)
                .map(|x| x.secs)
        };
        assert_eq!(secs_of("Hindsight - GitHub", Some("github.com")), Some(120));
        assert_eq!(
            secs_of("首页", Some("github.com")),
            Some(30),
            "同标题不同域名分开计"
        );
        assert_eq!(secs_of("首页", Some("youtube.com")), Some(45));
        assert_eq!(
            secs_of("旧记录", None),
            Some(60),
            "无域名行照常返回,host=None"
        );
        assert_eq!(chrome.titles.len(), 4, "不掺入组外应用");

        let code = app_day_detail(&pool, 0, "Code".into(), DeviceFilter::All)
            .await
            .unwrap();
        assert!(!code.is_browser, "Code 不是浏览器");
        assert_eq!(code.titles.len(), 1);
        assert_eq!(code.titles[0].host, None);
    }

    /// 测 [`app_range_detail`] 的组 key 解析：icon_process 是组内任一成员时，
    /// 时间柱与标题都应聚合**整个组**（跨 OS 成员 + 跨设备），且不掺入组外应用。
    /// titles 按用时降序、同标题跨成员合并。
    #[tokio::test]
    async fn app_day_detail_resolves_group_and_merges_members() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();

        // 组 "Visual Studio Code"：成员 mac="Code" + win="Code.exe"
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

        let s = |h: u32, m: u32| {
            Local
                .from_local_datetime(&today.and_hms_opt(h, m, 0).unwrap())
                .single()
                .unwrap()
        };
        // 本机 Code：10:00→10:05 (300s) 标题 "main.rs"
        insert_session_titled(
            &pool,
            TEST_SELF_ID,
            &today_str,
            "Code",
            "main.rs",
            s(10, 0),
            s(10, 5),
        )
        .await;
        // win 端 Code.exe：10:10→10:14 (240s) 标题 "lib.rs"；再补一段同标题 "main.rs" 60s
        insert_session_titled(
            &pool,
            "device-win",
            &today_str,
            "Code.exe",
            "lib.rs",
            s(10, 10),
            s(10, 14),
        )
        .await;
        insert_session_titled(
            &pool,
            "device-win",
            &today_str,
            "Code.exe",
            "main.rs",
            s(10, 20),
            s(10, 21),
        )
        .await;
        // 组外应用同时段活动：绝不能混进 Code 的详情
        insert_session_titled(
            &pool,
            TEST_SELF_ID,
            &today_str,
            "Random",
            "noise",
            s(10, 0),
            s(10, 30),
        )
        .await;

        // 用 win 侧成员名查询 → 应解析到组、把 mac 侧的量也算上
        let detail = app_day_detail(&pool, 0, "Code.exe".into(), DeviceFilter::All)
            .await
            .unwrap();
        let h10 = detail.buckets.iter().find(|b| b.key == "10").unwrap();
        assert_eq!(h10.secs, 300 + 240 + 60, "组内两成员 10 点的量应合并");
        let total: u32 = detail.buckets.iter().map(|b| b.secs).sum();
        assert_eq!(total, 600, "组外应用(Random)不该混入");

        // titles：main.rs 跨成员合并 = 300+60 = 360 > lib.rs 240，降序
        assert_eq!(detail.titles.len(), 2, "标题只该有组内两种");
        assert_eq!(detail.titles[0].title, "main.rs");
        assert_eq!(detail.titles[0].secs, 360);
        assert_eq!(detail.titles[1].title, "lib.rs");
        assert_eq!(detail.titles[1].secs, 240);

        // 设备过滤：Only(self) 只剩本机 Code 的 300s
        let only_self = app_day_detail(
            &pool,
            0,
            "Code.exe".into(),
            DeviceFilter::Only(TEST_SELF_ID.into()),
        )
        .await
        .unwrap();
        let h10_self = only_self.buckets.iter().find(|b| b.key == "10").unwrap();
        assert_eq!(h10_self.secs, 300, "Only(self) 不该带上 win 端时长");
    }

    /// 测 [`app_range_detail`] 无组回退：icon_process 没有任何组成员记录时，
    /// 组 key 退化为 process_name 本身——只聚合同名进程，不吸入其它无组进程。
    #[tokio::test]
    async fn app_day_detail_falls_back_to_process_name_without_group() {
        let pool = fresh_test_pool().await;
        let today = Local::now().date_naive();
        let today_str = today.format("%Y-%m-%d").to_string();
        let s = |h: u32, m: u32| {
            Local
                .from_local_datetime(&today.and_hms_opt(h, m, 0).unwrap())
                .single()
                .unwrap()
        };
        // 两个都没建组（v15 backfill 前的历史数据形态）
        insert_session_titled(
            &pool,
            TEST_SELF_ID,
            &today_str,
            "Lonely",
            "doc",
            s(14, 0),
            s(14, 10),
        )
        .await;
        insert_session_titled(
            &pool,
            TEST_SELF_ID,
            &today_str,
            "OtherApp",
            "noise",
            s(14, 0),
            s(14, 5),
        )
        .await;

        let detail = app_day_detail(&pool, 0, "Lonely".into(), DeviceFilter::All)
            .await
            .unwrap();
        let h14 = detail.buckets.iter().find(|b| b.key == "14").unwrap();
        assert_eq!(h14.secs, 600, "无组时按 process_name 精确匹配");
        let total: u32 = detail.buckets.iter().map(|b| b.secs).sum();
        assert_eq!(total, 600, "其它无组进程不该被吸进来");
        assert_eq!(detail.titles.len(), 1);
        assert_eq!(detail.titles[0].title, "doc");
        assert_eq!(detail.titles[0].secs, 600);
    }

    /// 测 [`app_week_detail`]（Day 粒度）：7 桶按日期有序、空天补 0、
    /// 范围外（上周日）的同名活动被排除。
    #[tokio::test]
    async fn app_week_detail_day_buckets_zero_filled_in_order() {
        let pool = fresh_test_pool().await;
        let (monday, _sunday) = week_range(0);
        let day = |off: i64| {
            (monday + Duration::days(off))
                .format("%Y-%m-%d")
                .to_string()
        };

        insert_activity(&pool, TEST_SELF_ID, &day(0), "Code", 600).await;
        insert_activity(&pool, TEST_SELF_ID, &day(2), "Code", 900).await;
        // 上周日的量：若范围下界写错（>= 变 >，或 monday 计算错）会漏进来
        insert_activity(&pool, TEST_SELF_ID, &day(-1), "Code", 12345).await;
        seed_solo_group(&pool, "Code", "code").await;

        let detail = app_week_detail(&pool, 0, "Code".into(), DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(detail.buckets.len(), 7, "周详情固定 7 桶");
        for (i, b) in detail.buckets.iter().enumerate() {
            assert_eq!(b.key, day(i as i64), "第 {i} 桶的日期 key 不符");
            let expect = match i {
                0 => 600,
                2 => 900,
                _ => 0,
            };
            assert_eq!(b.secs, expect, "{} 的 secs 不符", b.key);
        }
    }

    /// 测 [`app_month_detail`]（Day 粒度）：桶数 = 当月天数，逐日有序补 0。
    #[tokio::test]
    async fn app_month_detail_covers_whole_month() {
        let pool = fresh_test_pool().await;
        let (first, last) = month_range(0);
        let n_days = ((last - first).num_days() + 1) as usize;
        let day = |off: i64| (first + Duration::days(off)).format("%Y-%m-%d").to_string();

        insert_activity(&pool, TEST_SELF_ID, &day(0), "Code", 300).await;
        insert_activity(&pool, TEST_SELF_ID, &day(14), "Code", 450).await;
        seed_solo_group(&pool, "Code", "code").await;

        let detail = app_month_detail(&pool, 0, "Code".into(), DeviceFilter::All)
            .await
            .unwrap();
        assert_eq!(detail.buckets.len(), n_days, "桶数应等于当月天数(28~31)");
        for (i, b) in detail.buckets.iter().enumerate() {
            assert_eq!(b.key, day(i as i64));
            let expect = match i {
                0 => 300,
                14 => 450,
                _ => 0,
            };
            assert_eq!(b.secs, expect, "{} 的 secs 不符", b.key);
        }
    }

    /// 测 [`week_apps`]：同一应用跨多天求和成一行，范围外（上周）的量不掺入。
    #[tokio::test]
    async fn week_apps_sums_across_days_and_excludes_prev_week() {
        let pool = fresh_test_pool().await;
        let (monday, _) = week_range(0);
        let day = |off: i64| {
            (monday + Duration::days(off))
                .format("%Y-%m-%d")
                .to_string()
        };

        insert_activity(&pool, TEST_SELF_ID, &day(0), "Code", 300).await;
        insert_activity(&pool, TEST_SELF_ID, &day(2), "Code", 300).await;
        // 上周日一大段：若被算进来 minutes 会变 10+100=110，一眼可辨
        insert_activity(&pool, TEST_SELF_ID, &day(-1), "Code", 6000).await;
        seed_solo_group(&pool, "Code", "code").await;

        let apps = week_apps(&pool, 0, 50, DeviceFilter::All).await.unwrap();
        assert_eq!(apps.len(), 1, "同一应用跨天应合并成一行");
        assert_eq!(apps[0].process, "Code");
        assert_eq!(
            apps[0].minutes, 10,
            "300+300=600s=10min，上周的 6000s 不该掺入"
        );
        assert_eq!(apps[0].category_id, "code");
    }

    /// 测 [`month_days`]：行数 = 当月天数、按日期有序，有数据的天分类分钟正确、
    /// 无数据的天 segments 为空，上月末尾的量不掺入。
    #[tokio::test]
    async fn month_days_zero_fills_whole_month() {
        let pool = fresh_test_pool().await;
        let (first, last) = month_range(0);
        let n_days = ((last - first).num_days() + 1) as usize;
        let day = |off: i64| (first + Duration::days(off)).format("%Y-%m-%d").to_string();

        insert_activity(&pool, TEST_SELF_ID, &day(0), "Code", 300).await; // 5 min
        insert_activity(&pool, TEST_SELF_ID, &day(9), "Code", 600).await; // 10 min
        insert_activity(&pool, TEST_SELF_ID, &day(-1), "Code", 999).await; // 上月末，应被排除
        seed_solo_group(&pool, "Code", "code").await;

        let days = month_days(&pool, 0, DeviceFilter::All).await.unwrap();
        assert_eq!(days.len(), n_days, "行数应等于当月天数");
        for (i, d) in days.iter().enumerate() {
            assert_eq!(d.date, day(i as i64), "第 {i} 行日期不符");
            match i {
                0 | 9 => {
                    let code_min: u32 = d
                        .segments
                        .iter()
                        .filter(|s| s.category_id == "code")
                        .map(|s| s.minutes)
                        .sum();
                    assert_eq!(code_min, if i == 0 { 5 } else { 10 });
                }
                _ => assert!(d.segments.is_empty(), "{} 不该有数据", d.date),
            }
        }
    }

    /// 同 [`insert_session_with_times`]，但可指定 window_title——给详情抽屉的
    /// titles 聚合断言用。
    #[allow(clippy::too_many_arguments)]
    async fn insert_session_titled(
        pool: &DbPool,
        device_id: &str,
        local_date: &str,
        process_name: &str,
        title: &str,
        started: DateTime<Local>,
        ended: DateTime<Local>,
    ) {
        let device_id = device_id.to_string();
        let local_date = local_date.to_string();
        let process_name = process_name.to_string();
        let title = title.to_string();
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
                     ) VALUES(?, ?, ?, ?, ?, ?, ?, 'other', ?, ?, 'local')",
                    rusqlite::params![
                        started_str,
                        ended_str,
                        dur,
                        local_date,
                        local_hour,
                        process_name,
                        title,
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
