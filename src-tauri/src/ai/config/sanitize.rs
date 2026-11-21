//! Sanitizes user-submitted [`AiConfig`] values before persistence.

use std::collections::HashSet;

use super::{Advanced, AiConfig, CloudConnection, DaySegment, ModelRef, Pick};

const MAX_CLOUD_CONNECTIONS: usize = 10;
const MAX_AUTO_SUMMARY_TIMES: usize = 6;

/// Normalizes and clamps user-submitted [`AiConfig`] values into valid ranges.
///
/// Falls back to `old.segments` if filtering leaves no valid segments.
pub fn sanitize(mut next: AiConfig, old: &AiConfig) -> AiConfig {
    next.user_brief = next.user_brief.trim().to_string();
    next.models_path = next.models_path.trim().to_string();

    // Cloud connections: trim fields, validate dialect, drop entries missing id or endpoint,
    // deduplicate by id (keep first), cap at MAX_CLOUD_CONNECTIONS.
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut cloud: Vec<CloudConnection> = Vec::new();
    for mut c in next.cloud {
        c.id = c.id.trim().to_string();
        c.name = c.name.trim().to_string();
        c.dialect = sanitize_dialect(&c.dialect);
        c.endpoint = c.endpoint.trim().to_string();
        c.model = c.model.trim().to_string();
        c.api_key = c.api_key.trim().to_string();
        c.context_length = c.context_length.map(|n| n.clamp(512, 2_000_000));
        if c.id.is_empty() || c.endpoint.is_empty() || !seen_ids.insert(c.id.clone()) {
            continue;
        }
        cloud.push(c);
        if cloud.len() >= MAX_CLOUD_CONNECTIONS {
            break;
        }
    }
    next.cloud = cloud;

    // Picks: trim filenames/ids and clear dangling references to removed cloud connections.
    next.picks.daily = next
        .picks
        .daily
        .and_then(|m| sanitize_model_ref(m, &seen_ids));
    next.picks.chat = next
        .picks
        .chat
        .and_then(|m| sanitize_model_ref(m, &seen_ids));
    next.picks.weekly = match next.picks.weekly {
        Pick::Inherit => Pick::Inherit,
        Pick::Local { file } => {
            let file = file.trim().to_string();
            if file.is_empty() {
                Pick::Inherit
            } else {
                Pick::Local { file }
            }
        }
        Pick::Cloud { connection } => {
            let connection = connection.trim().to_string();
            if connection.is_empty() || !seen_ids.contains(&connection) {
                Pick::Inherit
            } else {
                Pick::Cloud { connection }
            }
        }
    };

    // Advanced overrides: drop empty/dangling entries, clamp local engine params,
    // and deduplicate by (task, model) keeping the last entry.
    let mut advanced: Vec<Advanced> = Vec::new();
    for mut a in next.advanced.into_iter().rev() {
        let Some(model) = sanitize_model_ref(a.model, &seen_ids) else {
            continue;
        };
        a.model = model;
        match a.model {
            ModelRef::Local { .. } => {
                a.batch_size = a.batch_size.map(|b| b.clamp(32, 32_768));
                a.parallel_slots = a.parallel_slots.map(|n| n.clamp(1, 32));
                a.ctx_size = a.ctx_size.map(|c| c.clamp(512, 262_144));
            }
            ModelRef::Cloud { .. } => {
                a.batch_size = None;
                a.parallel_slots = None;
                a.ctx_size = None;
            }
        }
        if !a.is_empty()
            && !advanced
                .iter()
                .any(|kept| kept.task == a.task && kept.model == a.model)
        {
            advanced.push(a);
        }
    }
    advanced.reverse();
    next.advanced = advanced;

    let valid_segments: Vec<DaySegment> = next
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

    next.auto_summary_times = {
        let mut out: Vec<String> = Vec::new();
        for item in next.auto_summary_times {
            let t = item.trim().to_string();
            if chrono::NaiveTime::parse_from_str(&t, "%H:%M").is_ok() && !out.contains(&t) {
                out.push(t);
            }
            if out.len() >= MAX_AUTO_SUMMARY_TIMES {
                break;
            }
        }
        out
    };

    next.prompt_language = match next.prompt_language.trim() {
        "tw" => "tw".to_string(),
        "en" => "en".to_string(),
        "ja" => "ja".to_string(),
        "pt" => "pt".to_string(),
        "es" => "es".to_string(),
        _ => "zh".to_string(),
    };
    next.prompt_overrides.system_zh = next.prompt_overrides.system_zh.trim().to_string();
    next.prompt_overrides.system_en = next.prompt_overrides.system_en.trim().to_string();
    next.prompt_overrides.system_ja = next.prompt_overrides.system_ja.trim().to_string();
    next.prompt_overrides.system_pt = next.prompt_overrides.system_pt.trim().to_string();
    next.prompt_overrides.system_tw = next.prompt_overrides.system_tw.trim().to_string();
    next.prompt_overrides.system_es = next.prompt_overrides.system_es.trim().to_string();

    next
}

