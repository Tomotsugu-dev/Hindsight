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

// 周报 system prompt（基于一周内日报全文做整周回顾）
const WEEKLY_ZH: &str = include_str!("../../resources/prompts/weekly_zh.md");
const WEEKLY_EN: &str = include_str!("../../resources/prompts/weekly_en.md");
const WEEKLY_JA: &str = include_str!("../../resources/prompts/weekly_ja.md");

/// 三语 (zh/en/ja) 之间挑一个 —— 把分散在多处的 match 收敛进单一 helper。
///
/// `prompt_language` 在 sanitize 时已被钳到 "zh" / "en" / "ja"，本函数对其它值
/// 兜底走 zh（与 sanitize 行为一致）。
fn pick_lang<'a>(lang: &str, zh: &'a str, en: &'a str, ja: &'a str) -> &'a str {
    match lang {
        "en" => en,
        "ja" => ja,
        _ => zh,
    }
}

/// 给定 settings.ai 选出当前生效的 system prompt 基础文本（不带 user_brief 后缀）。
///
/// 优先级：用户覆盖（非空）→ 内置默认。
fn pick_system_base(ai: &AiConfig) -> &str {
    let lang = ai.prompt_language.as_str();
    let ov = pick_lang(
        lang,
        &ai.prompt_overrides.system_zh,
        &ai.prompt_overrides.system_en,
        &ai.prompt_overrides.system_ja,
    )
    .trim();
    if !ov.is_empty() {
        ov
    } else {
        pick_lang(lang, PROMPT_ZH, PROMPT_EN, PROMPT_JA)
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
    /// step 1 落库的每张图描述，按 image_index 升序：(time_label, description)。
    /// time_label 是从截图文件名解析出的 `HH:MM` 本地时间，让 step 2 能按时间
    /// 顺序串故事；解析失败时是 "??:??"。
    pub image_descriptions: &'a [(String, String)],
}

/// step 1（单张图描述）的 system prompt——按当前语言选覆盖或内置默认。
///
/// 优先级：用户覆盖（`image_describe_overrides.system_<lang>` 非空）→ 内置默认。
/// 调试 tab 通过 `AiOverrides.image_describe_prompt` 临时覆盖；用户在 AI 设置以后
/// 也可暴露持久编辑入口。
pub fn build_image_describe_system_prompt(ai: &AiConfig) -> String {
    let lang = ai.prompt_language.as_str();
    let user_override = pick_lang(
        lang,
        &ai.image_describe_overrides.system_zh,
        &ai.image_describe_overrides.system_en,
        &ai.image_describe_overrides.system_ja,
    );
    let base = if !user_override.trim().is_empty() {
        user_override
    } else {
        pick_lang(
            lang,
            IMAGE_DESCRIBE_ZH,
            IMAGE_DESCRIBE_EN,
            IMAGE_DESCRIBE_JA,
        )
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
        let label = pick_lang(
            ai.prompt_language.as_str(),
            "关于用户：",
            "About the user: ",
            "ユーザーについて：",
        );
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
            "\n下面是该时段内 {} 张代表截图的逐一描述（已由 AI 看图后给出，按时间先后排列，每行行首 [HH:MM] 是截图本地时间）：\n",
            ctx.image_descriptions.len(),
        ));
        for (i, (t, d)) in ctx.image_descriptions.iter().enumerate() {
            out.push_str(&format!("{}. [{}] {}\n", i + 1, t, d.trim()));
        }
        out.push_str("\n请综合这些描述和应用统计写段总结，不要简单复述上面任意一条。");
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
            out.push_str(&format!("- {} ({} min · {})\n", name, minutes, category));
        }
    }
    if ctx.image_descriptions.is_empty() {
        out.push_str(
            "\n(No screenshots for this segment — write one short sentence based on the app stats above.)",
        );
    } else {
        out.push_str(&format!(
            "\nBelow are AI-generated descriptions of {} representative screenshots from this segment, in chronological order. The leading [HH:MM] is the screenshot's local time:\n",
            ctx.image_descriptions.len(),
        ));
        for (i, (t, d)) in ctx.image_descriptions.iter().enumerate() {
            out.push_str(&format!("{}. [{}] {}\n", i + 1, t, d.trim()));
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
            out.push_str(&format!("- {}（{} 分 · {}）\n", name, minutes, category));
        }
    }
    if ctx.image_descriptions.is_empty() {
        out.push_str(
            "\n（この時間帯のスクリーンショットがありません。上記のアプリ統計のみに基づいて一文で書いてください。）",
        );
    } else {
        out.push_str(&format!(
            "\n以下はこの時間帯の代表的なスクリーンショット {} 枚に対する AI による個別の記述です（時系列順、行頭の [HH:MM] は撮影時刻）：\n",
            ctx.image_descriptions.len(),
        ));
        for (i, (t, d)) in ctx.image_descriptions.iter().enumerate() {
            out.push_str(&format!("{}. [{}] {}\n", i + 1, t, d.trim()));
        }
        out.push_str(
            "\nこれらの記述とアプリ統計を組み合わせて時間帯の要約を書いてください。\
             どれか 1 行をそのまま繰り返さないでください。",
        );
    }
    out
}

