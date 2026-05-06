//! AI 总结的 prompt 模板（Phase 1B-γ）。
//!
//! - [`build_system_prompt`] 选语言 → 走用户覆盖或内置默认 → 拼用户简介
//! - [`build_user_prompt`] 拼当前段的元数据（label / 时段 / top apps），
//!   截图本体由 [`crate::ai::image::to_data_uri`] 转成 data URI 拼进 messages
//!
//! ## 数据 vs 代码
//!
//! 三套语言的内置 system prompt 是 *数据*，存在 [`src-tauri/resources/prompts/`]。
//! 通过 `include_str!` 编译时嵌入二进制——零运行时开销，发布产物自带，无需额外
//! 部署步骤。改 prompt 内容只动 `.md` 文件，不动 `.rs` 代码。
//!
//! 前端通过 vite `?raw` 直接引用 `src-tauri/resources/prompts/` 下的同一份 `.md`
//! 文件，没有副本——单一数据源，前后端共用。
//!
//! 用户在 [AISettings → 提示词] Section 里写的覆盖会落到
//! `settings.ai.prompt_overrides.system_<lang>`；非空就走覆盖，空就走内置默认。

use crate::ai::config::AiConfig;

// 路径相对当前 .rs 文件：`src-tauri/src/ai/prompt.rs` → `../..` 回到
// `src-tauri/` → `resources/prompts/...`。完全在 src-tauri crate 边界内。
const PROMPT_ZH: &str = include_str!("../../resources/prompts/system_zh.md");
const PROMPT_EN: &str = include_str!("../../resources/prompts/system_en.md");
const PROMPT_JA: &str = include_str!("../../resources/prompts/system_ja.md");

// 两步生成 step 1：单张截图的描述 prompt（vision 调用）
const IMAGE_DESCRIBE_ZH: &str = include_str!("../../resources/prompts/image_describe_zh.md");
const IMAGE_DESCRIBE_EN: &str = include_str!("../../resources/prompts/image_describe_en.md");
const IMAGE_DESCRIBE_JA: &str = include_str!("../../resources/prompts/image_describe_ja.md");

/// 给定 settings.ai 选出当前生效的 system prompt 基础文本（不带 user_brief 后缀）。
///
/// 优先级：用户覆盖（非空）→ 内置默认。`prompt_language` 在 sanitize 时已被钳到
/// "zh" / "en" / "ja"，这里直接 match。
fn pick_system_base(ai: &AiConfig) -> &str {
    match ai.prompt_language.as_str() {
        "en" => {
            let ov = ai.prompt_overrides.system_en.trim();
            if !ov.is_empty() {
                ov
            } else {
                PROMPT_EN
            }
        }
        "ja" => {
            let ov = ai.prompt_overrides.system_ja.trim();
            if !ov.is_empty() {
                ov
            } else {
                PROMPT_JA
            }
        }
        // "zh" 兜底——sanitize 已保证非中英日的值都被正规化到 zh
        _ => {
            let ov = ai.prompt_overrides.system_zh.trim();
            if !ov.is_empty() {
                ov
            } else {
                PROMPT_ZH
            }
        }
    }
}

/// 单段送 AI 时的上下文（step 2 段总结用）。
///
/// 两步生成下，step 2 是纯文本调用——把 step 1 拿到的每张图描述拼进 user prompt，
/// 不再传图片本身（节省 token + 让 LLM 专注做"汇总"而不是再看图）。
///
/// `top_apps` 是该段内按使用时长排序的应用列表，前若干项；
/// `image_descriptions` 是 step 1 输出的每张图描述（按抽帧顺序）。
pub struct SegmentContext<'a> {
    pub label: &'a str,
    pub start_hour: u8,
    pub end_hour: u8,
    /// (display_name, minutes, category_id) 三元组，按 minutes 降序
    pub top_apps: &'a [(String, u32, String)],
    /// step 1 落库的每张图描述，按 image_index 升序
    pub image_descriptions: &'a [String],
}

/// step 1（单张图描述）的 system prompt——按当前语言选覆盖或内置默认。
///
/// 优先级：用户覆盖（`image_describe_overrides.system_<lang>` 非空）→ 内置默认。
/// 调试 tab 通过 `AiOverrides.image_describe_prompt` 临时覆盖；用户在 AI 设置以后
/// 也可暴露持久编辑入口。
pub fn build_image_describe_system_prompt(ai: &AiConfig) -> String {
    let lang = ai.prompt_language.as_str();
    let user_override = match lang {
        "en" => &ai.image_describe_overrides.system_en,
        "ja" => &ai.image_describe_overrides.system_ja,
        _ => &ai.image_describe_overrides.system_zh,
    };
    let base = if !user_override.trim().is_empty() {
        user_override.as_str()
    } else {
        match lang {
            "en" => IMAGE_DESCRIBE_EN,
            "ja" => IMAGE_DESCRIBE_JA,
            _ => IMAGE_DESCRIBE_ZH,
        }
    };
    base.trim_end().to_string()
}

