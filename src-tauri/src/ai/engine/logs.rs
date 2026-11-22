//! `llama-server` 子进程输出的环形日志缓冲，用于启动失败时提取末尾诊断信息。

use std::collections::VecDeque;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::Mutex;

/// 环形缓冲最多保留的日志行数，超出后淘汰最早记录。
const MAX_LOG_LINES: usize = 500;

/// 线程安全的 `llama-server` 最近输出缓冲（最多 [`MAX_LOG_LINES`] 行）；`Clone` 共享同一份底层缓冲。
#[derive(Clone)]
pub(super) struct EngineLogs(Arc<Mutex<VecDeque<String>>>);

impl EngineLogs {
    pub(super) fn new() -> Self {
        Self(Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_LINES))))
    }

    pub(super) async fn clear(&self) {
        self.0.lock().await.clear();
    }

    /// 返回最近 `line_count` 行日志，按换行符拼接。
    pub(super) async fn tail(&self, line_count: usize) -> String {
        let buffer = self.0.lock().await;
        let start = buffer.len().saturating_sub(line_count);
        buffer
            .iter()
            .skip(start)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 启动一个后台异步任务持续排空 `reader` 并写入日志缓冲。
    ///
    /// 必须持续消费子进程管道，否则管道写满后会阻塞 `llama-server` 进程本身。
    /// `stream_tag` 通常为 `"stderr"` 或 `"stdout"`，作为每行前缀便于排查。
    pub(super) fn spawn_reader<R>(&self, reader: R, stream_tag: &'static str)
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        let buffer = Arc::clone(&self.0);
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let prefixed = format!("[{stream_tag}] {line}");
                        // 转发到统一日志框架；开发模式下额外输出到 stderr 方便直接观察 GPU offload 诊断。
                        log::info!("[llama-server] {prefixed}");
                        #[cfg(debug_assertions)]
                        eprintln!("[llama-server] {prefixed}");
                        let mut ring = buffer.lock().await;
                        if ring.len() >= MAX_LOG_LINES {
                            ring.pop_front();
                        }
                        ring.push_back(prefixed);
                    }
                    Ok(None) => break, // EOF：子进程已关闭输出管道
                    Err(_) => break,   // 读取异常：退出后台任务
                }
            }
        });
    }
}
