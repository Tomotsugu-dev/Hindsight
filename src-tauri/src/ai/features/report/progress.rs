//! 日报、周报共用的进度推送与停止按钮中断逻辑。
//!
//! 前端监听 [`SUMMARY_PROGRESS_EVENT`]（`"ai://summary-progress"`），按 `source`
//! （`"daily"` / `"weekly"`）和 `phase` 分发界面更新。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::{Error, Result};

/// 推送报告生成进度的 Tauri 事件名。
pub const SUMMARY_PROGRESS_EVENT: &str = "ai://summary-progress";

/// 推给前端的进度事件载荷。
///
/// 有效 `phase` 取值：
/// - `ocr_engine_starting`：报告前的 OCR 清积压阶段开始（`images_total` 为待识别帧数）；
///   仅在常驻 OCR 开启且存在积压时发出。
/// - `ocr_running`：OCR 清积压进度（`image_index` / `images_total`）；收到下一个 phase 即表示结束。
/// - `engine_starting`：本机 `llama-server` 正在冷启动加载模型。
/// - `segment_started`：日报开始处理某一个时段（`segment_idx`）。
/// - `summarizing`：正在等待模型返回日报单段或周报的正文。
/// - `segment_done`：单段（或周报唯一一行）已结束，附带 `status`、`content` 及失败时的 `message`。
/// - `all_done` / `cancelled` / `error`：整轮运行的终态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryProgress {
    /// `"daily"` 或 `"weekly"`，让日报页和周报页各自只消费自己的事件。
    pub source: String,
    /// 目标日期（`"YYYY-MM-DD"`）；周报固定为该周周一。
    pub date: String,
    pub phase: &'static str,
    pub segment_idx: Option<u32>,
    pub total_segments: u32,
    /// OCR 清积压时的待识别帧总数（日报段事件中固定填 `Some(0)`）。
    pub images_total: Option<u32>,
    /// `ocr_running` 阶段已处理的帧数。
    pub image_index: Option<u32>,
    /// `segment_done` 附带的正文，供前端逐段即时渲染。
    pub content: Option<String>,
    /// `segment_done` 对应的落库状态（`"ok"` / `"skipped_no_activity"` / `"error"`）。
    pub status: Option<&'static str>,
    /// 单段失败或顶层 `error` 阶段的可读错误描述。
    pub message: Option<String>,
}

impl SummaryProgress {
    /// 构造一份基础进度事件，其余可选字段默认填 `None`。
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

/// 进度事件出口的最小抽象。
///
/// 生产代码传入 [`AppHandle`]；单元测试注入内存记录器，不依赖 Tauri 运行时即可测编排逻辑。
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

/// 让异步操作支持停止按钮中断：每 250ms 轮询一次 `cancel`，置位后丢弃 `fut`
/// 并返回 [`Error::SummaryCancelled`]。
///
/// 丢弃在途 HTTP 请求会主动断开连接，`llama-server` 和云端接口都会随之停止生成。
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

    /// 停止按下的瞬间如果 future 刚好已完成，保留它的真实结果（成功或报错）而不吞成取消。
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

    /// 远端或引擎挂死不回包时，250ms 轮询必须在 `cancel` 置位后立即打断。
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
