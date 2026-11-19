//! query_stats：统计使用时长、使用次数，或最早和最近一次的时间。

use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ai::model::ToolSpec;
use crate::ai::tools::kinds::params::{
    clean_short_strings, date_schema, invalid, parse_range, read_args, short_strings_schema, spec,
};
use crate::ai::tools::kinds::Tool;
use crate::ai::tools::{like_pattern, truncate, ChatLang, ToolCtx, ToolError, ToolOutput};
use crate::error::Result;
use crate::storage::{DbResult, SqliteResultExt};

#[derive(Debug, Clone, PartialEq)]
pub struct StatsQuery {
    pub range: (NaiveDate, NaiveDate),
    pub apps: Vec<String>,
    pub title_keyword: Option<String>,
    /// 按分类名过滤(如"游戏")。与 bucket 正交——"游戏时间的周趋势"
    /// 的正确姿势就是 categories=["游戏"] + bucket=week;没有它,模型只能
    /// 靠猜应用名列举,认不出用户自定义归类的应用(实测漏算 2 小时)。
    pub categories: Vec<String>,
    pub group_by: GroupBy,
    pub top_n: usize,
    pub metric: StatMetric,
    /// 会话计数用:相邻活动间隔超过这么多分钟就算一段新会话
    pub gap_minutes: u32,
    /// 趋势分桶(与 group_by 互斥,validate 拦)
    pub bucket: Bucket,
    /// bucket=Week 且由 Day 自动升级而来(范围 > BUCKET_DAY_MAX_DAYS):
    /// 结果头部要向模型说明,免得它以为工具没按参数执行
    pub bucket_auto_week: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    None,
    App,
    Title,
    /// 按用户的应用分类(工作/娱乐…)分组;组→分类的指派 join 出来,与报表同口径
    Category,
}

/// 统计口径:时长(默认)还是"用了几次/玩了几次"的会话次数。
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatMetric {
    /// 累计使用时长(SUM duration_secs)
    Duration,
    /// 使用会话次数:相邻活动间隔超过 gap_minutes 就切一段
    SessionCount,
    /// 最早/最近一次记录的时间("第一次用 X 是什么时候"):
    /// MIN/MAX 聚合,忽略分组与分桶,豁免常规日期跨度上限
    FirstLast,
}

/// 趋势分桶:把范围切成时间桶看变化,与 group_by(排行)互斥。
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    None,
    /// 逐日;范围超 BUCKET_DAY_MAX_DAYS 天时 validate 自动升级为 Week
    Day,
    /// 逐周,周一为周起始(与报表口径一致),行首日期为该周周一
    Week,
    /// 按一天中的 0-23 时聚合(范围内所有天累计),看作息分布
    HourOfDay,
}

/// 模型传来的参数，检查之前。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Args {
    date_from: Option<String>,
    date_to: Option<String>,
    apps: Vec<String>,
    categories: Vec<String>,
    title_keyword: Option<String>,
    group_by: Option<GroupBy>,
    top_n: Option<usize>,
    metric: Option<StatMetric>,
    gap_minutes: Option<u32>,
    bucket: Option<Bucket>,
}

/// 常规查询的日期跨度上限(一年);first_last 口径的宽限见 [`FIRST_LAST_MAX_DAYS`]。
const RANGE_MAX_DAYS: i64 = 366;
/// first_last(MIN/MAX 聚合)的跨度上限:名义上限,只防荒谬输入。
const FIRST_LAST_MAX_DAYS: i64 = 36600;
/// 分组统计 top-N 上限
const TOP_N_MAX: usize = 10;
/// 逐日分桶的最大跨度:超过则自动按周汇总——60 天 ≈ 60 行是结果预算内
/// 能完整放下、模型也读得过来的上限。
const BUCKET_DAY_MAX_DAYS: i64 = 60;
/// 会话计数的间隔默认值与夹紧区间(分钟)。
const GAP_DEFAULT_MIN: u32 = 30;
const GAP_MIN: u32 = 5;
const GAP_MAX: u32 = 240;

impl Tool for StatsQuery {
    const NAME: &'static str = "query_stats";

    fn spec(_lang: ChatLang) -> ToolSpec {
        spec(
            Self::NAME,
            "统计应用/内容的使用时长或使用次数。可按应用名过滤(apps)、按分类名过滤(categories)、按窗口标题关键词过滤(title_keyword,如视频名),可分组排行(group_by,含按用户分类的工作/娱乐口径),可分桶看趋势(bucket)。问'用了多久/花了多少时间'用默认(metric=duration);问'启动/打开/玩了几次'用 metric=session_count;问'第一次/最近一次用X是什么时候'用 metric=first_last(此口径 date_from 可给很早的日期,如 2020-01-01)。问某类活动(如游戏)的趋势用 categories+bucket,不要自己列举应用名——用户归类过的应用你未必认识。",
            json!({"type": "object", "properties": {
                "date_from": date_schema("起始日期,格式 YYYY-MM-DD"),
                "date_to": date_schema("结束日期,可选,不给就到今天,格式 YYYY-MM-DD"),
                "apps": short_strings_schema("应用名过滤,可选", None),
                "categories": short_strings_schema("分类名过滤(如'游戏'),按用户的应用分类圈定范围,可选;分类名可先用 group_by=category 查到", None),
                "title_keyword": {"type": "string", "maxLength": 64, "description": "窗口标题关键词过滤,可选"},
                "group_by": {"type": "string", "enum": ["none", "app", "title", "category"], "description": "分组维度,默认 none;category=按用户设置的应用分类(工作/娱乐/游戏等)分组"},
                "top_n": {"type": "integer", "minimum": 1, "maximum": 10, "description": "分组时取前 N,默认 5"},
                "metric": {"type": "string", "enum": ["duration", "session_count", "first_last"], "description": "统计口径:duration=累计时长(默认);session_count=使用会话次数;first_last=最早与最近一次记录的时间(忽略分组/分桶)"},
                "gap_minutes": {"type": "integer", "minimum": 5, "maximum": 240, "description": "会话计数用:相邻活动间隔超过这么多分钟算一段新会话,默认 30。仅用户明确说'离开X分钟以上算一次'时才填"},
                "bucket": {"type": "string", "enum": ["none", "day", "week", "hour_of_day"], "description": "趋势分桶,与 group_by 互斥:day=逐日(最多 60 天,更长自动按周);week=逐周(行首日期为周一);hour_of_day=按一天 0-23 时聚合看作息分布"}
            }}),
        )
    }

    fn parse(
        args: &Value,
        today: NaiveDate,
        lang: ChatLang,
    ) -> std::result::Result<Self, ToolError> {
        let args: Args = read_args(args, lang)?;
        check(&args, today, lang).map_err(|msg| invalid(&msg, lang))
    }

    async fn execute(
        &self,
        ctx: &ToolCtx,
        _next_citation: usize,
        _max_chars: usize,
        lang: ChatLang,
    ) -> Result<ToolOutput> {
        run(ctx, self, lang).await
    }
}

