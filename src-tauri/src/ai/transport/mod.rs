//! Sends HTTP requests to model services.
//!
//! [`TransportPolicy`] sets the timeouts and retries.
//! The caller gives the path, the request headers and the request body, including the `stream` field.
//! [`Protocol`] answers what only the protocol knows.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde_json::Value;

use crate::error::{Error, Result};

pub(crate) mod fake;
mod sse;

/// What only the protocol knows. The adapter implements it and gives it to [`ModelTransport::post`].
pub trait Protocol {
    /// Joins the `data` of every event in one streaming response into the non-streaming shape.
    ///
    /// # Errors
    ///
    /// Returns a message for the user when the stream did not end properly.
    fn read_stream(&self, events: &[String]) -> std::result::Result<Value, String>;

    /// Fixes `body` from the error message of a 400. Returns `false` when it cannot.
    fn heal(&self, body: &mut Value, error: &str) -> bool;

    /// Tells if `data` is the last event of a stream.
    ///
    /// The transport stops reading the body after the last event.
    /// The default reads until the body ends.
    fn is_last(&self, _data: &str) -> bool {
        false
    }
}

/// Most rounds of 400 fixes. The longest chain seen in practice has two rounds:
/// a rename, then adding `none`.
const MAX_HEAL_ROUNDS: u32 = 3;

const BACKOFF_UNIT: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(1)
};

/// Timeout and retry settings for model requests.
///
/// Timeouts start again on each retry. A `None` timeout means no limit.
/// The three kinds of retry are counted separately.
#[derive(Debug, Clone)]
pub struct TransportPolicy {
    /// Longest time from connecting to reading the whole response body.
    pub total_timeout: Option<Duration>,
    /// Longest time to open the connection.
    pub connect_timeout: Option<Duration>,
    /// Longest time from sending the request to getting the response headers.
    ///
    /// While the body is read, the gap between two pieces of data has the same limit.
    pub read_timeout: Option<Duration>,

    /// Most retries after a 429.
    ///
    /// Each retry waits for the longer of `Retry-After` and the exponential backoff.
    pub max_rate_limit_retries: u32,
    /// Most retries after a connection failure, or a 2xx body that is cut short or cannot be parsed.
    ///
    /// Each retry sends the same request. Connection timeouts are not retried.
    pub max_transient_retries: u32,
    /// Most times to fix the request from a 400 error message and retry.
    pub max_heal_rounds: u32,
}

impl TransportPolicy {
    /// Returns the settings for cloud model services.
    ///
    /// These settings put no limit on the total time.
    pub fn cloud() -> Self {
        Self {
            total_timeout: None,
            connect_timeout: Some(Duration::from_secs(30)),
            read_timeout: Some(Duration::from_secs(60)),
            max_rate_limit_retries: 5,
            max_transient_retries: 2,
            max_heal_rounds: MAX_HEAL_ROUNDS,
        }
    }

    /// Returns the settings for the local llama-server.
    ///
    /// These settings limit only the total time.
    /// No failure is retried.
    pub fn local() -> Self {
        Self {
            total_timeout: Some(Duration::from_secs(600)),
            connect_timeout: None,
            read_timeout: None,
            max_rate_limit_retries: 0,
            max_transient_retries: 0,
            max_heal_rounds: 0,
        }
    }
}

/// Why a model request failed.
///
/// If the request was retried, this is why the last attempt failed.
/// [`fmt_send_err`] turns `Send` into a message for the user.
#[derive(Debug)]
pub enum TransportError {
    /// The request got no response, for example because the connection failed or timed out.
    Send(reqwest::Error),
    /// The server returned a non-2xx status other than 429.
    ///
    /// `body` is the response body, not truncated.
    Status { status: StatusCode, body: String },
    /// The server returned 429.
    ///
    /// `retry_after` is the wait given in seconds by the `Retry-After` header.
    RateLimited {
        retry_after: Option<Duration>,
        body: String,
    },
    /// The status was 2xx, but the response body was cut short or could not be parsed.
    Body(String),
}

impl TransportError {
    fn is_transient(&self) -> bool {
        match self {
            Self::Send(e) => e.is_connect() && !e.is_timeout(),
            Self::Body(_) => true,
            _ => false,
        }
    }
}

/// A client that sends requests to one model service.
#[derive(Clone)]
pub struct ModelTransport {
    base_url: String,
    policy: TransportPolicy,
    http: Client,
}

