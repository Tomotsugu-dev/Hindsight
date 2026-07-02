//! Google OAuth Installed App (PKCE) → 直接用 Google access_token 调 Drive REST。
//!
//! 流程：
//! 1. 生成 PKCE verifier/challenge
//! 2. 在 127.0.0.1 起一个一次性 HTTP listener，作为 OAuth redirect_uri
//! 3. 浏览器打开 Google 同意页（带 client_id / scope=drive.appdata + openid email / code_challenge）
//! 4. 用户同意后跳回本地，listener 收到 ?code=xxx
//! 5. 用 code + code_verifier 调 https://oauth2.googleapis.com/token
//!    → 拿 access_token + refresh_token + id_token
//! 6. 解 id_token JWT 拿 sub（用户的 Google 唯一 ID）+ email
//! 7. 用「机器 ID + 用户 home 路径」派生 32 字节 AES key 加密 refresh_token，密文落 auth_state 表
//!
//! ## 加密 key 的派生（不依赖 OS keyring）
//!
//! 历史：v0.4.4 之前 AES key 存在 OS keyring（Windows Credential Manager / macOS Keychain）。
//! 但实测 Credential Manager 条目会因为 OS / 安全软件 / 自动更新等不可控原因消失，
//! macOS Keychain 在 ad-hoc 签名换身份时也读不出来——key 一丢，DB 里的 enc 永远解不开，
//! 用户被迫重新登录。
//!
//! 现在改成每次现算：`SHA256("hindsight-auth-v1" || machine_id || user_home)`。
//! - `machine_id`：Windows 注册表 `MachineGuid` / macOS `IOPlatformUUID` / Linux `/etc/machine-id`，
//!   重装系统才变；OS 服务 / 安全软件碰不到。
//! - `user_home`：[`dirs::home_dir`]，删用户账号才变。
//!
//! 安全权衡：把 DB 文件搬到别的机器仍然解不开（machine_id 不一样），
//! 同机器另一个用户也读不出（home 路径不一样）。仅在「攻击者已经能登录该用户、
//! 能读 APPDATA」的场景下能解密——但这个层面攻击者本来就能直接读 cookie /
//! 浏览器密码 / 一切，不靠这一层加密防。
//!
//! 迁移：从老 keyring 方案升级上来的旧 enc 用新 key 解不开 → [`refresh_and_persist`]
//! 自动清 `auth_state`，UI 自然回到「未登录」，用户重登一次后此后永不再丢。

use std::time::Duration;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::{engine::general_purpose, Engine as _};
use rand::distributions::Alphanumeric;
use rand::{Rng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::time::timeout;

use crate::error::{Error, Result};
use crate::repo::settings;
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;

const OAUTH_SCOPE: &str = "openid email https://www.googleapis.com/auth/drive.appdata";
const OAUTH_TIMEOUT_SECS: u64 = 180;
/// AES key 派生的域分隔常量。改这个值会让所有用户被踢出登录（紧急 key rotation 用）。
const KEY_DERIVATION_SALT: &[u8] = b"hindsight-auth-v1";

/// OAuth 登录状态对外快照（前端「设备」页面 + auth 命令读）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthState {
    pub signed_in: bool,
    pub uid: Option<String>,
    pub email: Option<String>,
    /// Google OAuth client_id / client_secret 是否齐全（决定 UI 上"用 Google 登录"按钮是否可点）
    pub configured: bool,
    /// 多账号场景下登录到了不同账号，需要用户重启 app 才能切到新账号的 DB。
    /// `current_state` 永远返回 false；只有 `sign_in_with_google` 在切账号时会置 true。
    #[serde(default)]
    pub requires_restart: bool,
}

/// 拉当前登录状态（不联网，只查 DB）。
pub async fn current_state(pool: &DbPool) -> Result<AuthState> {
    let cfg = settings::load(pool).await.unwrap_or_default();
    let configured =
        !cfg.google_client_id.trim().is_empty() && !cfg.google_client_secret.trim().is_empty();

    let row: Option<(String, String)> = pool
        .0
        .call(|conn| {
            let r = conn
                .query_row("SELECT uid, email FROM auth_state WHERE id = 1", [], |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    ))
                })
                .ok();
            Ok(r)
        })
        .await?;
    let (uid, email) = row.unwrap_or_default();
    let signed_in = !uid.is_empty();
    Ok(AuthState {
        signed_in,
        uid: if uid.is_empty() { None } else { Some(uid) },
        email: if email.is_empty() { None } else { Some(email) },
        configured,
        requires_restart: false,
    })
}