/// 检查参数的值，补上默认值。不合法时返回原因。
fn check(args: &Args, today: NaiveDate, lang: ChatLang) -> std::result::Result<StatsQuery, String> {
    let metric = args.metric.unwrap_or(StatMetric::Duration);
    // first_last 是 MIN/MAX 聚合,大范围也便宜——豁免常规跨度上限,
    // 否则"我第一次用 X 是什么时候"这类必须给多年范围的问题会被拒
    let max_days = if metric == StatMetric::FirstLast {
        FIRST_LAST_MAX_DAYS
    } else {
        RANGE_MAX_DAYS
    };
    let range = parse_range(
        args.date_from.as_deref(),
        args.date_to.as_deref(),
        today,
        max_days,
        lang,
    )?
    .ok_or_else(|| lang.err_need_range(StatsQuery::NAME))?;
    let apps = clean_short_strings(&args.apps, "apps", lang)?;
    let categories = clean_short_strings(&args.categories, "categories", lang)?;
    let title_keyword = match args.title_keyword.as_deref().map(str::trim) {
        Some("") | None => None,
        Some(t) if t.chars().count() <= 64 => Some(t.to_string()),
        Some(_) => return Err(lang.err_title_kw_too_long().into()),
    };
    let group_by = args.group_by.unwrap_or(GroupBy::None);
    let mut bucket = args.bucket.unwrap_or(Bucket::None);
    // 分桶(趋势)与分组(排行)是两种输出形态,叠加会变成矩阵、吃穿结果预算
    if bucket != Bucket::None && group_by != GroupBy::None {
        return Err(lang.err_bucket_with_group().into());
    }
    let mut bucket_auto_week = false;
    if bucket == Bucket::Day && (range.1 - range.0).num_days() > BUCKET_DAY_MAX_DAYS {
        bucket = Bucket::Week;
        bucket_auto_week = true;
    }
    // 分类分组的组数天然少(内置+自定义十几个),默认 top-5 会截掉小类
    // (实测漏了"其他/设计")——未显式给 top_n 时放宽到上限
    let top_n_default = if group_by == GroupBy::Category {
        TOP_N_MAX
    } else {
        5
    };
    Ok(StatsQuery {
        range,
        apps,
        title_keyword,
        categories,
        group_by,
        top_n: args.top_n.unwrap_or(top_n_default).clamp(1, TOP_N_MAX),
        metric,
        gap_minutes: args
            .gap_minutes
            .unwrap_or(GAP_DEFAULT_MIN)
            .clamp(GAP_MIN, GAP_MAX),
        bucket,
        bucket_auto_week,
    })
}

/// 统计查询的基础 FROM 段(别名 a,与既有口径一致:裸活动表、不排 hidden 组)。
const STATS_FROM: &str = "FROM activities a";
/// FROM clause for grouping by category: activity → group → category, two joins.
///
/// Deliberately **not** [`crate::repo::sql::FROM_ACTIVITY_GROUP_CATEGORY`]: that
/// one joins on `c.id = g.category_id`, leaving `c.*` NULL for an ungrouped
/// activity, and its callers bucket those with `COALESCE(c.id, 'other')`. Here
/// the fallback is inside the join, so ungrouped activities reach the real
/// `other` row and `c.name` is the user's localised label rather than NULL —
/// which is what the model needs to restate in the answer.
/// The category lives on the group (`g.category_id`); an activity whose app has
/// no group falls back to `'other'`. `a.category_id` is a dead column (always
/// `'other'`, see `repo::activities`) and is deliberately not read here.
/// Hidden groups are filtered out in WHERE, matching the reports behaviour.
const STATS_FROM_CATEGORY: &str = "FROM activities a
     LEFT JOIN app_group_members gm
       ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
     LEFT JOIN app_groups g
       ON g.id = gm.group_id AND g.deleted_at IS NULL
     LEFT JOIN categories c
       ON c.id = COALESCE(NULLIF(g.category_id, 'none'), 'other') AND c.deleted_at IS NULL";

async fn run(ctx: &ToolCtx, q: &StatsQuery, lang: ChatLang) -> Result<ToolOutput> {
    let (from, to) = (q.range.0.to_string(), q.range.1.to_string());
    let apps: Vec<String> = q.apps.iter().map(|a| like_pattern(a)).collect();
    let title_like = q.title_keyword.as_deref().map(like_pattern);
    let cats: Vec<String> = q.categories.iter().map(|c| like_pattern(c)).collect();
    let StatsQuery {
        group_by,
        top_n,
        metric,
        gap_minutes,
        bucket,
        bucket_auto_week,
        ..
    } = *q;
    let content = ctx
        .main
        .call(move |conn| {
            // 动态拼的只有 WHERE 的占位段与 GROUP BY/分桶维度,全部来自白名单枚举,
            // 参数一律绑定——不存在字符串拼接注入面。
            let needs_category = group_by == GroupBy::Category || !cats.is_empty();
            let from_sql = if needs_category {
                STATS_FROM_CATEGORY
            } else {
                STATS_FROM
            };
            let mut where_sql = String::from("a.local_date BETWEEN ?1 AND ?2 AND a.excluded = 0");
            if needs_category {
                where_sql.push_str(" AND g.category_id IS NOT 'hidden'");
            }
            let mut bind: Vec<Box<dyn rusqlite::ToSql>> =
                vec![Box::new(from.clone()), Box::new(to.clone())];
            if !apps.is_empty() {
                let ors = vec!["a.process_name LIKE ? ESCAPE '\\'"; apps.len()].join(" OR ");
                where_sql.push_str(&format!(" AND ({ors})"));
                for a in &apps {
                    bind.push(Box::new(a.clone()));
                }
            }
            if !cats.is_empty() {
                // 显示名与分类 id 双匹配:模型可能填用户语言的名字("游戏"),
                // 也可能填英文 builtin id("game")
                let ors = vec![
                    "(c.name LIKE ? ESCAPE '\\' \
                      OR COALESCE(NULLIF(g.category_id, 'none'), 'other') LIKE ? ESCAPE '\\')";
                    cats.len()
                ]
                .join(" OR ");
                where_sql.push_str(&format!(" AND ({ors})"));
                for c in &cats {
                    bind.push(Box::new(c.clone()));
                    bind.push(Box::new(c.clone()));
                }
            }
            if let Some(t) = &title_like {
                where_sql.push_str(" AND a.window_title LIKE ? ESCAPE '\\'");
                bind.push(Box::new(t.clone()));
            }
            let params_ref: Vec<&dyn rusqlite::ToSql> = bind.iter().map(|b| b.as_ref()).collect();

            // first_last 忽略分组/分桶;bucket 只与 none 分组组合(validate 拦)
            if metric == StatMetric::FirstLast {
                return query_first_last(conn, from_sql, &where_sql, &params_ref, &from, &to, lang);
            }
            if bucket != Bucket::None {
                return query_bucket(
                    conn,
                    from_sql,
                    &where_sql,
                    &params_ref,
                    bucket,
                    bucket_auto_week,
                    metric,
                    gap_minutes,
                    &from,
                    &to,
                    lang,
                );
            }
            match metric {
                StatMetric::Duration => query_duration(
                    conn,
                    from_sql,
                    &where_sql,
                    &params_ref,
                    group_by,
                    top_n,
                    &from,
                    &to,
                    lang,
                ),
                StatMetric::SessionCount => query_session_count(
                    conn,
                    from_sql,
                    &where_sql,
                    &params_ref,
                    group_by,
                    top_n,
                    gap_minutes,
                    &from,
                    &to,
                    lang,
                ),
                StatMetric::FirstLast => unreachable!(),
            }
        })
        .await?;
    Ok(ToolOutput {
        content,
        citations: Vec::new(),
    })
}

