//! 检查模型发来的参数，转成 [`Query`]。

use chrono::NaiveDate;
use serde::Deserialize;

use super::ChatLang;

/// 分组统计 top-N 上限
const TOP_N_MAX: usize = 10;

/// 校验通过后的工具调用——只可能是这三种,未知工具在解析层就被拒。
// 分支名跟工具名一一对应，所以有一个叫 QueryStats。
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq)]
pub enum Query {
    SearchText {
        keywords: Vec<String>,
        range: Option<(NaiveDate, NaiveDate)>,
    },
    QueryStats {
        range: (NaiveDate, NaiveDate),
        apps: Vec<String>,
        title_keyword: Option<String>,
        /// 按分类名过滤(如"游戏")。与 bucket 正交——"游戏时间的周趋势"
        /// 的正确姿势就是 categories=["游戏"] + bucket=week;没有它,模型只能
        /// 靠猜应用名列举,认不出用户自定义归类的应用(实测漏算 2 小时)。
        categories: Vec<String>,
        group_by: GroupBy,
        top_n: usize,
        metric: StatMetric,
        /// 会话计数用:相邻活动间隔超过这么多分钟就算一段新会话
        gap_minutes: u32,
        /// 趋势分桶(与 group_by 互斥,validate 拦)
        bucket: Bucket,
        /// bucket=Week 且由 Day 自动升级而来(范围 > BUCKET_DAY_MAX_DAYS):
        /// 结果头部要向模型说明,免得它以为工具没按参数执行
        bucket_auto_week: bool,
    },
    GetTimeline {
        range: (NaiveDate, NaiveDate),
    },
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

/// 逐日分桶的最大跨度:超过则自动按周汇总——60 天 ≈ 60 行是结果预算内
/// 能完整放下、模型也读得过来的上限。
const BUCKET_DAY_MAX_DAYS: i64 = 60;

/// 会话计数的间隔默认值与夹紧区间(分钟)。
const GAP_DEFAULT_MIN: u32 = 30;
const GAP_MIN: u32 = 5;
const GAP_MAX: u32 = 240;

/// 模型给出的原始参数(两种适配器统一走这个宽松壳,校验后变 [`Query`])。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(super) struct RawParams {
    keywords: Vec<String>,
    date_from: Option<String>,
    date_to: Option<String>,
    apps: Vec<String>,
    title_keyword: Option<String>,
    group_by: Option<GroupBy>,
    top_n: Option<usize>,
    metric: Option<StatMetric>,
    gap_minutes: Option<u32>,
    bucket: Option<Bucket>,
    categories: Vec<String>,
}

/// 第②道墙:工具名 + 参数逐项校验。`today` 用于拒绝未来日期。
/// Err 的文案会回填给模型,写给模型看。
pub(super) fn validate(
    name: &str,
    raw: &RawParams,
    today: NaiveDate,
    lang: ChatLang,
) -> std::result::Result<Query, String> {
    match name {
        "search_text" => {
            let keywords = clean_keywords(&raw.keywords, lang)?;
            let range = parse_range_opt(raw, today, RANGE_MAX_DAYS, lang)?;
            Ok(Query::SearchText { keywords, range })
        }
        "query_stats" => {
            let metric = raw.metric.unwrap_or(StatMetric::Duration);
            // first_last 是 MIN/MAX 聚合,大范围也便宜——豁免常规跨度上限,
            // 否则"我第一次用 X 是什么时候"这类必须给多年范围的问题会被拒
            let max_days = if metric == StatMetric::FirstLast {
                FIRST_LAST_MAX_DAYS
            } else {
                RANGE_MAX_DAYS
            };
            let range = parse_range_opt(raw, today, max_days, lang)?
                .ok_or_else(|| lang.err_need_range("query_stats"))?;
            let apps = clean_short_strings(&raw.apps, 5, "apps", lang)?;
            let categories = clean_short_strings(&raw.categories, 5, "categories", lang)?;
            let title_keyword = match raw.title_keyword.as_deref().map(str::trim) {
                Some("") | None => None,
                Some(t) if t.chars().count() <= 64 => Some(t.to_string()),
                Some(_) => return Err(lang.err_title_kw_too_long().into()),
            };
            let group_by = raw.group_by.unwrap_or(GroupBy::None);
            let mut bucket = raw.bucket.unwrap_or(Bucket::None);
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
            Ok(Query::QueryStats {
                range,
                apps,
                title_keyword,
                categories,
                group_by,
                top_n: raw.top_n.unwrap_or(top_n_default).clamp(1, TOP_N_MAX),
                metric,
                gap_minutes: raw
                    .gap_minutes
                    .unwrap_or(GAP_DEFAULT_MIN)
                    .clamp(GAP_MIN, GAP_MAX),
                bucket,
                bucket_auto_week,
            })
        }
        "get_timeline" => {
            let range = parse_range_opt(raw, today, RANGE_MAX_DAYS, lang)?
                .ok_or_else(|| lang.err_need_range("get_timeline"))?;
            Ok(Query::GetTimeline { range })
        }
        other => Err(lang.err_unknown_tool(other)),
    }
}

