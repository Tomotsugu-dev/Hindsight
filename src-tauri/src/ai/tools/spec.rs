//! 给模型看的三个工具的说明。

use serde_json::{json, Value};

use crate::ai::model::ToolSpec;

pub(super) fn specs() -> Vec<ToolSpec> {
    let date = |desc: &str| json!({"type": "string", "maxLength": 10, "description": format!("{desc},格式 YYYY-MM-DD")});
    let names = |desc: &str, max_items: u32| json!({"type": "array", "items": {"type": "string", "maxLength": 64}, "maxItems": max_items, "description": desc});
    let spec = |name: &str, description: &str, parameters: Value| ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters,
    };
    vec![
        spec(
            "search_text",
            "全文搜索屏幕上出现过的文字(聊天/网页/代码/订单等,逐字索引)。返回头部标注总命中数,正文为相关度最高的若干条。没命中时换同义词、英文或更短的词重试。",
            json!({"type": "object", "properties": {
                "keywords": names("1-3 个关键词,逐字匹配", 3),
                "date_from": date("起始日期,可选"),
                "date_to": date("结束日期,可选")
            }, "required": ["keywords"]}),
        ),
        spec(
            "query_stats",
            "统计应用/内容的使用时长或使用次数。可按应用名过滤(apps)、按分类名过滤(categories)、按窗口标题关键词过滤(title_keyword,如视频名),可分组排行(group_by,含按用户分类的工作/娱乐口径),可分桶看趋势(bucket)。问'用了多久/花了多少时间'用默认(metric=duration);问'启动/打开/玩了几次'用 metric=session_count;问'第一次/最近一次用X是什么时候'用 metric=first_last(此口径 date_from 可给很早的日期,如 2020-01-01)。问某类活动(如游戏)的趋势用 categories+bucket,不要自己列举应用名——用户归类过的应用你未必认识。",
            json!({"type": "object", "properties": {
                "date_from": date("起始日期"),
                "date_to": date("结束日期"),
                "apps": names("应用名过滤,可选", 5),
                "categories": names("分类名过滤(如'游戏'),按用户的应用分类圈定范围,可选;分类名可先用 group_by=category 查到", 5),
                "title_keyword": {"type": "string", "maxLength": 64, "description": "窗口标题关键词过滤,可选"},
                "group_by": {"type": "string", "enum": ["none", "app", "title", "category"], "description": "分组维度,默认 none;category=按用户设置的应用分类(工作/娱乐/游戏等)分组"},
                "top_n": {"type": "integer", "minimum": 1, "maximum": 10, "description": "分组时取前 N,默认 5"},
                "metric": {"type": "string", "enum": ["duration", "session_count", "first_last"], "description": "统计口径:duration=累计时长(默认);session_count=使用会话次数;first_last=最早与最近一次记录的时间(忽略分组/分桶)"},
                "gap_minutes": {"type": "integer", "minimum": 5, "maximum": 240, "description": "会话计数用:相邻活动间隔超过这么多分钟算一段新会话,默认 30。仅用户明确说'离开X分钟以上算一次'时才填"},
                "bucket": {"type": "string", "enum": ["none", "day", "week", "hour_of_day"], "description": "趋势分桶,与 group_by 互斥:day=逐日(最多 60 天,更长自动按周);week=逐周(行首日期为周一);hour_of_day=按一天 0-23 时聚合看作息分布"}
            }, "required": ["date_from", "date_to"]}),
        ),
        spec(
            "get_timeline",
            "按小时抽样列出某时段的屏幕活动会话(时间、应用、标题),回答'某天/某下午在干什么'。返回头部标注总条数与覆盖范围,正文是抽样代表,不是全量。",
            json!({"type": "object", "properties": {
                "date_from": date("起始日期"),
                "date_to": date("结束日期")
            }, "required": ["date_from", "date_to"]}),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_specs_list_three_tools_with_required_ranges() {
        let specs = specs();
        let names: Vec<&str> = specs.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["search_text", "query_stats", "get_timeline"]);
        for t in &specs {
            let req = t.parameters["required"].as_array().unwrap();
            assert!(
                req.iter().any(|r| r == "date_from") || t.name == "search_text",
                "{} 缺日期必填",
                t.name
            );
        }
    }

    /// 本机的 grammar 从同一份定义生成，长度和条数的限制要写在定义里。
    #[test]
    fn tool_specs_carry_the_limits() {
        let specs = specs();
        let props = |name: &str| {
            specs.iter().find(|t| t.name == name).unwrap().parameters["properties"].clone()
        };
        let search = props("search_text");
        assert_eq!(search["keywords"]["maxItems"], 3);
        assert_eq!(search["keywords"]["items"]["maxLength"], 64);
        assert_eq!(search["date_from"]["maxLength"], 10);
        let stats = props("query_stats");
        assert_eq!(stats["categories"]["maxItems"], 5);
        assert_eq!(stats["top_n"]["maximum"], 10);
        assert_eq!(stats["gap_minutes"]["minimum"], 5);
    }
}