/// step 1 的 user prompt——只包含「这张截图来自的应用」上下文，让 LLM 看图前
/// 就知道是哪个应用，描述更准（尤其对冷门 / 内部 / 自定义皮肤的应用）。
///
/// `category_name` 为 None 时省略括号部分。
/// `app_display` 是 [`ScreenshotMeta::app_display`] —— 优先 app_groups.display_name，
/// 落回 process_name。
///
/// 直接吃 `lang: &str`（值为 "en"/"ja"/其它=zh）而不吃 `&AiConfig`——并发循环里每个
/// 闭包要 owned 数据，少一层 borrow 就少一道 lifetime。
pub fn build_image_describe_user_prompt(
    lang: &str,
    app_display: &str,
    category_name: Option<&str>,
) -> String {
    match lang {
        "en" => match category_name {
            Some(c) => format!("Screenshot is from {} (category: {})", app_display, c),
            None => format!("Screenshot is from {}", app_display),
        },
        "ja" => match category_name {
            Some(c) => format!(
                "このスクリーンショットのアプリ：{}（分類：{}）",
                app_display, c
            ),
            None => format!("このスクリーンショットのアプリ：{}", app_display),
        },
        _ => match category_name {
            Some(c) => format!("这张截图来自 {}（分类：{}）", app_display, c),
            None => format!("这张截图来自 {}", app_display),
        },
    }
}

/// 组装当次调用的完整 system prompt：
///   选语言 + 走覆盖 / 默认 + 拼用户简介。
///
/// 段范围 / top apps 由 user prompt 给——system 保持稳定，让模型在多段调用之间
/// 能复用 KV cache（llama.cpp 检测到 system 没变会跳过重算）。
pub fn build_system_prompt(ai: &AiConfig) -> String {
    let mut out = String::from(pick_system_base(ai).trim_end());
    let brief = ai.user_brief.trim();
    if !brief.is_empty() {
        let label = match ai.prompt_language.as_str() {
            "en" => "About the user: ",
            "ja" => "ユーザーについて：",
            _ => "关于用户：",
        };
        out.push_str("\n\n");
        out.push_str(label);
        out.push_str(brief);
    }
    out
}

/// user prompt：给当前段的标签 + 时段范围 + top apps 摘要。截图作为 messages 里的
/// `image_url` 项独立挂，不进这段 text。
///
/// 文案按 system prompt 选的语言走——保证 user 跟 system 在同一语种里。
pub fn build_user_prompt(ai: &AiConfig, ctx: &SegmentContext) -> String {
    match ai.prompt_language.as_str() {
        "en" => build_user_prompt_en(ctx),
        "ja" => build_user_prompt_ja(ctx),
        _ => build_user_prompt_zh(ctx),
    }
}

fn build_user_prompt_zh(ctx: &SegmentContext) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "时段：{}（{:02}:00 – {:02}:00）\n",
        ctx.label, ctx.start_hour, ctx.end_hour,
    ));
    if !ctx.top_apps.is_empty() {
        out.push_str("使用最多的应用：\n");
        for (name, minutes, category) in ctx.top_apps.iter().take(8) {
            out.push_str(&format!("- {}（{} 分钟 · {}）\n", name, minutes, category));
        }
    }
    if ctx.image_descriptions.is_empty() {
        out.push_str("\n（这段时间没有截图，仅基于上面的应用统计写一句话。）");
    } else {
        out.push_str(&format!(
            "\n下面是该时段内 {} 张代表截图的逐一描述（已由 AI 看图后给出）：\n",
            ctx.image_descriptions.len(),
        ));
        for (i, d) in ctx.image_descriptions.iter().enumerate() {
            out.push_str(&format!("{}. {}\n", i + 1, d.trim()));
        }
        out.push_str(
            "\n请综合这些描述和应用统计写段总结，不要简单复述上面任意一条。",
        );
    }
    out
}

fn build_user_prompt_en(ctx: &SegmentContext) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Segment: {} ({:02}:00 – {:02}:00)\n",
        ctx.label, ctx.start_hour, ctx.end_hour,
    ));
    if !ctx.top_apps.is_empty() {
        out.push_str("Top apps used:\n");
        for (name, minutes, category) in ctx.top_apps.iter().take(8) {
            out.push_str(&format!(
                "- {} ({} min · {})\n",
                name, minutes, category
            ));
        }
    }
    if ctx.image_descriptions.is_empty() {
        out.push_str(
            "\n(No screenshots for this segment — write one short sentence based on the app stats above.)",
        );
    } else {
        out.push_str(&format!(
            "\nBelow are AI-generated descriptions of {} representative screenshots from this segment, in order:\n",
            ctx.image_descriptions.len(),
        ));
        for (i, d) in ctx.image_descriptions.iter().enumerate() {
            out.push_str(&format!("{}. {}\n", i + 1, d.trim()));
        }
        out.push_str(
            "\nWrite a brief segment summary combining these descriptions and the app stats. \
             Don't just restate any single line.",
        );
    }
    out
}

fn build_user_prompt_ja(ctx: &SegmentContext) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "時間帯：{}（{:02}:00 – {:02}:00）\n",
        ctx.label, ctx.start_hour, ctx.end_hour,
    ));
    if !ctx.top_apps.is_empty() {
        out.push_str("最も使用されたアプリ：\n");
        for (name, minutes, category) in ctx.top_apps.iter().take(8) {
            out.push_str(&format!(
                "- {}（{} 分 · {}）\n",
                name, minutes, category
            ));
        }
    }
    if ctx.image_descriptions.is_empty() {
        out.push_str(
            "\n（この時間帯のスクリーンショットがありません。上記のアプリ統計のみに基づいて一文で書いてください。）",
        );
    } else {
        out.push_str(&format!(
            "\n以下はこの時間帯の代表的なスクリーンショット {} 枚に対する AI による個別の記述です：\n",
            ctx.image_descriptions.len(),
        ));
        for (i, d) in ctx.image_descriptions.iter().enumerate() {
            out.push_str(&format!("{}. {}\n", i + 1, d.trim()));
        }
        out.push_str(
            "\nこれらの記述とアプリ統計を組み合わせて時間帯の要約を書いてください。\
             どれか 1 行をそのまま繰り返さないでください。",
        );
    }
    out
}
