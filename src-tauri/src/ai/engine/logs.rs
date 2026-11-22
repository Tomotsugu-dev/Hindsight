//! llama-server 最近的输出。引擎起不来时，拿最后几行当原因给用户看。

use std::collections::VecDeque;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::Mutex;

/// 最多留多少行，老的被淘汰。
const RING_SIZE: usize = 500;

/// 最近 [`RING_SIZE`] 行 stderr、stdout。克隆出来的共用同一份。
#[derive(Clone)]
pub(super) struct Logs(Arc<Mutex<VecDeque<String>>>);

impl Logs {
    pub(super) fn new() -> Self {
        Self(Arc::new(Mutex::new(VecDeque::with_capacity(RING_SIZE))))
    }

    pub(super) async fn clear(&self) {
        self.0.lock().await.clear();
    }

    /// 最后 `n` 行，用换行连起来。
    pub(super) async fn tail(&self, n: usize) -> String {
        let g = self.0.lock().await;
        let start = g.len().saturating_sub(n);
        g.iter().skip(start).cloned().collect::<Vec<_>>().join("\n")
    }

    /// spawn 一个 tokio task，按行读 `reader`，每行都记下来。
    /// 不读的话管道满了会卡住 llama-server。
    ///
    /// `tag` = "stderr" / "stdout"，给行前面带个前缀方便区分。
    pub(super) fn collect<R>(&self, reader: R, tag: &'static str)
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        let ring = Arc::clone(&self.0);
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let prefixed = format!("[{tag}] {line}");
                        // llama-server 的 stderr / stdout 转发到 log 框架。release 时跟
                        // `env_logger` 的 RUST_LOG 过滤一致；dev 模式（debug_assertions）
                        // 再额外 eprintln 一份到 console，确保 cuBLAS init / offloaded
                        // layers 这种启动诊断信息一定能在终端看到。
                        log::info!("[llama-server] {prefixed}");
                        #[cfg(debug_assertions)]
                        eprintln!("[llama-server] {prefixed}");
                        let mut g = ring.lock().await;
                        if g.len() >= RING_SIZE {
                            g.pop_front();
                        }
                        g.push_back(prefixed);
                    }
                    Ok(None) => break, // EOF：子进程关闭了管道
                    Err(_) => break,   // 读错误：直接退出 task
                }
            }
        });
    }
}
