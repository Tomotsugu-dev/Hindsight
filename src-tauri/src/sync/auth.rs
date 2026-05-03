//! Google OAuth Installed App (PKCE) → Firebase Identity Toolkit signInWithIdp。
//!
//! 流程：
//! 1. 生成 PKCE verifier/challenge
//! 2. 在 127.0.0.1 起一个一次性 HTTP listener，作为 OAuth redirect_uri
//! 3. 浏览器打开 Google 同意页（带 client_id / scope / code_challenge）
//! 4. 用户同意后跳回本地，listener 收到 ?code=xxx
//! 5. 用 code + code_verifier 调 https://oauth2.googleapis.com/token 拿 google id_token
//! 6. 用 google id_token 调 Firebase signInWithIdp 拿 Firebase idToken / refreshToken / localId(uid)
//! 7. 生成 32 字节 AES key 存 OS keyring；用此 key 加密 Firebase refreshToken，密文落 auth_state 表

use std::time::Duration;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::{engine::general_purpose, Engine as _};
use rand::distributions::Alphanumeric;
use rand::{Rng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;
use tokio::time::timeout;

use crate::error::{Error, Result};
use crate::repo::settings::Settings;
use crate::storage::DbPool;
use crate::sync::config::FullConfig;

const KEYRING_SERVICE: &str = "Hindsight";
const KEYRING_USER: &str = "auth_key_v1";
const OAUTH_SCOPE: &str = "openid email";
const OAUTH_TIMEOUT_SECS: u64 = 180;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthState {
    pub signed_in: bool,
    pub uid: Option<String>,
    pub email: Option<String>,
    /// 同步配置是否完整（client_id + secret + api_key 都有）
    pub configured: bool,
}

pub async fn current_state(pool: &DbPool, settings: &Settings) -> Result<AuthState> {
    let configured = FullConfig::from_settings(settings).is_some();
    let row: Option<(String, String)> = pool
        .0
        .call(|conn| {
            let r = conn
                .query_row(
                    "SELECT uid, email FROM auth_state WHERE id = 1",
                    [],
                    |r| Ok((
                        r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    )),
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
        email: if email.is_empty() { None } else { Some(email) },
        configured,
    })
}

/// 完整登录流程：返回登录后的 AuthState。
pub async fn sign_in_with_google(pool: &DbPool, settings: &Settings) -> Result<AuthState> {
    let cfg = FullConfig::from_settings(settings).ok_or_else(|| {
        Error::Other(
            "云同步凭证未填写：请到 设置 → 云同步 里填入你自己的 Firebase 项目的 Client ID / Client Secret / API Key。"
                .into(),
        )
    })?;

    // 1) PKCE
    let verifier = generate_verifier();
    let challenge = derive_challenge(&verifier);
    let state = random_state();

    // 2) loopback listener
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| Error::Other(format!("启动 OAuth 回调 server 失败: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::Other(format!("读取本地端口失败: {e}")))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    // 3) 打开浏览器
    let auth_url = build_auth_url(&cfg.google_client_id, &redirect_uri, &challenge, &state);
    if let Err(e) = open::that(&auth_url) {
        return Err(Error::Other(format!("打开浏览器失败: {e}")));
    }

    // 4) 等回调（最多 OAUTH_TIMEOUT_SECS 秒）
    let code_state = timeout(Duration::from_secs(OAUTH_TIMEOUT_SECS), accept_callback(listener))
        .await
        .map_err(|_| Error::Other("等待 OAuth 回调超时（3 分钟内未完成）".into()))??;
    if code_state.state != state {
        return Err(Error::Other("OAuth state 不匹配（可能被劫持）".into()));
    }

    // 5) code → google id_token
    let google = exchange_code(
        &cfg.google_client_id,
        &cfg.google_client_secret,
        &code_state.code,
        &verifier,
        &redirect_uri,
    )
    .await?;

    // 6) google id_token → Firebase
    let fb = sign_in_with_idp(&cfg.firebase_api_key, &google.id_token).await?;

    // 7) 加密存储
    let key = ensure_keyring_key()?;
    let enc = aes_encrypt(&key, fb.refresh_token.as_bytes())?;

    let uid = fb.local_id.clone();
    let email = fb.email.clone().unwrap_or_default();
    let access = fb.id_token.clone();
    let expires_at = chrono::Utc::now()
        + chrono::Duration::seconds(fb.expires_in.parse::<i64>().unwrap_or(3600));
    let expires_at_str = expires_at.to_rfc3339();
    pool.0
        .call(move |conn| {
            conn.execute(
                "UPDATE auth_state SET
                   uid = ?1, email = ?2, refresh_token_enc = ?3,
                   access_token = ?4, expires_at = ?5
                 WHERE id = 1",
                rusqlite::params![uid, email, enc, access, expires_at_str],
            )
            .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(())
        })
        .await?;

    log::info!("Firebase 登录成功 uid={}", fb.local_id);

    current_state(pool, settings).await
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
            .map_err(tokio_rusqlite::Error::Rusqlite)?;
            Ok(())
        })
        .await?;
    // 清掉 keyring 里的 key（让密文也读不出来，等于真正"忘记"）
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        let _ = entry.delete_credential();
    }
    Ok(())
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

// ───────────── 内部：loopback callback ─────────────

struct CodeState {
    code: String,
    state: String,
}

async fn accept_callback(listener: TcpListener) -> Result<CodeState> {
    let (mut socket, _) = listener
        .accept()
        .await
        .map_err(|e| Error::Other(format!("接受回调连接失败: {e}")))?;

    // 用 std blocking 风格的 read/write 比较方便（请求体很小）
    let mut buf = vec![0u8; 4096];
    let n = {
        use tokio::io::AsyncReadExt;
        socket
            .read(&mut buf)
            .await
            .map_err(|e| Error::Other(format!("读取请求失败: {e}")))?
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
        callback_html(false, &html_escape(e))
    } else {
        callback_html(true, "可以关闭此页，回到 Hindsight。")
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
        return Err(Error::Other(format!("Google 拒绝授权: {e}")));
    }
    if code.is_empty() {
        return Err(Error::Other("回调没有带 code".into()));
    }
    Ok(CodeState { code, state })
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 渲染 OAuth 回调页 HTML —— 配色与 src/styles/tokens.css 对齐：
/// - accent: #6c5ce7（深紫）
/// - 背景：sidebar 同款 lavender → pink → peach 径向渐变
/// - 字体 Inter / 中文 fallback；过渡曲线 cubic-bezier(0.22, 1, 0.36, 1)
fn callback_html(success: bool, message: &str) -> String {
    let title = if success { "登录成功" } else { "登录失败" };
    let (icon_color, icon_bg) = if success {
        ("#6c5ce7", "rgba(108, 92, 231, 0.13)")
    } else {
        ("#ef4444", "rgba(239, 68, 68, 0.12)")
    };
    let icon_svg = if success {
        // checkmark
        r#"<svg width="44" height="44" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>"#
    } else {
        // alert circle
        r#"<svg width="44" height="44" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>"#
    };
    format!(
        r#"<!doctype html>
<html lang="zh">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Hindsight · {title}</title>
<style>
  *,*::before,*::after {{ box-sizing: border-box; }}
  html, body {{ margin: 0; padding: 0; height: 100%; }}
  body {{
    font-family: "Inter", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
                 "Helvetica Neue", Arial, "PingFang SC", "Microsoft YaHei", sans-serif;
    background:
      radial-gradient(120% 80% at 0% 0%, #efe7ff 0%, #ffe9f0 60%, #fff4e6 100%);
    color: #1d1c25;
    min-height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 48px;
    -webkit-font-smoothing: antialiased;
    -moz-osx-font-smoothing: grayscale;
  }}
  .card {{
    width: 100%;
    max-width: 580px;
    background: #ffffff;
    border: 1px solid rgba(20, 20, 40, 0.06);
    border-radius: 28px;
    box-shadow:
      0 1px 0 rgba(255, 255, 255, 0.7) inset,
      0 0 0 1px rgba(255, 255, 255, 0.5) inset,
      0 16px 48px rgba(20, 20, 40, 0.10),
      0 3px 8px rgba(20, 20, 40, 0.04);
    padding: 50px 44px 40px;
    text-align: center;
    animation: rise 360ms cubic-bezier(0.22, 1, 0.36, 1);
  }}
  @keyframes rise {{
    from {{ opacity: 0; transform: translateY(12px); }}
    to   {{ opacity: 1; transform: translateY(0);   }}
  }}
  .badge {{
    width: 80px;
    height: 80px;
    margin: 0 auto 22px;
    border-radius: 22px;
    background: {icon_bg};
    color: {icon_color};
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }}
  h1 {{
    font-size: 26px;
    font-weight: 650;
    color: #1d1c25;
    margin: 0 0 10px;
    letter-spacing: -0.01em;
  }}
  p {{
    margin: 0;
    font-size: 18px;
    line-height: 1.55;
    color: #6b6680;
    word-break: break-word;
  }}
  .brand {{
    margin-top: 32px;
    padding-top: 22px;
    border-top: 1px solid rgba(20, 20, 40, 0.06);
    font-size: 15px;
    color: #9a96aa;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    font-weight: 550;
  }}
</style>
</head>
<body>
  <div class="card">
    <div class="badge">{icon_svg}</div>
    <h1>{title}</h1>
    <p>{message}</p>
    <div class="brand">Hindsight</div>
  </div>
</body>
</html>"#,
        title = title,
        icon_color = icon_color,
        icon_bg = icon_bg,
        icon_svg = icon_svg,
        message = message,
    )
}

// ───────────── 内部：HTTP 调用 ─────────────

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct GoogleTokenResp {
    id_token: String,
    #[allow(dead_code)]
    refresh_token: Option<String>,
    #[allow(dead_code)]
    expires_in: i64,
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
        .map_err(|e| Error::Other(format!("调用 Google token 接口失败: {e}")))?;
    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Other(format!(
            "Google token 接口返回 {s}: {body}"
        )));
    }
    resp.json::<GoogleTokenResp>()
        .await
        .map_err(|e| Error::Other(format!("解析 Google token 响应失败: {e}")))
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
struct FirebaseIdpResp {
    #[serde(rename = "idToken")]
    id_token: String,
    #[serde(rename = "refreshToken")]
    refresh_token: String,
    #[serde(rename = "localId")]
    local_id: String,
    #[serde(rename = "expiresIn")]
    expires_in: String,
    email: Option<String>,
}

async fn sign_in_with_idp(api_key: &str, google_id_token: &str) -> Result<FirebaseIdpResp> {
    let client = reqwest::Client::new();
    let post_body = format!("id_token={google_id_token}&providerId=google.com");
    let payload = serde_json::json!({
        "postBody": post_body,
        "requestUri": "http://localhost",
        "returnSecureToken": true,
        "returnIdpCredential": false,
    });
    let url = format!(
        "https://identitytoolkit.googleapis.com/v1/accounts:signInWithIdp?key={api_key}"
    );
    let resp = client
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| Error::Other(format!("调用 Firebase signInWithIdp 失败: {e}")))?;
    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(Error::Other(format!("Firebase signInWithIdp 返回 {s}: {body}")));
    }
    resp.json::<FirebaseIdpResp>()
        .await
        .map_err(|e| Error::Other(format!("解析 Firebase 响应失败: {e}")))
}

// ───────────── 内部：keyring + AES-GCM ─────────────

/// 拿到 OS keyring 里的 32 字节 AES key；没有就生成并落盘。
fn ensure_keyring_key() -> Result<[u8; 32]> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| Error::Other(format!("打开 keyring 失败: {e}")))?;

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
        .map_err(|e| Error::Other(format!("写 keyring 失败: {e}")))?;
    Ok(k)
}

fn aes_encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| Error::Other(format!("AES 加密失败: {e}")))?;

    // 输出格式：[12 字节 nonce][密文+tag]
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

#[allow(dead_code)]
pub fn aes_decrypt(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>> {
    if ciphertext.len() < 13 {
        return Err(Error::Other("密文太短".into()));
    }
    let (nonce_bytes, ct) = ciphertext.split_at(12);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher
        .decrypt(nonce, ct)
        .map_err(|e| Error::Other(format!("AES 解密失败: {e}")))
}

