//! 假的 OpenAI 兼容服务器，只在测试里用。第 n 个连接按脚本的第 n 项回包，
//! 每个连接只处理一个请求。脚本用完就不再接受连接，多发的请求会被拒，
//! 测试据此发现「不该发的请求」。收到的每个请求都记下来，测试据此断言
//! 「发了几次、每次发了什么」。

#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;

/// 一个连接怎么回。
pub(crate) enum Reply {
    /// 回一个完整的 HTTP 响应。
    Http {
        status: u16,
        headers: Vec<(&'static str, String)>,
        body: String,
    },
    /// 读完请求后不回包，等客户端自己断开。
    Hang,
    /// 同 `Http`，但响应体分两次写出：前 `at` 个字节一次，其余隔一会儿再写。
    Split {
        status: u16,
        headers: Vec<(&'static str, String)>,
        body: String,
        at: usize,
    },
}

impl Reply {
    pub(crate) fn json(status: u16, body: Value) -> Self {
        Self::Http {
            status,
            headers: vec![("Content-Type", "application/json".into())],
            body: body.to_string(),
        }
    }

    /// 网关错误页那种 HTML。
    pub(crate) fn html(status: u16, body: &str) -> Self {
        Self::Http {
            status,
            headers: vec![("Content-Type", "text/html".into())],
            body: body.to_string(),
        }
    }

    /// 200 的 SSE 流，每块一行 `data:`。`done` 为 false 时不发 `[DONE]`，像流被中途掐断。
    pub(crate) fn sse(chunks: &[Value], done: bool) -> Self {
        let mut body = String::new();
        for c in chunks {
            body.push_str(&format!("data: {c}\n\n"));
        }
        if done {
            body.push_str("data: [DONE]\n\n");
        }
        Self::Http {
            status: 200,
            headers: vec![("Content-Type", "text/event-stream".into())],
            body,
        }
    }

    /// 响应体从 `text` 第一次出现处的第 1 个字节之后断开，分两次写出。
    /// `text` 是汉字时，断点正好在这个字的中间。
    pub(crate) fn split_inside(self, text: &str) -> Self {
        let Self::Http {
            status,
            headers,
            body,
        } = self
        else {
            panic!("split_inside 只用于 Http");
        };
        let at = body.find(text).expect("响应体里没有这段文字") + 1;
        Self::Split {
            status,
            headers,
            body,
            at,
        }
    }

    pub(crate) fn header(mut self, name: &'static str, value: &str) -> Self {
        if let Self::Http { headers, .. } = &mut self {
            headers.push((name, value.to_string()));
        }
        self
    }
}

/// 收到的一个请求。
pub(crate) struct Received {
    /// 请求行和请求头，转成了小写
    pub head: String,
    /// 请求体。不是 JSON（比如 GET）时是 `Null`
    pub body: Value,
}

pub(crate) struct FakeServer {
    pub port: u16,
    received: Arc<Mutex<Vec<Received>>>,
    disconnected: Arc<Notify>,
}

impl FakeServer {
    pub(crate) async fn start(script: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let received: Arc<Mutex<Vec<Received>>> = Arc::default();
        let disconnected = Arc::new(Notify::new());
        let (log, notify) = (received.clone(), disconnected.clone());
        tokio::spawn(async move {
            for reply in script {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let (head, body) = read_request(&mut sock).await;
                log.lock().unwrap().push(Received { head, body });
                match reply {
                    Reply::Http {
                        status,
                        headers,
                        body,
                    } => {
                        let resp = response(status, &headers, &body);
                        let _ = sock.write_all(resp.as_bytes()).await;
                        let _ = sock.shutdown().await;
                    }
                    Reply::Split {
                        status,
                        headers,
                        body,
                        at,
                    } => {
                        let resp = response(status, &headers, &body);
                        let cut = resp.len() - body.len() + at;
                        let _ = sock.write_all(&resp.as_bytes()[..cut]).await;
                        let _ = sock.flush().await;
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        let _ = sock.write_all(&resp.as_bytes()[cut..]).await;
                        let _ = sock.shutdown().await;
                    }
                    Reply::Hang => {
                        let mut tmp = [0u8; 64];
                        while matches!(sock.read(&mut tmp).await, Ok(n) if n > 0) {}
                        notify.notify_one();
                    }
                }
            }
        });
        Self {
            port,
            received,
            disconnected,
        }
    }

    /// 客户端用的地址，带 `/v1`。
    pub(crate) fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }

    /// 到目前为止收到的请求，按到达顺序。
    pub(crate) fn received(&self) -> std::sync::MutexGuard<'_, Vec<Received>> {
        self.received.lock().unwrap()
    }

    /// 等 `Hang` 的那个连接被客户端断开。等到了返回 true。
    pub(crate) async fn wait_disconnect(&self, timeout: Duration) -> bool {
        tokio::time::timeout(timeout, self.disconnected.notified())
            .await
            .is_ok()
    }
}

/// 读完一个请求：请求头，加上 Content-Length 那么长的请求体。
async fn read_request(sock: &mut tokio::net::TcpStream) -> (String, Value) {
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        let n = sock.read(&mut tmp).await.unwrap_or(0);
        if n == 0 {
            return (String::from_utf8_lossy(&buf).to_lowercase(), Value::Null);
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
            let len = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buf.len() >= pos + 4 + len {
                let body =
                    serde_json::from_slice(&buf[pos + 4..pos + 4 + len]).unwrap_or(Value::Null);
                return (head, body);
            }
        }
    }
}

fn response(status: u16, headers: &[(&'static str, String)], body: &str) -> String {
    let mut resp = format!("HTTP/1.1 {status} {}\r\n", reason(status));
    for (k, v) in headers {
        resp.push_str(&format!("{k}: {v}\r\n"));
    }
    resp.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ));
    resp
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Status",
    }
}