/// 完整登录流程：返回登录后的 AuthState。
pub async fn sign_in_with_google(pool: &DbPool) -> Result<AuthState> {
    let (client_id, client_secret) = load_creds(pool).await?;

    // 1) PKCE
    let verifier = generate_verifier();
    let challenge = derive_challenge(&verifier);
    let state = random_state();

    // 2) loopback listener
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| Error::OAuthSetup(format!("bind callback listener: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::OAuthSetup(format!("local_addr: {e}")))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    // 3) 打开浏览器
    let auth_url = build_auth_url(&client_id, &redirect_uri, &challenge, &state);
    if let Err(e) = open::that(&auth_url) {
        return Err(Error::OAuthSetup(format!("open browser: {e}")));
    }

    // 4) 等回调（最多 OAUTH_TIMEOUT_SECS 秒）
    let code_state = timeout(
        Duration::from_secs(OAUTH_TIMEOUT_SECS),
        accept_callback(listener),
    )
    .await
    .map_err(|_| Error::OAuthTimeout)??;
    if code_state.state != state {
        return Err(Error::OAuthStateMismatch);
    }

    // 5) code → access_token + refresh_token + id_token
    let google = exchange_code(
        &client_id,
        &client_secret,
        &code_state.code,
        &verifier,
        &redirect_uri,
    )
    .await?;

    // 6) 从 id_token 解 sub / email
    let (uid, email) = decode_id_token(google.id_token.as_deref().unwrap_or(""))
        .ok_or(Error::OAuthIdTokenInvalid("missing sub claim"))?;

    let refresh_token = google
        .refresh_token
        .ok_or(Error::OAuthMissingRefreshToken)?;

    // 7) 加密存储
    let key = derive_master_key()?;
    let enc = aes_encrypt(&key, refresh_token.as_bytes())?;

    let access = google.access_token.clone();
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(google.expires_in);
    let expires_at_str = expires_at.to_rfc3339();

    // 多账号分流：currently active uid vs 这次登的 uid
    //   None    → Case A: 第一次登录（匿名 DB）。把 token 写进当前 pool，标记 active_uid，
    //             下次启动 startup migration 把 hindsight.sqlite 改名 hindsight.<uid>.sqlite。
    //   uid 同  → Case B: 同账号续期/重新登。直接更新 auth_state，无重启。
    //   uid 不同 → Case C: 切账号。当前 pool 是旧账号的 DB，不能写新 token；同时把旧账号
    //             auth_state 清掉避免后台 sync 继续推到旧 Drive。更新 active_uid，告诉用户
    //             重启 app；重启后开新 DB，需要再做一次 OAuth 把 token 写进去。
    let prev_active = crate::account::active_uid();
    let switching = matches!(&prev_active, Some(prev) if prev != &uid);

    if switching {
        log::info!(
            "Google 登录到不同账号：{:?} -> {uid}，需要重启",
            prev_active
        );
        // 旧 DB 里的 auth_state 清掉，立刻停止后台 sync 推到旧 Drive
        pool.0
            .call(|conn| {
                conn.execute(
                    "UPDATE auth_state SET uid = NULL, email = NULL,
                       refresh_token_enc = NULL, access_token = NULL, expires_at = NULL
                     WHERE id = 1",
                    [],
                )
                .db()?;
                Ok(())
            })
            .await?;
        crate::account::set_active_uid(Some(&uid))?;
        let mut s = current_state(pool).await?;
        s.requires_restart = true;
        s.uid = Some(uid);
        s.email = if email.is_empty() { None } else { Some(email) };
        s.signed_in = true;
        return Ok(s);
    }

    // Case A / B：写当前 pool
    let uid_db = uid.clone();
    let email_db = email.clone();
    pool.0
        .call(move |conn| {
            conn.execute(
                "UPDATE auth_state SET
                   uid = ?1, email = ?2, refresh_token_enc = ?3,
                   access_token = ?4, expires_at = ?5
                 WHERE id = 1",
                rusqlite::params![uid_db, email_db, enc, access, expires_at_str],
            )
            .db()?;
            Ok(())
        })
        .await?;

    if prev_active.is_none() {
        // Case A：把 active_uid 立起来 + 声明 hindsight.sqlite 归属于这个 uid。
        // 下次启动 startup migration 会把文件 rename 为 hindsight.<uid>.sqlite。
        crate::account::set_active_uid(Some(&uid))?;
        crate::account::claim_legacy_for(&uid)?;
    }

    log::info!("Google 登录成功 uid={uid}");
    current_state(pool).await
}

/// 拿到一个有效的 Google access_token，过期就用 refresh_token 自动续。
/// 拉一个仍然有效的 access_token：
/// - 没登录 → `Error::NotSignedIn`
/// - 当前未过期 → 直接返回
/// - 已过期 → 用 DB 里的 refresh_token_enc 解出 refresh_token 调 Google 续一个，写回 DB 再返回
///
/// 同时返回当前 uid，方便上层路由。
///
/// 缓冲取 10 分钟：本地 expires_at 还剩 ≤10min 就提前续。原来 5 min 在
/// 笔记本盖盖醒来 / 系统时钟漂移的情况下不够用——access_token 在本地"还有效"
/// 时，Google 端可能已经把它拒了（401），用户被迫重新登录。
pub async fn ensure_valid_token(pool: &DbPool) -> Result<TokenInfo> {
    let (uid, rt_enc, access, expires_at) = read_auth_state(pool).await?;

    // 还在有效期 + 10 分钟以上缓冲 → 直接复用
    if let Ok(exp) = chrono::DateTime::parse_from_rfc3339(&expires_at) {
        let now = chrono::Utc::now();
        let exp_utc = exp.with_timezone(&chrono::Utc);
        if exp_utc - now > chrono::Duration::minutes(10) {
            return Ok(TokenInfo {
                uid,
                access_token: access,
            });
        }
    }

    log::debug!("access_token 即将过期或已过期，调 oauth2 端点续期…");
    refresh_and_persist(pool, uid, &rt_enc).await
}

/// 强制走 refresh 端点拿一份新的 access_token，不看本地 expires_at。
///
/// 给 Drive 在本地"未过期"但服务端返回 401 的场景用——典型情况：机器睡眠
/// 醒来、Google 端令牌轮换。调用方拿到新 token 后再重试一次 Drive 请求。
pub async fn force_refresh(pool: &DbPool) -> Result<TokenInfo> {
    let (uid, rt_enc, _access, _expires_at) = read_auth_state(pool).await?;
    log::info!("force_refresh：放弃当前 access_token，强制重新申请");
    refresh_and_persist(pool, uid, &rt_enc).await
}

async fn read_auth_state(pool: &DbPool) -> Result<(String, Vec<u8>, String, String)> {
    // 4 字段元组对应 auth_state 表 4 列；抽 type alias 反而要看两处才知道字段含义
    #[allow(clippy::type_complexity)]
    let row: Option<(
        Option<String>,
        Option<Vec<u8>>,
        Option<String>,
        Option<String>,
    )> = pool
        .0
        .call(|conn| {
            Ok(conn
                .query_row(
                    "SELECT uid, refresh_token_enc, access_token, expires_at
                     FROM auth_state WHERE id = 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .ok())
        })
        .await?;

    match row {
        Some((Some(uid), Some(rt_enc), Some(access), Some(expires_at))) => {
            Ok((uid, rt_enc, access, expires_at))
        }
        _ => Err(Error::NotSignedIn),
    }
}

async fn refresh_and_persist(pool: &DbPool, uid: String, rt_enc: &[u8]) -> Result<TokenInfo> {
    let (client_id, client_secret) = load_creds(pool).await?;
    let key = derive_master_key()?;
    // 解密失败的两种主要场景：
    //   1. 旧 keyring 方案的遗留 enc——v0.4.4 之前用 OS keyring 里随机 key 加密，
    //      升级到派生 key 方案后这种密文必然解不开，需要用户重登一次完成迁移。
    //   2. machine_id / home 路径变了（极罕见，重装系统 / 改用户名才会）。
    // 两种都是同一个恢复动作：清 auth_state 让 UI 回到「未登录」，让用户重登。
    // 返回 NotSignedIn 而非 Crypto，避免 sync engine 把它包成 [CRED_EXPIRED] 错误条幅。
    let rt_bytes = match aes_decrypt(&key, rt_enc) {
        Ok(b) => b,
        Err(Error::Crypto(_)) => {
            log::warn!(
                "refresh_token 解密失败（多半 keyring → 派生 key 方案迁移），\
                 自动清 auth_state 让 UI 回到未登录"
            );
            clear_auth_state(pool).await?;
            return Err(Error::NotSignedIn);
        }
        Err(e) => return Err(e),
    };
    let refresh_token =
        String::from_utf8(rt_bytes).map_err(|_| Error::Crypto("refresh_token utf-8 decode"))?;

    let fresh = refresh_with_google(&client_id, &client_secret, &refresh_token).await?;

    let new_access = fresh.access_token.clone();
    let new_expires =
        (chrono::Utc::now() + chrono::Duration::seconds(fresh.expires_in)).to_rfc3339();
    let new_access_db = new_access.clone();
    pool.0
        .call(move |conn| {
            conn.execute(
                "UPDATE auth_state SET access_token = ?1, expires_at = ?2 WHERE id = 1",
                rusqlite::params![new_access_db, new_expires],
            )
            .db()?;
            Ok(())
        })
        .await?;

    Ok(TokenInfo {
        uid,
        access_token: new_access,
    })
}

/// [`ensure_valid_token`] 的返回，包含当前 uid + 有效的 access_token。
#[derive(Debug, Clone)]
pub struct TokenInfo {
    /// 当前登录用户的 Google sub（id_token 解出来的）；将来 purge_cloud_data 等命令会用到
    #[allow(dead_code)]
    pub uid: String,
    pub access_token: String,
}

#[derive(Debug, Deserialize)]
struct GoogleRefreshResp {
    access_token: String,
    expires_in: i64,
}

async fn refresh_with_google(
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<GoogleRefreshResp> {
    // 网络抖动 / Google 端 5xx 是临时错误：指数退避重试 2 次。
    // 4xx（401/400）是 refresh_token 真的失效——立刻返回，重试无意义。
    const BACKOFFS_MS: [u64; 2] = [500, 2000];

    let client = reqwest::Client::new();
    let mut attempt = 0usize;
    loop {
        let send_res = client
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("refresh_token", refresh_token),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await;

        match send_res {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return Ok(resp.json::<GoogleRefreshResp>().await?);
                }
                if status.is_server_error() && attempt < BACKOFFS_MS.len() {
                    log::warn!(
                        "refresh 端点返回 {}，{}ms 后重试（第 {} 次）",
                        status.as_u16(),
                        BACKOFFS_MS[attempt],
                        attempt + 1
                    );
                    tokio::time::sleep(Duration::from_millis(BACKOFFS_MS[attempt])).await;
                    attempt += 1;
                    continue;
                }
                let body = resp.text().await.unwrap_or_default();
                // 401/400 多半是 refresh_token 失效（用户在 myaccount.google.com 撤销了授权）
                return Err(Error::OAuthHttp {
                    endpoint: "refresh",
                    status: status.as_u16(),
                    body,
                });
            }
            Err(e) => {
                if attempt < BACKOFFS_MS.len() {
                    log::warn!(
                        "refresh 端点网络错误：{e}，{}ms 后重试（第 {} 次）",
                        BACKOFFS_MS[attempt],
                        attempt + 1
                    );
                    tokio::time::sleep(Duration::from_millis(BACKOFFS_MS[attempt])).await;
                    attempt += 1;
                    continue;
                }
                return Err(Error::from(e));
            }
        }
    }
}

/// 退出登录：清 DB `auth_state`。
///
/// 派生 key 方案下不需要也无法"删 key"——key 是从 machine_id + home 现算的，
/// 不在任何地方持久化。清掉 DB 里的 `refresh_token_enc` 就等于"忘记"凭证。
pub async fn sign_out(pool: &DbPool) -> Result<()> {
    clear_auth_state(pool).await
}

/// 清空 `auth_state` 表的所有 token 字段。给 [`sign_out`] 跟解密失败时的自动恢复共用。
async fn clear_auth_state(pool: &DbPool) -> Result<()> {
    pool.0
        .call(|conn| {
            conn.execute(
                "UPDATE auth_state SET uid = NULL, email = NULL,
                   refresh_token_enc = NULL, access_token = NULL, expires_at = NULL
                 WHERE id = 1",
                [],
            )
            .db()?;
            Ok(())
        })
        .await?;
    Ok(())
}

// ───────────── 内部：从 settings 读凭证 ─────────────

async fn load_creds(pool: &DbPool) -> Result<(String, String)> {
    let s = settings::load(pool).await.unwrap_or_default();
    let id = s.google_client_id.trim().to_string();
    let secret = s.google_client_secret.trim().to_string();
    if id.is_empty() || secret.is_empty() {
        return Err(Error::OAuthNotConfigured(
            "Google Client ID / Client Secret 未填：到设备页按指引配置后再登录".into(),
        ));
    }
    Ok((id, secret))
}

// ───────────── 内部：PKCE ─────────────

fn generate_verifier() -> String {
    // RFC 7636: 43-128 字符的 unreserved
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(64)
        .map(char::from)
        .collect()
}

fn derive_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn random_state() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(24)
        .map(char::from)
        .collect()
}

fn build_auth_url(client_id: &str, redirect_uri: &str, challenge: &str, state: &str) -> String {
    format!(
        "https://accounts.google.com/o/oauth2/v2/auth\
         ?client_id={cid}\
         &redirect_uri={ru}\
         &response_type=code\
         &scope={scope}\
         &code_challenge={ch}\
         &code_challenge_method=S256\
         &state={st}\
         &access_type=offline\
         &prompt=consent",
        cid = urlencoding::encode(client_id),
        ru = urlencoding::encode(redirect_uri),
        scope = urlencoding::encode(OAUTH_SCOPE),
        ch = urlencoding::encode(challenge),
        st = urlencoding::encode(state),
    )
}

/// 解 id_token (JWT) 的 payload，返回 (sub, email)。失败返回 None。
fn decode_id_token(id_token: &str) -> Option<(String, String)> {
    let mid = id_token.split('.').nth(1)?;
    let bytes = general_purpose::URL_SAFE_NO_PAD
        .decode(mid)
        .or_else(|_| general_purpose::URL_SAFE.decode(mid))
        .ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    let sub = v.get("sub").and_then(|x| x.as_str())?.to_string();
    let email = v
        .get("email")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    Some((sub, email))
}

// ───────────── 内部：loopback callback ─────────────

struct CodeState {
    code: String,
    state: String,
}

async fn accept_callback(listener: TcpListener) -> Result<CodeState> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // 浏览器会开投机性预连接（可能不发任何数据）、杀软/端口扫描也会碰这个端口。
    // 只 accept 一次会被这类连接吃掉，真正的 /callback 请求永远进不来。
    // 所以循环 accept：不带 code/error 的连接用 404 打发掉继续等；
    // 整体时限由调用方的 OAUTH_TIMEOUT_SECS 控制，单连接读加小超时防止
    // 挂着不发数据的预连接堵住队列。
    loop {
        let (mut socket, _) = listener
            .accept()
            .await
            .map_err(|e| Error::OAuthSetup(format!("accept callback: {e}")))?;

        let mut buf = vec![0u8; 4096];
        let n = match timeout(Duration::from_secs(5), socket.read(&mut buf)).await {
            Ok(Ok(n)) => n,
            // 读超时 / 对端重置：不是回调，等下一个连接
            _ => continue,
        };
        if n == 0 {
            continue; // 空连接（预连接 / 探测）
        }
        let req = String::from_utf8_lossy(&buf[..n]).to_string();
        let first = req.lines().next().unwrap_or("");
        // GET /callback?code=...&state=... HTTP/1.1
        let path = first.split_whitespace().nth(1).unwrap_or("");
        let query = path.split_once('?').map(|x| x.1).unwrap_or("");
        let mut code = String::new();
        let mut state = String::new();
        let mut error: Option<String> = None;
        for kv in query.split('&') {
            let (k, v) = match kv.split_once('=') {
                Some(p) => p,
                None => continue,
            };
            let dec = urlencoding::decode(v).unwrap_or_default().to_string();
            match k {
                "code" => code = dec,
                "state" => state = dec,
                "error" => error = Some(dec),
                _ => {}
            }
        }

        if error.is_none() && code.is_empty() {
            // 不是 OAuth 回调（favicon / 健康探测等），打发掉继续等真正的回调
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(resp.as_bytes()).await;
            let _ = socket.shutdown().await;
            continue;
        }

        let body = if let Some(ref e) = error {
            super::auth_callback::render(false, &super::auth_callback::html_escape(e))
        } else {
            super::auth_callback::render(true, "可以关闭此页，回到 Hindsight。")
        };
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = socket.write_all(resp.as_bytes()).await;
        let _ = socket.shutdown().await;

        if let Some(e) = error {
            return Err(Error::OAuthDenied(e));
        }
        return Ok(CodeState { code, state });
    }
}

