//! AI 总结流程的进度事件 payload。
//!
//! 前端 `listen("ai://summary-progress", …)` 拿进度，按 `phase` 字段分发渲染。
//! 该结构体 / 常量从原 `summary.rs` 拎出，与 [`crate::commands::ai::PROGRESS_EVENT`]
//! 平级，统一在 `ai://` 命名空间。

use serde::Serialize;

/// 前端 listen 这个事件名拿进度。
pub const SUMMARY_PROGRESS_EVENT: &str = "ai://summary-progress";

/// 进度事件 payload。前端按 `phase` 分发渲染。
///
/// phase 取值：
/// - `engine_starting`：引擎冷启动中（首次加载模型 30-90s）
/// - `segment_started`：段进入 step 1（逐图描述）；imagesTotal 给图数
/// - `image_described`：单张图描述完成；image_index / image_path / image_description 一起带过来，
///   前端调试 tab 实时往面板里塞条目，不必等整段完成
/// - `segment_done`：段进入完成态（含 ok / skipped / error）；content 是段总结
/// - `all_done` / `cancelled` / `error`：整轮收尾
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryProgress {
    /// "daily" / "debug"——前端两个 tab 各自只 listen 自己 source 的事件，
    /// 避免一个 tab 跑时另一个 tab 跟着刷数据。
    pub source: String,
    pub date: String,
    pub phase: &'static str,
    pub segment_idx: Option<u32>,
    pub total_segments: u32,
    /// 段开跑时给前端 "12 张图待分析" 的提示
    pub images_total: Option<u32>,
    /// image_described 时该图在段内的下标（0-based）
    pub image_index: Option<u32>,
    /// image_described 时附该图绝对路径（前端可以用来显示缩略图）
    pub image_path: Option<String>,
    /// image_described 时附该图的描述文本
    pub image_description: Option<String>,
    /// image_described 时附该图调用 LLM 的耗时（毫秒）
    pub latency_ms: Option<u64>,
    /// image_described 时附 prompt token 数（llama-server 不返时为 None）
    pub prompt_tokens: Option<u32>,
    /// image_described 时附 completion token 数
    pub completion_tokens: Option<u32>,
    /// segment_done 时附该段总结，前端立刻渲染该段不等其它
    pub content: Option<String>,
    /// segment_done 时也会带上落库行的 status（ok / skipped_no_screenshots / error）
    /// 让前端知道是不是该段失败了
    pub status: Option<&'static str>,
    /// error 段的可读错误；error phase 也用这个携带顶层错误描述
    pub message: Option<String>,
}

impl SummaryProgress {
    /// 各 phase 字段大多都是 None，统一兜底构造器减少重复代码。
    pub(crate) fn base(
        source: String,
        date: String,
        phase: &'static str,
        total_segments: u32,
    ) -> Self {
        Self {
            source,
            date,
            phase,
            segment_idx: None,
            total_segments,
            images_total: None,
            image_index: None,
            image_path: None,
            image_description: None,
            latency_ms: None,
            prompt_tokens: None,
            completion_tokens: None,
            content: None,
            status: None,
            message: None,
        }
    }
}
