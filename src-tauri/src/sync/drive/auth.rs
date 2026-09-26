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
//! 7. 返回账号和 token；refresh_token 加密后写进 auth_state 表，由换后端的事务做
//!    （`backend_switch`）

use std::time::Duration;

use base64::{engine::general_purpose, Engine as _};
use rand::distributions::Alphanumeric;
use rand::Rng;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::time::timeout;

use crate::error::{Error, Result};
use crate::repo::settings;
use crate::storage::DbPool;
use crate::storage::SqliteResultExt;
use crate::sync::local_key::{aes_decrypt, derive_master_key};

const OAUTH_SCOPE: &str = "openid email https://www.googleapis.com/auth/drive.appdata";
const OAUTH_TIMEOUT_SECS: u64 = 180;

/// What the Devices page shows: whether anyone is signed in, which account, and
/// whether the "Sign in with Google" button is enabled. Returned by `auth_status`
/// and when sign-in completes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthState {
    pub signed_in: bool,
    pub uid: Option<String>,
    pub email: Option<String>,
    /// Google OAuth client_id / client_secret 是否齐全（决定 UI 上"用 Google 登录"按钮是否可点）
    pub configured: bool,
}

/// Get the current authentication state from the local database.
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
    })
}

pub struct GoogleSignIn {
    pub uid: String,
    pub email: String,
    pub refresh_token: String,
    pub access_token: String,
    pub expires_at: String,
}

/// 走完 Google 登录，返回账号和 token；写库由换后端的事务做（ADR-0011 §2）。
///
/// `on_url(auth_url, opened)`:授权 URL 生成后回调一次——`opened=false` 表示
/// 打开浏览器的调用已失败,前端应立即显示「复制登录链接」;`opened=true` 也
/// **不保证浏览器可见**(ShellExecute 成功 ≠ handler 真的弹了窗:默认浏览器
/// 关联损坏/提权进程/安全软件拦截都静默),前端等几秒仍未完成时同样给出兜底。
/// 打开失败**不中断流程**——回调监听照常等,用户用任何浏览器打开链接都能完成。
pub async fn sign_in_with_google(
    pool: &DbPool,
    on_url: impl FnOnce(&str, bool),
) -> Result<GoogleSignIn> {
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

    // 3) 打开浏览器(失败只降级为手动链接,不中断)
    let auth_url = build_auth_url(&client_id, &redirect_uri, &challenge, &state);
    let opened = match open::that(&auth_url) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("打开浏览器失败(等待用户手动打开链接): {e}");
            false
        }
    };
    on_url(&auth_url, opened);

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
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(google.expires_in);

    Ok(GoogleSignIn {
        uid,
        email,
        refresh_token,
        access_token: google.access_token,
        expires_at: expires_at.to_rfc3339(),
    })
}

/// Returns an access token that, by the local clock, stays valid for at least
/// 10 more minutes. If it is closer to expiry, a new one is fetched from Google
/// and written to the database first.
///
/// The margin lets the Drive calls made with the token all finish on it, and
/// absorbs a small difference between the local clock and Google's. Callers
/// that do not handle 401 rely on it entirely.
pub async fn ensure_valid_token(pool: &DbPool) -> Result<TokenInfo> {
    let (uid, rt_enc, access, expires_at) = read_auth_state(pool).await?;

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

    log::debug!("access token is about to expire or has expired, refreshing it");
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

/// Partial state is never repaired: writing and clearing are each one UPDATE
/// covering all five columns, so a missing one can only mean damaged data.
async fn read_auth_state(pool: &DbPool) -> Result<(String, Vec<u8>, String, String)> {
    #[allow(clippy::type_complexity)]
    let row: Option<(
        Option<String>,
        Option<Vec<u8>>,
        Option<String>,
        Option<String>,
    )> = pool
        .0
        .call(|conn| {
            conn.query_row(
                "SELECT uid, refresh_token_enc, access_token, expires_at
                    FROM auth_state WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .db()
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
    // A dropped connection or a 5xx is Google having a bad moment, so back off
    // and try twice more. A 400 or 401 means the refresh token itself is dead;
    // retrying it changes nothing, so give up at once.
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
                    operation: "refresh",
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
                // 重试耗尽仍连不上 = 连接层问题（非 Google 拒绝），给带自救指引的文案
                return Err(Error::OAuthUnreachable { source: e });
            }
        }
    }
}

/// Called when this machine cannot decrypt the refresh token: clears the Google
/// sign-in, so the Devices page shows "not signed in" and the user signs in
/// again.
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
        .await
        // send 层失败 = 连接根本没建立（DNS/TCP/TLS），不是 Google 拒绝——
        // 受限网络下"浏览器授权成功、客户端换 token 失败"就是这条路径
        .map_err(|e| Error::OAuthUnreachable { source: e })?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::OAuthHttp {
            operation: "token",
            status,
            body,
        });
    }
    Ok(resp.json::<GoogleTokenResp>().await?)
}
