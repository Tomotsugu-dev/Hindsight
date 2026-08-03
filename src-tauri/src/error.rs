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

    /// OAuth token 端点在连接层就失败（DNS/TCP/TLS 建连不通，不是 Google 拒绝）。
    /// 受限网络的典型场景：代理只开了"系统代理"模式而分流规则漏了 googleapis.com，
    /// 或压根没被接管。文案直接面向用户给自救指引（这条会原样显示在同步设置页）。
    #[error(
        "无法连接 Google 服务器（{source}）。如在受限网络：请开启代理的 TUN/增强模式，\
         或确认 googleapis.com 已加入代理规则"
    )]
    OAuthUnreachable {
        #[source]
        source: reqwest::Error,
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

    /// onnxruntime 动态库未安装——OCR 引擎首次加载时 dlopen 失败的"明确分类"错误。
    /// caller 应把这条往上抛到前端，前端弹框引导用户先下载推理库。
    #[error("embedding runtime missing")]
    EmbeddingRuntimeMissing,

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

    /// PP-OCRv5 识别失败（模型加载 / 预处理 / 推理 / 后处理任一阶段）。
    /// caller（消化 worker）按单帧失败处理：标 `ocr_state=2` 重试，不中断批次。
    #[error("ocr failed: {0}")]
    Ocr(String),

    /// OCR **基础设施**故障：识别进程超时 / 崩溃 / 拉不起来、引擎缺失、DB 抖动。
    ///
    /// 与 [`Error::Ocr`]（这一张图本身有问题）分开是有代价教训的：两者混为一谈时，
    /// 一次设施级故障会把当时在处理的每一帧的三次重试预算连续烧光，帧被永久放弃，
    /// 而截图随后被保留策略删除——2026-07-17 就这样丢了三小时的文字（243 帧，
    /// `attempts` 全部为 3）。消化循环据此把这类错误走「不计入帧重试」的路径。
    #[error("ocr infrastructure: {0}")]
    OcrInfra(String),

    /// 模型下载被用户主动取消（点暂停）。**不是 fatal**——`.partial` 文件保留，
    /// 下次再调 `download_from_hf` 同 file 名时走 Range 续传。
    /// caller（download_model command）应把这条单独 catch，让前端表达成"已暂停"
    /// 而非"下载失败"。
    #[error("download cancelled: {0}")]
    DownloadCancelled(String),

    /// AI 总结被用户停止（cancel 标志在 LLM 请求 / 引擎加载进行中被置位，
    /// 对应的 future 已被丢弃中断）。**不是失败**——调用链看到它应当优雅收尾：
    /// emit "cancelled" 进度、不给该段写 error 行，让下次生成自然重跑。
    #[error("summary cancelled")]
    SummaryCancelled,

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as StdError;

    // 断言套路（全模块通用）：先独立拿底层错误自己的 to_string() 当期望值，
    // 再断言包装后的 Display 包含它 —— 这样既不抄 error.rs 里的格式常量，
    // 也不依赖第三方库某个版本的具体文案。

    /// io::Error 经 #[from] 进入 Io 变体：Display 透传底层文案，source() 链保留。
    /// source() 保留是关键行为 —— 上层日志靠它打印完整 cause 链。
    #[test]
    fn io_from_preserves_message_and_source() {
        let inner = std::io::Error::new(std::io::ErrorKind::NotFound, "screenshot dir gone");
        let inner_msg = inner.to_string();
        let e: Error = inner.into();
        assert!(matches!(e, Error::Io(_)));
        assert!(
            e.to_string().contains(&inner_msg),
            "Display 应包含底层 io 错误文案: {e}"
        );
        let src = e.source().expect("Io 变体应保留 source");
        assert!(src.downcast_ref::<std::io::Error>().is_some());
    }

    /// rusqlite::Error → Sqlite 变体。QueryReturnedNoRows 是最容易稳定构造的变体。
    #[test]
    fn sqlite_from_preserves_message_and_source() {
        let inner = rusqlite::Error::QueryReturnedNoRows;
        let inner_msg = inner.to_string();
        let e: Error = inner.into();
        assert!(matches!(e, Error::Sqlite(_)));
        assert!(e.to_string().contains(&inner_msg));
        let src = e.source().expect("Sqlite 变体应保留 source");
        assert!(src.downcast_ref::<rusqlite::Error>().is_some());
    }

    /// tokio_rusqlite::Error → Db 变体。ConnectionClosed 无参可直接构造。
    #[test]
    fn db_from_tokio_rusqlite() {
        let inner = tokio_rusqlite::Error::ConnectionClosed;
        let inner_msg = inner.to_string();
        let e: Error = inner.into();
        assert!(matches!(e, Error::Db(_)));
        assert!(e.to_string().contains(&inner_msg));
        assert!(e.source().is_some(), "Db 变体应保留 source");
    }

    /// serde_json::Error → Json 变体。用真实解析失败构造，不手搓错误。
    #[test]
    fn json_from_parse_failure() {
        let inner = serde_json::from_str::<serde_json::Value>("{ not json").unwrap_err();
        let inner_msg = inner.to_string();
        let e: Error = inner.into();
        assert!(matches!(e, Error::Json(_)));
        assert!(e.to_string().contains(&inner_msg));
        let src = e.source().expect("Json 变体应保留 source");
        assert!(src.downcast_ref::<serde_json::Error>().is_some());
    }

    /// reqwest::Error → Http 变体。技巧：给 RequestBuilder 一个非法 URL，
    /// 错误在 build() 阶段同步产生，完全不碰网络（CI 无网也稳定）。
    #[test]
    fn http_from_reqwest_builder_error() {
        let inner = reqwest::Client::new()
            .get("这不是一个URL")
            .build()
            .expect_err("非法 URL 必须在 build() 就失败");
        let inner_msg = inner.to_string();
        let e: Error = inner.into();
        assert!(matches!(e, Error::Http(_)));
        assert!(e.to_string().contains(&inner_msg));
        let src = e.source().expect("Http 变体应保留 source");
        assert!(src.downcast_ref::<reqwest::Error>().is_some());
    }

    /// Utf8Error → SyncUtf8 变体。构造方式对应真实场景：
    /// ndjson 分块读取把一个多字节汉字从中间截断。
    #[test]
    fn sync_utf8_from_invalid_bytes() {
        let truncated = &"好".as_bytes()[..2]; // 3 字节字符只取前 2 字节 → 非法 UTF-8
        let inner = std::str::from_utf8(truncated).unwrap_err();
        let inner_msg = inner.to_string();
        let e: Error = inner.into();
        assert!(matches!(e, Error::SyncUtf8(_)));
        assert!(e.to_string().contains(&inner_msg));
    }

    /// OAuthHttp：结构体变体三个字段必须全部出现在文案里 ——
    /// 排障时 endpoint/status/body 缺一个都定位不了问题。
    #[test]
    fn oauth_http_display_carries_all_fields() {
        let e = Error::OAuthHttp {
            endpoint: "refresh",
            status: 400,
            body: "invalid_grant".into(),
        };
        let msg = e.to_string();
        assert!(msg.contains("refresh"));
        assert!(msg.contains("400"));
        assert!(msg.contains("invalid_grant"));
    }

    /// OAuthUnreachable：这条会原样显示给用户，所以断言两点行为 ——
    /// (1) 自救指引关键词在（代理/googleapis.com）；(2) source 链保留供日志。
    #[test]
    fn oauth_unreachable_shows_guidance_and_keeps_source() {
        let inner = reqwest::Client::new().get("not a url").build().unwrap_err();
        let inner_msg = inner.to_string();
        let e = Error::OAuthUnreachable { source: inner };
        let msg = e.to_string();
        assert!(msg.contains(&inner_msg), "用户文案里应嵌入底层原因");
        assert!(msg.contains("googleapis.com"), "自救指引应点名要放行的域名");
        assert!(msg.contains("代理"), "受限网络场景应提示检查代理");
        let src = e.source().expect("OAuthUnreachable 应保留 source");
        assert!(src.downcast_ref::<reqwest::Error>().is_some());
    }

    /// DriveHttp：同 OAuthHttp，stage/status/body 三要素齐全才可排障。
    #[test]
    fn drive_http_display_carries_all_fields() {
        let e = Error::DriveHttp {
            stage: "upload",
            status: 507,
            body: "storageQuotaExceeded".into(),
        };
        let msg = e.to_string();
        assert!(msg.contains("upload"));
        assert!(msg.contains("507"));
        assert!(msg.contains("storageQuotaExceeded"));
    }

    /// SyncParse：kind 标明是哪个远端文件坏了，source 保留 serde 细节。
    /// 注意它不是 #[from]，必须手工构造 —— 也顺带验证了字段搭配可用。
    #[test]
    fn sync_parse_names_kind_and_keeps_source() {
        let inner = serde_json::from_str::<serde_json::Value>("[broken").unwrap_err();
        let inner_msg = inner.to_string();
        let e = Error::SyncParse {
            kind: "categories",
            source: inner,
        };
        let msg = e.to_string();
        assert!(msg.contains("categories"), "文案必须指明是哪类远端文件");
        assert!(msg.contains(&inner_msg));
        let src = e.source().expect("SyncParse 应保留 source");
        assert!(src.downcast_ref::<serde_json::Error>().is_some());
    }

    /// stage+details 型变体（EngineBinary / ImageProcessing）：
    /// stage 供 caller 分流重试策略，details 供人读，两者都必须在文案里。
    #[test]
    fn stage_details_variants_carry_both_fields() {
        let e = Error::EngineBinary {
            stage: "verify",
            details: "sha256 mismatch".into(),
        };
        let msg = e.to_string();
        assert!(msg.contains("verify") && msg.contains("sha256 mismatch"));

        let e = Error::ImageProcessing {
            stage: "decode",
            details: "corrupt png header".into(),
        };
        let msg = e.to_string();
        assert!(msg.contains("decode") && msg.contains("corrupt png header"));
    }

    /// InvalidInput 与 InvalidInputDyn 对用户是同一类错误，前缀应一致；
    /// Dyn 版存在的意义就是能携带运行期值，断言该值真的出现在文案里。
    #[test]
    fn invalid_input_static_and_dyn_share_shape() {
        let s = Error::InvalidInput("name empty").to_string();
        let d = Error::InvalidInputDyn("段下标越界：5".into()).to_string();
        assert!(s.contains("name empty"));
        assert!(d.contains("段下标越界：5"), "Dyn 版必须带上运行期值");
        // 两者前缀一致：取 static 版去掉自身 payload 后的前缀，Dyn 版应以它开头
        let prefix = s.strip_suffix("name empty").expect("payload 应在结尾");
        assert!(
            d.starts_with(prefix),
            "两个 InvalidInput 变体对外应是同一形状: {s:?} vs {d:?}"
        );
    }

    /// 无参"信号型"变体：文案互不相同（上层/用户靠文案区分），
    /// 且各自带能说明处置方式的关键词。
    #[test]
    fn signal_variants_are_distinct_and_meaningful() {
        let not_signed_in = Error::NotSignedIn.to_string();
        let timeout = Error::OAuthTimeout.to_string();
        let scope = Error::DriveScopeInsufficient.to_string();
        let cancelled = Error::SummaryCancelled.to_string();
        let runtime_missing = Error::EmbeddingRuntimeMissing.to_string();

        let all = [
            &not_signed_in,
            &timeout,
            &scope,
            &cancelled,
            &runtime_missing,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b, "信号型变体文案不可重复，否则上层无法区分");
            }
        }
        // DriveScopeInsufficient 面向用户：必须点出缺的 scope 和自救动作（重新登录）
        assert!(scope.contains("drive.appdata"));
        assert!(scope.contains("重新"));
        // SummaryCancelled 不是失败，文案不应带 error/failed 字样
        let lower = cancelled.to_lowercase();
        assert!(!lower.contains("error") && !lower.contains("failed"));
    }

    /// 动态 payload 变体（DownloadCancelled / Ocr / Other）：运行期信息不可丢。
    /// Other 是兜底，Display 就是原文 —— 上层拼日志时不应出现多余前缀。
    #[test]
    fn dynamic_payload_variants_keep_payload() {
        assert!(Error::DownloadCancelled("qwen-7b.gguf".into())
            .to_string()
            .contains("qwen-7b.gguf"));
        assert!(Error::Ocr("inference shape mismatch".into())
            .to_string()
            .contains("inference shape mismatch"));
        assert_eq!(
            Error::Other("raw message".into()).to_string(),
            "raw message"
        );
    }

    /// From<Error> for String 是错误抵达前端的"序列化形态"（Tauri command 的
    /// Err(String)）：必须与 Display 完全一致，前端看到的就是用户文案本身。
    #[test]
    fn into_string_equals_display() {
        let e = Error::OAuthDenied("access_denied".into());
        let display = e.to_string();
        let s: String = e.into();
        assert_eq!(s, display);
        assert!(s.contains("access_denied"));
    }
}