/// 时长口径:SUM(duration_secs),可分组 top-N。
#[allow(clippy::too_many_arguments)]
fn query_duration(
    conn: &rusqlite::Connection,
    from_sql: &str,
    where_sql: &str,
    params: &[&dyn rusqlite::ToSql],
    group_by: GroupBy,
    top_n: usize,
    from: &str,
    to: &str,
    lang: ChatLang,
) -> DbResult<String> {
    match group_by {
        GroupBy::None => {
            let secs: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COALESCE(SUM(a.duration_secs),0) {from_sql} WHERE {where_sql}"
                    ),
                    params,
                    |r| r.get(0),
                )
                .db()?;
            Ok(lang.stats_total(from, to, &lang.fmt_secs(secs)))
        }
        GroupBy::App | GroupBy::Title | GroupBy::Category => {
            let dim = group_dim(group_by);
            // 全集组数:让模型知道 top-N 之外还有多少组,别把前 5 当成全部。
            // 不足 1 分钟的组整组滤掉——显示出来是"xx: 0 分钟"的噪音行
            // (实测分类分组冒出过"fun: 0 分钟"),universe 与列表同口径。
            let universe: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM (SELECT 1 {from_sql} WHERE {where_sql}
                         GROUP BY {dim} HAVING SUM(a.duration_secs) >= 60)"
                    ),
                    params,
                    |r| r.get(0),
                )
                .db()?;
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {dim}, SUM(a.duration_secs) d {from_sql}
                     WHERE {where_sql} GROUP BY {dim} HAVING d >= 60
                     ORDER BY d DESC LIMIT {top_n}"
                ))
                .db()?;
            let rows = stmt
                .query_map(params, |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            if rows.is_empty() {
                return Ok(lang.no_match(from, to));
            }
            let body = rows
                .iter()
                .map(|(name, secs)| format!("{}: {}", truncate(name, 50), lang.fmt_secs(*secs)))
                .collect::<Vec<_>>()
                .join("\n");
            let header = lang.duration_header(from, to, universe, rows.len());
            Ok(format!("{header}\n{body}"))
        }
    }
}

/// 会话次数口径:按 started_at 排序取活动流,相邻两条间隔(前一条 ended → 后一条 started)
/// 超过 gap_minutes 就切一段。分组时每组各自切分并计数。
/// Hindsight 记录的是前台焦点会话,非进程启动——间隔切分把"Alt-Tab 切走再回来"
/// 归并成同一次使用,是对"用了几次/玩了几次"最接近的近似。
#[allow(clippy::too_many_arguments)]
fn query_session_count(
    conn: &rusqlite::Connection,
    from_sql: &str,
    where_sql: &str,
    params: &[&dyn rusqlite::ToSql],
    group_by: GroupBy,
    top_n: usize,
    gap_minutes: u32,
    from: &str,
    to: &str,
    lang: ChatLang,
) -> DbResult<String> {
    let gap_secs = gap_minutes as i64 * 60;
    match group_by {
        GroupBy::None => {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT a.started_at, a.ended_at {from_sql}
                     WHERE {where_sql} ORDER BY a.started_at"
                ))
                .db()?;
            let rows = stmt
                .query_map(params, |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            if rows.is_empty() {
                return Ok(lang.no_match(from, to));
            }
            let n = count_sessions(&rows, gap_secs);
            Ok(lang.sessions_total(from, to, n, gap_minutes))
        }
        GroupBy::App | GroupBy::Title | GroupBy::Category => {
            let dim = group_dim(group_by);
            let rows = fetch_grouped_spans(conn, from_sql, where_sql, params, dim)?;
            if rows.is_empty() {
                return Ok(lang.no_match(from, to));
            }
            let mut counts = count_sessions_by_group(rows, gap_secs);
            counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let universe = counts.len();
            counts.truncate(top_n);
            let body = counts
                .iter()
                .map(|(name, n)| format!("{}: {}", truncate(name, 50), lang.count_suffix(*n)))
                .collect::<Vec<_>>()
                .join("\n");
            let header =
                lang.sessions_grouped_header(from, to, universe, counts.len(), gap_minutes);
            Ok(format!("{header}\n{body}"))
        }
    }
}

/// 取 (组, started_at, ended_at) 活动流,按组、组内按时间升序——
/// 分组与分桶的会话计数共用(桶也是一种"组",维度表达式不同而已)。
fn fetch_grouped_spans(
    conn: &rusqlite::Connection,
    from_sql: &str,
    where_sql: &str,
    params: &[&dyn rusqlite::ToSql],
    dim: &str,
) -> DbResult<Vec<(String, String, String)>> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {dim} AS g, a.started_at, a.ended_at {from_sql}
             WHERE {where_sql} ORDER BY g, a.started_at"
        ))
        .db()?;
    let rows = stmt
        .query_map(params, |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .db()?
        .collect::<rusqlite::Result<Vec<_>>>()
        .db()?;
    Ok(rows)
}

/// 按组聚拢再逐组切分计数(输入已按组排序);保持输入的组顺序,排序/截断由调用方定。
fn count_sessions_by_group(
    rows: Vec<(String, String, String)>,
    gap_secs: i64,
) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    let mut cur_group: Option<String> = None;
    let mut cur_rows: Vec<(String, String)> = Vec::new();
    for (g, s, e) in rows {
        if cur_group.as_deref() != Some(g.as_str()) {
            if let Some(name) = cur_group.take() {
                counts.push((name, count_sessions(&cur_rows, gap_secs)));
            }
            cur_group = Some(g);
            cur_rows = Vec::new();
        }
        cur_rows.push((s, e));
    }
    if let Some(name) = cur_group.take() {
        counts.push((name, count_sessions(&cur_rows, gap_secs)));
    }
    counts
}

fn group_dim(group_by: GroupBy) -> &'static str {
    match group_by {
        GroupBy::App => "a.process_name",
        GroupBy::Title => "COALESCE(a.window_title,'(无标题)')",
        // Display name: the user-visible name from `categories`; falls back to the
        // raw id when the category row is deleted or missing, then to 'other' when
        // the app has no group at all. (The model restates it in the answer's
        // language, and 'other' reads the same as any English builtin id.)
        GroupBy::Category => "COALESCE(c.name, NULLIF(g.category_id, 'none'), 'other')",
        GroupBy::None => unreachable!("group_dim 只在分组分支被调用"),
    }
}

