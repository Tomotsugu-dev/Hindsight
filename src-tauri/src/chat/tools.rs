//! Chat 工具执行层——四道墙的 ②③④(①grammar 在 llm 适配器)。
//!
//! 安全模型:LLM 没有任何写能力、没有任何 SQL 表面。
//! - ② 语义校验:工具名白名单、日期可解析且 from≤to 且跨度≤366 天、
//!   关键词长度封顶、FTS 词强制字符串字面量、LIKE 通配符转义、limit 服务端夹紧;
//! - ③ 固定查询:每个工具一条写死的参数化 SQL,LLM 只填空;
//! - ④ 只读连接:主库与记忆库都以 SQLITE_OPEN_READ_ONLY 打开,
//!   前三道全被穿透时写操作在 SQLite 层直接报错。
//!
//! 校验错误返回 `Err(String)`(中文短句)——engine 会把它回填给模型重试,
//! 所以措辞要能指导模型改参数。

use chrono::NaiveDate;

use super::lang::ChatLang;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tokio_rusqlite::Connection;

use crate::error::Result;
use crate::storage::SqliteResultExt;

/// 单工具返回给模型的结果字符上限——超出截断,防止一个大会话吃光上下文。
const RESULT_CHAR_BUDGET: usize = 4000;
/// 搜索命中数上限(服务端硬夹紧,模型无法调大)
const SEARCH_LIMIT: usize = 8;
/// 搜索的窗口标题层命中显示上限(屏幕文字索引之外的兜底证据层)
const TITLE_LIMIT: usize = 8;
/// 时间线会话数上限
const TIMELINE_LIMIT: usize = 50;
/// 分组统计 top-N 上限
const TOP_N_MAX: usize = 10;

/// 只读工具上下文——第④道墙。两个库都是 READ_ONLY 连接。
pub struct ToolCtx {
    main: Connection,
    mem: Connection,
}

impl ToolCtx {
    /// 打开主库与记忆库的只读连接。
    pub async fn open_readonly() -> Result<Self> {
        use rusqlite::OpenFlags;
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let main = Connection::open_with_flags(crate::storage::db_path()?, flags).await?;
        let mem = Connection::open_with_flags(crate::memory::memory_db_path()?, flags).await?;
        Ok(Self { main, mem })
    }
}

