//! 具体的工具，按干什么分三类：找、算、读。
//!
//! 这里是登记处：`Tool` trait、`Query` 和工具名单。只有这里认识全部工具；
//! 加一个工具，只改这个文件和新工具自己。

pub(super) mod params;
pub(super) mod read;
pub(super) mod search;
pub(super) mod stats;

use chrono::NaiveDate;
use serde_json::Value;

use self::read::timeline::TimelineQuery;
use self::search::text::SearchQuery;
use self::stats::query_stats::StatsQuery;
use super::{ChatLang, ToolCtx, ToolError, ToolOutput};
use crate::ai::model::{ToolCall, ToolSpec};
use crate::error::Result;

/// 一个工具。实现它的是这个工具检查过的参数。
trait Tool: Sized {
    /// 给模型看的工具名。
    const NAME: &'static str;

    /// 给模型看的说明和参数格式。
    fn spec() -> ToolSpec;

    /// 解析、检查模型传来的参数。不合法时，原因写给模型看。
    fn parse(
        args: &Value,
        today: NaiveDate,
        lang: ChatLang,
    ) -> std::result::Result<Self, ToolError>;

    /// 查库，写成文字和证据。引用编号从 `next_citation` 开始。
    async fn execute(
        &self,
        ctx: &ToolCtx,
        next_citation: usize,
        lang: ChatLang,
    ) -> Result<ToolOutput>;
}

/// 检查过的调用。一个工具一个分支。
// 分支名跟工具名一一对应，所以有一个叫 QueryStats。
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq)]
pub enum Query {
    SearchText(SearchQuery),
    QueryStats(StatsQuery),
    GetTimeline(TimelineQuery),
}

/// 给模型看的全部工具。
pub(super) fn specs() -> Vec<ToolSpec> {
    vec![
        SearchQuery::spec(),
        StatsQuery::spec(),
        TimelineQuery::spec(),
    ]
}

/// 按工具名找到工具，交给它解析参数。
pub(super) fn parse(
    call: &ToolCall,
    today: NaiveDate,
    lang: ChatLang,
) -> std::result::Result<Query, ToolError> {
    let args = &call.arguments;
    match call.name.as_str() {
        SearchQuery::NAME => SearchQuery::parse(args, today, lang).map(Query::SearchText),
        StatsQuery::NAME => StatsQuery::parse(args, today, lang).map(Query::QueryStats),
        TimelineQuery::NAME => TimelineQuery::parse(args, today, lang).map(Query::GetTimeline),
        other => {
            let known: Vec<String> = specs().into_iter().map(|s| s.name).collect();
            let msg = lang.err_unknown_tool(other, &known.join(" / "));
            Err(params::invalid(&msg, lang))
        }
    }
}

impl Query {
    /// 交给对应的工具执行。
    pub(super) async fn execute(
        &self,
        ctx: &ToolCtx,
        next_citation: usize,
        lang: ChatLang,
    ) -> Result<ToolOutput> {
        match self {
            Self::SearchText(q) => q.execute(ctx, next_citation, lang).await,
            Self::QueryStats(q) => q.execute(ctx, next_citation, lang).await,
            Self::GetTimeline(q) => q.execute(ctx, next_citation, lang).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall {
            id: "c1".into(),
            name: name.into(),
            arguments: args,
        }
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 5).unwrap()
    }

    #[test]
    fn unknown_tool_rejected_with_the_names_that_exist() {
        let e = parse(&call("drop_table", Value::Null), today(), ChatLang::ZhHans).unwrap_err();
        assert!(e.0.contains("未知工具 drop_table"), "{}", e.0);
        assert!(
            e.0.contains("search_text / query_stats / get_timeline"),
            "{}",
            e.0
        );
    }

    /// 名单漏写一行时，这个测试会失败：说明里的每个工具名都要能交给对应的工具解析。
    #[test]
    fn every_spec_name_reaches_its_tool() {
        let names: Vec<String> = specs().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["search_text", "query_stats", "get_timeline"]);
        for name in &names {
            if let Err(e) = parse(
                &call(name, serde_json::json!({})),
                today(),
                ChatLang::ZhHans,
            ) {
                assert!(!e.0.contains("未知工具"), "{name} 没登记: {}", e.0);
            }
        }
    }

    #[test]
    fn args_that_are_not_an_object_get_a_format_error() {
        let args = serde_json::json!("2026-07-01");
        let e = parse(&call("get_timeline", args), today(), ChatLang::ZhHans).unwrap_err();
        assert!(e.0.starts_with("参数格式错误"), "{}", e.0);
    }
}