// ───────────── 内部：HTTP 调用 ─────────────

#[derive(Debug, Deserialize)]
struct GoogleTokenResp {
    access_token: String,
    expires_in: i64,
    refresh_token: Option<String>,
    id_token: Option<String>,
}

async fn exchange_code(
    client_id: &str,
    client_secret: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<GoogleTokenResp> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("code_verifier", verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::OAuthHttp {
            endpoint: "token",
            status,
            body,
        });
    }
    Ok(resp.json::<GoogleTokenResp>().await?)
}

// ───────────── 内部：派生 key + AES-GCM ─────────────

/// 派生 32 字节 AES key：`SHA256(salt || machine_id || user_home)`。
///
/// 每次都现算，不持久化在任何 OS keyring / Keychain / 文件里。三个输入：
/// - `salt` = [`KEY_DERIVATION_SALT`]：域分隔常量
/// - `machine_id`：平台特定的稳定标识符（重装系统才变）
/// - `user_home`：[`dirs::home_dir`]，删用户账号才变
///
/// 见模块顶部说明的「为什么不再用 keyring」段落。
fn derive_master_key() -> Result<[u8; 32]> {
    let machine = read_machine_id()?;
    let user = read_user_home_bytes();
    let mut hasher = Sha256::new();
    hasher.update(KEY_DERIVATION_SALT);
    hasher.update(b"|machine|");
    hasher.update(&machine);
    hasher.update(b"|user|");
    hasher.update(&user);
    let result = hasher.finalize();
    let mut k = [0u8; 32];
    k.copy_from_slice(&result);
    Ok(k)
}