// ───────────────────────────── 周报 ─────────────────────────────

/// 周报送 LLM 时的上下文（单步纯文本调用）。
///
/// 周报跟段总结的关键差异：上游已经是文字（每天的日报全文），不再过 vision，
/// 因此 step 1 完全跳过；本结构体只承载一周日维度的纯文本。
pub struct WeeklyContext<'a> {
    /// 周一（YYYY-MM-DD）
    pub week_start: &'a str,
    /// 周日（YYYY-MM-DD）
    pub week_end: &'a str,
    /// 按日期升序的每日条目：(date_str "YYYY-MM-DD", weekday_short, day_text)。
    /// `day_text` 是当日所有 daily 段总结按时段顺序拼好的全文（缺失 / skipped 段
    /// 不进入 day_text）；当天无任何可用日报时调用方应跳过这一项不传进来。
    /// `weekday_short` 是按当前 prompt 语言写的星期简写（"周一" / "Mon" / "月"）。
    pub days: &'a [(String, String, String)],
    /// 跨整周的 top apps：(display_name, minutes, category_id) 按 minutes 降序。
    /// 跟 daily 段总结的 `SegmentContext.top_apps` 同语义——给 LLM 一个一周内
    /// "用户主要在干什么"的弱信号，不必从日报全文里反推。
    pub top_apps: &'a [(String, u32, String)],
}

/// 周报 system prompt：选语言 → 走用户覆盖 / 内置默认 → 拼用户简介。
///
/// 复用 daily 段总结那套 [`AiConfig::user_brief`] / [`PromptOverrides`] 字段——
/// 用户在 AI 设置里写过的"关于我"既影响日报也影响周报，不必再设一份。
/// MVP 不暴露 weekly 专属覆盖；未来需要时再扩 `weekly_overrides` 字段。
pub fn build_weekly_system_prompt(ai: &AiConfig) -> String {
    let lang = ai.prompt_language.as_str();
    let base = pick_lang(lang, WEEKLY_ZH, WEEKLY_EN, WEEKLY_JA);
    let mut out = String::from(base.trim_end());
    let brief = ai.user_brief.trim();
    if !brief.is_empty() {
        let label = pick_lang(lang, "关于用户：", "About the user: ", "ユーザーについて：");
        out.push_str("\n\n");
        out.push_str(label);
        out.push_str(brief);
    }
    out
}

/// 周报 user prompt：把一周内每天的日报全文按日期顺序拼起来。
pub fn build_weekly_user_prompt(ai: &AiConfig, ctx: &WeeklyContext) -> String {
    match ai.prompt_language.as_str() {
        "en" => build_weekly_user_prompt_en(ctx),
        "ja" => build_weekly_user_prompt_ja(ctx),
        _ => build_weekly_user_prompt_zh(ctx),
    }
}

fn build_weekly_user_prompt_zh(ctx: &WeeklyContext) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "本周范围：{} – {}\n\n",
        ctx.week_start, ctx.week_end
    ));
    // 完全没数据：让 LLM 直说无法生成。后端有兜底分支基本走不到这里，
    // 但留着保险——避免 LLM 在零数据上幻觉。
    if ctx.days.is_empty() && ctx.top_apps.is_empty() {
        out.push_str(
            "（这一周还没有任何日报数据，也没有应用使用记录，请基于这一点说明无法生成周报。）",
        );
        return out;
    }
    if !ctx.top_apps.is_empty() {
        out.push_str("本周使用最多的应用：\n");
        for (name, minutes, category) in ctx.top_apps.iter().take(8) {
            out.push_str(&format!("- {}（{} 分钟 · {}）\n", name, minutes, category));
        }
        out.push('\n');
    }
    // 仅应用统计 / 无日报：让 LLM 在应用统计上做简化回顾，不要凭空补具体动作或剧情。
    // 跟 weekly_*.md 的 marker 说明配合，保证两个分支的语气一致。
    if ctx.days.is_empty() {
        out.push_str(
            "本周没有任何日报，仅根据上面的应用统计写一段简短的整周回顾——只描述大致用了哪些应用、各占多少时间，不要凭空补具体动作、剧情或情境。",
        );
        return out;
    }
    out.push_str(&format!(
        "下面是本周 {} 天的日报全文（按日期顺序排列；未列出的日期当天没有数据）：\n\n",
        ctx.days.len(),
    ));
    for (date, weekday, body) in ctx.days.iter() {
        out.push_str(&format!("【{} {}】\n", date, weekday));
        out.push_str(body.trim());
        out.push_str("\n\n");
    }
    out.push_str("请综合这一周的应用统计和日报内容写一段整周回顾，不要按天复述。");
    out
}

