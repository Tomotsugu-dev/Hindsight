//! 日报、周报测试共用的夹具：进度收集、假云端、云端设置。
//!
//! 编排测试的思路：settings 里把模型路由到「云端」，endpoint 指向本地
//! 127.0.0.1 的脚本化假 OpenAI 服务——这样 runner 的引擎管理分支（本地
//! llama-server 启停）天然短路，其余编排（材料合成 / 落库 / 进度事件 /
//! 取消 / 失败续跑）全部走真代码真 DB。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::ai::config::{CloudConnection, DaySegment, ModelRef};
use crate::ai::features::report::summary_progress::SummaryProgress;
use crate::ai::features::report::summary_runner::ProgressSink;
use crate::repo::settings as settings_repo;
use crate::storage::DbPool;

/// [`ProgressSink`] 的测试实现：把每个进度事件按序收进 Vec。
/// `cancel_on_segment_done` 模拟「用户看到某段完成后按停止」——在 emit
/// segment_done 的同一瞬间置位取消标志，检验段边界的取消检查。
#[derive(Clone, Default)]
pub(super) struct RecordingSink {
    pub(super) events: Arc<Mutex<Vec<SummaryProgress>>>,
    pub(super) cancel_on_segment_done: Option<Arc<AtomicBool>>,
}

impl ProgressSink for RecordingSink {
    fn emit_progress(&self, payload: SummaryProgress) {
        if payload.phase == "segment_done" {
            if let Some(c) = &self.cancel_on_segment_done {
                c.store(true, Ordering::Relaxed);
            }
        }
        self.events.lock().unwrap().push(payload);
    }
}

/// 收集到的事件压成 (phase, segment_idx) 序列，给断言比对用。
pub(super) fn phase_seq(events: &[SummaryProgress]) -> Vec<(&'static str, Option<u32>)> {
    events.iter().map(|p| (p.phase, p.segment_idx)).collect()
}

/// 脚本化假云端的单发行为。
pub(super) enum Canned {
    /// 回一发 OpenAI 兼容 200，content 为给定文本
    Ok(&'static str),
    /// 回 500——模拟云端内部错误（连接是通的，业务失败）
    Http500,
}

/// 起一个脚本化的本地假 OpenAI 服务：第 N 个连接消费脚本第 N 项；
/// 脚本耗尽后一律回 500——这样「不应该发出的多余请求」会以 error 行
/// 的形式在断言里暴露出来，而不是静默成功。
/// 返回 (端口, 已捕获的请求 body 列表)——body 用来断言材料过滤真的生效。
pub(super) async fn spawn_scripted_openai_server(
    script: Vec<Canned>,
) -> (u16, Arc<Mutex<Vec<String>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let bodies_in = bodies.clone();
    tokio::spawn(async move {
        let mut script = VecDeque::from(script);
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            // 读完整个请求（headers + Content-Length body）再回包，
            // 避免半途关闭触发 RST 让客户端报奇怪的连接错误
            let mut buf: Vec<u8> = Vec::new();
            let mut tmp = [0u8; 4096];
            let mut body_start = 0usize;
            while let Ok(n) = sock.read(&mut tmp).await {
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                    let cl = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if buf.len() >= pos + 4 + cl {
                        body_start = pos + 4;
                        break;
                    }
                }
            }
            bodies_in
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf[body_start..]).into_owned());
            let resp = match script.pop_front().unwrap_or(Canned::Http500) {
                Canned::Ok(content) => {
                    let body = serde_json::json!({
                        "choices": [{
                            "message": { "role": "assistant", "content": content },
                            "finish_reason": "stop"
                        }],
                        "usage": { "prompt_tokens": 42, "completion_tokens": 17 }
                    })
                    .to_string();
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                }
                Canned::Http500 => {
                    let body = r#"{"error":"internal"}"#;
                    format!(
                        "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                }
            };
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.shutdown().await;
        }
    });
    (port, bodies)
}

/// 把「云端模型 + 指定段划分」写进 settings_store。周报默认继承日报的模型。
/// runner 每次都从 DB 现读 settings——这是唯一的注入口。
pub(super) async fn seed_cloud_settings(pool: &DbPool, port: u16, segments: Vec<DaySegment>) {
    // 两个路径设非空：避免 load() 的「空路径补默认值」dirty 回写干扰测试
    let mut s = settings_repo::Settings {
        screenshot_path: "/tmp/hindsight-test-shots".to_string(),
        ..Default::default()
    };
    s.ai.models_path = "/tmp/hindsight-test-models".to_string();
    s.ai.cloud = vec![CloudConnection {
        id: "c1".to_string(),
        name: "test".to_string(),
        dialect: "openai".to_string(),
        endpoint: format!("http://127.0.0.1:{port}/v1"),
        model: "canned-model".to_string(),
        api_key: String::new(),
        context_length: None,
    }];
    s.ai.picks.daily = Some(ModelRef::Cloud {
        connection: "c1".to_string(),
    });
    s.ai.segments = segments;
    settings_repo::save(pool, &s).await.unwrap();
}
