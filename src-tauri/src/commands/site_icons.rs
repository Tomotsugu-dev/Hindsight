use std::collections::HashMap;

use tauri::State;

use crate::icons::site::{self, DownloadRound, ReqwestHttp};
use crate::storage::DbPool;

/// Returns the downloaded icons as host to file path, for the frontend to turn into image
/// URLs with `convertFileSrc`.
#[tauri::command]
pub async fn get_site_icons() -> Result<HashMap<String, String>, String> {
    Ok(site::icon_paths_by_host(
        &site::dir().map_err(String::from)?,
    ))
}

/// Downloads icons for some of `hosts` while the switch is on (ADR-0014). The frontend passes
/// the websites in the order it shows them and calls again while `remaining` is above 0.
#[tauri::command]
pub async fn download_site_icons(
    pool: State<'_, DbPool>,
    hosts: Vec<String>,
) -> Result<DownloadRound, String> {
    let settings = crate::repo::settings::load(&pool)
        .await
        .map_err(String::from)?;
    if !settings.download_site_icons {
        return Ok(DownloadRound::default());
    }
    let http = ReqwestHttp::new().map_err(String::from)?;
    let dir = site::dir().map_err(String::from)?;
    Ok(site::download_round(&http, &dir, &hosts).await)
}