fn build_weekly_user_prompt_en(ctx: &WeeklyContext) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Week range: {} – {}\n\n",
        ctx.week_start, ctx.week_end
    ));
    if ctx.days.is_empty() && ctx.top_apps.is_empty() {
        out.push_str(
            "(There is no daily report data for this week, nor any app usage records. Please state this and stop.)",
        );
        return out;
    }
    if !ctx.top_apps.is_empty() {
        out.push_str("Top apps used this week:\n");
        for (name, minutes, category) in ctx.top_apps.iter().take(8) {
            out.push_str(&format!("- {} ({} min · {})\n", name, minutes, category));
        }
        out.push('\n');
    }
    if ctx.days.is_empty() {
        out.push_str(
            "No daily reports for this week. Write a short weekly review based only on the app stats above — only describe roughly which apps were used and how the time was split, do not invent specific actions, storylines, or scenes.",
        );
        return out;
    }
    out.push_str(&format!(
        "Below are the full daily reports for {} days of this week, in chronological order. Days not listed had no data.\n\n",
        ctx.days.len(),
    ));
    for (date, weekday, body) in ctx.days.iter() {
        out.push_str(&format!("[{} {}]\n", date, weekday));
        out.push_str(body.trim());
        out.push_str("\n\n");
    }
    out.push_str(
        "Synthesize the week into a single review paragraph, combining the app stats and daily reports. Do not retell day by day.",
    );
    out
}

fn build_weekly_user_prompt_ja(ctx: &WeeklyContext) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "今週の範囲：{} – {}\n\n",
        ctx.week_start, ctx.week_end
    ));
    if ctx.days.is_empty() && ctx.top_apps.is_empty() {
        out.push_str(
            "（今週は日報データもアプリ使用記録も一切ありません。その旨を述べて終了してください。）",
        );
        return out;
    }
    if !ctx.top_apps.is_empty() {
        out.push_str("今週最も使用されたアプリ：\n");
        for (name, minutes, category) in ctx.top_apps.iter().take(8) {
            out.push_str(&format!("- {}（{} 分 · {}）\n", name, minutes, category));
        }
        out.push('\n');
    }
    if ctx.days.is_empty() {
        out.push_str(
            "今週は日報が一切ありません。上記のアプリ統計のみに基づき、一週間の短いレビューを書いてください——どのアプリをどれくらいの時間使ったかの大まかな描写のみ。具体的な動作、ストーリー、シーンを捏造しないでください。",
        );
        return out;
    }
    out.push_str(&format!(
        "以下は今週 {} 日分の日報本文です（日付順、記載のない日はデータなし）：\n\n",
        ctx.days.len(),
    ));
    for (date, weekday, body) in ctx.days.iter() {
        out.push_str(&format!("【{} {}】\n", date, weekday));
        out.push_str(body.trim());
        out.push_str("\n\n");
    }
    out.push_str("アプリ統計と日報を組み合わせ、一週間のレビュー段落を 1 つ書いてください。日ごとに繰り返さないでください。");
    out
}

/// 给定本地日期返回当前 prompt 语言对应的星期简写——周报 user prompt 用。
pub fn weekday_short(lang: &str, weekday: chrono::Weekday) -> &'static str {
    use chrono::Weekday::*;
    match lang {
        "en" => match weekday {
            Mon => "Mon",
            Tue => "Tue",
            Wed => "Wed",
            Thu => "Thu",
            Fri => "Fri",
            Sat => "Sat",
            Sun => "Sun",
        },
        "ja" => match weekday {
            Mon => "月",
            Tue => "火",
            Wed => "水",
            Thu => "木",
            Fri => "金",
            Sat => "土",
            Sun => "日",
        },
        _ => match weekday {
            Mon => "周一",
            Tue => "周二",
            Wed => "周三",
            Thu => "周四",
            Fri => "周五",
            Sat => "周六",
            Sun => "周日",
        },
    }
}