/// 常规查询的日期跨度上限(一年);first_last 口径的宽限见 [`FIRST_LAST_MAX_DAYS`]。
const RANGE_MAX_DAYS: i64 = 366;
/// first_last(MIN/MAX 聚合)的跨度上限:名义上限,只防荒谬输入。
const FIRST_LAST_MAX_DAYS: i64 = 36600;

fn parse_range_opt(
    raw: &RawParams,
    today: NaiveDate,
    max_days: i64,
    lang: ChatLang,
) -> std::result::Result<Option<(NaiveDate, NaiveDate)>, String> {
    let (Some(f), Some(t)) = (raw.date_from.as_deref(), raw.date_to.as_deref()) else {
        return Ok(None);
    };
    let from = NaiveDate::parse_from_str(f.trim(), "%Y-%m-%d")
        .map_err(|_| lang.err_bad_date("date_from", f))?;
    let to = NaiveDate::parse_from_str(t.trim(), "%Y-%m-%d")
        .map_err(|_| lang.err_bad_date("date_to", t))?;
    if from > to {
        return Err(lang.err_from_after_to().into());
    }
    if (to - from).num_days() > max_days {
        return Err(lang.err_range_too_long().into());
    }
    if from > today {
        return Err(lang.err_from_in_future().into());
    }
    Ok(Some((from, to)))
}

fn clean_keywords(raw: &[String], lang: ChatLang) -> std::result::Result<Vec<String>, String> {
    let out = clean_short_strings(raw, 3, "keywords", lang)?;
    if out.is_empty() {
        return Err(lang.err_keywords_empty().into());
    }
    Ok(out)
}