impl ModelTransport {
    /// Creates a client.
    ///
    /// `base_url` is the API address, including `/v1`. A trailing `/` is removed.
    /// Every request carries `headers`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LlmResponse`] when the HTTP client cannot be built.
    pub fn new(base_url: &str, headers: HeaderMap, policy: TransportPolicy) -> Result<Self> {
        let mut builder = Client::builder().default_headers(headers);
        if let Some(t) = policy.total_timeout {
            builder = builder.timeout(t);
        }
        if let Some(t) = policy.connect_timeout {
            builder = builder.connect_timeout(t);
        }
        if let Some(t) = policy.read_timeout {
            builder = builder.read_timeout(t);
        }
        let http = builder
            .build()
            .map_err(|e| Error::LlmResponse(format!("cannot build the HTTP client: {e}")))?;
        Ok(Self {
            base_url: base_url.trim().trim_end_matches('/').to_string(),
            policy,
            http,
        })
    }

    /// Sends `POST` to `path` and returns the response body in the non-streaming shape.
    ///
    /// `path` starts with `/` and follows `base_url`.
    /// `protocol` joins a streaming response into the same shape.
    /// Failed requests are retried as [`TransportPolicy`] says.
    /// After a 400, `protocol` may fix the request body before the retry.
    ///
    /// # Errors
    ///
    /// Returns why the last attempt failed, once no more retries are left.
    pub async fn post(
        &self,
        path: &str,
        body: &Value,
        protocol: &impl Protocol,
    ) -> std::result::Result<Value, TransportError> {
        let url = format!("{}{path}", self.base_url);
        let mut body = body.clone();
        let (mut healed, mut limited, mut dropped) = (0u32, 0u32, 0u32);
        loop {
            let err = match self.post_once(&url, &body, protocol).await {
                Ok(reply) => return Ok(reply),
                Err(e) => e,
            };
            let wait = match &err {
                TransportError::Status { status, body: text }
                    if *status == StatusCode::BAD_REQUEST =>
                {
                    if healed < self.policy.max_heal_rounds && protocol.heal(&mut body, text) {
                        healed += 1;
                        log::warn!(
                            "model request: 400, retrying with a fixed request (round {healed})"
                        );
                        Some(Duration::ZERO)
                    } else {
                        None
                    }
                }
                TransportError::RateLimited { retry_after, .. }
                    if limited < self.policy.max_rate_limit_retries =>
                {
                    limited += 1;
                    // Moonshot often sends 1 second in Retry-After, but it limits requests per minute.
                    let backoff = BACKOFF_UNIT * (1u32 << limited);
                    let wait = retry_after.map_or(backoff, |ra| ra.max(backoff));
                    log::info!("model request: 429, retry {limited} in {}s", wait.as_secs());
                    Some(wait)
                }
                e if e.is_transient() && dropped < self.policy.max_transient_retries => {
                    dropped += 1;
                    let wait = BACKOFF_UNIT * 2 * dropped;
                    log::warn!(
                        "model request: connection or response failed, retry {dropped} in {}s",
                        wait.as_secs()
                    );
                    Some(wait)
                }
                _ => None,
            };
            match wait {
                Some(w) if !w.is_zero() => tokio::time::sleep(w).await,
                Some(_) => {}
                None => return Err(err),
            }
        }
    }

    async fn post_once(
        &self,
        url: &str,
        body: &Value,
        protocol: &impl Protocol,
    ) -> std::result::Result<Value, TransportError> {
        let resp = self.send(self.http.post(url).json(body)).await?;
        let is_sse = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/event-stream"));
        if !is_sse {
            return read_json(resp).await;
        }
        let (events, cut) = sse::read_events(resp, |data| protocol.is_last(data)).await;
        // A cut after the last event does not matter when the protocol finds the reply complete.
        protocol
            .read_stream(&events)
            .map_err(|incomplete| TransportError::Body(cut.unwrap_or(incomplete)))
    }

    async fn send(&self, req: RequestBuilder) -> std::result::Result<Response, TransportError> {
        let resp = req.send().await.map_err(TransportError::Send)?;
        let status = resp.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            let body = resp.text().await.unwrap_or_default();
            return Err(TransportError::RateLimited { retry_after, body });
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(TransportError::Status { status, body });
        }
        Ok(resp)
    }
}

async fn read_json(resp: Response) -> std::result::Result<Value, TransportError> {
    resp.json::<Value>().await.map_err(|e| {
        TransportError::Body(format!(
            "cannot read or parse the response body: {}",
            error_chain(&e)
        ))
    })
}