/// 分桶维度表达式(白名单)。Week 的周一换算与报表口径一致:
/// strftime('%w') 0=周日,(w+6)%7 = 距本周周一的天数。
fn bucket_dim(bucket: Bucket) -> &'static str {
    match bucket {
        Bucket::Day => "a.local_date",
        Bucket::Week => {
            "date(a.local_date, '-' || ((strftime('%w', a.local_date) + 6) % 7) || ' days')"
        }
        Bucket::HourOfDay => "printf('%02d:00', a.local_hour)",
        Bucket::None => unreachable!("bucket_dim 只在分桶分支被调用"),
    }
}

/// 趋势分桶:每桶一行,按桶(=时间)升序全量列出,不做 top-N——行数天然有界:
/// day ≤ BUCKET_DAY_MAX_DAYS+1(validate 保证),week ≤ 53,hour_of_day ≤ 24。
/// 会话计数口径下跨桶边界的一段连续使用(如跨午夜)在两个桶各计一次,
/// 与"逐桶独立统计"的语义一致。
#[allow(clippy::too_many_arguments)]
fn query_bucket(
    conn: &rusqlite::Connection,
    from_sql: &str,
    where_sql: &str,
    params: &[&dyn rusqlite::ToSql],
    bucket: Bucket,
    auto_week: bool,
    metric: StatMetric,
    gap_minutes: u32,
    from: &str,
    to: &str,
    lang: ChatLang,
) -> DbResult<String> {
    let dim = bucket_dim(bucket);
    // 逐日的行首带本地化星期:实测模型自己换算"两周前的 07-16 是星期几"会成列出错,
    // 行里直接给出后模型无需再算
    let label = |b: &str| -> String {
        if bucket == Bucket::Day {
            if let Ok(d) = NaiveDate::parse_from_str(b, "%Y-%m-%d") {
                return format!("{b}({})", lang.weekday(&d.format("%u").to_string()));
            }
        }
        b.to_string()
    };
    let lines: Vec<String> = match metric {
        StatMetric::Duration => {
            // 不足 1 分钟的桶滤掉,理由同 query_duration 的分组 HAVING:
            // "某天: 0 分钟"是噪音,滤掉后与"无记录的日子未列出"语义一致
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {dim} AS b, SUM(a.duration_secs) s {from_sql}
                     WHERE {where_sql} GROUP BY b HAVING s >= 60 ORDER BY b"
                ))
                .db()?;
            let rows = stmt
                .query_map(params, |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            rows.iter()
                .map(|(b, secs)| format!("{}: {}", label(b), lang.fmt_secs(*secs)))
                .collect()
        }
        StatMetric::SessionCount => {
            let rows = fetch_grouped_spans(conn, from_sql, where_sql, params, dim)?;
            // 不排序不截断:count_sessions_by_group 保持输入的桶序(=时间升序)
            count_sessions_by_group(rows, gap_minutes as i64 * 60)
                .iter()
                .map(|(b, n)| format!("{}: {}", label(b), lang.count_suffix(*n)))
                .collect()
        }
        StatMetric::FirstLast => unreachable!("first_last 在 run 已先行分派"),
    };
    if lines.is_empty() {
        return Ok(lang.no_match(from, to));
    }
    // 会话计数口径把间隔规则并进头部括注
    let gap = (metric == StatMetric::SessionCount).then_some(gap_minutes);
    let header = match bucket {
        Bucket::Day => lang.bucket_day_header(from, to, lines.len(), gap),
        Bucket::Week => lang.bucket_week_header(from, to, lines.len(), auto_week, gap),
        Bucket::HourOfDay => lang.bucket_hour_header(from, to, gap),
        Bucket::None => unreachable!(),
    };
    Ok(format!("{header}\n{}", lines.join("\n")))
}

