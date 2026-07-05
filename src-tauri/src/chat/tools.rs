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
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tokio_rusqlite::Connection;

use crate::error::Result;
use crate::storage::SqliteResultExt;

/// 单工具返回给模型的结果字符上限——超出截断,防止一个大会话吃光上下文。
const RESULT_CHAR_BUDGET: usize = 4000;
/// 搜索命中数上限(服务端硬夹紧,模型无法调大)
const SEARCH_LIMIT: usize = 8;
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
        group_by: GroupBy,
        top_n: usize,
        metric: StatMetric,
        /// 会话计数用:相邻活动间隔超过这么多分钟就算一段新会话
        gap_minutes: u32,
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
}

/// 统计口径:时长(默认)还是"用了几次/玩了几次"的会话次数。
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatMetric {
    /// 累计使用时长(SUM duration_secs)
    Duration,
    /// 使用会话次数:相邻活动间隔超过 gap_minutes 就切一段
    SessionCount,
}

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
}

/// 第②道墙:工具名 + 参数逐项校验。`today` 用于拒绝未来日期。
/// Err 的文案会回填给模型,写给模型看。
pub fn validate(
    name: &str,
    raw: &RawParams,
    today: NaiveDate,
) -> std::result::Result<ToolCall, String> {
    match name {
        "search_text" => {
            let keywords = clean_keywords(&raw.keywords)?;
            let range = parse_range_opt(raw, today)?;
            Ok(ToolCall::SearchText { keywords, range })
        }
        "query_stats" => {
            let range = parse_range_opt(raw, today)?
                .ok_or("query_stats 需要 date_from 和 date_to(YYYY-MM-DD)")?;
            let apps = clean_short_strings(&raw.apps, 5, "apps")?;
            let title_keyword = match raw.title_keyword.as_deref().map(str::trim) {
                Some("") | None => None,
                Some(t) if t.chars().count() <= 64 => Some(t.to_string()),
                Some(_) => return Err("title_keyword 过长(≤64 字符)".into()),
            };
            Ok(ToolCall::QueryStats {
                range,
                apps,
                title_keyword,
                group_by: raw.group_by.unwrap_or(GroupBy::None),
                top_n: raw.top_n.unwrap_or(5).clamp(1, TOP_N_MAX),
                metric: raw.metric.unwrap_or(StatMetric::Duration),
                gap_minutes: raw
                    .gap_minutes
                    .unwrap_or(GAP_DEFAULT_MIN)
                    .clamp(GAP_MIN, GAP_MAX),
            })
        }
        "get_timeline" => {
            let range = parse_range_opt(raw, today)?
                .ok_or("get_timeline 需要 date_from 和 date_to(YYYY-MM-DD)")?;
            Ok(ToolCall::GetTimeline { range })
        }
        other => Err(format!(
            "未知工具 {other},只能用 search_text / query_stats / get_timeline"
        )),
    }
}

fn parse_range_opt(
    raw: &RawParams,
    today: NaiveDate,
) -> std::result::Result<Option<(NaiveDate, NaiveDate)>, String> {
    let (Some(f), Some(t)) = (raw.date_from.as_deref(), raw.date_to.as_deref()) else {
        return Ok(None);
    };
    let from = NaiveDate::parse_from_str(f.trim(), "%Y-%m-%d")
        .map_err(|_| format!("date_from 不是有效日期: {f}"))?;
    let to = NaiveDate::parse_from_str(t.trim(), "%Y-%m-%d")
        .map_err(|_| format!("date_to 不是有效日期: {t}"))?;
    if from > to {
        return Err("date_from 晚于 date_to".into());
    }
    if (to - from).num_days() > 366 {
        return Err("时间跨度超过 366 天,请缩小范围".into());
    }
    if from > today {
        return Err("date_from 在未来".into());
    }
    Ok(Some((from, to)))
}

fn clean_keywords(raw: &[String]) -> std::result::Result<Vec<String>, String> {
    let out = clean_short_strings(raw, 3, "keywords")?;
    if out.is_empty() {
        return Err("keywords 不能为空".into());
    }
    Ok(out)
}

fn clean_short_strings(
    raw: &[String],
    max_items: usize,
    field: &str,
) -> std::result::Result<Vec<String>, String> {
    let mut out = Vec::new();
    for s in raw.iter().take(max_items) {
        let t = s.trim();
        if t.is_empty() {
            continue;
        }
        if t.chars().count() > 64 {
            return Err(format!("{field} 里有超过 64 字符的项"));
        }
        out.push(t.to_string());
    }
    Ok(out)
}