fn clean_short_strings(
    raw: &[String],
    max_items: usize,
    field: &str,
    lang: ChatLang,
) -> std::result::Result<Vec<String>, String> {
    let mut out = Vec::new();
    for s in raw.iter().take(max_items) {
        let t = s.trim();
        if t.is_empty() {
            continue;
        }
        if t.chars().count() > 64 {
            return Err(lang.err_item_too_long(field));
        }
        out.push(t.to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 5).unwrap()
    }

    fn raw(json: serde_json::Value) -> RawParams {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn validate_rejects_unknown_tool_and_bad_dates() {
        let e = validate(
            "drop_table",
            &RawParams::default(),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap_err();
        assert!(e.contains("未知工具"));

        let e = validate(
            "query_stats",
            &raw(serde_json::json!({"date_from": "2026-07-10", "date_to": "2026-07-01"})),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap_err();
        assert!(e.contains("晚于"));

        let e = validate(
            "query_stats",
            &raw(serde_json::json!({"date_from": "2026-08-01", "date_to": "2026-08-02"})),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap_err();
        assert!(e.contains("未来"));

        let e = validate(
            "query_stats",
            &raw(serde_json::json!({"date_from": "2020-01-01", "date_to": "2026-07-01"})),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap_err();
        assert!(e.contains("366"));
    }

    #[test]
    fn top_n_clamped_server_side() {
        let call = validate(
            "query_stats",
            &raw(serde_json::json!({
                "date_from": "2026-07-01", "date_to": "2026-07-05",
                "group_by": "app", "top_n": 9999
            })),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap();
        match call {
            Query::QueryStats { top_n, .. } => assert_eq!(top_n, TOP_N_MAX),
            _ => panic!(),
        }
    }

    #[test]
    fn stat_metric_and_gap_defaults_and_clamp() {
        // 不填 → duration + 默认 30 分钟
        let call = validate(
            "query_stats",
            &raw(serde_json::json!({"date_from": "2026-07-01", "date_to": "2026-07-05"})),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap();
        match call {
            Query::QueryStats {
                metric,
                gap_minutes,
                ..
            } => {
                assert_eq!(metric, StatMetric::Duration);
                assert_eq!(gap_minutes, GAP_DEFAULT_MIN);
            }
            _ => panic!(),
        }
        // session_count + 越界间隔被夹到上界
        let call = validate(
            "query_stats",
            &raw(serde_json::json!({
                "date_from": "2026-07-01", "date_to": "2026-07-05",
                "metric": "session_count", "gap_minutes": 9999
            })),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap();
        match call {
            Query::QueryStats {
                metric,
                gap_minutes,
                ..
            } => {
                assert_eq!(metric, StatMetric::SessionCount);
                assert_eq!(gap_minutes, GAP_MAX);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn validate_rejects_bucket_with_group() {
        let e = validate(
            "query_stats",
            &raw(serde_json::json!({
                "date_from": "2026-07-01", "date_to": "2026-07-05",
                "group_by": "app", "bucket": "day"
            })),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap_err();
        assert!(e.contains("不能同时使用"), "{e}");
    }

    #[test]
    fn validate_auto_upgrades_long_day_bucket_to_week() {
        // 90 天的逐日请求 → 自动改逐周,并带 auto 标志(结果头部要向模型说明)
        let call = validate(
            "query_stats",
            &raw(serde_json::json!({
                "date_from": "2026-04-01", "date_to": "2026-06-30", "bucket": "day"
            })),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap();
        match call {
            Query::QueryStats {
                bucket,
                bucket_auto_week,
                ..
            } => {
                assert_eq!(bucket, Bucket::Week);
                assert!(bucket_auto_week);
            }
            _ => panic!(),
        }
        // 60 天以内保持逐日
        let call = validate(
            "query_stats",
            &raw(serde_json::json!({
                "date_from": "2026-06-01", "date_to": "2026-07-01", "bucket": "day"
            })),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap();
        match call {
            Query::QueryStats {
                bucket,
                bucket_auto_week,
                ..
            } => {
                assert_eq!(bucket, Bucket::Day);
                assert!(!bucket_auto_week);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn validate_category_group_defaults_to_max_top_n() {
        // 分类总数少,未显式给 top_n 时放宽到上限(默认 5 实测截掉了小分类)
        let call = validate(
            "query_stats",
            &raw(serde_json::json!({
                "date_from": "2026-07-01", "date_to": "2026-07-05",
                "group_by": "category"
            })),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap();
        match call {
            Query::QueryStats { top_n, .. } => assert_eq!(top_n, TOP_N_MAX),
            _ => panic!(),
        }
        // 显式给的仍然尊重
        let call = validate(
            "query_stats",
            &raw(serde_json::json!({
                "date_from": "2026-07-01", "date_to": "2026-07-05",
                "group_by": "category", "top_n": 3
            })),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap();
        match call {
            Query::QueryStats { top_n, .. } => assert_eq!(top_n, 3),
            _ => panic!(),
        }
    }

    #[test]
    fn validate_first_last_exempts_range_cap() {
        // 同样的多年范围:duration 拒(366 上限,另一测试盯着),first_last 放行
        let call = validate(
            "query_stats",
            &raw(serde_json::json!({
                "date_from": "2020-01-01", "date_to": "2026-07-01",
                "metric": "first_last"
            })),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap();
        match call {
            Query::QueryStats { metric, .. } => assert_eq!(metric, StatMetric::FirstLast),
            _ => panic!(),
        }
    }

    #[test]
    fn validate_blank_only_keywords_rejected() {
        // 全是空白/空串的 keywords 清洗后为空——必须报"不能为空",
        // 否则会拼出空 FTS MATCH 走到 SQL 层才炸,错误信息模型看不懂
        let e = validate(
            "search_text",
            &raw(serde_json::json!({"keywords": [" ", ""]})),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap_err();
        assert!(e.contains("不能为空"), "{e}");
    }

    #[test]
    fn validate_keywords_truncated_to_three_and_trimmed() {
        // 4 词只取前 3(RESULT 预算与 FTS AND 语义都不欢迎长词列表),且逐项 trim
        let call = validate(
            "search_text",
            &raw(serde_json::json!({"keywords": ["  rust  ", "go", "zig", "c"]})),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap();
        match call {
            Query::SearchText { keywords, .. } => {
                assert_eq!(keywords, vec!["rust", "go", "zig"]);
            }
            _ => panic!(),
        }
        // 截断发生在清洗之前:空白项也占名额,take(3) 后才被丢弃——
        // 钉住这个顺序,悄悄改成"先清洗再截断"会让第 4 词意外入选
        let call = validate(
            "search_text",
            &raw(serde_json::json!({"keywords": [" ", "a", "b", "c"]})),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap();
        match call {
            Query::SearchText { keywords, .. } => assert_eq!(keywords, vec!["a", "b"]),
            _ => panic!(),
        }
    }

    #[test]
    fn validate_item_too_long_rejected_field_named() {
        // 65 字符(按 chars 计)超限;64 恰好放行。错误文案必须点名字段,
        // 模型才知道该改哪个参数
        let long = "x".repeat(65);
        let ok64 = "x".repeat(64);
        let e = validate(
            "search_text",
            &raw(serde_json::json!({"keywords": [long.clone()]})),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap_err();
        assert!(e.contains("keywords") && e.contains("64"), "{e}");

        let call = validate(
            "search_text",
            &raw(serde_json::json!({"keywords": [ok64]})),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap();
        match call {
            Query::SearchText { keywords, .. } => assert_eq!(keywords.len(), 1),
            _ => panic!(),
        }

        // query_stats 的 apps / categories 走同一清洗循环,各自报各自的字段名
        let e = validate(
            "query_stats",
            &raw(serde_json::json!({
                "date_from": "2026-07-01", "date_to": "2026-07-05",
                "apps": [long.clone()]
            })),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap_err();
        assert!(e.contains("apps"), "{e}");

        let e = validate(
            "query_stats",
            &raw(serde_json::json!({
                "date_from": "2026-07-01", "date_to": "2026-07-05",
                "categories": [long]
            })),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap_err();
        assert!(e.contains("categories"), "{e}");
    }

    #[test]
    fn validate_apps_categories_cleaned_and_capped_at_five() {
        // apps 上限 5 项(截断不报错),空项丢弃、逐项 trim;categories 同一循环体。
        // 与 keywords 一样是"先 take 后清洗":空串占掉一个名额
        let call = validate(
            "query_stats",
            &raw(serde_json::json!({
                "date_from": "2026-07-01", "date_to": "2026-07-05",
                "apps": [" A ", "", "B", "C", "D", "E", "F"],
                "categories": ["  游戏 ", "", "工作"]
            })),
            today(),
            ChatLang::ZhHans,
        )
        .unwrap();
        match call {
            Query::QueryStats {
                apps, categories, ..
            } => {
                // take(5) 拿到 [" A ","","B","C","D"],清洗后 4 项;E/F 被截掉
                assert_eq!(apps, vec!["A", "B", "C", "D"]);
                assert_eq!(categories, vec!["游戏", "工作"]);
            }
            _ => panic!(),
        }
    }
}
