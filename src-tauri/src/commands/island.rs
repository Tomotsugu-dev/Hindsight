//! 岛的页面调用的命令。
//!
//! 前三个是同步命令：Tauri 在主线程上执行同步命令，岛只能在主线程上读写。

use tauri::{AppHandle, State};

use crate::island::content::Content;
use crate::island::{self, Layout, PanelSize};
use crate::plugins;
use crate::storage::DbPool;

/// 见 [`island::content`]。
#[tauri::command]
pub fn island_content() -> Option<Content> {
    island::content()
}

/// 见 [`island::measured`]。
#[tauri::command]
pub fn island_measured(
    left: Option<f64>,
    right: Option<f64>,
    panel: Option<PanelSize>,
) -> Option<Layout> {
    island::measured(left, right, panel)
}

/// 见 [`island::ready`]。
#[tauri::command]
pub fn island_ready() {
    island::ready();
}

/// 见 [`plugins::island::clicked`]。
#[tauri::command]
pub async fn island_clicked(
    app: AppHandle,
    pool: State<'_, DbPool>,
    button: Option<String>,
) -> Result<(), String> {
    plugins::island::clicked(&app, &pool, button)
        .await
        .map_err(Into::into)
}