/// 用户身份：home 目录路径的 utf-8 字节。
/// Windows: `C:\Users\xxx`；macOS: `/Users/xxx`；Linux: `/home/xxx`。
/// 删 / 改用户账号才会变；HOME 临时被覆盖也无所谓——`dirs::home_dir` 在 Windows
/// 走 `KNOWNFOLDERID_Profile` SHGetKnownFolderPath，不依赖环境变量。
fn read_user_home_bytes() -> Vec<u8> {
    dirs::home_dir()
        .map(|p| {
            p.into_os_string()
                .to_string_lossy()
                .into_owned()
                .into_bytes()
        })
        .unwrap_or_default()
}

/// Windows 实现：读注册表 `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid`。
/// 该值在 Windows 安装时一次性生成，重装系统才变；任何用户都可读，OS 服务不会清。
#[cfg(target_os = "windows")]
fn read_machine_id() -> Result<Vec<u8>> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use winapi::shared::minwindef::HKEY;
    use winapi::um::winnt::{KEY_READ, KEY_WOW64_64KEY, REG_SZ};
    use winapi::um::winreg::{RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_LOCAL_MACHINE};

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    // SAFETY: 全部 winapi 入口都按 MSDN 文档传参；buf 长度足够 GUID 字符串（38 + null）。
    unsafe {
        let subkey = to_wide("SOFTWARE\\Microsoft\\Cryptography");
        let value = to_wide("MachineGuid");
        let mut hkey: HKEY = std::ptr::null_mut();
        // KEY_WOW64_64KEY：32 位进程跑在 64 位 Windows 时强制读 64 位视图，
        // 否则被 WOW64 重定向到 SOFTWARE\WOW6432Node 拿不到 MachineGuid
        let r = RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            0,
            KEY_READ | KEY_WOW64_64KEY,
            &mut hkey,
        );
        if r != 0 {
            return Err(Error::Other(format!(
                "RegOpenKeyExW HKLM\\SOFTWARE\\Microsoft\\Cryptography failed: {r}"
            )));
        }
        let mut buf = [0u16; 128];
        let mut size: u32 = std::mem::size_of_val(&buf) as u32;
        let mut ty: u32 = 0;
        let r = RegQueryValueExW(
            hkey,
            value.as_ptr(),
            std::ptr::null_mut(),
            &mut ty,
            buf.as_mut_ptr() as *mut u8,
            &mut size,
        );
        RegCloseKey(hkey);
        if r != 0 {
            return Err(Error::Other(format!(
                "RegQueryValueExW MachineGuid failed: {r}"
            )));
        }
        if ty != REG_SZ {
            return Err(Error::Other(format!(
                "MachineGuid 注册表值类型 {ty}，期望 REG_SZ"
            )));
        }
        // size 是字节数，含尾部 null。换算成 u16 数量并去掉 null。
        let chars = (size as usize / 2).saturating_sub(1);
        let s = String::from_utf16_lossy(&buf[..chars]);
        Ok(s.into_bytes())
    }
}

