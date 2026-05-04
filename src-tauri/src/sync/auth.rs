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
//! 7. 生成 32 字节 AES key 存 OS keyring；用此 key 加密 refresh_token，密文落 auth_state 表

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
use crate::db::SqliteResultExt;

const KEYRING_SERVICE: &str = "Hindsight";
const KEYRING_USER: &str = "auth_key_v1";
const OAUTH_SCOPE: &str = "openid email https://www.googleapis.com/auth/drive.appdata";
const OAUTH_TIMEOUT_SECS: u64 = 180;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthState {
    pub signed_in: bool,
    pub uid: Option<String>,
    pub email: Option<String>,
    /// Google OAuth client_id / client_secret 是否齐全（决定 UI 上"用 Google 登录"按钮是否可点）
    pub configured: bool,
}

pub async fn current_state(pool: &DbPool) -> Result<AuthState> {
    let cfg = settings::load(pool).await.unwrap_or_default();
    let configured =
        !cfg.google_client_id.trim().is_empty() && !cfg.google_client_secret.trim().is_empty();

    let row: Option<(String, String)> = pool
        .0
        .call(|conn| {
            let r = conn
                .query_row(
                    "SELECT uid, email FROM auth_state WHERE id = 1",
                    [],
                    |r| {
                        Ok((
                            r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                            r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        ))
                    },
                )
                .ok();
            Ok(r)
        })
        .await?;
    let (uid, email) = row.unwrap_or_default();
    let signed_in = !uid.is_empty();
    Ok(AuthState {
        signed_in,
        uid: if uid.is_empty() { None } else { Some(uid) },
        email: if email.is_empty() {
            None
        } else {
            Some(email)
        },
        configured,
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
    let code_state =
        timeout(Duration::from_secs(OAUTH_TIMEOUT_SECS), accept_callback(listener))
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

    let refresh_token = google.refresh_token.ok_or(Error::OAuthMissingRefreshToken)?;

    // 7) 加密存储
    let key = ensure_keyring_key()?;
    let enc = aes_encrypt(&key, refresh_token.as_bytes())?;

    let access = google.access_token.clone();
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(google.expires_in);
    let expires_at_str = expires_at.to_rfc3339();
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

    log::info!("Google 登录成功 uid={uid}");

    current_state(pool).await
}

/// 拿到一个有效的 Google access_token，过期就用 refresh_token 自动续。
/// 同时返回当前 uid，方便上层路由。
pub async fn ensure_valid_token(pool: &DbPool) -> Result<TokenInfo> {
    let row: Option<(Option<String>, Option<Vec<u8>>, Option<String>, Option<String>)> = pool
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

    let Some((Some(uid), Some(rt_enc), Some(access), Some(expires_at))) = row else {
        return Err(Error::NotSignedIn);
    };

    // 还在有效期 + 5 分钟以上缓冲 → 直接复用
    if let Ok(exp) = chrono::DateTime::parse_from_rfc3339(&expires_at) {
        let now = chrono::Utc::now();
        let exp_utc = exp.with_timezone(&chrono::Utc);
        if exp_utc - now > chrono::Duration::minutes(5) {
            return Ok(TokenInfo {
                uid,
                access_token: access,
            });
        }
    }

    // 续期
    let (client_id, client_secret) = load_creds(pool).await?;
    let key = ensure_keyring_key()?;
    let rt_bytes = aes_decrypt(&key, &rt_enc)?;
    let refresh_token =
        String::from_utf8(rt_bytes).map_err(|_| Error::Crypto("refresh_token utf-8 decode"))?;

    log::debug!("access_token 已过期，调 oauth2 端点续期…");
    let fresh = refresh_with_google(&client_id, &client_secret, &refresh_token).await?;

    let new_access = fresh.access_token.clone();
    let new_expires = (chrono::Utc::now() + chrono::Duration::seconds(fresh.expires_in)).to_rfc3339();
    let new_access_db = new_access.clone();
    let new_expires_db = new_expires.clone();
    pool.0
        .call(move |conn| {
            conn.execute(
                "UPDATE auth_state SET access_token = ?1, expires_at = ?2 WHERE id = 1",
                rusqlite::params![new_access_db, new_expires_db],
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
    let client = reqwest::Client::new();
    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        // 401/400 多半是 refresh_token 失效（用户在 myaccount.google.com 撤销了授权）
        return Err(Error::OAuthHttp { endpoint: "refresh", status, body });
    }
    Ok(resp.json::<GoogleRefreshResp>().await?)
}

pub async fn sign_out(pool: &DbPool) -> Result<()> {
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
    // 清掉 keyring 里的 key（让密文也读不出来，等于真正"忘记"）
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        let _ = entry.delete_credential();
    }
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
    let (mut socket, _) = listener
        .accept()
        .await
        .map_err(|e| Error::OAuthSetup(format!("accept callback: {e}")))?;

    let mut buf = vec![0u8; 4096];
    let n = {
        use tokio::io::AsyncReadExt;
        socket
            .read(&mut buf)
            .await
            .map_err(|e| Error::OAuthSetup(format!("read callback: {e}")))?
    };
    let req = String::from_utf8_lossy(&buf[..n]).to_string();
    let first = req.lines().next().unwrap_or("");
    // GET /callback?code=...&state=... HTTP/1.1
    let path = first.split_whitespace().nth(1).unwrap_or("");
    let query = path.splitn(2, '?').nth(1).unwrap_or("");
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
    {
        use tokio::io::AsyncWriteExt;
        let _ = socket.write_all(resp.as_bytes()).await;
        let _ = socket.shutdown().await;
    }

    if let Some(e) = error {
        return Err(Error::OAuthDenied(e));
    }
    if code.is_empty() {
        return Err(Error::OAuthMissingCode);
    }
    Ok(CodeState { code, state })
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
        return Err(Error::OAuthHttp { endpoint: "token", status, body });
    }
    Ok(resp.json::<GoogleTokenResp>().await?)
}

// ───────────── 内部：keyring + AES-GCM ─────────────

/// 拿到 OS keyring 里的 32 字节 AES key；没有就生成并落盘。
fn ensure_keyring_key() -> Result<[u8; 32]> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| Error::Keyring(format!("open: {e}")))?;

    if let Ok(s) = entry.get_password() {
        if let Ok(bytes) = general_purpose::STANDARD.decode(s.as_bytes()) {
            if bytes.len() == 32 {
                let mut k = [0u8; 32];
                k.copy_from_slice(&bytes);
                return Ok(k);
            }
        }
    }

    let mut k = [0u8; 32];
    OsRng.fill_bytes(&mut k);
    let s = general_purpose::STANDARD.encode(k);
    entry
        .set_password(&s)
        .map_err(|e| Error::Keyring(format!("write: {e}")))?;
    Ok(k)
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

pub fn aes_decrypt(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>> {
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
