use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

use crate::account;
use crate::storage::{db_path_for, migrations, DbPool};
use crate::sync::backend_switch::{
    self, decide_connect_action, switch_backend, update_credentials, ConnectAction, NewBackend,
};
use crate::sync::cloud::CloudBackend;
use crate::sync::drive::auth::{self, AuthState};
use crate::sync::engine::SyncEngine;
use crate::sync::webdav::{self, account_hash::account_hash};

/// The event that sends the Google authorization link to the frontend once it
/// is ready, as `{ url, opened }`. When the browser does not open, the frontend
/// shows "Copy sign-in link" so the user can open it themselves. `opened` false
/// means opening the browser failed: show it at once. `opened` true does not
/// guarantee a window appeared: show it if sign-in has not finished after a few
/// seconds.
pub const OAUTH_URL_EVENT: &str = "sync://oauth-url";

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OauthUrlPayload {
    url: String,
    opened: bool,
}

/// The Google sign-in state: whether signed in, the uid and email, and whether
/// Google sign-in is configured.
#[tauri::command]
pub async fn auth_status(pool: State<'_, DbPool>) -> Result<AuthState, String> {
    auth::current_state(&pool).await.map_err(Into::into)
}

/// Signs in with Google (a local listener and the browser's consent page), then
/// switches to that Google account (ADR-0011 §1). The authorization URL is sent
/// to the frontend through [`OAUTH_URL_EVENT`], for when the browser does not
/// open. Switching to another account's database restarts the app and does not
/// return.
#[tauri::command]
pub async fn sign_in_with_google(
    app: AppHandle,
    pool: State<'_, DbPool>,
    engine: State<'_, Arc<SyncEngine>>,
) -> Result<AuthState, String> {
    let google = auth::sign_in_with_google(&pool, |url, opened| {
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
    let to = NewBackend::Drive {
        uid: &google.uid,
        email: &google.email,
        refresh_token: &google.refresh_token,
        access_token: &google.access_token,
        expires_at: &google.expires_at,
    };
    let cloud = CloudBackend::drive((*pool).clone());
    switch_to_account(&app, &pool, &engine, to, &google.uid, cloud).await?;
    auth::current_state(&pool).await.map_err(Into::into)
}

/// Signs out of the current sync backend, Google Drive or WebDAV.
#[tauri::command]
pub async fn sign_out(pool: State<'_, DbPool>) -> Result<(), String> {
    backend_switch::sign_out(&pool).await.map_err(Into::into)
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
