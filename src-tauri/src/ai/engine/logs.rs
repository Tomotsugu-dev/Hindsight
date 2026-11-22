//! Recent output of the `llama-server` child process, kept in a ring buffer.
//! When a start fails, the last lines explain why.

use std::collections::VecDeque;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::Mutex;

/// The most lines the buffer keeps. When it is full, the oldest line is dropped.
const MAX_LOG_LINES: usize = 500;

/// The latest `llama-server` output, at most [`MAX_LOG_LINES`] lines.
///
/// Clones share the same buffer.
#[derive(Clone)]
pub(super) struct EngineLogs(Arc<Mutex<VecDeque<String>>>);

impl EngineLogs {
    pub(super) fn new() -> Self {
        Self(Arc::new(Mutex::new(VecDeque::with_capacity(MAX_LOG_LINES))))
    }

    pub(super) async fn clear(&self) {
        self.0.lock().await.clear();
    }

    /// Returns the last `line_count` lines, joined with newlines.
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

    /// Starts a background task that reads `reader` line by line into the buffer.
    ///
    /// The pipe must be read all the time. If it fills up, `llama-server` blocks.
    /// `stream_tag` is usually `"stderr"` or `"stdout"`. It goes in front of each line.
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
                        // Send each line to the app log too. Dev builds also print it to the
                        // terminal, so start-up details such as GPU offload are easy to see.
                        log::info!("[llama-server] {prefixed}");
                        #[cfg(debug_assertions)]
                        eprintln!("[llama-server] {prefixed}");
                        let mut ring = buffer.lock().await;
                        if ring.len() >= MAX_LOG_LINES {
                            ring.pop_front();
                        }
                        ring.push_back(prefixed);
                    }
                    Ok(None) => break, // EOF: the child process closed the pipe
                    Err(_) => break,   // Read error: stop reading this pipe
                }
            }
        });
    }
}
