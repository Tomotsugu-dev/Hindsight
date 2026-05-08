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

    /// Drive 返回 403 + ACCESS_TOKEN_SCOPE_INSUFFICIENT：
    /// 当前 token 没有 drive.appdata 权限（多半是 scope 升级前登的旧账号），
    /// 必须让用户重新【用 Google 登录】走一次同意页。和普通 401 不同，
    /// 单纯刷新 access_token 解决不了。
    #[error("drive scope insufficient：当前登录缺少 drive.appdata 权限，请重新【用 Google 登录】")]
    DriveScopeInsufficient,

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

    /// 动态消息版的 InvalidInput —— 段下标越界 / 段时间范围非法 / 模型名带分隔符等
    /// 取 String 而非 &'static str：消息里要带运行期值（"段下标越界：5"）
    #[error("invalid input: {0}")]
    InvalidInputDyn(String),

    /// sync_now 跑完了但 push/pull 内部记下了 last_error（多半是 token 不可用）。
    /// 用 String 因为这里聚合的是「内部 push/pull 各自塞回 status 的人类可读信息」，不需要 caller match。
    #[error("sync incomplete: {0}")]
    SyncIncomplete(String),

    // ───────────── AI 引擎相关 ─────────────
    /// llama.cpp binary 下载 / 校验 / 解压失败。`stage` 用静态字符串区分阶段
    /// （"download" / "verify" / "extract" / "cleanup"），让 caller 能 match 重试策略。
    /// 字段名用 `details` 避免被 thiserror 当成 `#[source]` 处理（String 不 impl StdError）
    #[error("engine binary {stage}: {details}")]
    EngineBinary {
        stage: &'static str,
        details: String,
    },

    /// 选中的 GGUF 文件（active_main / active_mmproj）在磁盘上找不到。
    /// caller 一般会引导用户去重新选模型
    #[error("model file missing: {0}")]
    ModelFileMissing(String),

    /// llama-server 启动 / health 检查 / 端口冲突等
    #[error("engine start: {0}")]
    EngineStart(String),

    /// llama-server 已经在 Starting 状态，不允许并发启动
    #[error("engine already starting")]
    EngineBusy,

    /// LLM HTTP 响应解析失败 / 状态码非 2xx / 内容为空。caller 一般标 status='error'
    #[error("llm response: {0}")]
    LlmResponse(String),

    /// 截图缩放 / 编码 / 读取失败。`stage` = "read" / "decode" / "encode" / "spawn_blocking"
    #[error("image processing {stage}: {details}")]
    ImageProcessing {
        stage: &'static str,
        details: String,
    },

    /// 模型下载被用户主动取消（点暂停）。**不是 fatal**——`.partial` 文件保留，
    /// 下次再调 `download_from_hf` 同 file 名时走 Range 续传。
    /// caller（download_model command）应把这条单独 catch，让前端表达成"已暂停"
    /// 而非"下载失败"。
    #[error("download cancelled: {0}")]
    DownloadCancelled(String),

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
