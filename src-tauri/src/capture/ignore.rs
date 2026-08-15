//! 忽略规则：决定一条活动行是否**计入统计**。
//!
//! 与 [`super::privacy`] 的区别（两者正交，容易混）：
//! - privacy 管"要不要截图"——命中则不存图，但活动行照常入库、照常计时；
//! - 本模块管"算不算时长"——命中则活动行照常入库、照常截图，但 `excluded = 1`，
//!   所有报表 / 导出 / AI 总结 / chat 查询都跳过它。
//!
//! 语义等同内建 `hidden` 分类，只是粒度细到**窗口标题**：`hidden` 归的是整个应用，
//! 这里能只排除某个应用下的某类窗口（典型场景：终端里挂机跑下载，终端本身要留）。
//!
//! 可逆：规则删掉后跑一次 [`crate::repo::activities::reapply_ignore_rules`]
//! 重算全表，历史数据重新计入。

use serde::{Deserialize, Serialize};

/// 一条忽略规则 = 进程名（精确）+ 窗口标题关键词（子串，可选）。
///
/// 为什么必须带进程名、不允许"纯标题关键词"：标题关键词单独生效的话，
/// 用户填一个 `Download` 就会把所有应用里带这个词的窗口一起吞掉，
/// 而被吞掉的时长不报错、不提示，只是统计对不上——静默丢数据是这类
/// 功能最容易出的事故。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IgnoreRule {
    /// 进程名。精确匹配，忽略大小写、忽略首尾空白。
    pub process_name: String,
    /// 窗口标题关键词。子串匹配，忽略大小写。
    ///
    /// `None` = 整个进程都不计入统计（即 RemoveAppDialog 注释里提到、但一直
    /// 没实现的「不再记录」的统计版）。
    ///
    /// 注意 `Some("")` / `Some("   ")` **不等于** `None`：全空白关键词一律不命中。
    /// 因为 `contains("")` 恒为 true，把空串当"匹配一切"会让一次 UI 手滑
    /// 静默排除整个应用；想排除整个应用必须显式传 `None`。
    #[serde(default)]
    pub title_keyword: Option<String>,
}

/// 当前窗口是否命中任一规则 → 该活动行不计入统计。
///
/// 空列表 = 不排除任何东西。
pub fn is_excluded(app_name: &str, title: &str, rules: &[IgnoreRule]) -> bool {
    if rules.is_empty() {
        return false;
    }
    let app = app_name.trim().to_lowercase();
    if app.is_empty() {
        return false;
    }
    let title_lower = title.to_lowercase();

    rules.iter().any(|rule| {
        let want = rule.process_name.trim().to_lowercase();
        // 空进程名的规则视为无效：否则它会匹配所有窗口。
        if want.is_empty() || want != app {
            return false;
        }
        match rule.title_keyword.as_deref() {
            // 无标题条件 = 整个进程
            None => true,
            Some(kw) => {
                let kw = kw.trim().to_lowercase();
                // 全空白关键词不命中（见字段文档）
                !kw.is_empty() && title_lower.contains(&kw)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(process: &str, title: Option<&str>) -> IgnoreRule {
        IgnoreRule {
            process_name: process.to_string(),
            title_keyword: title.map(String::from),
        }
    }

    #[test]
    fn 空规则列表不排除() {
        assert!(!is_excluded("Windows Terminal Host", "任意标题", &[]));
    }

    #[test]
    fn 进程加标题都命中才排除() {
        let rules = vec![rule("Windows Terminal Host", Some("Download videos"))];
        assert!(is_excluded(
            "Windows Terminal Host",
            "✳ Download videos from July 17 onwards with uv",
            &rules,
        ));
    }

    #[test]
    fn 进程对但标题不对不排除() {
        let rules = vec![rule("Windows Terminal Host", Some("Download videos"))];
        // 同一个终端里干别的活,必须照常计入
        assert!(!is_excluded(
            "Windows Terminal Host",
            "vim src/main.rs",
            &rules,
        ));
    }

    #[test]
    fn 标题对但进程不对不排除() {
        let rules = vec![rule("Windows Terminal Host", Some("Download videos"))];
        assert!(!is_excluded("Chrome", "Download videos - YouTube", &rules));
    }

    #[test]
    fn 无标题条件时整个进程被排除() {
        let rules = vec![rule("SomeDownloader", None)];
        assert!(is_excluded("SomeDownloader", "任意标题", &rules));
        assert!(is_excluded("SomeDownloader", "", &rules));
    }

    #[test]
    fn 进程名忽略大小写和首尾空白() {
        let rules = vec![rule("  windows terminal host  ", Some("download"))];
        assert!(is_excluded(
            "Windows Terminal Host",
            "DOWNLOAD videos",
            &rules,
        ));
    }

    #[test]
    fn 进程名是精确匹配不是子串() {
        // 防止 "Terminal" 这条规则顺带吃掉 "Windows Terminal Host"
        let rules = vec![rule("Terminal", None)];
        assert!(!is_excluded("Windows Terminal Host", "x", &rules));
    }

    #[test]
    fn 全空白标题关键词不命中() {
        // contains("") 恒 true —— 必须挡住,否则一次手滑排除整个应用
        let rules = vec![rule("Windows Terminal Host", Some("   "))];
        assert!(!is_excluded("Windows Terminal Host", "任意标题", &rules));
    }

    #[test]
    fn 空进程名的规则无效() {
        let rules = vec![rule("   ", Some("download"))];
        assert!(!is_excluded("Chrome", "download videos", &rules));
    }

    #[test]
    fn 空应用名不命中() {
        let rules = vec![rule("Chrome", None)];
        assert!(!is_excluded("", "x", &rules));
    }

    #[test]
    fn 多条规则任一命中即排除() {
        let rules = vec![
            rule("Chrome", Some("YouTube")),
            rule("Windows Terminal Host", Some("Download videos")),
        ];
        assert!(is_excluded(
            "Windows Terminal Host",
            "⠂ Download videos from July 17",
            &rules,
        ));
    }

    #[test]
    fn spinner字符变化不影响匹配() {
        // Claude Code 的转圈动画每帧换字符,同一段任务会产生 ⠐/✳/⠂ 三种标题。
        // 关键词只取文字部分,三种前缀都要命中。
        let rules = vec![rule("Windows Terminal Host", Some("Download videos from"))];
        for prefix in ["⠐ ", "✳ ", "⠂ ", ""] {
            let title = format!("{prefix}Download videos from July 17 onwards with uv");
            assert!(is_excluded("Windows Terminal Host", &title, &rules));
        }
    }
}
