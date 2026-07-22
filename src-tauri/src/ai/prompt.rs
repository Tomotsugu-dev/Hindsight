//! AI 总结的 prompt 模板。
//!
//! - [`build_system_prompt`] 选语言 → 走用户覆盖或内置默认 → 拼用户简介
//! - [`build_user_prompt`] 拼当前段的元数据（label / 时段 / top apps）
//!   和活动记录时间线（逐小时的应用时长 + 窗口标题样例）
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
const PROMPT_PT: &str = include_str!("../../resources/prompts/system_pt.md");
const PROMPT_TW: &str = include_str!("../../resources/prompts/system_tw.md");

// 周报 system prompt（基于一周内日报全文做整周回顾）
const WEEKLY_ZH: &str = include_str!("../../resources/prompts/weekly_zh.md");
const WEEKLY_EN: &str = include_str!("../../resources/prompts/weekly_en.md");
const WEEKLY_JA: &str = include_str!("../../resources/prompts/weekly_ja.md");
const WEEKLY_PT: &str = include_str!("../../resources/prompts/weekly_pt.md");
const WEEKLY_TW: &str = include_str!("../../resources/prompts/weekly_tw.md");

/// 五语 (zh/tw/en/ja/pt) 之间挑一个 —— 把分散在多处的 match 收敛进单一 helper。
///
/// `prompt_language` 在 sanitize 时已被钳到 "zh" / "tw" / "en" / "ja" / "pt"，本函数对
/// 其它值兜底走 zh（与 sanitize 行为一致）。
///
/// 注：system / image-describe / weekly 这三种 system prompt 的内置默认走本函数选
/// 专属文本（决定模型输出语言）。而 user prompt 的「脚手架」文案（"Segment:" /
/// "Top apps:" 等结构性框架）pt 复用英文、tw 复用简体——这些只是喂给模型的结构提示，
/// 模型在对应 system prompt 主导下仍输出目标语言，没必要再翻一份。
fn pick_lang<'a>(
    lang: &str,
    zh: &'a str,
    tw: &'a str,
    en: &'a str,
    ja: &'a str,
    pt: &'a str,
) -> &'a str {
    match lang {
        "tw" => tw,
        "en" => en,
        "ja" => ja,
        "pt" => pt,
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
        &ai.prompt_overrides.system_tw,
        &ai.prompt_overrides.system_en,
        &ai.prompt_overrides.system_ja,
        &ai.prompt_overrides.system_pt,
    )
    .trim();
    if !ov.is_empty() {
        ov
    } else {
        pick_lang(lang, PROMPT_ZH, PROMPT_TW, PROMPT_EN, PROMPT_JA, PROMPT_PT)
    }
}

/// 单段送 AI 时的上下文。
///
/// 单步纯文本生成：材料 = 活动记录时间线（逐小时的应用时长 + 窗口标题样例）
/// + top apps 统计，不再有截图描述。
///
/// `top_apps` 是该段内按使用时长排序的应用列表，前若干项；
/// `timeline` 是从 activities 合成的逐小时清单：(time_label "HH:00-HH:00",
/// "应用 时长（窗口标题样例）· …")，按小时升序。
pub struct SegmentContext<'a> {
    pub label: &'a str,
    pub start_hour: u8,
    pub end_hour: u8,
    /// (display_name, minutes, category_id) 三元组，按 minutes 降序
    pub top_apps: &'a [(String, u32, String)],
    /// 逐小时活动时间线：(time_label, 该小时的应用/时长/标题清单)
    pub timeline: &'a [(String, String)],
    /// 云端截图洞察行(可为空):(HH:MM, "应用 | 一句话 | 实体")。
    /// 有值时拼进 user prompt 作为"具体在做什么"的补充材料。
    pub insights: &'a [(String, String)],
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
            "關於使用者：",
            "About the user: ",
            "ユーザーについて：",
            "Sobre o usuário: ",
        );
        out.push_str("\n\n");
        out.push_str(label);
        out.push_str(brief);
    }
    out
}

