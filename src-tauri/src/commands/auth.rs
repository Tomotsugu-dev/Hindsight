use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

use crate::storage::DbPool;
use crate::sync::auth::{self, AuthState};
use crate::sync::engine::SyncEngine;

/// OAuth 授权 URL 就绪事件:payload = { url, opened }。
/// opened=false → 前端立即显示「复制登录链接」;true → 等几秒未完成再显示兜底。
pub const OAUTH_URL_EVENT: &str = "sync://oauth-url";

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OauthUrlPayload {
    url: String,
    opened: bool,
}

/// 拉当前 OAuth 登录状态（已登录 uid + email + 最后刷新时间，或 NotSignedIn）。
#[tauri::command]
pub async fn auth_status(pool: State<'_, DbPool>) -> Result<AuthState, String> {
    auth::current_state(&pool).await.map_err(Into::into)
}

/// 触发 Google OAuth 登录（弹本地 listener + 浏览器同意页）。
/// 授权 URL 生成后经 [`OAUTH_URL_EVENT`] 推给前端(浏览器没弹出来时的手动兜底)。
/// 成功后清掉同步引擎里残留的"凭证失效"错误，让 status 立刻刷新成正常。
#[tauri::command]
pub async fn sign_in_with_google(
    app: AppHandle,
    pool: State<'_, DbPool>,
    engine: State<'_, Arc<SyncEngine>>,
) -> Result<AuthState, String> {
    let next = auth::sign_in_with_google(&pool, |url, opened| {
        let payload = OauthUrlPayload {
            url: url.to_string(),
            opened,
        };
        if let Err(e) = app.emit(OAUTH_URL_EVENT, &payload) {
            log::warn!("emit {OAUTH_URL_EVENT} 失败: {e}");
        }
    })
    .await
    .map_err(String::from)?;
    // 登录成功 = 拿到新 token，旧的 "登录凭证失效" 错误立刻作废
    engine.clear_last_error().await;
    Ok(next)
}

/// 退出登录：清 DB 里的 auth_state（refresh_token_enc / access_token / expires_at 全置 NULL）。
/// 派生 key 方案下 key 不持久化，无需另删 keyring。
/// 多账号场景：同时清掉 active_user.json 让下次启动不再绑定该 uid。
#[tauri::command]
pub async fn sign_out(pool: State<'_, DbPool>) -> Result<(), String> {
    auth::sign_out(&pool).await.map_err(Into::into)
}

/// 重启 app —— 切账号后用：active_user.json 已指向新 uid，重启后 db_path() 自动切到新 DB。
#[tauri::command]
pub fn restart_app(app: AppHandle) {
    app.restart();
}
