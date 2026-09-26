use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

use crate::account;
use crate::storage::{db_path_for, migrations, DbPool};
use crate::sync::backend_switch::{
    decide_connect_action, switch_backend, update_credentials, ConnectAction, NewBackend,
};
use crate::sync::cloud::CloudBackend;
use crate::sync::drive::auth::{self, AuthState};
use crate::sync::engine::SyncEngine;
use crate::sync::webdav::{self, account_hash::account_hash};

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

/// Connects to a WebDAV account (ADR-0011 §1). Switching to another account's
/// database restarts the app and does not return.
#[tauri::command]
pub async fn connect_webdav(
    app: AppHandle,
    pool: State<'_, DbPool>,
    engine: State<'_, Arc<SyncEngine>>,
    server_url: String,
    user: String,
    password: String,
) -> Result<(), String> {
    webdav::test_login(&server_url, &user, &password).await?;
    let self_id = engine.self_id().to_string();
    let cloud = CloudBackend::webdav(&server_url, (*pool).clone(), self_id)?;
    let to = NewBackend::WebDav {
        server_url: &server_url,
        user: &user,
        password: &password,
    };
    let db_uid = account_hash(&server_url, &user)?;
    switch_to_account(&app, &pool, &engine, to, &db_uid, cloud).await
}

/// Switches to the account in `to` (ADR-0011 §1). `db_uid` is the uid in that
/// account's database file name. `cloud` is the new backend, which the caller
/// builds before anything is written. Switching to another account's database
/// restarts the app and does not return.
async fn switch_to_account(
    app: &AppHandle,
    pool: &DbPool,
    engine: &SyncEngine,
    to: NewBackend<'_>,
    db_uid: &str,
    cloud: CloudBackend,
) -> Result<(), String> {
    let active_uid = account::active_uid();
    let action = decide_connect_action(pool, active_uid.as_deref(), &to).await?;
    if action == ConnectAction::UpdateCredentials {
        update_credentials(pool, to).await?;
        engine.clear_last_error().await;
        return Ok(());
    }

    let _paused = engine.pause_flushes().await;
    match action {
        ConnectAction::UseThisDatabase => switch_backend(pool, engine.self_id(), to).await?,
        ConnectAction::ClaimThisDatabase => {
            switch_backend(pool, engine.self_id(), to).await?;
            account::set_active_uid(Some(db_uid)).map_err(|e| e.to_string())?;
            account::claim_legacy_for(db_uid).map_err(|e| e.to_string())?;
        }
        ConnectAction::SwitchDatabase => {
            let other = DbPool::open(&db_path_for(Some(db_uid))?).await?;
            migrations::run(&other).await?;
            switch_backend(&other, engine.self_id(), to).await?;
            account::set_active_uid(Some(db_uid)).map_err(|e| e.to_string())?;
            app.restart()
        }
        ConnectAction::UpdateCredentials => {}
    }
    engine.replace_cloud(cloud);
    engine.clear_last_error().await;
    Ok(())
}