/// user prompt：当前段的标签 + 时段范围 + top apps 摘要 + 逐小时活动时间线。
///
/// 文案按 system prompt 选的语言走——保证 user 跟 system 在同一语种里。
pub fn build_user_prompt(ai: &AiConfig, ctx: &SegmentContext) -> String {
    match ai.prompt_language.as_str() {
        // pt 复用英文脚手架（葡语 system prompt 主导输出语言）
        "en" | "pt" => build_user_prompt_en(ctx),
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
    if ctx.timeline.is_empty() {
        out.push_str(
            "\n（这段时间没有逐小时的活动记录。仅基于上面的应用统计写 2-4 句概括：用了哪些应用、大致时间分配，不要虚构任何具体操作或内容。）",
        );
    } else {
        out.push_str(&format!(
            "\n下面是该时段的活动记录时间线，共 {} 个小时段（每行格式：应用 累计时长（窗口标题样例）· …，按时长降序）：\n",
            ctx.timeline.len(),
        ));
        for (i, (t, d)) in ctx.timeline.iter().enumerate() {
            out.push_str(&format!("{}. [{}] {}\n", i + 1, t, d.trim()));
        }
        out.push_str(
            "\n请基于这份时间线按时间顺序写这个时段的活动日志。窗口标题是了解具体在做什么的主要线索（文件名 / 网页标题 / 视频标题等），可以据此描述具体活动，但不要推测标题之外的细节。同一活动全篇只写一次（相似条目合并），遵守系统规则的段落与句数上限；严禁逐条复述，严禁把上面的材料（时段/应用列表/时间线行）原样抄进输出——直接从日志正文第一句开始写。",
        );
    }
    if !ctx.insights.is_empty() {
        out.push_str(&format!(
            "\n屏幕画面洞察（云端视觉分析逐帧生成，按时刻排列，共 {} 条）：\n",
            ctx.insights.len()
        ));
        for (t, line) in ctx.insights {
            out.push_str(&format!("- [{t}] {line}\n"));
        }
        out.push_str("这些洞察描述了画面上实际发生的事，比窗口标题更具体，优先用它们充实活动细节；与时间线冲突时以时间线的时长为准。\n");
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
    if ctx.timeline.is_empty() {
        out.push_str(
            "\n(No per-hour activity records for this segment — write 2-4 short sentences based only on the app stats above; do not invent specific actions or content.)",
        );
    } else {
        out.push_str(&format!(
            "\nBelow is this segment's activity timeline, {} hourly lines (each line: app total-time (window-title samples) · …, sorted by duration):\n",
            ctx.timeline.len(),
        ));
        for (i, (t, d)) in ctx.timeline.iter().enumerate() {
            out.push_str(&format!("{}. [{}] {}\n", i + 1, t, d.trim()));
        }
        out.push_str(
            "\nWrite this segment's journal entry from the timeline, in time order. Window titles are the primary clue to what was actually done (file names / page titles / video titles); do not speculate beyond them. Each activity appears once (merge similar lines), within the paragraph and sentence caps from the system rules. Never rewrite the lines one-by-one, and never copy the material above (segment line / app list / timeline) into the output — start directly with the first sentence of the journal.",
        );
    }
    if !ctx.insights.is_empty() {
        out.push_str(&format!(
            "\nScreen insights (frame-level cloud vision analysis, in time order, {} lines):\n",
            ctx.insights.len()
        ));
        for (t, line) in ctx.insights {
            out.push_str(&format!("- [{t}] {line}\n"));
        }
        out.push_str("These insights describe what was actually on screen — more specific than window titles; prefer them for concrete detail. When they conflict with the timeline, trust the timeline's durations.\n");
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
    if ctx.timeline.is_empty() {
        out.push_str(
            "\n（この時間帯には時間別の活動記録がありません。上記のアプリ統計のみに基づいて 2〜4 文で書いてください。具体的な操作や内容を捏造しないでください。）",
        );
    } else {
        out.push_str(&format!(
            "\n以下はこの時間帯の活動記録タイムラインです（全 {} 行、各行：アプリ 累計時間（ウィンドウタイトル例）· …、時間降順）：\n",
            ctx.timeline.len(),
        ));
        for (i, (t, d)) in ctx.timeline.iter().enumerate() {
            out.push_str(&format!("{}. [{}] {}\n", i + 1, t, d.trim()));
        }
        out.push_str(
            "\nこのタイムラインに基づき、時系列順にこの時間帯の活動ログを書いてください。ウィンドウタイトル（ファイル名・ページタイトル・動画タイトル）が主要な手がかりです。タイトル以外の細部を推測しないでください。同じ活動は全体で一度だけ（類似行は統合）、システム規則の段落数・文数上限を厳守。1 行ずつ書き写さず、上の材料（時間帯行・アプリ一覧・タイムライン行）を出力にコピーしないでください——ログ本文の最初の一文から直接書き始めてください。",
        );
    }
    if !ctx.insights.is_empty() {
        out.push_str(&format!(
            "\n画面インサイト（クラウド視覚分析によるフレーム単位の記述、時刻順、全 {} 行）：\n",
            ctx.insights.len()
        ));
        for (t, line) in ctx.insights {
            out.push_str(&format!("- [{t}] {line}\n"));
        }
        out.push_str("これらは画面上で実際に起きていたことの記述で、ウィンドウタイトルより具体的です。活動の詳細にはこちらを優先し、タイムラインと矛盾する場合は時間配分はタイムラインを信頼してください。\n");
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
    let base = pick_lang(lang, WEEKLY_ZH, WEEKLY_TW, WEEKLY_EN, WEEKLY_JA, WEEKLY_PT);
    let mut out = String::from(base.trim_end());
    let brief = ai.user_brief.trim();
    if !brief.is_empty() {
        let label = pick_lang(
            lang,
            "关于用户：",
            "關於使用者：",
            "About the user: ",
            "ユーザーについて：",
            "Sobre o usuário: ",
        );
        out.push_str("\n\n");
        out.push_str(label);
        out.push_str(brief);
    }
    out
}

/// 周报 user prompt：把一周内每天的日报全文按日期顺序拼起来。
pub fn build_weekly_user_prompt(ai: &AiConfig, ctx: &WeeklyContext) -> String {
    match ai.prompt_language.as_str() {
        // pt 复用英文脚手架（葡语 weekly system prompt 主导输出语言）
        "en" | "pt" => build_weekly_user_prompt_en(ctx),
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
        "pt" => match weekday {
            Mon => "Seg",
            Tue => "Ter",
            Wed => "Qua",
            Thu => "Qui",
            Fri => "Sex",
            Sat => "Sáb",
            Sun => "Dom",
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