/// 校验通过后的工具调用——只可能是这三种,未知工具在解析层就被拒。
#[derive(Debug, Clone, PartialEq)]
pub enum ToolCall {
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

/// 模型给出的原始参数(两种适配器统一走这个宽松壳,校验后变 [`ToolCall`])。
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct RawParams {
    pub keywords: Vec<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub apps: Vec<String>,
    pub title_keyword: Option<String>,
    pub group_by: Option<GroupBy>,
    pub top_n: Option<usize>,
    pub metric: Option<StatMetric>,
    pub gap_minutes: Option<u32>,
    pub bucket: Option<Bucket>,
    pub categories: Vec<String>,
}

/// 第②道墙:工具名 + 参数逐项校验。`today` 用于拒绝未来日期。
/// Err 的文案会回填给模型,写给模型看。
pub fn validate(
    name: &str,
    raw: &RawParams,
    today: NaiveDate,
    lang: ChatLang,
) -> std::result::Result<ToolCall, String> {
    match name {
        "search_text" => {
            let keywords = clean_keywords(&raw.keywords, lang)?;
            let range = parse_range_opt(raw, today, RANGE_MAX_DAYS, lang)?;
            Ok(ToolCall::SearchText { keywords, range })
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
            Ok(ToolCall::QueryStats {
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
            Ok(ToolCall::GetTimeline { range })
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

/// FTS MATCH 注入防护:每个词包成带引号的字符串字面量(内部引号剥除),
/// 多词以空格连接 = 隐式 AND。模型/用户输入永远进不了 FTS 语法位。
/// pub(crate):搜索页命令([`crate::commands::screen_memory::memory_search`])同款复用。
pub(crate) fn fts_literal(keywords: &[String]) -> String {
    keywords
        .iter()
        .map(|k| format!("\"{}\"", k.replace('"', " ")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// LIKE 通配符转义(ESCAPE '\')。pub(crate) 理由同 [`fts_literal`]。
pub(crate) fn like_pattern(s: &str) -> String {
    let escaped = s
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// 证据引用——前端渲染证据卡的数据。`index` 是本轮对话内的全局编号,
/// 模型在答案里写 [index] 引用它。Deserialize 供聊天历史从库里回读。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub index: usize,
    pub app: String,
    pub title: String,
    pub started_ts: String,
    pub ended_ts: String,
    /// 证据帧(命中行首现帧/会话首帧);可能已被 retention 删除,前端兜底
    pub frame_path: Option<String>,
}

/// 工具执行结果:给模型看的紧凑文本 + 给前端的结构化引用。
pub struct ToolOutput {
    pub for_llm: String,
    pub citations: Vec<Citation>,
}

/// 第③道墙:固定查询执行。`next_citation` 是引用编号的起点(engine 维护全局序)。
pub async fn execute(
    ctx: &ToolCtx,
    call: &ToolCall,
    next_citation: usize,
    lang: ChatLang,
) -> Result<ToolOutput> {
    match call {
        ToolCall::SearchText { keywords, range } => {
            search_text(ctx, keywords, *range, next_citation, lang).await
        }
        ToolCall::QueryStats {
            range,
            apps,
            title_keyword,
            categories,
            group_by,
            top_n,
            metric,
            gap_minutes,
            bucket,
            bucket_auto_week,
        } => {
            query_stats(
                ctx,
                *range,
                apps,
                title_keyword.as_deref(),
                categories,
                *group_by,
                *top_n,
                *metric,
                *gap_minutes,
                *bucket,
                *bucket_auto_week,
                lang,
            )
            .await
        }
        ToolCall::GetTimeline { range } => get_timeline(ctx, *range, next_citation, lang).await,
    }
}

/// activities 的报表口径 FROM/JOIN/WHERE 段(?1=from ?2=to,追加条件从 ?3 起编号)。
/// 与 repo/reports.rs 同口径:组是 cross-OS 同步的真相,显式指派到 hidden 的组剔除,
/// 未分组的活动(g.category_id 为 NULL)经 NULL-safe 比较照常通过。
const ACTIVITY_JOIN: &str = "FROM activities a
     LEFT JOIN app_group_members gm
       ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
     LEFT JOIN app_groups g
       ON g.id = gm.group_id AND g.deleted_at IS NULL
     WHERE a.local_date BETWEEN ?1 AND ?2
       AND g.category_id IS NOT 'hidden'";

/// 覆盖披露:范围内活动日数(主库)、其中有屏幕文字索引的日数与待识别帧数(记忆库)。
/// 拼在 timeline/search 结果最前——"没搜到"究竟是"屏幕上没出现过"还是"索引不全",
/// 模型只有靠这行才能区分,措辞约束在 system_prompt 第 8 条。
/// 活动日数不做隐藏组过滤:这行回答的是"电脑用没用/索引全不全",不是内容本身。
async fn coverage_line(ctx: &ToolCtx, from: &str, to: &str, lang: ChatLang) -> Result<String> {
    let (f, t) = (from.to_string(), to.to_string());
    let activity_days: i64 = ctx
        .main
        .call(move |conn| {
            conn.query_row(
                "SELECT COUNT(DISTINCT local_date) FROM activities
                 WHERE local_date BETWEEN ?1 AND ?2",
                params![f, t],
                |r| r.get(0),
            )
            .db()
        })
        .await?;
    let (f, t) = (from.to_string(), to.to_string());
    let (covered_days, pending): (i64, i64) = ctx
        .mem
        .call(move |conn| {
            let covered = conn
                .query_row(
                    "SELECT COUNT(DISTINCT local_date) FROM text_sessions
                     WHERE local_date BETWEEN ?1 AND ?2",
                    params![f, t],
                    |r| r.get(0),
                )
                .db()?;
            let pending = conn
                .query_row(
                    "SELECT COUNT(*) FROM frames
                     WHERE ocr_state = 0 AND local_date BETWEEN ?1 AND ?2",
                    params![f, t],
                    |r| r.get(0),
                )
                .db()?;
            Ok((covered, pending))
        })
        .await?;
    Ok(lang.coverage_line(activity_days, covered_days, pending))
}

async fn search_text(
    ctx: &ToolCtx,
    keywords: &[String],
    range: Option<(NaiveDate, NaiveDate)>,
    next_citation: usize,
    lang: ChatLang,
) -> Result<ToolOutput> {
    let fts = fts_literal(keywords);
    let first_kw = like_pattern(&keywords[0]);
    let (from, to) = match range {
        Some((f, t)) => (f.to_string(), t.to_string()),
        None => ("0000-00-00".into(), "9999-99-99".into()),
    };
    let coverage = coverage_line(ctx, &from, &to, lang).await?;
    let (from2, to2) = (from.clone(), to.clone());
    let (total, rows) = ctx
        .mem
        .call(move |conn| {
            // 总命中数:必须让模型知道命中规模(8 条窗口外还有没有东西),
            // 否则它会把"前 8 条"当成"只有 8 条"。
            let total: i64 = conn
                .query_row(
                    "SELECT COUNT(*)
                     FROM text_sessions_fts
                     JOIN text_sessions s ON s.id = text_sessions_fts.rowid
                     WHERE text_sessions_fts MATCH ?1
                       AND s.local_date BETWEEN ?2 AND ?3",
                    params![fts, from, to],
                    |r| r.get(0),
                )
                .db()?;
            let mut stmt = conn
                .prepare(
                    "SELECT s.id, COALESCE(s.app_id,''), COALESCE(s.title,''),
                            s.started_ts, s.ended_ts,
                            snippet(text_sessions_fts, 0, '<<', '>>', '…', 16)
                     FROM text_sessions_fts
                     JOIN text_sessions s ON s.id = text_sessions_fts.rowid
                     WHERE text_sessions_fts MATCH ?1
                       AND s.local_date BETWEEN ?2 AND ?3
                     ORDER BY rank LIMIT ?4",
                )
                .db()?;
            let hits = stmt
                .query_map(params![fts, from, to, SEARCH_LIMIT as i64], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            // 每条命中补证据帧:该会话里第一条含关键词的行的首现帧
            let mut out = Vec::with_capacity(hits.len());
            for (id, app, title, started, ended, snippet) in hits {
                let frame: Option<(String, String)> = conn
                    .query_row(
                        "SELECT first_path, first_ts FROM session_lines
                         WHERE session_id = ?1 AND text LIKE ?2 ESCAPE '\\' LIMIT 1",
                        params![id, first_kw],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .ok();
                out.push((id, app, title, started, ended, snippet, frame));
            }
            Ok((total, out))
        })
        .await?;

    // 标题层:窗口标题 LIKE 全部关键词(AND)。屏幕文字索引之外的兜底证据——
    // 没开截图/OCR 的用户靠它回答"我用没用过 X";有索引的用户靠它补足
    // 索引空窗(电池暂停、待识别积压)里的记录。
    let title_likes: Vec<String> = keywords.iter().map(|k| like_pattern(k)).collect();
    let (title_total, title_rows) = ctx
        .main
        .call(move |conn| {
            let like_sql = (0..title_likes.len())
                .map(|i| format!("a.window_title LIKE ?{} ESCAPE '\\'", i + 3))
                .collect::<Vec<_>>()
                .join(" AND ");
            let mut bind: Vec<&dyn rusqlite::ToSql> = vec![&from2, &to2];
            for l in &title_likes {
                bind.push(l);
            }
            let total: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) {ACTIVITY_JOIN} AND {like_sql}"),
                    bind.as_slice(),
                    |r| r.get(0),
                )
                .db()?;
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT COALESCE(g.display_name, a.process_name),
                            COALESCE(a.window_title,''),
                            a.started_at, a.ended_at, NULLIF(a.screenshot_path,'')
                     {ACTIVITY_JOIN} AND {like_sql}
                     ORDER BY a.started_at DESC LIMIT {TITLE_LIMIT}"
                ))
                .db()?;
            let hits = stmt
                .query_map(bind.as_slice(), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok((total, hits))
        })
        .await?;
    // 跨层去重:标题命中若落在已返回的屏幕文字会话时间段内,是同一证据的弱化版,剔除。
    // 两库时间戳同为本地时区 RFC3339(回填即原样搬运),字符串比较即可。
    let title_rows: Vec<_> = title_rows
        .into_iter()
        .filter(|(_, _, s, _, _)| {
            !rows.iter().any(|(_, _, _, l2s, l2e, _, _)| {
                s.as_str() >= l2s.as_str() && s.as_str() <= l2e.as_str()
            })
        })
        .collect();

    let mut citations = Vec::new();
    let mut lines = Vec::new();
    for (i, (_id, app, title, started, ended, snippet, frame)) in rows.iter().enumerate() {
        let idx = next_citation + i;
        lines.push(format!(
            "[{idx}] {} | {} | {} ~ {} | 片段: {}",
            app,
            truncate(title, 40),
            &started[..16.min(started.len())],
            &ended[..16.min(ended.len())],
            truncate(snippet, 120)
        ));
        citations.push(Citation {
            index: idx,
            app: app.clone(),
            title: title.clone(),
            started_ts: started.clone(),
            ended_ts: ended.clone(),
            frame_path: frame.as_ref().map(|(p, _)| p.clone()),
        });
    }
    let mut title_lines = Vec::new();
    for (j, (app, title, started, ended, shot)) in title_rows.iter().enumerate() {
        let idx = next_citation + rows.len() + j;
        title_lines.push(format!(
            "[{idx}] {} | {} | {} ~ {}",
            app,
            truncate(title, 60),
            &started[..16.min(started.len())],
            &ended[..16.min(ended.len())],
        ));
        citations.push(Citation {
            index: idx,
            app: app.clone(),
            title: title.clone(),
            started_ts: started.clone(),
            ended_ts: ended.clone(),
            frame_path: shot.clone(),
        });
    }

    // 组装:覆盖行永远在最前;两层各自报数分节列出;全空才是"没有命中"。
    let mut sections = vec![coverage];
    if !lines.is_empty() {
        sections.push(format!(
            "{}\n{}",
            lang.search_header(total, lines.len()),
            lines.join("\n")
        ));
    }
    if !title_lines.is_empty() {
        sections.push(format!(
            "{}\n{}",
            lang.search_title_header(title_total, title_lines.len()),
            title_lines.join("\n")
        ));
    }
    if lines.is_empty() && title_lines.is_empty() {
        sections.push(lang.search_no_hit().to_string());
    }
    let for_llm = truncate(&sections.join("\n"), RESULT_CHAR_BUDGET);
    Ok(ToolOutput { for_llm, citations })
}

/// 统计查询的基础 FROM 段(别名 a,与既有口径一致:裸活动表、不排 hidden 组)。
const STATS_FROM: &str = "FROM activities a";
/// 按分类分组时的 FROM 段:活动 → 组 → 分类两跳 join。分类真相在组上
/// (g.category_id),活动行上的 a.category_id 是采集时的快照兜底;
/// hidden 组在 WHERE 里剔除,与报表口径一致。
const STATS_FROM_CATEGORY: &str = "FROM activities a
     LEFT JOIN app_group_members gm
       ON gm.process_name = a.process_name AND gm.deleted_at IS NULL
     LEFT JOIN app_groups g
       ON g.id = gm.group_id AND g.deleted_at IS NULL
     LEFT JOIN categories c
       ON c.id = COALESCE(g.category_id, a.category_id) AND c.deleted_at IS NULL";

#[allow(clippy::too_many_arguments)]
async fn query_stats(
    ctx: &ToolCtx,
    (from, to): (NaiveDate, NaiveDate),
    apps: &[String],
    title_keyword: Option<&str>,
    categories: &[String],
    group_by: GroupBy,
    top_n: usize,
    metric: StatMetric,
    gap_minutes: u32,
    bucket: Bucket,
    bucket_auto_week: bool,
    lang: ChatLang,
) -> Result<ToolOutput> {
    let (from, to) = (from.to_string(), to.to_string());
    let apps: Vec<String> = apps.iter().map(|a| like_pattern(a)).collect();
    let title_like = title_keyword.map(like_pattern);
    let cats: Vec<String> = categories.iter().map(|c| like_pattern(c)).collect();
    let for_llm = ctx
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
            let mut where_sql = String::from("a.local_date BETWEEN ?1 AND ?2");
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
                      OR COALESCE(g.category_id, a.category_id) LIKE ? ESCAPE '\\')";
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
        for_llm: truncate(&for_llm, RESULT_CHAR_BUDGET),
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
) -> tokio_rusqlite::Result<String> {
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
) -> tokio_rusqlite::Result<String> {
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
) -> tokio_rusqlite::Result<Vec<(String, String, String)>> {
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
        // 显示名:分类表的用户可见名;分类已删/未建行时回落 id;完全未分类归 'other'
        // (模型按回答语言转述,'other' 与英文 builtin id 同样能被正确理解)
        GroupBy::Category => "COALESCE(c.name, COALESCE(g.category_id, a.category_id, 'other'))",
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
) -> tokio_rusqlite::Result<String> {
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
        StatMetric::FirstLast => unreachable!("first_last 在 query_stats 已先行分派"),
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
) -> tokio_rusqlite::Result<String> {
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

/// 每个小时桶最多取几条代表活动(按时长选)。
const TIMELINE_PER_HOUR: i64 = 3;

async fn get_timeline(
    ctx: &ToolCtx,
    (from, to): (NaiveDate, NaiveDate),
    next_citation: usize,
    lang: ChatLang,
) -> Result<ToolOutput> {
    let single_day = from == to;
    let (from, to) = (from.to_string(), to.to_string());
    // 主源是主库活动记录:不开截图/OCR 的用户也有完整时间线,电池暂停或
    // 待识别积压造成的索引空窗也不再啃掉时段尾巴;屏幕文字层的有无由覆盖行披露。
    let coverage = coverage_line(ctx, &from, &to, lang).await?;
    let (total, span, rows) = ctx
        .main
        .call(move |conn| {
            let (total, first, last): (i64, Option<String>, Option<String>) = conn
                .query_row(
                    &format!("SELECT COUNT(*), MIN(a.started_at), MAX(a.ended_at) {ACTIVITY_JOIN}"),
                    params![from, to],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .db()?;
            // 分层抽样:按(日,小时)分桶,桶内取时长最长的前 N 条——预算覆盖整段时间轴。
            // 不能用"ORDER BY started_at LIMIT 50":活跃日几千条活动时那是"最早的
            // 半小时",模型会把开头当成全天(2026-07-08 的误报正是这么来的)。
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT app, title, started_at, ended_at, shot FROM (
                         SELECT COALESCE(g.display_name, a.process_name) AS app,
                                COALESCE(a.window_title,'') AS title,
                                a.started_at, a.ended_at,
                                NULLIF(a.screenshot_path,'') AS shot,
                                ROW_NUMBER() OVER (
                                    PARTITION BY a.local_date, a.local_hour
                                    ORDER BY a.duration_secs DESC
                                ) AS rn
                         {ACTIVITY_JOIN}
                     ) WHERE rn <= ?3 ORDER BY started_at LIMIT ?4",
                ))
                .db()?;
            let out = stmt
                .query_map(
                    params![from, to, TIMELINE_PER_HOUR, TIMELINE_LIMIT as i64],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, Option<String>>(4)?,
                        ))
                    },
                )
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok((total, first.zip(last), out))
        })
        .await?;

    let mut citations = Vec::new();
    let mut lines = Vec::new();
    // 单日范围显示 HH:MM;跨日范围带上 MM-DD,避免不同日期的条目混淆
    let ts_disp = |ts: &str| -> String {
        if single_day {
            ts[11.min(ts.len())..16.min(ts.len())].to_string()
        } else {
            ts[5.min(ts.len())..16.min(ts.len())].to_string()
        }
    };
    for (i, (app, title, started, ended, shot)) in rows.iter().enumerate() {
        let idx = next_citation + i;
        lines.push(format!(
            "[{idx}] {} ~ {} | {} | {}",
            ts_disp(started),
            ts_disp(ended),
            app,
            truncate(title, 50)
        ));
        citations.push(Citation {
            index: idx,
            app: app.clone(),
            title: title.clone(),
            started_ts: started.clone(),
            ended_ts: ended.clone(),
            frame_path: shot.clone(),
        });
    }
    let for_llm = if lines.is_empty() {
        format!("{coverage}\n{}", lang.timeline_empty())
    } else {
        // 头部先声明总量与覆盖范围:样本 ≠ 全量,让模型据此下结论
        let header = match &span {
            Some((first, last)) if total as usize > rows.len() => lang.timeline_header_sampled(
                total,
                &first[..16.min(first.len())],
                &last[..16.min(last.len())],
                rows.len(),
                TIMELINE_PER_HOUR,
            ),
            _ => lang.timeline_header_all(total),
        };
        truncate(
            &format!("{coverage}\n{header}\n{}", lines.join("\n")),
            RESULT_CHAR_BUDGET,
        )
    };
    Ok(ToolOutput { for_llm, citations })
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max_chars).collect();
        format!("{cut}…")
    }
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
    fn fts_injection_neutralized() {
        // 模型想夹带 FTS 语法/引号 → 全部变成字面量
        let kws = vec![r#"a" OR b NEAR(c)"#.to_string()];
        let lit = fts_literal(&kws);
        assert_eq!(lit, "\"a  OR b NEAR(c)\"");
        // LIKE 通配符转义
        assert_eq!(like_pattern("100%_a"), "%100\\%\\_a%");
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
            ToolCall::QueryStats { top_n, .. } => assert_eq!(top_n, TOP_N_MAX),
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
            ToolCall::QueryStats {
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
            ToolCall::QueryStats {
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
            ToolCall::QueryStats {
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
            ToolCall::QueryStats {
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
            ToolCall::QueryStats { top_n, .. } => assert_eq!(top_n, TOP_N_MAX),
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
            ToolCall::QueryStats { top_n, .. } => assert_eq!(top_n, 3),
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
            ToolCall::QueryStats { metric, .. } => assert_eq!(metric, StatMetric::FirstLast),
            _ => panic!(),
        }
    }
}

#[cfg(test)]
mod behavior_tests {
    //! 行为级测试:内存库直测 execute(),盯"披露层"——
    //! 2026-07-08 事故的根因是工具把"第一页"当"全部"喂给模型,
    //! 这里把三态(全量/抽样/命中规模)钉死成回归。
    use super::*;

    async fn ctx_with(mem_sql: &'static str, main_sql: &'static str) -> ToolCtx {
        let mem = Connection::open(":memory:").await.unwrap();
        mem.call(move |c| {
            c.execute_batch(&format!(
                "CREATE TABLE text_sessions (
                     id INTEGER PRIMARY KEY, local_date TEXT, started_ts TEXT,
                     ended_ts TEXT, app_id TEXT, title TEXT, text TEXT DEFAULT '');
                 CREATE VIRTUAL TABLE text_sessions_fts USING fts5(
                     text, content='text_sessions', content_rowid='id', tokenize='trigram');
                 CREATE TABLE session_lines (
                     session_id INTEGER, line_no INTEGER, text TEXT,
                     first_path TEXT, first_ts TEXT);
                 CREATE TABLE frames (
                     path TEXT PRIMARY KEY, ts TEXT, local_date TEXT,
                     ocr_state INTEGER NOT NULL DEFAULT 0);
                 {mem_sql}
                 INSERT INTO text_sessions_fts(rowid, text)
                     SELECT id, text FROM text_sessions;"
            ))?;
            Ok(())
        })
        .await
        .unwrap();
        let main = Connection::open(":memory:").await.unwrap();
        main.call(move |c| {
            c.execute_batch(&format!(
                "CREATE TABLE activities (
                     started_at TEXT, ended_at TEXT, duration_secs INTEGER,
                     local_date TEXT, local_hour INTEGER,
                     process_name TEXT, window_title TEXT, screenshot_path TEXT,
                     category_id TEXT);
                 CREATE TABLE app_group_members (
                     process_name TEXT, group_id TEXT, deleted_at TEXT);
                 CREATE TABLE app_groups (
                     id TEXT, display_name TEXT, category_id TEXT, deleted_at TEXT);
                 CREATE TABLE categories (
                     id TEXT, name TEXT, deleted_at TEXT);
                 {main_sql}"
            ))?;
            Ok(())
        })
        .await
        .unwrap();
        ToolCtx { main, mem }
    }

    fn day() -> (NaiveDate, NaiveDate) {
        let d = NaiveDate::from_ymd_opt(2026, 7, 8).unwrap();
        (d, d)
    }

    /// 造一天 10 个小时 × 每小时 20 条活动的 INSERT 串(主库)。
    fn dense_day_sql() -> &'static str {
        use std::sync::OnceLock;
        static SQL: OnceLock<String> = OnceLock::new();
        SQL.get_or_init(|| {
            let mut s = String::new();
            for h in 9..19 {
                for m in 0..20 {
                    s.push_str(&format!(
                        "INSERT INTO activities(started_at, ended_at, duration_secs,
                             local_date, local_hour, process_name, window_title)
                         VALUES ('2026-07-08T{h:02}:{m:02}:00+09:00',
                                 '2026-07-08T{h:02}:{m:02}:40+09:00', 40,
                                 '2026-07-08', {h}, 'App', '活动 {h}-{m}');\n"
                    ));
                }
            }
            s
        })
    }

    #[tokio::test]
    async fn timeline_sampling_covers_whole_range_and_discloses_total() {
        // 泄漏 dense_day_sql 到 'static:测试进程内一次性,可接受
        let main: &'static str = Box::leak(dense_day_sql().to_string().into_boxed_str());
        let ctx = ctx_with("", main).await;
        let out = execute(
            &ctx,
            &ToolCall::GetTimeline { range: day() },
            1,
            ChatLang::ZhHans,
        )
        .await
        .unwrap();
        // 披露:总数与"样本"措辞
        assert!(out.for_llm.contains("共 200 条活动记录"), "{}", out.for_llm);
        assert!(out.for_llm.contains("样本"), "{}", out.for_llm);
        // 覆盖:10 个小时全部出现(旧实现只会给最早的 50 条 = 前 3 个小时)
        let hours: std::collections::BTreeSet<&str> = out
            .for_llm
            .lines()
            .filter(|l| l.starts_with('['))
            .filter_map(|l| l.split("] ").nth(1))
            .map(|rest| &rest[..2])
            .collect();
        assert_eq!(hours.len(), 10, "每个小时都要有代表: {hours:?}");
        // 每小时至多 3 条
        for h in 9..19 {
            let prefix = format!("] {h:02}:");
            let n = out.for_llm.lines().filter(|l| l.contains(&prefix)).count();
            assert!(n <= 3, "{h} 点出现 {n} 条");
        }
        assert!(out.citations.len() <= TIMELINE_LIMIT);
    }

    #[tokio::test]
    async fn timeline_headers_localized_english() {
        let main: &'static str = Box::leak(dense_day_sql().to_string().into_boxed_str());
        let ctx = ctx_with("", main).await;
        let out = execute(
            &ctx,
            &ToolCall::GetTimeline { range: day() },
            1,
            ChatLang::En,
        )
        .await
        .unwrap();
        assert!(
            out.for_llm.contains("200 activity records in this period"),
            "{}",
            out.for_llm
        );
        assert!(out.for_llm.contains("sample"), "{}", out.for_llm);
        // 骨架(覆盖行 + 头部行)不应残留中文;正文标题是用户数据,语言不限
        for skel in out.for_llm.lines().take(2) {
            assert!(!skel.contains("活动") && !skel.contains("覆盖"), "{skel}");
        }
    }

    #[tokio::test]
    async fn timeline_small_day_lists_all_without_sampling_wording() {
        let ctx = ctx_with(
            "",
            "INSERT INTO activities(started_at, ended_at, duration_secs,
                 local_date, local_hour, process_name, window_title) VALUES
             ('2026-07-08T09:00:00+09:00','2026-07-08T09:05:00+09:00',300,'2026-07-08',9,'A','t1'),
             ('2026-07-08T10:00:00+09:00','2026-07-08T10:05:00+09:00',300,'2026-07-08',10,'B','t2');",
        )
        .await;
        let out = execute(
            &ctx,
            &ToolCall::GetTimeline { range: day() },
            1,
            ChatLang::ZhHans,
        )
        .await
        .unwrap();
        assert!(
            out.for_llm.contains("共 2 条活动记录,全部列出"),
            "{}",
            out.for_llm
        );
        assert!(!out.for_llm.contains("样本"));
        assert_eq!(out.citations.len(), 2);
    }

    #[tokio::test]
    async fn timeline_excludes_hidden_group_and_uses_display_name() {
        let ctx = ctx_with(
            "",
            "INSERT INTO app_groups VALUES
                 ('g1','Visual Studio Code','dev',NULL),
                 ('g2','Secret','hidden',NULL);
             INSERT INTO app_group_members VALUES
                 ('Code.exe','g1',NULL), ('secret.exe','g2',NULL);
             INSERT INTO activities(started_at, ended_at, duration_secs,
                 local_date, local_hour, process_name, window_title) VALUES
             ('2026-07-08T09:00:00+09:00','2026-07-08T09:30:00+09:00',1800,'2026-07-08',9,'Code.exe','main.rs'),
             ('2026-07-08T10:00:00+09:00','2026-07-08T10:30:00+09:00',1800,'2026-07-08',10,'secret.exe','diary');",
        )
        .await;
        let out = execute(
            &ctx,
            &ToolCall::GetTimeline { range: day() },
            1,
            ChatLang::ZhHans,
        )
        .await
        .unwrap();
        assert!(
            out.for_llm.contains("Visual Studio Code"),
            "{}",
            out.for_llm
        );
        assert!(!out.for_llm.contains("secret"), "{}", out.for_llm);
        assert!(out.for_llm.contains("共 1 条活动记录"), "{}", out.for_llm);
    }

    // ── 覆盖披露边界(设计矩阵 A/B/C/G)──────────────────

    /// A:完全没开截图/OCR 的用户——时间线照常给活动记录,覆盖行讲明索引缺席。
    #[tokio::test]
    async fn timeline_works_with_empty_memory_index() {
        let ctx = ctx_with(
            "",
            "INSERT INTO activities(started_at, ended_at, duration_secs,
                 local_date, local_hour, process_name, window_title) VALUES
             ('2026-07-08T09:00:00+09:00','2026-07-08T09:30:00+09:00',1800,'2026-07-08',9,'A','t');",
        )
        .await;
        let out = execute(
            &ctx,
            &ToolCall::GetTimeline { range: day() },
            1,
            ChatLang::ZhHans,
        )
        .await
        .unwrap();
        assert!(out.for_llm.contains("共 1 条活动记录"), "{}", out.for_llm);
        assert!(out.for_llm.contains("均无屏幕文字索引"), "{}", out.for_llm);
    }

