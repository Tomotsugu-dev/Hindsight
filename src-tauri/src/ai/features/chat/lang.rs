//! 聊天自己的多语言文字：系统提示词、循环里回填给模型的话、降级文案、问题改写器。
//! 文字在 `lang/chat.json`。`ChatLang` 本身和工具的文字在 [`crate::ai::tools`]。
//!
//! 回答语言策略(产品决定):**跟随提问语言优先,界面语言兜底**——
//! 规则写进各语言的系统提示词第 6 条。

use std::sync::LazyLock;

use chrono::NaiveDate;
use serde::Deserialize;

use crate::ai::tools::lang::text::{load, Text};
use crate::ai::tools::ChatLang;

/// 对应 `lang/chat.json`，一句话一个字段。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatText {
    system_prompt: Text,
    history_replay_note: Text,
    dup_call: Text,
    tool_exec_failed: Text,
    steps_exhausted: Text,
    degraded_no_evidence: Text,
    degraded_with_evidence: Text,
    rewrite_prompt: Text,
    err_nothing_to_regenerate: Text,
    err_conversation_busy: Text,
}

const TEXT_JSON: &str = include_str!("lang/chat.json");

static TEXT: LazyLock<ChatText> = LazyLock::new(|| load(TEXT_JSON));

impl ChatLang {
    /// 系统提示词(整篇按界面语言;第 6 条 = 提问语言优先、本语言兜底)。
    pub fn system_prompt(self, today: NaiveDate) -> String {
        let wd = self.weekday(&today.format("%u").to_string());
        TEXT.system_prompt
            .fill(self, &[("today", &today), ("wd", &wd)])
    }

    // ── engine 循环内回填给模型的文案 ─────────────────────