/// FTS MATCH 注入防护:每个词包成带引号的字符串字面量(内部引号剥除),
/// 多词以空格连接 = 隐式 AND。模型/用户输入永远进不了 FTS 语法位。
fn fts_literal(keywords: &[String]) -> String {
    keywords
        .iter()
        .map(|k| format!("\"{}\"", k.replace('"', " ")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// LIKE 通配符转义(ESCAPE '\')。
fn like_pattern(s: &str) -> String {
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
pub async fn execute(ctx: &ToolCtx, call: &ToolCall, next_citation: usize) -> Result<ToolOutput> {
    match call {
        ToolCall::SearchText { keywords, range } => {
            search_text(ctx, keywords, *range, next_citation).await
        }
        ToolCall::QueryStats {
            range,
            apps,
            title_keyword,
            group_by,
            top_n,
            metric,
            gap_minutes,
        } => {
            query_stats(
                ctx,
                *range,
                apps,
                title_keyword.as_deref(),
                *group_by,
                *top_n,
                *metric,
                *gap_minutes,
            )
            .await
        }
        ToolCall::GetTimeline { range } => get_timeline(ctx, *range, next_citation).await,
    }
}

async fn search_text(
    ctx: &ToolCtx,
    keywords: &[String],
    range: Option<(NaiveDate, NaiveDate)>,
    next_citation: usize,
) -> Result<ToolOutput> {
    let fts = fts_literal(keywords);
    let first_kw = like_pattern(&keywords[0]);
    let (from, to) = match range {
        Some((f, t)) => (f.to_string(), t.to_string()),
        None => ("0000-00-00".into(), "9999-99-99".into()),
    };
    let rows = ctx
        .mem
        .call(move |conn| {
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
            Ok(out)
        })
        .await?;

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
    let for_llm = if lines.is_empty() {
        "没有命中。可尝试换关键词(同义词/英文/更短的词)再搜。".to_string()
    } else {
        truncate(&lines.join("\n"), RESULT_CHAR_BUDGET)
    };
    Ok(ToolOutput { for_llm, citations })
}

#[allow(clippy::too_many_arguments)]
async fn query_stats(
    ctx: &ToolCtx,
    (from, to): (NaiveDate, NaiveDate),
    apps: &[String],
    title_keyword: Option<&str>,
    group_by: GroupBy,
    top_n: usize,
    metric: StatMetric,
    gap_minutes: u32,
) -> Result<ToolOutput> {
    let (from, to) = (from.to_string(), to.to_string());
    let apps: Vec<String> = apps.iter().map(|a| like_pattern(a)).collect();
    let title_like = title_keyword.map(like_pattern);
    let for_llm = ctx
        .main
        .call(move |conn| {
            // 动态拼的只有 WHERE 的占位段与 GROUP BY 维度,全部来自白名单枚举,
            // 参数一律绑定——不存在字符串拼接注入面。
            let mut where_sql = String::from("local_date BETWEEN ?1 AND ?2");
            let mut bind: Vec<Box<dyn rusqlite::ToSql>> =
                vec![Box::new(from.clone()), Box::new(to.clone())];
            if !apps.is_empty() {
                let ors = vec!["process_name LIKE ? ESCAPE '\\'"; apps.len()].join(" OR ");
                where_sql.push_str(&format!(" AND ({ors})"));
                for a in &apps {
                    bind.push(Box::new(a.clone()));
                }
            }
            if let Some(t) = &title_like {
                where_sql.push_str(" AND window_title LIKE ? ESCAPE '\\'");
                bind.push(Box::new(t.clone()));
            }
            let params_ref: Vec<&dyn rusqlite::ToSql> = bind.iter().map(|b| b.as_ref()).collect();

            match metric {
                StatMetric::Duration => {
                    query_duration(conn, &where_sql, &params_ref, group_by, top_n, &from, &to)
                }
                StatMetric::SessionCount => query_session_count(
                    conn,
                    &where_sql,
                    &params_ref,
                    group_by,
                    top_n,
                    gap_minutes,
                    &from,
                    &to,
                ),
            }
        })
        .await?;
    Ok(ToolOutput {
        for_llm: truncate(&for_llm, RESULT_CHAR_BUDGET),
        citations: Vec::new(),
    })
}

/// 时长口径:SUM(duration_secs),可分组 top-N。
fn query_duration(
    conn: &rusqlite::Connection,
    where_sql: &str,
    params: &[&dyn rusqlite::ToSql],
    group_by: GroupBy,
    top_n: usize,
    from: &str,
    to: &str,
) -> tokio_rusqlite::Result<String> {
    match group_by {
        GroupBy::None => {
            let secs: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COALESCE(SUM(duration_secs),0) FROM activities WHERE {where_sql}"
                    ),
                    params,
                    |r| r.get(0),
                )
                .db()?;
            Ok(format!("{from} ~ {to} 合计: {}", fmt_secs(secs)))
        }
        GroupBy::App | GroupBy::Title => {
            let dim = group_dim(group_by);
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {dim}, SUM(duration_secs) d FROM activities
                     WHERE {where_sql} GROUP BY {dim} ORDER BY d DESC LIMIT {top_n}"
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
                return Ok(format!("{from} ~ {to} 无匹配记录"));
            }
            let body = rows
                .iter()
                .map(|(name, secs)| format!("{}: {}", truncate(name, 50), fmt_secs(*secs)))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(format!("{from} ~ {to} 按时长排序:\n{body}"))
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
    where_sql: &str,
    params: &[&dyn rusqlite::ToSql],
    group_by: GroupBy,
    top_n: usize,
    gap_minutes: u32,
    from: &str,
    to: &str,
) -> tokio_rusqlite::Result<String> {
    let gap_secs = gap_minutes as i64 * 60;
    match group_by {
        GroupBy::None => {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT started_at, ended_at FROM activities
                     WHERE {where_sql} ORDER BY started_at"
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
                return Ok(format!("{from} ~ {to} 无匹配记录"));
            }
            let n = count_sessions(&rows, gap_secs);
            Ok(format!(
                "{from} ~ {to} 使用会话次数: {n} 次(以间隔≥{gap_minutes} 分钟算一次)"
            ))
        }
        GroupBy::App | GroupBy::Title => {
            let dim = group_dim(group_by);
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {dim} AS g, started_at, ended_at FROM activities
                     WHERE {where_sql} ORDER BY g, started_at"
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
            if rows.is_empty() {
                return Ok(format!("{from} ~ {to} 无匹配记录"));
            }
            // 按组聚合(rows 已按 g 排序)→ 每组切分计数
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
            counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            counts.truncate(top_n);
            let body = counts
                .iter()
                .map(|(name, n)| format!("{}: {n} 次", truncate(name, 50)))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(format!(
                "{from} ~ {to} 使用会话次数(以间隔≥{gap_minutes} 分钟算一次):\n{body}"
            ))
        }
    }
}

fn group_dim(group_by: GroupBy) -> &'static str {
    if group_by == GroupBy::App {
        "process_name"
    } else {
        "COALESCE(window_title,'(无标题)')"
    }
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

async fn get_timeline(
    ctx: &ToolCtx,
    (from, to): (NaiveDate, NaiveDate),
    next_citation: usize,
) -> Result<ToolOutput> {
    let (from, to) = (from.to_string(), to.to_string());
    let rows = ctx
        .mem
        .call(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT COALESCE(app_id,''), COALESCE(title,''), started_ts, ended_ts
                     FROM text_sessions
                     WHERE local_date BETWEEN ?1 AND ?2
                     ORDER BY started_ts LIMIT ?3",
                )
                .db()?;
            let out = stmt
                .query_map(params![from, to, TIMELINE_LIMIT as i64], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })
                .db()?
                .collect::<rusqlite::Result<Vec<_>>>()
                .db()?;
            Ok(out)
        })
        .await?;

    let mut citations = Vec::new();
    let mut lines = Vec::new();
    for (i, (app, title, started, ended)) in rows.iter().enumerate() {
        let idx = next_citation + i;
        lines.push(format!(
            "[{idx}] {} ~ {} | {} | {}",
            &started[11.min(started.len())..16.min(started.len())],
            &ended[11.min(ended.len())..16.min(ended.len())],
            app,
            truncate(title, 50)
        ));
        citations.push(Citation {
            index: idx,
            app: app.clone(),
            title: title.clone(),
            started_ts: started.clone(),
            ended_ts: ended.clone(),
            frame_path: None,
        });
    }
    let for_llm = if lines.is_empty() {
        "该时段没有屏幕记忆会话(可能未开截图或超出保留范围)。".to_string()
    } else {
        truncate(&lines.join("\n"), RESULT_CHAR_BUDGET)
    };
    Ok(ToolOutput { for_llm, citations })
}

fn fmt_secs(secs: i64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h > 0 {
        format!("{h} 小时 {m} 分钟")
    } else {
        format!("{m} 分钟")
    }
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
        let e = validate("drop_table", &RawParams::default(), today()).unwrap_err();
        assert!(e.contains("未知工具"));

        let e = validate(
            "query_stats",
            &raw(serde_json::json!({"date_from": "2026-07-10", "date_to": "2026-07-01"})),
            today(),
        )
        .unwrap_err();
        assert!(e.contains("晚于"));

        let e = validate(
            "query_stats",
            &raw(serde_json::json!({"date_from": "2026-08-01", "date_to": "2026-08-02"})),
            today(),
        )
        .unwrap_err();
        assert!(e.contains("未来"));

        let e = validate(
            "query_stats",
            &raw(serde_json::json!({"date_from": "2020-01-01", "date_to": "2026-07-01"})),
            today(),
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
}
