pub(super) mod params;
pub(super) mod read;
pub(super) mod search;
pub(super) mod stats;

use chrono::NaiveDate;
use serde_json::Value;

use self::read::timeline::TimelineQuery;
use self::search::activity::ActivityQuery;
use self::search::ocr::OcrQuery;
use self::stats::query_stats::StatsQuery;
use super::{ChatLang, ToolCtx, ToolError, ToolOutput};
use crate::ai::model::{ToolCall, ToolSpec};
use crate::error::Result;

trait Tool: Sized {
    const NAME: &'static str;

    fn spec(lang: ChatLang) -> ToolSpec;

    fn parse(
        args: &Value,
        today: NaiveDate,
        lang: ChatLang,
    ) -> std::result::Result<Self, ToolError>;

    async fn execute(
        &self,
        ctx: &ToolCtx,
        next_citation: usize,
        max_chars: usize,
        lang: ChatLang,
    ) -> Result<ToolOutput>;
}

// `QueryStats` is named after the tool query_stats, so it starts with the enum name.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq)]
pub enum Query {
    SearchActivity(ActivityQuery),
    SearchOcr(OcrQuery),
    QueryStats(StatsQuery),
    GetTimeline(TimelineQuery),
}

pub(super) fn specs(lang: ChatLang) -> Vec<ToolSpec> {
    vec![
        ActivityQuery::spec(lang),
        OcrQuery::spec(lang),
        StatsQuery::spec(lang),
        TimelineQuery::spec(lang),
    ]
}

pub(super) fn parse(
    call: &ToolCall,
    today: NaiveDate,
    lang: ChatLang,
) -> std::result::Result<Query, ToolError> {
    let args = &call.arguments;
    match call.name.as_str() {
        ActivityQuery::NAME => ActivityQuery::parse(args, today, lang).map(Query::SearchActivity),
        OcrQuery::NAME => OcrQuery::parse(args, today, lang).map(Query::SearchOcr),
        StatsQuery::NAME => StatsQuery::parse(args, today, lang).map(Query::QueryStats),
        TimelineQuery::NAME => TimelineQuery::parse(args, today, lang).map(Query::GetTimeline),
        other => {
            let known: Vec<String> = specs(lang).into_iter().map(|s| s.name).collect();
            let msg = lang.err_unknown_tool(other, &known.join(" / "));
            Err(params::invalid(&msg, lang))
        }
    }
}

impl Query {
    pub(super) async fn execute(
        &self,
        ctx: &ToolCtx,
        next_citation: usize,
        max_chars: usize,
        lang: ChatLang,
    ) -> Result<ToolOutput> {
        match self {
            Self::SearchActivity(q) => q.execute(ctx, next_citation, max_chars, lang).await,
            Self::SearchOcr(q) => q.execute(ctx, next_citation, max_chars, lang).await,
            Self::QueryStats(q) => q.execute(ctx, next_citation, max_chars, lang).await,
            Self::GetTimeline(q) => q.execute(ctx, next_citation, max_chars, lang).await,
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
            e.0.contains("search_titles / search_screen_text / query_stats / get_timeline"),
            "{}",
            e.0
        );
    }

    #[test]
    fn every_spec_name_reaches_its_tool() {
        let names: Vec<String> = specs(ChatLang::ZhHans)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(
            names,
            [
                "search_titles",
                "search_screen_text",
                "query_stats",
                "get_timeline"
            ]
        );
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