    /// 历史使用守则:只在"改写失败、消毒历史真的进上下文"的兜底分支追加进
    /// system(engine.rs 的 None 臂)。不放 system_prompt 固定规则——主路径
    /// (首问/改写成功)零历史,放那儿是死条款,每问最多 7 次往返白付 token,
    /// 还让模型对不存在的"历史对话"做无谓对齐。
    /// 只禁"数字/结论":引用编号已有双层硬防御(sanitize_history_content 进入
    /// 侧物理剥除 + bind_citations 输出侧过滤),提示词无需重复管。
    pub fn history_replay_note(self) -> &'static str {
        TEXT.history_replay_note.get(self)
    }

    pub fn dup_call(self) -> &'static str {
        TEXT.dup_call.get(self)
    }

    pub fn tool_exec_failed(self) -> &'static str {
        TEXT.tool_exec_failed.get(self)
    }

    // 三条回填文案统一限定"本轮工具返回的资料"——旧措辞"(以上)已有资料"在
    // 兜底路径(消毒历史在上下文里)会被读成"包括上轮答案",作为更靠后的消息
    // 直接压过历史使用守则,诱导模型复述陈旧结论而不是承认本轮没查到。
    pub fn steps_exhausted(self) -> &'static str {
        TEXT.steps_exhausted.get(self)
    }

    // ── 降级文案(用户可见) ─────────────────────────────

    pub fn degraded_no_evidence(self) -> &'static str {
        TEXT.degraded_no_evidence.get(self)
    }

    pub fn degraded_with_evidence(self) -> &'static str {
        TEXT.degraded_with_evidence.get(self)
    }

    /// 「问题自立化」改写器的系统提示词(多轮第二问起,先把新问题改写成
    /// 不依赖上下文的自足问题,再让回答器以零历史状态作答——历史污染物理隔离)。
    pub fn rewrite_prompt(self, today: NaiveDate) -> String {
        TEXT.rewrite_prompt.fill(self, &[("today", &today)])
    }

    /// 同会话并发拒的用户可见文案(唯一直接展示给用户而非模型的条目)。
    pub fn err_nothing_to_regenerate(self) -> &'static str {
        TEXT.err_nothing_to_regenerate.get(self)
    }

    pub fn err_conversation_busy(self) -> &'static str {
        TEXT.err_conversation_busy.get(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tools::lang::text::assert_same_placeholders;

    // chat.json 能读成 ChatText，每句话 6 种语言的占位符一样。
    #[test]
    fn chat_json_reads_and_keeps_placeholders_in_step() {
        LazyLock::force(&TEXT);
        assert_same_placeholders(TEXT_JSON);
    }

    // 文字里不能有连续空格，它们会原样发给模型。
    #[test]
    fn chat_text_has_no_runs_of_spaces() {
        use std::collections::BTreeMap;

        let all: BTreeMap<String, BTreeMap<String, String>> =
            serde_json::from_str(TEXT_JSON).unwrap();
        for (key, langs) in &all {
            for (lang, s) in langs {
                assert!(!s.contains("  "), "{key} 的 {lang} 有连续空格");
            }
        }
    }

    /// 六语 × 全部格式化方法的批量烟测:每个变体都产出非空文案,
    /// 且插值参数(日期/数字/工具名)确实出现在结果里——防"漏改某语言"和
    /// "占位符没插进去"两类低级错误,一次覆盖全部 match 臂。
    #[test]
    fn all_langs_produce_nonempty_interpolated_strings() {
        const LANGS: [ChatLang; 6] = [
            ChatLang::ZhHans,
            ChatLang::ZhHant,
            ChatLang::En,
            ChatLang::Ja,
            ChatLang::Pt,
            ChatLang::Es,
        ];
        let today = NaiveDate::from_ymd_opt(2026, 7, 26).unwrap();
        for lang in LANGS {
            // 静态文案:非空即可
            for s in [
                lang.dup_call(),
                lang.tool_exec_failed(),
                lang.steps_exhausted(),
                lang.degraded_no_evidence(),
                lang.degraded_with_evidence(),
                lang.err_conversation_busy(),
            ] {
                assert!(!s.is_empty(), "{lang:?} 有空文案");
            }
            // 插值类:参数必须真的出现在产物里
            assert!(lang.system_prompt(today).contains("2026-07-26"), "{lang:?}");
            // 规则数对齐:恒 7 条固定规则;历史使用守则不进 system_prompt
            // (它在 history_replay_note,由 engine 仅在兜底分支追加)。
            // 任何语言臂漏改/多改规则条目,这里立刻红。
            let sp = lang.system_prompt(today);
            assert!(
                sp.contains("\n7. ") && !sp.contains("\n8. "),
                "{lang:?} system_prompt 应恰含 1-7 条规则"
            );
            assert!(!lang.history_replay_note().is_empty(), "{lang:?}");
            assert!(
                lang.rewrite_prompt(today).contains("2026-07-26"),
                "{lang:?}"
            );
        }
    }

    #[test]
    fn rewrite_prompt_embeds_today_in_all_langs() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 7, 20).unwrap();
        for lang in [
            ChatLang::ZhHans,
            ChatLang::ZhHant,
            ChatLang::En,
            ChatLang::Ja,
            ChatLang::Pt,
            ChatLang::Es,
        ] {
            let p = lang.rewrite_prompt(today);
            assert!(p.contains("2026-07-20"), "{lang:?} 缺 today");
        }
    }

    #[test]
    fn system_prompt_language_policy_present() {
        let d = NaiveDate::from_ymd_opt(2026, 7, 8).unwrap();
        assert!(ChatLang::En.system_prompt(d).contains("reply in English"));
        assert!(ChatLang::ZhHans.system_prompt(d).contains("简体中文"));
        assert!(ChatLang::Ja.system_prompt(d).contains("日本語"));
        assert!(ChatLang::Es.system_prompt(d).contains("en español"));
    }

    /// 历史使用守则与三条循环回填文案必须限定"本轮"——旧措辞"(以上)已有资料"
    /// 在兜底路径会被读成"包括上轮答案",诱导复述陈旧结论(code review 实锤)。
    /// 五语言逐一钉死限定词,防止未来措辞回退。
    #[test]
    fn engine_feedback_strings_scope_to_this_turn() {
        let cases: [(ChatLang, &str); 6] = [
            (ChatLang::ZhHans, "本轮"),
            (ChatLang::ZhHant, "本輪"),
            (ChatLang::En, "this turn"),
            (ChatLang::Ja, "今回"),
            // nesta/desta rodada 两种搭配都合法,断言公共词干
            (ChatLang::Pt, "rodada"),
            // en/de esta ronda 同理
            (ChatLang::Es, "ronda"),
        ];
        for (lang, marker) in cases {
            assert!(
                lang.history_replay_note().contains(marker),
                "{lang:?} history_replay_note 缺本轮限定"
            );
            assert!(
                lang.steps_exhausted().contains(marker),
                "{lang:?} steps_exhausted 缺本轮限定"
            );
            assert!(
                lang.dup_call().contains(marker),
                "{lang:?} dup_call 缺本轮限定"
            );
            assert!(
                lang.tool_exec_failed().contains(marker),
                "{lang:?} tool_exec_failed 缺本轮限定"
            );
        }
    }
}
