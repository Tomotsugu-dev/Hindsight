//! 保存 AI 设置前把用户提交的值洗干净。

use super::segments::sanitize_hex_color;
use super::{Advanced, AiConfig, AiSegment, ModelRef, Pick};

/// 把用户提交的 AiConfig 钳到合法范围。
///
/// 注意：segments 过滤后**全空**时回退到 `old.segments`，避免用户误删空了
/// 整组时段（前端 UI 也不应允许空，但兜底一层稳）。
///
/// 字段处理：
/// - 字符串：trim
/// - segments：过滤掉 `start_hour >= end_hour` 或 `end_hour > 24` 的项
/// - 数值字段：clamp 到合理范围
pub fn sanitize(mut next: AiConfig, old: &AiConfig) -> AiConfig {
    next.endpoint = next.endpoint.trim().to_string();
    next.model = next.model.trim().to_string();
    next.api_key = next.api_key.trim().to_string();
    next.user_brief = next.user_brief.trim().to_string();

    // external_provider：只接受预设值，非法回退到 "openai"。
    // ⚠️ 必须与前端 ExternalApiTab 的 PROVIDER_KEYS 逐项对齐——少一项,
    // 用户在下拉里选中它就会被这里打回 "openai"(kimi/kimi-cn 曾如此)。
    next.external_provider = match next.external_provider.trim() {
        "openai" | "deepseek" | "kimi" | "kimi-cn" | "openrouter" | "together" | "groq"
        | "custom" => next.external_provider.trim().to_string(),
        _ => "openai".to_string(),
    };

    // chat_thinking:档位白名单(强度值域各家不同,UI 按服务商展示子集,
    // 注入层就近降级);"on" 是分档前的旧值,迁移为 "high"。非法回退 "auto"。
    next.chat_thinking = match next.chat_thinking.trim() {
        "on" => "high".to_string(),
        v @ ("off" | "low" | "medium" | "high" | "max") => v.to_string(),
        _ => "auto".to_string(),
    };

    // 文件名为空的本机模型等于没选
    if let Pick::Local { file } = &mut next.picks.weekly {
        *file = file.trim().to_string();
        if file.is_empty() {
            next.picks.weekly = Pick::Inherit;
        }
    }
    // 自动的项不用存；同一个「任务 + 模型」只留最后一项
    let mut advanced: Vec<Advanced> = Vec::new();
    for mut a in next.advanced.into_iter().rev() {
        if let ModelRef::Local { file } = &mut a.model {
            *file = file.trim().to_string();
        }
        if a.reasoning.is_some()
            && !advanced
                .iter()
                .any(|kept| kept.task == a.task && kept.model == a.model)
        {
            advanced.push(a);
        }
    }
    advanced.reverse();
    next.advanced = advanced;

    let valid_segments: Vec<AiSegment> = next
        .segments
        .into_iter()
        .filter(|s| s.start_hour < s.end_hour && s.end_hour <= 24)
        .map(|mut s| {
            s.label = s.label.trim().to_string();
            s.color = sanitize_hex_color(&s.color);
            s
        })
        .collect();
    next.segments = if valid_segments.is_empty() {
        old.segments.clone()
    } else {
        valid_segments
    };

    // 已保存云端配置:字段 trim;端点为空的条目没有意义,丢弃;上限 10
    next.external_profiles = next
        .external_profiles
        .into_iter()
        .map(|mut p| {
            p.name = p.name.trim().to_string();
            p.provider = p.provider.trim().to_string();
            p.endpoint = p.endpoint.trim().to_string();
            p.api_key = p.api_key.trim().to_string();
            p.model = p.model.trim().to_string();
            p
        })
        .filter(|p| !p.endpoint.is_empty())
        .take(10)
        .collect();

    // 自动总结时间:必须是合法 "HH:MM",否则视为未设置(尽快模式)
    next.auto_summary_at = next.auto_summary_at.and_then(|s| {
        let s = s.trim().to_string();
        chrono::NaiveTime::parse_from_str(&s, "%H:%M")
            .ok()
            .map(|_| s)
    });
    // 多时间点:逐项校验、去重保首见、保持添加顺序、上限 6(与定时补识别同规)
    next.auto_summary_times = {
        let mut out: Vec<String> = Vec::new();
        for item in next.auto_summary_times {
            let t = item.trim().to_string();
            if chrono::NaiveTime::parse_from_str(&t, "%H:%M").is_ok() && !out.contains(&t) {
                out.push(t);
            }
            if out.len() >= 6 {
                break;
            }
        }
        out
    };

    // 引擎启动级参数 clamp：
    // batch ≥ 32 是 llama-server 不接受过小值的安全下限
    // ctx 上限给 256K（极限场景，超出基本任何卡都装不下，没必要再大）
    // parallel_slots ≥ 1，给 32 上限避免误填出格的值
    next.batch_size = next.batch_size.map(|b| b.clamp(32, 32_768));
    next.parallel_slots = next.parallel_slots.map(|n| n.clamp(1, 32));
    next.ctx_size = next.ctx_size.map(|c| c.clamp(512, 262_144));
    next.summary_batch_size = next.summary_batch_size.map(|b| b.clamp(32, 32_768));
    next.summary_parallel_slots = next.summary_parallel_slots.map(|n| n.clamp(1, 32));
    next.summary_ctx_size = next.summary_ctx_size.map(|c| c.clamp(512, 262_144));

    next.models_path = next.models_path.trim().to_string();
    next.active_main = next.active_main.trim().to_string();
    next.active_mmproj = next.active_mmproj.trim().to_string();
    next.summary_main = next.summary_main.trim().to_string();
    next.summary_mmproj = next.summary_mmproj.trim().to_string();
    next.chat_main = next.chat_main.trim().to_string();

    // prompt_language 限制取值；非法回退到 zh
    next.prompt_language = match next.prompt_language.trim() {
        "tw" => "tw".to_string(),
        "en" => "en".to_string(),
        "ja" => "ja".to_string(),
        "pt" => "pt".to_string(),
        "es" => "es".to_string(),
        _ => "zh".to_string(),
    };
    // 覆盖文本不 trim 中间空白（用户可能想保留缩进），仅去前后整体空白
    next.prompt_overrides.system_zh = next.prompt_overrides.system_zh.trim().to_string();
    next.prompt_overrides.system_en = next.prompt_overrides.system_en.trim().to_string();
    next.prompt_overrides.system_ja = next.prompt_overrides.system_ja.trim().to_string();
    next.prompt_overrides.system_pt = next.prompt_overrides.system_pt.trim().to_string();
    next.prompt_overrides.system_tw = next.prompt_overrides.system_tw.trim().to_string();
    next.prompt_overrides.system_es = next.prompt_overrides.system_es.trim().to_string();

    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::config::tests::{base, VALID_LANGS};
    use crate::ai::config::{ExternalProfile, ReasoningLevel, Task};

    fn seg(label: &str, start: u8, end: u8, color: &str) -> AiSegment {
        AiSegment {
            label: label.to_string(),
            start_hour: start,
            end_hour: end,
            color: color.to_string(),
        }
    }

    /// external_profiles:trim、空端点丢弃、上限 10、camelCase 契约、旧配置回落空表。
    #[test]
    fn external_profiles_semantics() {
        let mk = |endpoint: &str, key: &str| ExternalProfile {
            name: format!(" p-{endpoint} "),
            provider: "openai".into(),
            endpoint: endpoint.into(),
            api_key: key.into(),
            model: " gpt-4o-mini ".into(),
        };
        let old = AiConfig::default();

        let cfg = AiConfig {
            external_profiles: vec![
                mk(" https://api.openai.com/v1 ", " sk-1 "),
                mk("", "sk-孤儿钥匙"), // 空端点:无意义,丢弃
            ],
            ..Default::default()
        };
        let out = sanitize(cfg, &old).external_profiles;
        assert_eq!(out.len(), 1, "空端点条目应被丢弃");
        assert_eq!(out[0].endpoint, "https://api.openai.com/v1", "应 trim");
        assert_eq!(out[0].api_key, "sk-1");
        assert_eq!(out[0].model, "gpt-4o-mini");

        // 上限 10
        let many = AiConfig {
            external_profiles: (0..12).map(|i| mk(&format!("https://e{i}"), "k")).collect(),
            ..Default::default()
        };
        assert_eq!(sanitize(many, &old).external_profiles.len(), 10);

        // 序列化契约 + 旧配置无字段回落空表
        let one = AiConfig {
            external_profiles: vec![mk("https://e", "k")],
            ..Default::default()
        };
        let json = serde_json::to_string(&one).unwrap();
        assert!(json.contains("\"externalProfiles\""), "{json}");
        assert!(json.contains("\"apiKey\":\"k\""), "{json}");
        let parsed: AiConfig = serde_json::from_str("{}").unwrap();
        assert!(parsed.external_profiles.is_empty());
    }

    /// auto_summary_times:净化(校验/去重/顺序/上限 6)、effective 三态、
    /// 序列化契约与旧配置回落。
    #[test]
    fn auto_summary_times_semantics() {
        let v = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mk = |items: &[&str]| AiConfig {
            auto_summary_times: items.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        let old = AiConfig::default();

        // 净化:非法剔除、重复保首见、保持添加顺序、trim
        assert_eq!(
            sanitize(mk(&["23:00", "乱", " 12:00 ", "23:00", "27:00"]), &old).auto_summary_times,
            v(&["23:00", "12:00"])
        );
        // 上限 6
        assert_eq!(
            sanitize(
                mk(&["01:00", "02:00", "03:00", "04:00", "05:00", "06:00", "07:00"]),
                &old
            )
            .auto_summary_times
            .len(),
            6
        );

        // effective:列表优先;空回落旧单字段;都空 = 空(尽快模式)
        let both = AiConfig {
            auto_summary_at: Some("03:00".into()),
            auto_summary_times: v(&["12:00"]),
            ..Default::default()
        };
        assert_eq!(both.effective_auto_summary_times(), v(&["12:00"]));
        let legacy_only = AiConfig {
            auto_summary_at: Some("03:00".into()),
            ..Default::default()
        };
        assert_eq!(legacy_only.effective_auto_summary_times(), v(&["03:00"]));
        assert!(AiConfig::default()
            .effective_auto_summary_times()
            .is_empty());

        // 序列化:camelCase + 旧 JSON 无此字段回落空表
        let json = serde_json::to_string(&mk(&["23:00"])).unwrap();
        assert!(json.contains("\"autoSummaryTimes\":[\"23:00\"]"), "{json}");
        let parsed: AiConfig = serde_json::from_str("{}").unwrap();
        assert!(parsed.auto_summary_times.is_empty());
    }

    /// auto_summary_at:合法 HH:MM 保留(含 trim),非法清为 None(尽快模式)。
    #[test]
    fn sanitize_auto_summary_at_validates_format() {
        let mk = |v: &str| AiConfig {
            auto_summary_at: Some(v.to_string()),
            ..Default::default()
        };
        let old = AiConfig::default();
        assert_eq!(
            sanitize(mk("03:00"), &old).auto_summary_at.as_deref(),
            Some("03:00")
        );
        assert_eq!(
            sanitize(mk("  22:30 "), &old).auto_summary_at.as_deref(),
            Some("22:30"),
            "前后空白应被 trim 后保留"
        );
        assert_eq!(sanitize(mk("25:00"), &old).auto_summary_at, None);
        assert_eq!(sanitize(mk("凌晨"), &old).auto_summary_at, None);
        assert_eq!(sanitize(mk(""), &old).auto_summary_at, None);
        // 未设置维持 None
        assert_eq!(sanitize(AiConfig::default(), &old).auto_summary_at, None);
    }

    #[test]
    fn sanitize_prompt_language_whitelist_and_fallback() {
        // 白名单内的值（含带空白）原样保留；名单外一律回退 zh——
        // prompt 模板按语言取套，落一个未知 tag 会让取模板处 panic 或拿错语言。
        for lang in VALID_LANGS {
            let mut next = base();
            next.prompt_language = format!("  {lang}  ");
            let out = sanitize(next, &base());
            assert_eq!(out.prompt_language, lang, "合法语言应 trim 后保留");
        }
        for bad in ["fr", "zh-CN", "EN", "", "  ", "日本語"] {
            let mut next = base();
            next.prompt_language = bad.to_string();
            let out = sanitize(next, &base());
            assert_eq!(out.prompt_language, "zh", "非法语言 {bad:?} 应回退 zh");
        }
    }

    #[test]
    fn sanitize_external_provider_whitelist_and_fallback() {
        // provider 决定 UI placeholder 与思考参数注入分支,落库前归一避免前端
        // 拿到未知值渲染错乱。⚠️ 这份清单必须与前端 PROVIDER_KEYS 完全一致:
        // 曾漏掉 kimi/kimi-cn,用户在下拉里选中 Kimi 会被打回 openai(选不中),
        // 且思考参数走错分支。
        for p in [
            "openai",
            "deepseek",
            "kimi",
            "kimi-cn",
            "openrouter",
            "together",
            "groq",
            "custom",
        ] {
            let mut next = base();
            next.external_provider = format!(" {p} ");
            let out = sanitize(next, &base());
            assert_eq!(out.external_provider, p);
        }
        for bad in ["azure", "OpenAI", "", "gpt"] {
            let mut next = base();
            next.external_provider = bad.to_string();
            let out = sanitize(next, &base());
            assert_eq!(
                out.external_provider, "openai",
                "非法 provider {bad:?} 应回退 openai"
            );
        }
    }

    #[test]
    fn sanitize_trims_all_string_fields() {
        // 这些字段最终会拼 URL / 文件路径 / header，首尾空白会造成难查的 404 或找不到文件
        let mut next = base();
        next.endpoint = "  https://api.example.com/v1  ".to_string();
        next.model = "\tgpt-4o-mini\n".to_string();
        next.api_key = " sk-abc ".to_string();
        next.user_brief = "  程序员  ".to_string();
        next.models_path = " /data/models ".to_string();
        next.active_main = " main.gguf ".to_string();
        next.active_mmproj = " mmproj.gguf ".to_string();
        next.summary_main = " sum.gguf ".to_string();
        next.summary_mmproj = " sum-mm.gguf ".to_string();
        next.chat_main = " chat.gguf ".to_string();
        let out = sanitize(next, &base());
        assert_eq!(out.endpoint, "https://api.example.com/v1");
        assert_eq!(out.model, "gpt-4o-mini");
        assert_eq!(out.api_key, "sk-abc");
        assert_eq!(out.user_brief, "程序员");
        assert_eq!(out.models_path, "/data/models");
        assert_eq!(out.active_main, "main.gguf");
        assert_eq!(out.active_mmproj, "mmproj.gguf");
        assert_eq!(out.summary_main, "sum.gguf");
        assert_eq!(out.summary_mmproj, "sum-mm.gguf");
        assert_eq!(out.chat_main, "chat.gguf");
    }

    #[test]
    fn sanitize_prompt_overrides_trim_edges_keep_inner_indent() {
        // 用户自定义 prompt 里的内部缩进/换行是有意排版，只能去首尾整体空白；
        // 纯空白视为"没写覆盖"归空，否则会以空白 prompt 顶掉内置默认。
        let text = "\n  第一行\n    缩进第二行\n";
        let mut next = base();
        next.prompt_overrides.system_zh = text.to_string();
        next.prompt_overrides.system_en = format!("  {text}  ");
        next.prompt_overrides.system_ja = "   \n\t  ".to_string(); // 纯空白 → 空
        next.prompt_overrides.system_pt = "single".to_string();
        next.prompt_overrides.system_tw = "\t保留\t中间\ttab\t".to_string();
        next.prompt_overrides.system_es = format!("\n{text}\n");
        let out = sanitize(next, &base());
        assert_eq!(out.prompt_overrides.system_zh, "第一行\n    缩进第二行");
        assert_eq!(out.prompt_overrides.system_en, "第一行\n    缩进第二行");
        assert_eq!(out.prompt_overrides.system_ja, "");
        assert_eq!(out.prompt_overrides.system_pt, "single");
        assert_eq!(out.prompt_overrides.system_tw, "保留\t中间\ttab");
        assert_eq!(out.prompt_overrides.system_es, "第一行\n    缩进第二行");
    }

    #[test]
    fn sanitize_segments_filters_invalid_trims_label_normalizes_color() {
        // 非法时段（start>=end、end>24）静默丢弃而非整体拒绝——
        // 让用户拖拽编辑中间态提交时不至于全盘失败
        let mut next = base();
        next.segments = vec![
            seg("  早上  ", 6, 9, " #ABC "), // 合法：label 去空白、颜色归一
            seg("倒置", 9, 6, ""),           // start > end → 丢
            seg("零长", 12, 12, ""),         // start == end → 丢
            seg("越界", 20, 25, ""),         // end > 24 → 丢
            seg("晚上", 18, 24, "#ff8800"),  // 合法：end==24 是边界允许值
        ];
        let out = sanitize(next, &base());
        assert_eq!(out.segments.len(), 2, "只留两条合法段");
        assert_eq!(out.segments[0].label, "早上");
        assert_eq!(out.segments[0].color, "#aabbcc");
        assert_eq!(out.segments[1].label, "晚上");
        assert_eq!(out.segments[1].color, "#ff8800");
    }

    #[test]
    fn sanitize_segments_all_invalid_falls_back_to_old() {
        // 全部段都非法时若接受空组，AI 总结将无段可跑；兜底回退旧值
        let mut old = base();
        old.segments = vec![seg("旧段", 0, 24, "")];
        let mut next = base();
        next.segments = vec![seg("坏", 10, 5, ""), seg("坏2", 3, 30, "")];
        let out = sanitize(next, &old);
        assert_eq!(out.segments.len(), 1);
        assert_eq!(out.segments[0].label, "旧段");
        assert_eq!(
            (out.segments[0].start_hour, out.segments[0].end_hour),
            (0, 24)
        );
    }

    #[test]
    fn sanitize_engine_params_clamped_to_documented_ranges() {
        // 期望范围独立取自文档约定：batch 32~32768、slots 1~32、ctx 512~262144。
        // 越界值钳到边界而不是拒绝——保证引擎启动参数永远可用。
        let mut next = base();
        next.batch_size = Some(1);
        next.parallel_slots = Some(0);
        next.ctx_size = Some(100);
        next.summary_batch_size = Some(1_000_000);
        next.summary_parallel_slots = Some(999);
        next.summary_ctx_size = Some(u32::MAX);
        let out = sanitize(next, &base());
        assert_eq!(out.batch_size, Some(32), "batch 下界 32");
        assert_eq!(out.parallel_slots, Some(1), "slots 下界 1");
        assert_eq!(out.ctx_size, Some(512), "ctx 下界 512");
        assert_eq!(out.summary_batch_size, Some(32_768), "batch 上界 32768");
        assert_eq!(out.summary_parallel_slots, Some(32), "slots 上界 32");
        assert_eq!(out.summary_ctx_size, Some(262_144), "ctx 上界 262144");
    }

    #[test]
    fn sanitize_engine_params_none_stays_none_and_valid_untouched() {
        // None 语义是"走 llama.cpp 默认"，clamp 不得把它实体化成数字
        let none_case = sanitize(base(), &base());
        assert_eq!(none_case.batch_size, None);
        assert_eq!(none_case.parallel_slots, None);
        assert_eq!(none_case.ctx_size, None);
        assert_eq!(none_case.summary_batch_size, None);
        assert_eq!(none_case.summary_parallel_slots, None);
        assert_eq!(none_case.summary_ctx_size, None);

        // 合法值（含正好在边界上的）不应被改动
        let mut next = base();
        next.batch_size = Some(512);
        next.parallel_slots = Some(4);
        next.ctx_size = Some(8192);
        next.summary_batch_size = Some(32); // 恰在下界
        next.summary_parallel_slots = Some(32); // 恰在上界
        next.summary_ctx_size = Some(262_144); // 恰在上界
        let out = sanitize(next, &base());
        assert_eq!(out.batch_size, Some(512));
        assert_eq!(out.parallel_slots, Some(4));
        assert_eq!(out.ctx_size, Some(8192));
        assert_eq!(out.summary_batch_size, Some(32));
        assert_eq!(out.summary_parallel_slots, Some(32));
        assert_eq!(out.summary_ctx_size, Some(262_144));
    }

    /// 存设置时：空文件名的本机模型当没选；自动的项不存；同一个「任务 + 模型」只留最后一项。
    #[test]
    fn sanitize_cleans_report_settings() {
        let item = |file: &str, reasoning| Advanced {
            task: Task::Daily,
            model: ModelRef::Local { file: file.into() },
            reasoning,
        };
        let mut next = base();
        next.picks.weekly = Pick::Local { file: "  ".into() };
        next.advanced = vec![
            item("a.gguf", Some(ReasoningLevel::Low)),
            item(" a.gguf ", Some(ReasoningLevel::Max)),
            item("b.gguf", None),
        ];
        let out = sanitize(next, &base());
        assert_eq!(out.picks.weekly, Pick::Inherit);
        assert_eq!(
            out.advanced,
            vec![item("a.gguf", Some(ReasoningLevel::Max))]
        );
    }
}