/// Returns the `Authorization: Bearer` header for an API key. An empty key gives no header.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] when the key has characters that a header cannot hold.
pub fn bearer_headers(api_key: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let key = api_key.trim();
    if !key.is_empty() {
        let mut value = HeaderValue::from_str(&format!("Bearer {key}")).map_err(|_| {
            Error::InvalidInput("the API key has characters that cannot go in a request header")
        })?;
        value.set_sensitive(true);
        headers.insert(AUTHORIZATION, value);
    }
    Ok(headers)
}

/// The `Display` of a reqwest error shows only the error kind and the URL.
/// The cause is in the `source` chain.
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        s.push_str(" ← ");
        s.push_str(&c.to_string());
        cur = c.source();
    }
    s
}

/// Turns the error in [`TransportError::Send`] into a message for the user.
///
/// The message has the error kind and the causes from the `source` chain.
/// It does not repeat the request URL.
pub(crate) fn fmt_send_err(e: reqwest::Error) -> String {
    let head = if e.is_timeout() {
        "request timed out"
    } else if e.is_connect() {
        "connection failed (check that the service is running and the port is right)"
    } else if e.is_request() {
        "cannot build the request"
    } else {
        "network error"
    };
    match std::error::Error::source(&e) {
        Some(cause) => format!("{head}: {}", error_chain(cause)),
        None => head.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fake::{FakeServer, Reply};
    use serde_json::json;

    fn transport(server: &FakeServer, policy: TransportPolicy) -> ModelTransport {
        ModelTransport::new(&server.base_url(), HeaderMap::new(), policy).unwrap()
    }

    /// 测试用的协议，不是 Chat Completions：
    /// 流的最后一个事件是 `[DONE]` 才完整，拼成 `{"events": [...]}`；
    /// 400 的报错里有 "fixable" 就能改，每改一次 `round` 加一。
    struct Plain;

    impl Protocol for Plain {
        fn read_stream(&self, events: &[String]) -> std::result::Result<Value, String> {
            match events.split_last() {
                Some((last, rest)) if last == "[DONE]" => Ok(json!({ "events": rest })),
                _ => Err("incomplete".into()),
            }
        }

        fn heal(&self, body: &mut Value, error: &str) -> bool {
            if !error.contains("fixable") {
                return false;
            }
            body["round"] = json!(body["round"].as_u64().unwrap_or(0) + 1);
            true
        }

        fn is_last(&self, data: &str) -> bool {
            data == "[DONE]"
        }
    }

    fn fixable_400() -> Reply {
        Reply::json(400, json!({"error": "fixable"}))
    }

    /// 路径和请求头都由调用方给，传输层原样发出，自己不加鉴权头。
    #[tokio::test]
    async fn sends_the_callers_path_and_headers() {
        let server = FakeServer::start(vec![Reply::json(200, json!({"a": 1}))]).await;
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", HeaderValue::from_static("k1"));
        let t = ModelTransport::new(&server.base_url(), headers, TransportPolicy::cloud()).unwrap();
        t.post("/messages", &json!({}), &Plain).await.unwrap();
        let got = server.received();
        assert!(
            got[0].head.starts_with("post /v1/messages"),
            "{}",
            got[0].head
        );
        assert!(got[0].head.contains("x-api-key: k1"));
        assert!(!got[0].head.contains("authorization"));
    }

    #[test]
    fn bearer_headers_skips_empty_key_and_rejects_bad_characters() {
        assert!(bearer_headers("  ").unwrap().is_empty());
        let h = bearer_headers(" sk-1 ").unwrap();
        assert_eq!(h[AUTHORIZATION], "Bearer sk-1");
        assert!(h[AUTHORIZATION].is_sensitive());
        assert!(bearer_headers("sk\n1").is_err());
    }

    #[tokio::test]
    async fn json_reply_as_is_and_stream_joined_by_protocol() {
        let server = FakeServer::start(vec![
            Reply::json(200, json!({"a": 1})),
            Reply::sse(&[json!("乙")], true),
        ])
        .await;
        let t = transport(&server, TransportPolicy::cloud());
        let a = t.post("/x", &json!({}), &Plain).await.unwrap();
        let b = t.post("/x", &json!({}), &Plain).await.unwrap();
        assert_eq!(a, json!({"a": 1}));
        assert_eq!(b, json!({"events": ["\"乙\""]}));
    }

    /// 协议认出最后一个事件后就不再读，后面的事件不交给协议。
    #[tokio::test]
    async fn stops_reading_after_the_last_event() {
        let server = FakeServer::start(vec![Reply::Http {
            status: 200,
            headers: vec![("Content-Type", "text/event-stream".into())],
            body: "data: a\n\ndata: [DONE]\n\ndata: late\n\n".into(),
        }])
        .await;
        let got = transport(&server, TransportPolicy::cloud())
            .post("/x", &json!({}), &Plain)
            .await
            .unwrap();
        assert_eq!(got, json!({"events": ["a"]}));
    }

    #[tokio::test]
    async fn heals_400_at_most_three_rounds() {
        let server = FakeServer::start((0..4).map(|_| fixable_400()).collect()).await;
        let err = transport(&server, TransportPolicy::cloud())
            .post("/x", &json!({}), &Plain)
            .await
            .unwrap_err();
        assert!(
            matches!(err, TransportError::Status { status, .. } if status == StatusCode::BAD_REQUEST),
            "{err:?}"
        );
        let got = server.received();
        assert_eq!(got.len(), 4);
        assert_eq!(got[1].body["round"], 1, "第二次发的是改过的请求");
        assert_eq!(got[3].body["round"], 3);
    }

    #[tokio::test]
    async fn gives_up_on_400_it_cannot_heal() {
        let server = FakeServer::start(vec![Reply::json(400, json!({"error": "E5012"}))]).await;
        let err = transport(&server, TransportPolicy::cloud())
            .post("/x", &json!({}), &Plain)
            .await
            .unwrap_err();
        assert!(matches!(err, TransportError::Status { .. }), "{err:?}");
        assert_eq!(server.received().len(), 1);
    }

    #[tokio::test]
    async fn reports_rate_limited_after_five_retries() {
        let script = (0..6)
            .map(|_| Reply::json(429, json!({"error": {"message": "slow down"}})))
            .collect();
        let server = FakeServer::start(script).await;
        let err = transport(&server, TransportPolicy::cloud())
            .post("/x", &json!({}), &Plain)
            .await
            .unwrap_err();
        match err {
            TransportError::RateLimited { body, .. } => {
                assert!(body.contains("slow down"), "{body}");
            }
            other => panic!("应是 RateLimited: {other:?}"),
        }
        assert_eq!(server.received().len(), 6);
    }

    #[tokio::test]
    async fn retries_cut_stream_twice() {
        let cut = || Reply::sse(&[json!("半")], false);
        let server = FakeServer::start(vec![cut(), cut(), cut()]).await;
        let err = transport(&server, TransportPolicy::cloud())
            .post("/x", &json!({}), &Plain)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, TransportError::Body(m) if m == "incomplete"),
            "协议说不完整就是断流: {err:?}"
        );
        assert_eq!(server.received().len(), 3);
    }

    #[tokio::test]
    async fn local_policy_never_retries() {
        for reply in [
            Reply::json(429, json!({})),
            fixable_400(),
            Reply::sse(&[], false),
        ] {
            let server = FakeServer::start(vec![reply]).await;
            let r = transport(&server, TransportPolicy::local())
                .post("/x", &json!({}), &Plain)
                .await;
            assert!(r.is_err());
            assert_eq!(server.received().len(), 1);
        }
    }

    #[tokio::test]
    async fn status_error_keeps_whole_body() {
        let long = "错".repeat(500);
        let server = FakeServer::start(vec![Reply::json(500, json!({ "error": long }))]).await;
        let err = transport(&server, TransportPolicy::cloud())
            .post("/x", &json!({}), &Plain)
            .await
            .unwrap_err();
        match err {
            TransportError::Status { status, body } => {
                assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
                assert!(body.contains(&long));
            }
            other => panic!("应是 Status: {other:?}"),
        }
    }

    #[tokio::test]
    async fn fmt_send_err_names_kind_and_cause() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let e = reqwest::get(format!("http://127.0.0.1:{port}/v1/models"))
            .await
            .unwrap_err();
        let s = fmt_send_err(e);
        assert!(s.starts_with("connection failed"), "{s}");
        assert!(s.contains("): "), "缺底层原因:{s}");
        assert!(!s.contains("127.0.0.1"), "不该重复 URL:{s}");
    }

    #[test]
    fn error_chain_walks_sources() {
        #[derive(Debug)]
        struct Outer(std::io::Error);
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "error decoding response body")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let e = Outer(std::io::Error::other("connection reset by peer"));
        let s = error_chain(&e);
        assert!(s.contains("error decoding response body"), "{s}");
        assert!(s.contains(" ← "), "缺链接符:{s}");
        assert!(s.contains("connection reset by peer"), "缺底层原因:{s}");
    }
}
