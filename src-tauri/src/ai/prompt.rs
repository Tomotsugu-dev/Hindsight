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

/// 单段送 AI 时的上下文。
///
/// `top_apps` 是该段内按使用时长排序的应用列表，前若干项；用来给模型一个
/// "用户在干什么"的锚点，避免模型只看截图猜半天。
pub struct SegmentContext<'a> {
    pub label: &'a str,
    pub start_hour: u8,
    pub end_hour: u8,
    /// (display_name, minutes, category_id) 三元组，按 minutes 降序
    pub top_apps: &'a [(String, u32, String)],
    /// 该段实际送给模型的截图张数（抽帧后）
    pub image_count: usize,
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
    if ctx.image_count == 0 {
        out.push_str("\n（这段时间没有截图，仅基于上面的应用统计写一句话。）");
    } else {
        out.push_str(&format!(
            "\n下面是该时段内 {} 张代表截图，请综合截图内容和应用统计给出总结。",
            ctx.image_count,
        ));
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
    if ctx.image_count == 0 {
        out.push_str(
            "\n(No screenshots for this segment — write one short sentence based on the app stats above.)",
        );
    } else {
        out.push_str(&format!(
            "\nBelow are {} representative screenshots from this segment. Combine them with the app stats and write a brief summary.",
            ctx.image_count,
        ));
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
    if ctx.image_count == 0 {
        out.push_str(
            "\n（この時間帯のスクリーンショットがありません。上記のアプリ統計のみに基づいて一文で書いてください。）",
        );
    } else {
        out.push_str(&format!(
            "\n以下はこの時間帯の代表的なスクリーンショット {} 枚です。スクリーンショットの内容とアプリ統計を組み合わせて要約してください。",
            ctx.image_count,
        ));
    }
    out
}
