//! 日报、周报跑起来时和前端的来往：往外报进度，往里收停止。
//!
//! 前端 `listen("ai://summary-progress", …)` 拿进度，按 `phase` 字段分发渲染。
//! 事件名与 [`crate::commands::ai_binary::ENGINE_DOWNLOAD_PROGRESS_EVENT`]
//! 平级，统一在 `ai://` 命名空间。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::{Error, Result};

/// 前端 listen 这个事件名拿进度。
pub const SUMMARY_PROGRESS_EVENT: &str = "ai://summary-progress";

/// 进度事件 payload。前端按 `phase` 分发渲染。
///
/// phase 取值：
/// - `ocr_engine_starting`：总结前的 OCR 清积压阶段开始（可能含 OCR 模型加载）；
///   `images_total` = 待识别帧总数。零积压时整个 OCR 阶段不发任何事件
/// - `ocr_running`：OCR 清积压进行中；`images_total` = 总数、`image_index` = 已处理数,
///   前端渲染 "识别截图文字 x/y…"。结束后无专属完结事件——下一个 phase
///   (`engine_starting`)到达即表示 OCR 阶段结束
/// - `engine_starting`：引擎冷启动中（首次加载模型 30-90s）
/// - `dedup_running`：MobileNet embedding 余弦去重中（首次跑全段 5-10s/千张）；
///   `images_total` 是去重前段内总图数，给前端展示"段 X：N 张相似度去重中…"
/// - `segment_started`：段进入 step 1（逐图描述）；imagesTotal 给去重 + 抽帧后实际送 LLM 的图数
/// - `step1_done`：daily Phase 1 单段图描述全部跑完，但**还没**做段总结；前端**不**应据此
///   把段 row 切到"已生成"——只有 `segment_done` + status=ok 才算真完成。其它 status 等同段失败。
/// - `summarizing`：step 1 跑完，正在跑 step 2 段总结（单次 chat 无进度条）；前端段卡片
///   body 文案从"正在让模型看截图"切到"生成段总结中…"，spinner 持续到 `segment_done`
/// - `segment_done`：段进入完成态（含 ok / skipped / error）；content 是段总结
/// - `all_done` / `cancelled` / `error`：整轮收尾
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryProgress {
    /// "daily" / "weekly"——日报、周报各自只 listen 自己 source 的事件，
    /// 避免一个页面跑时另一个页面跟着刷数据。
    pub source: String,
    pub date: String,
    pub phase: &'static str,
    pub segment_idx: Option<u32>,
    pub total_segments: u32,
    /// 段开跑时给前端 "12 张图待分析" 的提示
    pub images_total: Option<u32>,
    /// ocr_running 时已识别的帧数
    pub image_index: Option<u32>,
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
            content: None,
            status: None,
            message: None,
        }
    }
}

/// 进度事件出口的最小抽象。生产代码里就是 [`AppHandle`]（emit 给前端）；
/// 抽成 trait 是为了让编排逻辑能脱离 Tauri 运行时做单元测试——`tauri` 依赖
/// 没开 `test` feature，测试进程里根本构造不出 AppHandle，这是 runner
/// 对 Tauri 的唯一硬依赖，砍断它其余全部编排都可测。
pub trait ProgressSink {
    fn emit_progress(&self, payload: SummaryProgress);
}

impl ProgressSink for AppHandle {
    fn emit_progress(&self, payload: SummaryProgress) {
        if let Err(e) = self.emit(SUMMARY_PROGRESS_EVENT, &payload) {
            log::warn!("emit {SUMMARY_PROGRESS_EVENT} 失败: {e}");
        }
    }
}

/// 把一个 future 变成"可被停止按钮中断的"：每 250ms 轮询一次 cancel 标志，
/// 置位则**丢弃 future**（reqwest 请求随之断开连接）并返回 [`Error::SummaryCancelled`]。
///
/// 正在路上的 LLM 请求（本地超时 600s）和引擎加载（最长 90s）都能被中断；
/// llama-server 检测到客户端断开会停掉该 slot 的生成，云端 API 同理，中断是安全的。
pub(crate) async fn cancellable<T>(
    cancel: &Arc<AtomicBool>,
    fut: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::pin!(fut);
    loop {
        tokio::select! {
            r = &mut fut => return r,
            _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error::SummaryCancelled);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 停止按钮按下的同一瞬间 LLM 恰好返回：已完成的结果（包括真实错误）不能被
    /// 吞成 SummaryCancelled——否则用户点了停止就永远看不到真正的失败原因。
    #[tokio::test]
    async fn cancellable_ready_result_beats_cancel_flag() {
        let cancel = Arc::new(AtomicBool::new(true));
        let ok = cancellable(&cancel, async { Ok::<_, Error>(42u32) }).await;
        assert_eq!(ok.unwrap(), 42, "已就绪的 Ok 结果应原样返回");

        let err = cancellable(&cancel, async {
            Err::<u32, _>(Error::InvalidInput("boom"))
        })
        .await;
        assert!(
            matches!(err, Err(Error::InvalidInput("boom"))),
            "已就绪的 Err 应透传原错误而非 SummaryCancelled: {err:?}"
        );
    }

    /// 引擎挂死不回包时按停止：250ms 轮询必须能打断永远 pending 的 future。
    /// 这是停止按钮的核心保证——没有它用户只能干等 600s 超时。
    #[tokio::test]
    async fn cancellable_pending_future_returns_cancelled() {
        let cancel = Arc::new(AtomicBool::new(true));
        let r: Result<u32> = cancellable(&cancel, std::future::pending::<Result<u32>>()).await;
        assert!(
            matches!(r, Err(Error::SummaryCancelled)),
            "挂死 future + cancel 置位应返回 SummaryCancelled: {r:?}"
        );
    }
}