    /// A(搜索面):索引为空时关键词靠窗口标题层兜底命中。
    #[tokio::test]
    async fn search_falls_back_to_title_hits_without_index() {
        let ctx = ctx_with(
            "",
            "INSERT INTO activities(started_at, ended_at, duration_secs,
                 local_date, local_hour, process_name, window_title, screenshot_path) VALUES
             ('2026-07-08T09:00:00+09:00','2026-07-08T09:10:00+09:00',600,'2026-07-08',9,
              'chrome','keychron K3 发货通知','shots/a.jpg');",
        )
        .await;
        let out = execute(
            &ctx,
            &ToolCall::SearchText {
                keywords: vec!["keychron".into()],
                range: None,
            },
            1,
            ChatLang::ZhHans,
        )
        .await
        .unwrap();
        assert!(out.for_llm.contains("窗口标题命中 1 条"), "{}", out.for_llm);
        assert!(out.for_llm.contains("均无屏幕文字索引"), "{}", out.for_llm);
        assert!(!out.for_llm.contains("没有命中"), "{}", out.for_llm);
        assert_eq!(out.citations.len(), 1);
        assert_eq!(out.citations[0].frame_path.as_deref(), Some("shots/a.jpg"));
    }

    /// B:截图开了但识别没跑完——覆盖行报待识别帧数,不误判成"未开启"。
    #[tokio::test]
    async fn coverage_discloses_pending_frames() {
        let ctx = ctx_with(
            "INSERT INTO frames(path, ts, local_date, ocr_state) VALUES
             ('f1','2026-07-08T09:00:00+09:00','2026-07-08',0),
             ('f2','2026-07-08T09:01:00+09:00','2026-07-08',0),
             ('f3','2026-07-08T09:02:00+09:00','2026-07-08',0);",
            "INSERT INTO activities(started_at, ended_at, duration_secs,
                 local_date, local_hour, process_name, window_title) VALUES
             ('2026-07-08T09:00:00+09:00','2026-07-08T09:30:00+09:00',1800,'2026-07-08',9,'A','t');",
        )
        .await;
        let out = execute(
            &ctx,
            &ToolCall::GetTimeline { range: day() },
            1,
            ChatLang::ZhHans,
        )
        .await
        .unwrap();
        assert!(out.for_llm.contains("3 帧截图待识别"), "{}", out.for_llm);
        assert!(!out.for_llm.contains("未开启"), "{}", out.for_llm);
    }

    /// C:时有时无的用户——分母是"活动日",分子是"有索引的日"。
    #[tokio::test]
    async fn coverage_reports_partial_days() {
        let ctx = ctx_with(
            "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, text)
             VALUES ('2026-07-08','2026-07-08T09:00:00+09:00',
                     '2026-07-08T09:05:00+09:00','A','t','some text');",
            "INSERT INTO activities(started_at, ended_at, duration_secs,
                 local_date, local_hour, process_name, window_title) VALUES
             ('2026-07-07T09:00:00+09:00','2026-07-07T09:30:00+09:00',1800,'2026-07-07',9,'A','t'),
             ('2026-07-08T09:00:00+09:00','2026-07-08T09:30:00+09:00',1800,'2026-07-08',9,'A','t');",
        )
        .await;
        let range = (
            NaiveDate::from_ymd_opt(2026, 7, 7).unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 8).unwrap(),
        );
        let out = execute(&ctx, &ToolCall::GetTimeline { range }, 1, ChatLang::ZhHans)
            .await
            .unwrap();
        assert!(
            out.for_llm.contains("2 个活动日中 1 日有屏幕文字索引"),
            "{}",
            out.for_llm
        );
    }

    /// G:覆盖完整且两层都零命中——才允许模型说"屏幕上没出现过"。
    #[tokio::test]
    async fn search_full_coverage_zero_hit_states_no_hit() {
        let ctx = ctx_with(
            "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, text)
             VALUES ('2026-07-08','2026-07-08T09:00:00+09:00',
                     '2026-07-08T09:05:00+09:00','A','notes','完全无关的内容');",
            "INSERT INTO activities(started_at, ended_at, duration_secs,
                 local_date, local_hour, process_name, window_title) VALUES
             ('2026-07-08T09:00:00+09:00','2026-07-08T09:30:00+09:00',1800,'2026-07-08',9,'A','notes');",
        )
        .await;
        let out = execute(
            &ctx,
            &ToolCall::SearchText {
                keywords: vec!["keychron".into()],
                range: Some(day()),
            },
            1,
            ChatLang::ZhHans,
        )
        .await
        .unwrap();
        assert!(out.for_llm.contains("没有命中"), "{}", out.for_llm);
        assert!(
            out.for_llm.contains("1 个活动日中 1 日有屏幕文字索引"),
            "{}",
            out.for_llm
        );
        assert!(out.citations.is_empty());
    }

    /// 跨层去重:标题命中落在已返回的屏幕文字会话时间段内 → 剔除,段外保留。
    #[tokio::test]
    async fn title_hit_inside_returned_session_span_dropped() {
        let ctx = ctx_with(
            "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, text)
             VALUES ('2026-07-08','2026-07-08T09:00:00+09:00',
                     '2026-07-08T10:00:00+09:00','chrome','评测','keychron 键盘评测正文');",
            "INSERT INTO activities(started_at, ended_at, duration_secs,
                 local_date, local_hour, process_name, window_title) VALUES
             ('2026-07-08T09:30:00+09:00','2026-07-08T09:40:00+09:00',600,'2026-07-08',9,
              'chrome','keychron 评测页'),
             ('2026-07-08T11:00:00+09:00','2026-07-08T11:10:00+09:00',600,'2026-07-08',11,
              'chrome','keychron 下单页');",
        )
        .await;
        let out = execute(
            &ctx,
            &ToolCall::SearchText {
                keywords: vec!["keychron".into()],
                range: Some(day()),
            },
            1,
            ChatLang::ZhHans,
        )
        .await
        .unwrap();
        // FTS 层 1 条 + 标题层只剩段外的 11:00 那条
        assert!(out.for_llm.contains("keychron 下单页"), "{}", out.for_llm);
        assert!(!out.for_llm.contains("keychron 评测页"), "{}", out.for_llm);
        assert_eq!(out.citations.len(), 2);
        // 编号连续:FTS 层 [1],标题层 [2]
        assert_eq!(out.citations[1].index, 2);
        assert_eq!(out.citations[1].title, "keychron 下单页");
    }

    #[tokio::test]
    async fn search_discloses_total_hits_beyond_window() {
        let mut sql = String::new();
        for i in 0..20 {
            sql.push_str(&format!(
                "INSERT INTO text_sessions(local_date, started_ts, ended_ts, app_id, title, text)
                 VALUES ('2026-07-08','2026-07-08T09:{i:02}:00+09:00',
                         '2026-07-08T09:{i:02}:30+09:00','A','t{i}','keychron 订单第 {i} 条记录');\n"
            ));
        }
        let mem: &'static str = Box::leak(sql.into_boxed_str());
        let ctx = ctx_with(mem, "").await;
        let out = execute(
            &ctx,
            &ToolCall::SearchText {
                keywords: vec!["keychron".into()],
                range: None,
            },
            1,
            ChatLang::ZhHans,
        )
        .await
        .unwrap();
        assert!(out.for_llm.contains("共 20 条命中"), "{}", out.for_llm);
        assert!(out.for_llm.contains("前 8 条"), "{}", out.for_llm);
        assert_eq!(out.citations.len(), SEARCH_LIMIT);
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
        let out = execute(
            &ctx,
            &ToolCall::QueryStats {
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
            },
            1,
            ChatLang::ZhHans,
        )
        .await
        .unwrap();
        assert!(out.for_llm.contains("共 8 组"), "{}", out.for_llm);
        assert!(out.for_llm.contains("前 5 组"), "{}", out.for_llm);
    }

    /// QueryStats 的全默认构造,测试逐项覆盖自己关心的字段。
    fn stats_call(range: (NaiveDate, NaiveDate)) -> ToolCall {
        ToolCall::QueryStats {
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
        let mut call = stats_call(day());
        if let ToolCall::QueryStats { group_by, .. } = &mut call {
            *group_by = GroupBy::Category;
        }
        let out = execute(&ctx, &call, 1, ChatLang::ZhHans).await.unwrap();
        assert!(
            out.for_llm.contains("工作: 1 小时 0 分钟"),
            "{}",
            out.for_llm
        );
        assert!(out.for_llm.contains("other: 30 分钟"), "{}", out.for_llm);
        assert!(!out.for_llm.contains("零星"), "{}", out.for_llm);
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
        let mut call = stats_call(range);
        if let ToolCall::QueryStats { bucket, .. } = &mut call {
            *bucket = Bucket::Day;
        }
        let out = execute(&ctx, &call, 1, ChatLang::ZhHans).await.unwrap();
        assert!(
            out.for_llm.contains("逐日统计(2 个有记录的日子"),
            "{}",
            out.for_llm
        );
        // 行首带本地化星期(07-06 周一/07-08 周三),模型不用自己换算
        assert!(
            out.for_llm.contains("2026-07-06(周一): 40 分钟"),
            "{}",
            out.for_llm
        );
        assert!(
            out.for_llm.contains("2026-07-08(周三): 1 小时 0 分钟"),
            "{}",
            out.for_llm
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
        let mut call = stats_call(day());
        if let ToolCall::QueryStats { categories, .. } = &mut call {
            *categories = vec!["游戏".into()];
        }
        let out = execute(&ctx, &call, 1, ChatLang::ZhHans).await.unwrap();
        assert!(
            out.for_llm.contains("合计: 1 小时 0 分钟"),
            "{}",
            out.for_llm
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
        let mut call = stats_call(range);
        if let ToolCall::QueryStats { bucket, .. } = &mut call {
            *bucket = Bucket::Week;
        }
        let out = execute(&ctx, &call, 1, ChatLang::ZhHans).await.unwrap();
        assert!(out.for_llm.contains("逐周统计(1 周"), "{}", out.for_llm);
        assert!(
            out.for_llm.contains("2026-07-06: 1 小时 0 分钟"),
            "{}",
            out.for_llm
        );
        // 非自动升级:不出现自动改周的说明
        assert!(!out.for_llm.contains("自动改为按周"), "{}", out.for_llm);
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
        let mut call = stats_call(range);
        if let ToolCall::QueryStats { bucket, .. } = &mut call {
            *bucket = Bucket::HourOfDay;
        }
        let out = execute(&ctx, &call, 1, ChatLang::ZhHans).await.unwrap();
        assert!(
            out.for_llm.contains("按一天中的小时聚合"),
            "{}",
            out.for_llm
        );
        assert!(out.for_llm.contains("09:00: 40 分钟"), "{}", out.for_llm);
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
        let mut call = stats_call(range);
        if let ToolCall::QueryStats { metric, .. } = &mut call {
            *metric = StatMetric::FirstLast;
        }
        let out = execute(&ctx, &call, 1, ChatLang::ZhHans).await.unwrap();
        assert!(out.for_llm.contains("共 2 条匹配活动"), "{}", out.for_llm);
        assert!(
            out.for_llm.contains("最早一次始于 2026-07-05 09:00"),
            "{}",
            out.for_llm
        );
        assert!(
            out.for_llm.contains("最近一次止于 2026-07-08 21:00"),
            "{}",
            out.for_llm
        );
    }

    /// 手动验收(不经过 LLM):直连真实库跑 get_timeline + search_text,
    /// 打印模型将看到的工具原文——覆盖行、两层命中、总数披露、证据帧数。
    /// 只读连接,不写任何数据。跑法(在 src-tauri 目录下):
    ///
    ///   CHAT_DATE=2026-07-19 CHAT_KW=B站 \
    ///     cargo test --lib manual_real_db -- --ignored --nocapture
    ///
    /// 可选:
    /// - CHAT_DATE_TO=2026-07-19 跨日范围(默认单日);
    /// - CHAT_KW 不设则只跑时间线;
    /// - CHAT_MEM_EMPTY=1 把记忆库换成空库(主库仍真实)——模拟
    ///   "从没开过截图/OCR"的用户(边界 A),看覆盖行与标题层兜底。
    #[tokio::test]
    #[ignore]
    async fn manual_real_db() {
        let date = std::env::var("CHAT_DATE").expect("设 CHAT_DATE=YYYY-MM-DD");
        let from = NaiveDate::parse_from_str(&date, "%Y-%m-%d").expect("CHAT_DATE 格式不对");
        let to = std::env::var("CHAT_DATE_TO")
            .ok()
            .map(|t| NaiveDate::parse_from_str(&t, "%Y-%m-%d").expect("CHAT_DATE_TO 格式不对"))
            .unwrap_or(from);
        let ctx = if std::env::var("CHAT_MEM_EMPTY").is_ok() {
            // 真实主库 + 空记忆库(borrow 测试 schema)= 零截图用户视角
            let empty = ctx_with("", "").await;
            let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
            let main = Connection::open_with_flags(crate::storage::db_path().unwrap(), flags)
                .await
                .unwrap();
            ToolCtx {
                main,
                mem: empty.mem,
            }
        } else {
            ToolCtx::open_readonly().await.unwrap()
        };

        let out = execute(
            &ctx,
            &ToolCall::GetTimeline { range: (from, to) },
            1,
            ChatLang::ZhHans,
        )
        .await
        .unwrap();
        let with_frame = out
            .citations
            .iter()
            .filter(|c| c.frame_path.is_some())
            .count();
        println!("\n── get_timeline({from} ~ {to}) ──────────────");
        println!("{}", out.for_llm);
        println!(
            "(引用 {} 条,其中带证据帧 {} 条)",
            out.citations.len(),
            with_frame
        );

        if let Ok(kw) = std::env::var("CHAT_KW") {
            let out = execute(
                &ctx,
                &ToolCall::SearchText {
                    keywords: vec![kw.clone()],
                    range: Some((from, to)),
                },
                1,
                ChatLang::ZhHans,
            )
            .await
            .unwrap();
            println!("\n── search_text(\"{kw}\", {from} ~ {to}) ──────");
            println!("{}", out.for_llm);
            println!("(引用 {} 条)", out.citations.len());
        }
    }
}
