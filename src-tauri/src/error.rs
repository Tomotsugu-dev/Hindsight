use thiserror::Error;

/// 全 crate 的错误枚举。原则：
/// - 每个 variant 表达"是哪种问题"，而不是把字符串塞进 Other —— 上层能 match
/// - 用 #[source] / #[from] 保留原始错误，e.source() 能拿到底层 cause
/// - Other 仅作真不知道怎么分类的兜底，不应是主流
#[derive(Debug, Error)]
pub enum Error {
    // ───────────── 基础设施 / 透传 ─────────────
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("db: {0}")]
    Db(#[from] tokio_rusqlite::Error),

    #[error("sql: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    #[error("capture: {0}")]
    Capture(String),

    // ───────────── OAuth / 认证 ─────────────
    /// 用户没登录 Google。push/pull 看到这条会 silently 跳过，不当错误展示。
    #[error("not signed in")]
    NotSignedIn,

    /// Google OAuth client_id / secret 没配齐
    #[error("oauth not configured: {0}")]
    OAuthNotConfigured(String),

    /// OAuth HTTP 端点返回非 2xx（token 申请 / 续期）
    #[error("oauth {endpoint} returned {status}: {body}")]
    OAuthHttp {
        endpoint: &'static str, // "token" / "refresh"
        status: u16,
        body: String,
    },

    /// 等 OAuth 回调 3 分钟没等到
    #[error("oauth callback timeout")]
    OAuthTimeout,

    #[error("oauth state mismatch (possible CSRF)")]
    OAuthStateMismatch,

    #[error("oauth denied by user: {0}")]
    OAuthDenied(String),

    #[error("oauth callback missing code")]
    OAuthMissingCode,

    #[error("oauth refresh_token missing in response (need access_type=offline + prompt=consent)")]
    OAuthMissingRefreshToken,

    #[error("oauth id_token invalid: {0}")]
    OAuthIdTokenInvalid(&'static str),

    /// 浏览器 / TcpListener / socket 这类围绕 OAuth 的低层 IO/系统问题
    #[error("oauth setup: {0}")]
    OAuthSetup(String),

    // ───────────── 凭证安全 ─────────────
    #[error("keyring: {0}")]
    Keyring(String),

    #[error("crypto: {0}")]
    Crypto(&'static str),

    // ───────────── Drive REST ─────────────
    /// Drive HTTP 返回非 2xx（list / upload / download / etc）
    #[error("drive {stage} returned {status}: {body}")]
    DriveHttp {
        stage: &'static str,
        status: u16,
        body: String,
    },

    // ───────────── 同步合并阶段 ─────────────
    /// 远端 JSON payload 解析失败（categories.json / app_groups.json 等）
    #[error("sync parse {kind} JSON: {source}")]
    SyncParse {
        kind: &'static str,
        #[source]
        source: serde_json::Error,
    },

    /// ndjson 文件 UTF-8 解码失败
    #[error("sync ndjson utf8: {0}")]
    SyncUtf8(#[from] std::str::Utf8Error),

    // ───────────── 用户输入 ─────────────
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),

    // ───────────── 真兜底（少用）─────────────
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<Error> for String {
    fn from(e: Error) -> String {
        e.to_string()
    }
}
