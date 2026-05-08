//! 本地 llama-server binary 的下载 / 删除 / 打开目录命令（Phase 1B-α）。
//!
//! `download_binary` 触发后通过 [`ENGINE_DOWNLOAD_PROGRESS_EVENT`] 流式推送进度给前端；
//! 命令本身在下载完成（或失败）后才 resolve。
//! 整目录覆盖删除前都会主动 stop 引擎，避免 Windows 锁文件。

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::ai::binary::{self, DownloadPhase};
use crate::ai::server::EngineSupervisor;

/// 进度事件 payload。前端 listen 这条事件名拿到。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadProgressPayload {
    phase: DownloadPhase,
    /// 已下载字节数；只在 phase=downloading 有意义，其它阶段是 0
    downloaded: u64,
    /// 总字节数；服务端不一定给，所以是 Option
    total: Option<u64>,
}

const ENGINE_DOWNLOAD_PROGRESS_EVENT: &str = "ai://engine-download-progress";

/// 触发 llama-server binary 下载。
///
/// 进度通过 [`ENGINE_DOWNLOAD_PROGRESS_EVENT`] 事件流式推送给前端，命令本身在下载结束后才 resolve。
/// 失败时返回错误字符串（前端 toast 展示）；同时事件流里**不会**再有 Done 信号，
/// 调用方应同时监听命令返回值和 Done 事件，按更早到的那个判定。
///
/// 下载前会主动 stop 当前引擎实例——Windows 上 .exe 在运行时无法被覆盖 / 删除，
/// 不停先就会让 download() 内部 `remove_dir_all` 失败。
#[tauri::command]
pub async fn download_binary(
    app: AppHandle,
    supervisor: State<'_, Arc<EngineSupervisor>>,
) -> Result<(), String> {
    if let Err(e) = supervisor.stop().await {
        log::warn!("download_binary 前 stop 引擎失败（可能本就没跑）: {e}");
    }
    let app_for_emit = app.clone();
    binary::download(move |phase, downloaded, total| {
        let payload = DownloadProgressPayload {
            phase,
            downloaded,
            total,
        };
        if let Err(e) = app_for_emit.emit(ENGINE_DOWNLOAD_PROGRESS_EVENT, &payload) {
            // emit 失败不致命，后端仍继续干活；只是前端会丢这一帧进度
            log::warn!("emit {ENGINE_DOWNLOAD_PROGRESS_EVENT} 失败: {e}");
        }
    })
    .await
    .map_err(Into::into)
}

/// 删除已安装的 binary（platform_dir 整个目录抹掉）。
///
/// 同样要先 stop 引擎，否则 Windows 锁住 .exe 删不掉。
#[tauri::command]
pub async fn delete_binary(supervisor: State<'_, Arc<EngineSupervisor>>) -> Result<(), String> {
    if let Err(e) = supervisor.stop().await {
        log::warn!("delete_binary 前 stop 引擎失败（可能本就没跑）: {e}");
    }
    binary::delete().await.map_err(Into::into)
}

/// 在系统文件管理器里打开 binary 所在目录。
///
/// 用 `open` crate（已在依赖里）调系统默认 file manager。
/// 未安装时也能开——目录可能存在（之前下载残留）但文件不全。
#[tauri::command]
pub async fn open_engine_dir() -> Result<(), String> {
    let bin = binary::binary_path().map_err(String::from)?;
    let dir = bin
        .parent()
        .ok_or_else(|| "binary 路径无父目录".to_string())?;
    open::that(dir).map_err(|e| format!("打开目录失败：{e}"))?;
    Ok(())
}