/// macOS 实现：从 IOKit 注册表读 `IOPlatformUUID`（板载唯一标识，主板换才变）。
/// 命令行 `ioreg -d2 -c IOPlatformExpertDevice` 是 Apple 自带的工具，每台 macOS 都有；
/// 不引第三方 IOKit binding crate，shell 解析最简单。
#[cfg(target_os = "macos")]
fn read_machine_id() -> Result<Vec<u8>> {
    let out = std::process::Command::new("ioreg")
        .args(["-d2", "-c", "IOPlatformExpertDevice"])
        .output()
        .map_err(|e| Error::Other(format!("spawn ioreg failed: {e}")))?;
    if !out.status.success() {
        return Err(Error::Other(format!(
            "ioreg exited with status {:?}",
            out.status.code()
        )));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    // 行格式：`    "IOPlatformUUID" = "ABC-123-..."`
    for line in stdout.lines() {
        if !line.contains("IOPlatformUUID") {
            continue;
        }
        // 第 4 个 `"` 切分 → 取 UUID 子串
        let parts: Vec<&str> = line.split('"').collect();
        if parts.len() >= 4 {
            let uuid = parts[3].trim();
            if !uuid.is_empty() {
                return Ok(uuid.as_bytes().to_vec());
            }
        }
    }
    Err(Error::Other(
        "ioreg 输出里没找到 IOPlatformUUID".to_string(),
    ))
}

/// Linux 实现：systemd 风格的 `/etc/machine-id`，无 systemd 的退化到 dbus 同款。
#[cfg(target_os = "linux")]
fn read_machine_id() -> Result<Vec<u8>> {
    std::fs::read_to_string("/etc/machine-id")
        .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))
        .map(|s| s.trim().as_bytes().to_vec())
        .map_err(|e| Error::Other(format!("read machine-id failed: {e}")))
}

/// 其它 unix（FreeBSD 等）：`/etc/machine-id` 走通就用，否则报错。
#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn read_machine_id() -> Result<Vec<u8>> {
    std::fs::read_to_string("/etc/machine-id")
        .map(|s| s.trim().as_bytes().to_vec())
        .map_err(|e| Error::Other(format!("read /etc/machine-id failed: {e}")))
}

fn aes_encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| Error::Crypto("aes encrypt"))?;

    // 输出格式：[12 字节 nonce][密文+tag]
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// AES-256-GCM 解密。`ciphertext` 头 12 字节是 nonce，剩下是 ciphertext+tag。
/// `key` 来自 [`derive_master_key`]——同一个 (machine_id, user_home) 组合永远给同一把 key。
fn aes_decrypt(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>> {
    if ciphertext.len() < 13 {
        return Err(Error::Crypto("ciphertext too short"));
    }
    let (nonce_bytes, ct) = ciphertext.split_at(12);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ct)
        .map_err(|_| Error::Crypto("aes decrypt"))
}