/// Validates a provider dialect key against the frontend allowlist; unknown values fall back to `custom`.
fn sanitize_dialect(raw: &str) -> String {
    match raw.trim() {
        v @ ("openai" | "deepseek" | "kimi" | "kimi-cn" | "openrouter" | "together" | "groq"
        | "custom") => v.to_string(),
        _ => "custom".to_string(),
    }
}

fn sanitize_model_ref(model: ModelRef, cloud_ids: &HashSet<String>) -> Option<ModelRef> {
    match model {
        ModelRef::Local { file } => {
            let file = file.trim().to_string();
            (!file.is_empty()).then_some(ModelRef::Local { file })
        }
        ModelRef::Cloud { connection } => {
            let connection = connection.trim().to_string();
            (!connection.is_empty() && cloud_ids.contains(&connection))
                .then_some(ModelRef::Cloud { connection })
        }
    }
}

/// Normalizes a hex color (`#rgb` or `#rrggbb`) to lowercase `#rrggbb`, clearing invalid inputs.
fn sanitize_hex_color(raw: &str) -> String {
    let Some(body) = raw.trim().strip_prefix('#') else {
        return String::new();
    };
    if !matches!(body.len(), 3 | 6) || !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return String::new();
    }
    if body.len() == 3 {
        let mut out = String::with_capacity(7);
        out.push('#');
        for c in body.chars() {
            let lc = c.to_ascii_lowercase();
            out.push(lc);
            out.push(lc);
        }
        out
    } else {
        format!("#{}", body.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::config::tests::{base, VALID_LANGS};
    use crate::ai::config::{ReasoningLevel, Task};

    fn seg(label: &str, start: u8, end: u8, color: &str) -> DaySegment {
        DaySegment {
            label: label.to_string(),
            start_hour: start,
            end_hour: end,
            color: color.to_string(),
        }
    }

    fn conn(id: &str, dialect: &str, endpoint: &str) -> CloudConnection {
        CloudConnection {
            id: id.into(),
            name: format!(" n-{id} "),
            dialect: dialect.into(),
            endpoint: endpoint.into(),
            model: " gpt-4o-mini ".into(),
            api_key: " sk-1 ".into(),
            context_length: None,
        }
    }

    #[test]
    fn sanitize_cloud_connections_trims_validates_dedups_and_caps() {
        let old = base();
        let mut cfg = base();
        cfg.cloud = vec![
            conn(" c1 ", " deepseek ", " https://api.deepseek.com/v1 "),
            conn("c1", "openai", "https://duplicate.example.com/v1"),
            conn("", "openai", "https://no-id.example.com/v1"),
            conn("c2", "openai", "   "),
            conn("c3", "unknown-vendor", "https://custom.example.com/v1"),
        ];
        let out = sanitize(cfg, &old).cloud;
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "c1");
        assert_eq!(out[0].name, "n- c1");
        assert_eq!(out[0].dialect, "deepseek");
        assert_eq!(out[0].endpoint, "https://api.deepseek.com/v1");
        assert_eq!(out[0].model, "gpt-4o-mini");
        assert_eq!(out[0].api_key, "sk-1");
        assert_eq!(out[1].id, "c3");
        assert_eq!(out[1].dialect, "custom");

        let mut many = base();
        many.cloud = (0..12)
            .map(|i| conn(&format!("c{i}"), "openai", &format!("https://e{i}")))
            .collect();
        assert_eq!(sanitize(many, &old).cloud.len(), MAX_CLOUD_CONNECTIONS);
    }

    #[test]
    fn sanitize_dialect_accepts_all_known_providers() {
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
            let mut cfg = base();
            cfg.cloud = vec![conn("c1", &format!(" {p} "), "https://api.example.com/v1")];
            assert_eq!(sanitize(cfg, &base()).cloud[0].dialect, p);
        }
    }

    #[test]
    fn sanitize_clears_dangling_cloud_picks_and_advanced_entries() {
        let mut cfg = base();
        cfg.cloud = vec![conn("c1", "openai", "https://api.openai.com/v1")];
        cfg.picks.daily = Some(ModelRef::Cloud {
            connection: "deleted".into(),
        });
        cfg.picks.weekly = Pick::Cloud {
            connection: "deleted".into(),
        };
        cfg.picks.chat = Some(ModelRef::Cloud {
            connection: " c1 ".into(),
        });
        cfg.advanced = vec![
            Advanced {
                task: Task::Daily,
                model: ModelRef::Cloud {
                    connection: "deleted".into(),
                },
                reasoning: Some(ReasoningLevel::High),
                ctx_size: None,
                parallel_slots: None,
                batch_size: None,
            },
            Advanced {
                task: Task::Chat,
                model: ModelRef::Cloud {
                    connection: " c1 ".into(),
                },
                reasoning: Some(ReasoningLevel::High),
                ctx_size: Some(16_384),
                parallel_slots: Some(2),
                batch_size: Some(256),
            },
        ];

        let out = sanitize(cfg, &base());
        assert_eq!(out.picks.daily, None);
        assert_eq!(out.picks.weekly, Pick::Inherit);
        assert_eq!(
            out.picks.chat,
            Some(ModelRef::Cloud {
                connection: "c1".into()
            })
        );
        // Cloud advanced entry drops local engine fields, keeps reasoning.
        assert_eq!(
            out.advanced,
            vec![Advanced {
                task: Task::Chat,
                model: ModelRef::Cloud {
                    connection: "c1".into()
                },
                reasoning: Some(ReasoningLevel::High),
                ctx_size: None,
                parallel_slots: None,
                batch_size: None,
            }]
        );
    }

    #[test]
    fn sanitize_advanced_clamps_engine_params_and_dedups() {
        let local = |f: &str| ModelRef::Local { file: f.into() };
        let mut cfg = base();
        cfg.picks.weekly = Pick::Local { file: "  ".into() };
        cfg.advanced = vec![
            Advanced {
                task: Task::Daily,
                model: local("a.gguf"),
                reasoning: Some(ReasoningLevel::Low),
                ctx_size: Some(100),
                parallel_slots: Some(0),
                batch_size: Some(1),
            },
            Advanced {
                task: Task::Daily,
                model: local(" a.gguf "),
                reasoning: Some(ReasoningLevel::Max),
                ctx_size: Some(u32::MAX),
                parallel_slots: Some(999),
                batch_size: Some(1_000_000),
            },
            Advanced {
                task: Task::Daily,
                model: local("b.gguf"),
                reasoning: None,
                ctx_size: None,
                parallel_slots: None,
                batch_size: None,
            },
            Advanced {
                task: Task::Chat,
                model: local("c.gguf"),
                reasoning: None,
                ctx_size: Some(100),
                parallel_slots: Some(0),
                batch_size: Some(1),
            },
        ];

        let out = sanitize(cfg, &base());
        assert_eq!(out.picks.weekly, Pick::Inherit);
        assert_eq!(
            out.advanced,
            vec![
                Advanced {
                    task: Task::Daily,
                    model: local("a.gguf"),
                    reasoning: Some(ReasoningLevel::Max),
                    ctx_size: Some(262_144),
                    parallel_slots: Some(32),
                    batch_size: Some(32_768),
                },
                Advanced {
                    task: Task::Chat,
                    model: local("c.gguf"),
                    reasoning: None,
                    ctx_size: Some(512),
                    parallel_slots: Some(1),
                    batch_size: Some(32),
                },
            ]
        );
    }

    #[test]
    fn auto_summary_times_semantics() {
        let v = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mk = |items: &[&str]| AiConfig {
            auto_summary_times: v(items),
            ..base()
        };
        let old = base();

        assert_eq!(
            sanitize(mk(&["23:00", "bad", " 12:00 ", "23:00", "27:00"]), &old).auto_summary_times,
            v(&["23:00", "12:00"])
        );
        assert_eq!(
            sanitize(
                mk(&["01:00", "02:00", "03:00", "04:00", "05:00", "06:00", "07:00"]),
                &old
            )
            .auto_summary_times
            .len(),
            6
        );
    }

    #[test]
    fn sanitize_prompt_language_whitelist_and_fallback() {
        for lang in VALID_LANGS {
            let mut next = base();
            next.prompt_language = format!("  {lang}  ");
            assert_eq!(sanitize(next, &base()).prompt_language, lang);
        }
        for bad in ["fr", "zh-CN", "EN", "", "  ", "日本語"] {
            let mut next = base();
            next.prompt_language = bad.to_string();
            assert_eq!(sanitize(next, &base()).prompt_language, "zh");
        }
    }

    #[test]
    fn sanitize_prompt_overrides_trim_edges_keep_inner_indent() {
        let text = "\n  line1\n    indented\n";
        let mut next = base();
        next.prompt_overrides.system_zh = text.to_string();
        next.prompt_overrides.system_en = format!("  {text}  ");
        next.prompt_overrides.system_ja = "   \n\t  ".to_string();
        next.prompt_overrides.system_pt = "single".to_string();
        next.prompt_overrides.system_tw = "\tkeep\tinner\ttab\t".to_string();
        next.prompt_overrides.system_es = format!("\n{text}\n");
        let out = sanitize(next, &base());
        assert_eq!(out.prompt_overrides.system_zh, "line1\n    indented");
        assert_eq!(out.prompt_overrides.system_en, "line1\n    indented");
        assert_eq!(out.prompt_overrides.system_ja, "");
        assert_eq!(out.prompt_overrides.system_pt, "single");
        assert_eq!(out.prompt_overrides.system_tw, "keep\tinner\ttab");
        assert_eq!(out.prompt_overrides.system_es, "line1\n    indented");
    }

    #[test]
    fn sanitize_segments_filters_invalid_and_falls_back_when_empty() {
        let mut next = base();
        next.segments = vec![
            seg("  Morning  ", 6, 9, " #ABC "),
            seg("reversed", 9, 6, ""),
            seg("zero", 12, 12, ""),
            seg("overflow", 20, 25, ""),
            seg("Evening", 18, 24, "#ff8800"),
        ];
        let out = sanitize(next, &base());
        assert_eq!(out.segments.len(), 2);
        assert_eq!(out.segments[0].label, "Morning");
        assert_eq!(out.segments[0].color, "#aabbcc");
        assert_eq!(out.segments[1].label, "Evening");
        assert_eq!(out.segments[1].color, "#ff8800");

        let mut old = base();
        old.segments = vec![seg("old", 0, 24, "")];
        let mut bad = base();
        bad.segments = vec![seg("bad", 10, 5, "")];
        let out = sanitize(bad, &old);
        assert_eq!(out.segments.len(), 1);
        assert_eq!(out.segments[0].label, "old");
    }

    #[test]
    fn hex_color_valid_forms_normalized() {
        assert_eq!(sanitize_hex_color("#abc"), "#aabbcc");
        assert_eq!(sanitize_hex_color("#AbC"), "#aabbcc");
        assert_eq!(sanitize_hex_color("#FF8800"), "#ff8800");
        assert_eq!(sanitize_hex_color("  #1a2b3c  "), "#1a2b3c");
        assert_eq!(sanitize_hex_color(""), "");
    }

    #[test]
    fn hex_color_invalid_forms_cleared() {
        for bad in [
            "abc123", "#abcd", "#ab", "#abcdefa", "#ggg", "#12345z", "   ",
        ] {
            assert_eq!(sanitize_hex_color(bad), "", "invalid hex {bad:?}");
        }
    }
}