/// 首末口径:范围内最早开始与最晚结束的活动时间 + 总条数,忽略分组/分桶。
fn query_first_last(
    conn: &rusqlite::Connection,
    from_sql: &str,
    where_sql: &str,
    params: &[&dyn rusqlite::ToSql],
    from: &str,
    to: &str,
    lang: ChatLang,
) -> DbResult<String> {
    let (first, last, n): (Option<String>, Option<String>, i64) = conn
        .query_row(
            &format!(
                "SELECT MIN(a.started_at), MAX(a.ended_at), COUNT(*) {from_sql} WHERE {where_sql}"
            ),
            params,
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .db()?;
    let (Some(first), Some(last)) = (first, last) else {
        return Ok(lang.no_match(from, to));
    };
    // 截到分钟并去掉 T(与搜索结果行同款粒度),模型好读也省字符
    let fmt = |ts: &str| ts[..16.min(ts.len())].replace('T', " ");
    Ok(lang.first_last_line(from, to, &fmt(&first), &fmt(&last), n))
}

/// 数会话段:第一条起 1 段,之后每当"本条 started − 上一条 ended > gap_secs"再 +1。
/// `rows` 是按时间升序的 (started_at, ended_at) RFC3339;时间戳解析失败时保守归并
/// (不切),宁可少算也不虚高。
fn count_sessions(rows: &[(String, String)], gap_secs: i64) -> usize {
    let ts = |s: &str| {
        chrono::DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.timestamp())
    };
    let mut sessions = 0usize;
    let mut prev_end: Option<i64> = None;
    for (start, end) in rows {
        let (Some(s), Some(e)) = (ts(start), ts(end)) else {
            // 解析不了:并入当前会话(若还没有会话则起一段)
            if sessions == 0 {
                sessions = 1;
            }
            continue;
        };
        match prev_end {
            Some(pe) if s - pe <= gap_secs => {} // 间隔内 → 同一会话
            _ => sessions += 1,                  // 首条 or 超间隔 → 新会话
        }
        prev_end = Some(e.max(prev_end.unwrap_or(e)));
    }
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tools::test_util::{ctx_with, day};

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 5).unwrap()
    }

    /// 解析 query_stats 的参数；不合法时返回给模型看的原因。
    fn parse_args(args: Value) -> std::result::Result<StatsQuery, String> {
        StatsQuery::parse(&args, today(), ChatLang::ZhHans).map_err(|e| e.0)
    }

    #[test]
    fn parse_needs_at_least_one_date() {
        let e = parse_args(json!({})).unwrap_err();
        assert!(e.contains("query_stats"), "{e}");
        // 只给开始：到今天为止
        let q = parse_args(json!({"date_from": "2026-07-01"})).unwrap();
        assert_eq!(
            q.range,
            (NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(), today())
        );
    }

    #[test]
    fn parse_rejects_bad_ranges() {
        let e = parse_args(json!({"date_from": "2026-07-03", "date_to": "2026-07-01"}));
        assert!(e.unwrap_err().contains("晚于"));
        let e = parse_args(json!({"date_from": "2026-08-01", "date_to": "2026-08-02"}));
        assert!(e.unwrap_err().contains("未来"));
        let e = parse_args(json!({"date_from": "2020-01-01", "date_to": "2026-07-01"}));
        assert!(e.unwrap_err().contains("366"));
    }

    #[test]
    fn parse_first_last_allows_ranges_over_a_year() {
        // 同样的多年范围：duration 被拒(上一个测试)，first_last 放行
        let q = parse_args(json!({
            "date_from": "2020-01-01", "date_to": "2026-07-01", "metric": "first_last"
        }))
        .unwrap();
        assert_eq!(q.metric, StatMetric::FirstLast);
    }

    #[test]
    fn parse_fills_metric_and_gap_defaults_and_clamps_gap() {
        let q = parse_args(json!({"date_from": "2026-07-01", "date_to": "2026-07-05"})).unwrap();
        assert_eq!(q.metric, StatMetric::Duration);
        assert_eq!(q.gap_minutes, GAP_DEFAULT_MIN);
        assert_eq!(q.group_by, GroupBy::None);
        assert_eq!(q.bucket, Bucket::None);
        assert_eq!(q.top_n, 5);

        let q = parse_args(json!({
            "date_from": "2026-07-01", "date_to": "2026-07-05",
            "metric": "session_count", "gap_minutes": 9999
        }))
        .unwrap();
        assert_eq!(q.metric, StatMetric::SessionCount);
        assert_eq!(q.gap_minutes, GAP_MAX);
        let q = parse_args(json!({
            "date_from": "2026-07-01", "date_to": "2026-07-05", "gap_minutes": 1
        }))
        .unwrap();
        assert_eq!(q.gap_minutes, GAP_MIN);
    }

    #[test]
    fn parse_clamps_top_n() {
        let q = parse_args(json!({
            "date_from": "2026-07-01", "date_to": "2026-07-05",
            "group_by": "app", "top_n": 9999
        }))
        .unwrap();
        assert_eq!(q.top_n, TOP_N_MAX);
        let q = parse_args(json!({
            "date_from": "2026-07-01", "date_to": "2026-07-05",
            "group_by": "app", "top_n": 0
        }))
        .unwrap();
        assert_eq!(q.top_n, 1);
    }

    #[test]
    fn parse_category_group_defaults_to_max_top_n() {
        // 分类总数少,未显式给 top_n 时放宽到上限(默认 5 实测截掉了小分类)
        let q = parse_args(json!({
            "date_from": "2026-07-01", "date_to": "2026-07-05", "group_by": "category"
        }))
        .unwrap();
        assert_eq!(q.top_n, TOP_N_MAX);
        // 显式给的仍然尊重
        let q = parse_args(json!({
            "date_from": "2026-07-01", "date_to": "2026-07-05",
            "group_by": "category", "top_n": 3
        }))
        .unwrap();
        assert_eq!(q.top_n, 3);
    }

    #[test]
    fn parse_rejects_bucket_with_group() {
        let e = parse_args(json!({
            "date_from": "2026-07-01", "date_to": "2026-07-05",
            "group_by": "app", "bucket": "day"
        }))
        .unwrap_err();
        assert!(e.contains("不能同时使用"), "{e}");
    }

    #[test]
    fn parse_turns_a_long_day_bucket_into_weeks() {
        // 90 天的逐日请求 → 自动改逐周,并带 auto 标志(结果头部要向模型说明)
        let q = parse_args(json!({
            "date_from": "2026-04-01", "date_to": "2026-06-30", "bucket": "day"
        }))
        .unwrap();
        assert_eq!(q.bucket, Bucket::Week);
        assert!(q.bucket_auto_week);
        // 60 天以内保持逐日
        let q = parse_args(json!({
            "date_from": "2026-06-01", "date_to": "2026-07-01", "bucket": "day"
        }))
        .unwrap();
        assert_eq!(q.bucket, Bucket::Day);
        assert!(!q.bucket_auto_week);
    }

    #[test]
    fn parse_cleans_apps_and_categories_without_a_cap() {
        // 去掉空白和空的，不限个数
        let q = parse_args(json!({
            "date_from": "2026-07-01", "date_to": "2026-07-05",
            "apps": [" A ", "", "B", "C", "D", "E", "F"],
            "categories": ["  游戏 ", "", "工作"]
        }))
        .unwrap();
        assert_eq!(q.apps, ["A", "B", "C", "D", "E", "F"]);
        assert_eq!(q.categories, ["游戏", "工作"]);
    }

    #[test]
    fn parse_names_the_field_whose_item_is_too_long() {
        let long = "x".repeat(65);
        let e = parse_args(json!({
            "date_from": "2026-07-01", "date_to": "2026-07-05", "apps": [long.clone()]
        }))
        .unwrap_err();
        assert!(e.contains("apps"), "{e}");
        let e = parse_args(json!({
            "date_from": "2026-07-01", "date_to": "2026-07-05", "categories": [long]
        }))
        .unwrap_err();
        assert!(e.contains("categories"), "{e}");
    }

    #[test]
    fn parse_trims_title_keyword_and_drops_a_blank_one() {
        let q = parse_args(json!({
            "date_from": "2026-07-01", "date_to": "2026-07-05", "title_keyword": "  rust  "
        }))
        .unwrap();
        assert_eq!(q.title_keyword.as_deref(), Some("rust"));
        let q = parse_args(json!({
            "date_from": "2026-07-01", "date_to": "2026-07-05", "title_keyword": "   "
        }))
        .unwrap();
        assert_eq!(q.title_keyword, None);
        // 按字符数算：64 个可以，65 个不合法
        let ok = parse_args(json!({
            "date_from": "2026-07-01", "date_to": "2026-07-05", "title_keyword": "字".repeat(64)
        }));
        assert!(ok.is_ok());
        let e = parse_args(json!({
            "date_from": "2026-07-01", "date_to": "2026-07-05", "title_keyword": "字".repeat(65)
        }))
        .unwrap_err();
        assert!(e.contains("title_keyword"), "{e}");
    }

    #[test]
    fn parse_rejects_an_unknown_metric_as_a_format_error() {
        let e = parse_args(json!({
            "date_from": "2026-07-01", "date_to": "2026-07-05", "metric": "speed"
        }))
        .unwrap_err();
        assert!(e.starts_with("参数格式错误"), "{e}");
    }

    /// 本机的 grammar 从同一份定义生成，长度和条数的限制要写在定义里。
    #[test]
    fn spec_carries_the_limits() {
        let spec = StatsQuery::spec(ChatLang::ZhHans);
        assert_eq!(spec.name, "query_stats");
        assert!(spec.parameters.get("required").is_none());
        let props = &spec.parameters["properties"];
        assert!(props["apps"].get("maxItems").is_none());
        assert!(props["categories"].get("maxItems").is_none());
        assert_eq!(props["title_keyword"]["maxLength"], 64);
        assert_eq!(props["top_n"]["maximum"], TOP_N_MAX);
        assert_eq!(props["gap_minutes"]["minimum"], GAP_MIN);
        assert_eq!(props["gap_minutes"]["maximum"], GAP_MAX);
    }

    #[test]
    fn count_sessions_gap_split() {
        // 三段活动:10:00-10:05,10:20-10:25(距上一段 15min),12:00-12:10(距上段 95min)
        let rows = vec![
            (
                "2026-07-05T10:00:00+09:00".into(),
                "2026-07-05T10:05:00+09:00".into(),
            ),
            (
                "2026-07-05T10:20:00+09:00".into(),
                "2026-07-05T10:25:00+09:00".into(),
            ),
            (
                "2026-07-05T12:00:00+09:00".into(),
                "2026-07-05T12:10:00+09:00".into(),
            ),
        ];
        // 间隔 30min:前两段并成一次(15min<30),第三段另起 → 2 次
        assert_eq!(count_sessions(&rows, 30 * 60), 2);
        // 间隔 10min:三段各自独立(15min、95min 都超) → 3 次
        assert_eq!(count_sessions(&rows, 10 * 60), 3);
        // 间隔 120min:全部并成一次 → 1 次
        assert_eq!(count_sessions(&rows, 120 * 60), 1);
        // 空 → 0
        assert_eq!(count_sessions(&[], 30 * 60), 0);
    }

    #[tokio::test]
    async fn stats_grouped_discloses_universe_beyond_top_n() {
        let mut sql = String::new();
        for a in 0..8 {
            sql.push_str(&format!(
                "INSERT INTO activities(started_at, ended_at, duration_secs,
                     local_date, local_hour, process_name, window_title) VALUES
                 ('2026-07-08T0{a}:00:00+09:00','2026-07-08T0{a}:30:00+09:00',
                  {}, '2026-07-08', {a}, 'App{a}', 'w');\n",
                (a + 1) * 600
            ));
        }
        let main: &'static str = Box::leak(sql.into_boxed_str());
        let ctx = ctx_with("", main).await;
        let q = StatsQuery {
            range: day(),
            apps: vec![],
            title_keyword: None,
            group_by: GroupBy::App,
            top_n: 5,
            metric: StatMetric::Duration,
            gap_minutes: 30,
            bucket: Bucket::None,
            bucket_auto_week: false,
            categories: vec![],
        };
        let out = run(&ctx, &q, ChatLang::ZhHans).await.unwrap();
        assert!(out.content.contains("共 8 组"), "{}", out.content);
        assert!(out.content.contains("前 5 组"), "{}", out.content);
    }

    /// StatsQuery 的全默认构造,测试逐项覆盖自己关心的字段。
    fn stats_call(range: (NaiveDate, NaiveDate)) -> StatsQuery {
        StatsQuery {
            range,
            apps: vec![],
            title_keyword: None,
            categories: vec![],
            group_by: GroupBy::None,
            top_n: 5,
            metric: StatMetric::Duration,
            gap_minutes: 30,
            bucket: Bucket::None,
            bucket_auto_week: false,
        }
    }

    #[tokio::test]
    async fn stats_category_groups_and_falls_back_to_other() {
        // work.exe → 组 g1 → 分类 cat1(名"工作");fun.exe 未分组 → 'other';
        // tiny.exe 的"零星"分类只有 30 秒 → 不足 1 分钟整组滤掉(0 分钟噪音行)
        let main = "
            INSERT INTO activities(started_at, ended_at, duration_secs,
                local_date, local_hour, process_name, window_title) VALUES
              ('2026-07-08T09:00:00+09:00','2026-07-08T10:00:00+09:00',3600,
               '2026-07-08', 9, 'work.exe', 'w'),
              ('2026-07-08T11:00:00+09:00','2026-07-08T11:30:00+09:00',1800,
               '2026-07-08', 11, 'fun.exe', 'f'),
              ('2026-07-08T12:00:00+09:00','2026-07-08T12:00:30+09:00',30,
               '2026-07-08', 12, 'tiny.exe', 't');
            INSERT INTO app_group_members(process_name, group_id) VALUES
              ('work.exe','g1'), ('tiny.exe','g2');
            INSERT INTO app_groups(id, display_name, category_id) VALUES
              ('g1','Work','cat1'), ('g2','Tiny','cat2');
            INSERT INTO categories(id, name) VALUES ('cat1','工作'), ('cat2','零星');";
        let ctx = ctx_with("", main).await;
        let mut q = stats_call(day());
        q.group_by = GroupBy::Category;
        let out = run(&ctx, &q, ChatLang::ZhHans).await.unwrap();
        assert!(
            out.content.contains("工作: 1 小时 0 分钟"),
            "{}",
            out.content
        );
        assert!(out.content.contains("other: 30 分钟"), "{}", out.content);
        assert!(!out.content.contains("零星"), "{}", out.content);
    }

    #[tokio::test]
    async fn stats_category_counts_user_uncategorized_as_other() {
        // steam 的组被用户设成未分类（存成 none），要和没分组的 fun.exe 一起算进 'other'，
        // 不能单独冒出一个叫 none 的分类
        let main = "
            INSERT INTO activities(started_at, ended_at, duration_secs,
                local_date, local_hour, process_name, window_title) VALUES
              ('2026-07-08T09:00:00+09:00','2026-07-08T09:30:00+09:00',1800,
               '2026-07-08', 9, 'steam', 's'),
              ('2026-07-08T11:00:00+09:00','2026-07-08T11:30:00+09:00',1800,
               '2026-07-08', 11, 'fun.exe', 'f');
            INSERT INTO app_group_members(process_name, group_id) VALUES ('steam','g1');
            INSERT INTO app_groups(id, display_name, category_id) VALUES ('g1','Steam','none');";
        let ctx = ctx_with("", main).await;
        let mut q = stats_call(day());
        q.group_by = GroupBy::Category;
        let out = run(&ctx, &q, ChatLang::ZhHans).await.unwrap();
        assert!(
            out.content.contains("other: 1 小时 0 分钟"),
            "{}",
            out.content
        );
        assert!(!out.content.contains("none"), "{}", out.content);
    }

    #[tokio::test]
    async fn stats_bucket_day_lists_days_and_discloses_gaps() {
        // 7-06 与 7-08 有记录,7-07 空——头部要申明"无记录的日子未列出"
        let main = "
            INSERT INTO activities(started_at, ended_at, duration_secs,
                local_date, local_hour, process_name, window_title) VALUES
              ('2026-07-06T09:00:00+09:00','2026-07-06T09:40:00+09:00',2400,
               '2026-07-06', 9, 'App', 'w'),
              ('2026-07-08T09:00:00+09:00','2026-07-08T10:00:00+09:00',3600,
               '2026-07-08', 9, 'App', 'w');";
        let ctx = ctx_with("", main).await;
        let range = (
            NaiveDate::from_ymd_opt(2026, 7, 6).unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 8).unwrap(),
        );
        let mut q = stats_call(range);
        q.bucket = Bucket::Day;
        let out = run(&ctx, &q, ChatLang::ZhHans).await.unwrap();
        assert!(
            out.content.contains("逐日统计(2 个有记录的日子"),
            "{}",
            out.content
        );
        // 行首带本地化星期(07-06 周一/07-08 周三),模型不用自己换算
        assert!(
            out.content.contains("2026-07-06(周一): 40 分钟"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("2026-07-08(周三): 1 小时 0 分钟"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn stats_category_filter_covers_user_assigned_apps() {
        // Cardinal 被用户归入"游戏"(模型自己列举应用名时不会想到它)——
        // categories=["游戏"] 过滤必须把它算进来,known.exe 的"浏览"不算
        let main = "
            INSERT INTO activities(started_at, ended_at, duration_secs,
                local_date, local_hour, process_name, window_title) VALUES
              ('2026-07-08T09:00:00+09:00','2026-07-08T10:00:00+09:00',3600,
               '2026-07-08', 9, 'Cardinal', 'w'),
              ('2026-07-08T11:00:00+09:00','2026-07-08T11:30:00+09:00',1800,
               '2026-07-08', 11, 'known.exe', 'f');
            INSERT INTO app_group_members(process_name, group_id) VALUES
              ('Cardinal','g1'), ('known.exe','g2');
            INSERT INTO app_groups(id, display_name, category_id) VALUES
              ('g1','Cardinal','cat_game'), ('g2','Known','cat_browse');
            INSERT INTO categories(id, name) VALUES
              ('cat_game','游戏'), ('cat_browse','浏览');";
        let ctx = ctx_with("", main).await;
        let mut q = stats_call(day());
        q.categories = vec!["游戏".into()];
        let out = run(&ctx, &q, ChatLang::ZhHans).await.unwrap();
        assert!(
            out.content.contains("合计: 1 小时 0 分钟"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn stats_bucket_week_labels_rows_by_monday() {
        // 2026-07-07(周二)与 07-08(周三)同属 07-06(周一)起始的那一周 → 合成一行
        let main = "
            INSERT INTO activities(started_at, ended_at, duration_secs,
                local_date, local_hour, process_name, window_title) VALUES
              ('2026-07-07T09:00:00+09:00','2026-07-07T09:30:00+09:00',1800,
               '2026-07-07', 9, 'App', 'w'),
              ('2026-07-08T09:00:00+09:00','2026-07-08T09:30:00+09:00',1800,
               '2026-07-08', 9, 'App', 'w');";
        let ctx = ctx_with("", main).await;
        let range = (
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 14).unwrap(),
        );
        let mut q = stats_call(range);
        q.bucket = Bucket::Week;
        let out = run(&ctx, &q, ChatLang::ZhHans).await.unwrap();
        assert!(out.content.contains("逐周统计(1 周"), "{}", out.content);
        assert!(
            out.content.contains("2026-07-06: 1 小时 0 分钟"),
            "{}",
            out.content
        );
        // 非自动升级:不出现自动改周的说明
        assert!(!out.content.contains("自动改为按周"), "{}", out.content);
    }

    #[tokio::test]
    async fn stats_bucket_hour_of_day_aggregates_across_days() {
        // 两天的 9 时各 20 分钟 → "09:00: 40 分钟"一行
        let main = "
            INSERT INTO activities(started_at, ended_at, duration_secs,
                local_date, local_hour, process_name, window_title) VALUES
              ('2026-07-07T09:00:00+09:00','2026-07-07T09:20:00+09:00',1200,
               '2026-07-07', 9, 'App', 'w'),
              ('2026-07-08T09:10:00+09:00','2026-07-08T09:30:00+09:00',1200,
               '2026-07-08', 9, 'App', 'w');";
        let ctx = ctx_with("", main).await;
        let range = (
            NaiveDate::from_ymd_opt(2026, 7, 7).unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 8).unwrap(),
        );
        let mut q = stats_call(range);
        q.bucket = Bucket::HourOfDay;
        let out = run(&ctx, &q, ChatLang::ZhHans).await.unwrap();
        assert!(
            out.content.contains("按一天中的小时聚合"),
            "{}",
            out.content
        );
        assert!(out.content.contains("09:00: 40 分钟"), "{}", out.content);
    }

    #[tokio::test]
    async fn stats_first_last_reports_span() {
        let main = "
            INSERT INTO activities(started_at, ended_at, duration_secs,
                local_date, local_hour, process_name, window_title) VALUES
              ('2026-07-05T09:00:00+09:00','2026-07-05T09:30:00+09:00',1800,
               '2026-07-05', 9, 'App', 'w'),
              ('2026-07-08T20:00:00+09:00','2026-07-08T21:00:00+09:00',3600,
               '2026-07-08', 20, 'App', 'w');";
        let ctx = ctx_with("", main).await;
        let range = (
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 14).unwrap(),
        );
        let mut q = stats_call(range);
        q.metric = StatMetric::FirstLast;
        let out = run(&ctx, &q, ChatLang::ZhHans).await.unwrap();
        assert!(out.content.contains("共 2 条匹配活动"), "{}", out.content);
        assert!(
            out.content.contains("最早一次始于 2026-07-05 09:00"),
            "{}",
            out.content
        );
        assert!(
            out.content.contains("最近一次止于 2026-07-08 21:00"),
            "{}",
            out.content
        );
    }

    // ── session_count 整链(内存库直测 execute)─────────────
    //
    // 三个 app、间隔已知的活动段;gap=30 分钟时各组会话数手工可推:
    //   alpha.exe: 09:00-09:05 / 09:20-09:25(隔 15,并)/ 12:00-12:05(隔 155,切)→ 2 次
    //   beta.exe : 10:30-10:35 / 12:30-12:35(隔 115,切)/ 14:00-14:05(隔 85,切)→ 3 次
    //   zeta.exe : 15:00-15:05 / 16:00-16:05(隔 55,切)→ 2 次
    // 不分组的全流(时间升序混排):09:00 块|10:30|12:00+12:30(隔 25,并)|14:00|15:00|16:00 → 6 次
    const SESSIONS_SQL: &str = "
        INSERT INTO activities(started_at, ended_at, duration_secs,
            local_date, local_hour, process_name, window_title) VALUES
          ('2026-07-08T09:00:00+09:00','2026-07-08T09:05:00+09:00',300,'2026-07-08',9,'alpha.exe','w'),
          ('2026-07-08T09:20:00+09:00','2026-07-08T09:25:00+09:00',300,'2026-07-08',9,'alpha.exe','w'),
          ('2026-07-08T12:00:00+09:00','2026-07-08T12:05:00+09:00',300,'2026-07-08',12,'alpha.exe','w'),
          ('2026-07-08T10:30:00+09:00','2026-07-08T10:35:00+09:00',300,'2026-07-08',10,'beta.exe','w'),
          ('2026-07-08T12:30:00+09:00','2026-07-08T12:35:00+09:00',300,'2026-07-08',12,'beta.exe','w'),
          ('2026-07-08T14:00:00+09:00','2026-07-08T14:05:00+09:00',300,'2026-07-08',14,'beta.exe','w'),
          ('2026-07-08T15:00:00+09:00','2026-07-08T15:05:00+09:00',300,'2026-07-08',15,'zeta.exe','w'),
          ('2026-07-08T16:00:00+09:00','2026-07-08T16:05:00+09:00',300,'2026-07-08',16,'zeta.exe','w');";

    #[tokio::test]
    async fn stats_session_count_total_uses_gap_and_wording() {
        let ctx = ctx_with("", SESSIONS_SQL).await;
        let mut q = stats_call(day());
        q.metric = StatMetric::SessionCount;
        let out = run(&ctx, &q, ChatLang::ZhHans).await.unwrap();
        // 总次数 + 间隔规则必须一起披露,模型才能向用户解释"次"的口径
        assert!(
            out.content.contains("使用会话次数: 6 次"),
            "{}",
            out.content
        );
        assert!(out.content.contains("间隔≥30 分钟"), "{}", out.content);
        // 统计类结果没有证据卡
        assert!(out.citations.is_empty());

        // gap 放大到 120 分钟:全流最大间隔 85 分钟 → 全并成 1 次。
        // 这证明 gap_minutes 真进了切分逻辑,不是只写进文案
        let mut q = stats_call(day());
        q.metric = StatMetric::SessionCount;
        q.gap_minutes = 120;
        let out = run(&ctx, &q, ChatLang::ZhHans).await.unwrap();
        assert!(
            out.content.contains("使用会话次数: 1 次"),
            "{}",
            out.content
        );
        assert!(out.content.contains("间隔≥120 分钟"), "{}", out.content);
    }

    #[tokio::test]
    async fn stats_session_count_grouped_sorts_count_desc_then_name_asc() {
        let ctx = ctx_with("", SESSIONS_SQL).await;
        let mut q = stats_call(day());
        q.metric = StatMetric::SessionCount;
        q.group_by = GroupBy::App;
        let out = run(&ctx, &q, ChatLang::ZhHans).await.unwrap();
        // 每组次数各自独立切分
        let beta = out.content.find("beta.exe: 3 次").expect(&out.content);
        let alpha = out.content.find("alpha.exe: 2 次").expect(&out.content);
        let zeta = out.content.find("zeta.exe: 2 次").expect(&out.content);
        // 次数降序;同为 2 次的 alpha/zeta 按名字升序——排序稳定,模型两次问答不见"名次漂移"
        assert!(beta < alpha && alpha < zeta, "{}", out.content);
        // 3 组 ≤ top_n=5:全量列出,头部不出现"取前 N 组"的截断措辞
        assert!(out.content.contains("使用会话次数"), "{}", out.content);
        assert!(!out.content.contains("取前"), "{}", out.content);
    }

    #[tokio::test]
    async fn stats_session_count_top_n_truncates_and_discloses_universe() {
        let ctx = ctx_with("", SESSIONS_SQL).await;
        let mut q = stats_call(day());
        q.metric = StatMetric::SessionCount;
        q.group_by = GroupBy::App;
        q.top_n = 2;
        let out = run(&ctx, &q, ChatLang::ZhHans).await.unwrap();
        // universe 披露:让模型知道前 2 组之外还有第 3 组(2026-07-08 事故的同款教训)
        assert!(
            out.content.contains("共 3 组,按次数取前 2 组"),
            "{}",
            out.content
        );
        assert!(out.content.contains("beta.exe: 3 次"), "{}", out.content);
        assert!(out.content.contains("alpha.exe: 2 次"), "{}", out.content);
        // 同次数并列时名字靠后的 zeta 被截掉
        assert!(!out.content.contains("zeta.exe"), "{}", out.content);
    }

    // ── apps 过滤的参数绑定 ─────────────────────────────

    #[tokio::test]
    async fn stats_apps_filter_sums_only_matching() {
        // 3 个 app,只圈 alpha+beta:合计必须是 3600+1800=5400,gamma 的 600 不掺入
        let main = "
            INSERT INTO activities(started_at, ended_at, duration_secs,
                local_date, local_hour, process_name, window_title) VALUES
              ('2026-07-08T09:00:00+09:00','2026-07-08T10:00:00+09:00',3600,
               '2026-07-08', 9, 'alpha.exe', 'w'),
              ('2026-07-08T11:00:00+09:00','2026-07-08T11:30:00+09:00',1800,
               '2026-07-08', 11, 'beta.exe', 'w'),
              ('2026-07-08T13:00:00+09:00','2026-07-08T13:10:00+09:00',600,
               '2026-07-08', 13, 'gamma.exe', 'w');";
        let ctx = ctx_with("", main).await;
        let mut q = stats_call(day());
        q.apps = vec!["alpha".into(), "beta".into()];
        let out = run(&ctx, &q, ChatLang::ZhHans).await.unwrap();
        assert!(
            out.content.contains("合计: 1 小时 30 分钟"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn stats_apps_categories_title_bind_positionally() {
        // apps/categories/title 的占位符全是匿名 '?',正确性完全靠 push 顺序
        // (apps → categories → title)。三个过滤条件各自只放行部分行,
        // 交集唯一(work.exe 的"周报"行,3600s):任何两个绑定串位,
        // 结果都不再是 1 小时(例如 process LIKE '%工作%' 直接空集)。
        let main = "
            INSERT INTO activities(started_at, ended_at, duration_secs,
                local_date, local_hour, process_name, window_title) VALUES
              ('2026-07-08T09:00:00+09:00','2026-07-08T10:00:00+09:00',3600,
               '2026-07-08', 9, 'work.exe', '周报 W28'),
              ('2026-07-08T11:00:00+09:00','2026-07-08T11:30:00+09:00',1800,
               '2026-07-08', 11, 'work.exe', '摸鱼视频'),
              ('2026-07-08T12:00:00+09:00','2026-07-08T12:20:00+09:00',1200,
               '2026-07-08', 12, 'workfun.exe', '周报 W28'),
              ('2026-07-08T13:00:00+09:00','2026-07-08T13:15:00+09:00',900,
               '2026-07-08', 13, 'other.exe', '周报 W28');
            INSERT INTO app_group_members(process_name, group_id) VALUES
              ('work.exe','g1'), ('workfun.exe','g2'), ('other.exe','g3');
            INSERT INTO app_groups(id, display_name, category_id) VALUES
              ('g1','Work','cat_work'), ('g2','WorkFun','cat_fun'), ('g3','Other','cat_work');
            INSERT INTO categories(id, name) VALUES ('cat_work','工作'), ('cat_fun','娱乐');";
        let ctx = ctx_with("", main).await;
        let mut q = stats_call(day());
        q.apps = vec!["work".into()];
        q.categories = vec!["工作".into()];
        q.title_keyword = Some("周报".into());
        let out = run(&ctx, &q, ChatLang::ZhHans).await.unwrap();
        assert!(
            out.content.contains("合计: 1 小时 0 分钟"),
            "{}",
            out.content
        );
    }

    #[tokio::test]
    async fn stats_apps_like_wildcards_escaped_no_overmatch() {
        // '100Xzapp.exe' 是给未转义模式设的陷阱:%100%_a% 里 '%' 吃掉 'Xz'、
        // '_' 吃掉任意一字符就能命中它;转义后只允许字面 '100%_a' 子串。
        // 合计 1 小时 = 只有字面匹配那行;若转义失效会算出 1.5 小时
        let main = "
            INSERT INTO activities(started_at, ended_at, duration_secs,
                local_date, local_hour, process_name, window_title) VALUES
              ('2026-07-08T09:00:00+09:00','2026-07-08T10:00:00+09:00',3600,
               '2026-07-08', 9, '100%_app.exe', 'w'),
              ('2026-07-08T11:00:00+09:00','2026-07-08T11:30:00+09:00',1800,
               '2026-07-08', 11, '100Xzapp.exe', 'w');";
        let ctx = ctx_with("", main).await;
        let mut q = stats_call(day());
        q.apps = vec!["100%_a".into()];
        let out = run(&ctx, &q, ChatLang::ZhHans).await.unwrap();
        assert!(
            out.content.contains("合计: 1 小时 0 分钟"),
            "{}",
            out.content
        );
    }
}
