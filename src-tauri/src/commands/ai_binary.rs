//! 本地 llama-server binary 的下载 / 删除 / 打开目录命令（Phase 1B-α）。
//!
//! `download_binary` 触发后通过 [`ENGINE_DOWNLOAD_PROGRESS_EVENT`] 流式推送进度给前端；
//! 命令本身在下载完成（或失败）后才 resolve。
//! 整目录覆盖删除前都会主动 stop 引擎，避免 Windows 锁文件。

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::ai::binary::{self, DownloadPhase};
use crate::ai::embedding_runtime;
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
    /// "engine"（llama.cpp binary）或 "runtime"（onnxruntime dylib）。
    /// 让前端知道当前在下两阶段中的哪一段，可显示不同的提示文字。
    stage: &'static str,
}

const ENGINE_DOWNLOAD_PROGRESS_EVENT: &str = "ai://engine-download-progress";

/// 触发 AI 引擎下载——分两阶段：
///   1. llama.cpp binary（按平台 ~50-150MB）
///   2. onnxruntime dylib（~30MB，给截图相似度去重 embedding 用）
///
/// 阶段切换通过事件 payload 的 `stage` 字段告诉前端，进度条文案随之切换。
/// 阶段 1 失败立即返回；阶段 2 失败时阶段 1 的产物已落盘，下次重跑会重下两份。
///
/// 进度通过 [`ENGINE_DOWNLOAD_PROGRESS_EVENT`] 事件流式推送给前端，命令本身在两阶段
/// 都结束后才 resolve。失败时返回错误字符串（前端 toast 展示）。
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

    // 阶段 1：llama.cpp binary
    let app_for_engine = app.clone();
    binary::download(move |phase, downloaded, total| {
        emit_progress(&app_for_engine, phase, downloaded, total, "engine");
    })
    .await
    .map_err(String::from)?;

    // 阶段 2：onnxruntime
    let app_for_runtime = app.clone();
    embedding_runtime::download(move |phase, downloaded, total| {
        emit_progress(&app_for_runtime, phase, downloaded, total, "runtime");
    })
    .await
    .map_err(Into::into)
}

fn emit_progress(
    app: &AppHandle,
    phase: DownloadPhase,
    downloaded: u64,
    total: Option<u64>,
    stage: &'static str,
) {
    let payload = DownloadProgressPayload {
        phase,
        downloaded,
        total,
        stage,
    };
    if let Err(e) = app.emit(ENGINE_DOWNLOAD_PROGRESS_EVENT, &payload) {
        // emit 失败不致命，后端仍继续干活；只是前端会丢这一帧进度
        log::warn!("emit {ENGINE_DOWNLOAD_PROGRESS_EVENT} 失败: {e}");
    }
}

/// 删除已安装的 binary + onnxruntime（"AI 引擎"是 binary + runtime 一对）。
///
/// 同样要先 stop 引擎，否则 Windows 锁住 .exe 删不掉。
/// onnxruntime 删除失败仅 warn 不致命——可能因为 ort 已经 dlopen 了 dll 而 Windows 拒
/// unlink，那时让用户重启 app 后再删。
#[tauri::command]
pub async fn delete_binary(supervisor: State<'_, Arc<EngineSupervisor>>) -> Result<(), String> {
    if let Err(e) = supervisor.stop().await {
        log::warn!("delete_binary 前 stop 引擎失败（可能本就没跑）: {e}");
    }
    binary::delete().await.map_err(String::from)?;
    if let Err(e) = embedding_runtime::delete().await {
        log::warn!(
            "delete_binary：onnxruntime 删除失败（可能 dll 已被 dlopen，重启 app 后再试）: {e}"
        );
    }
    Ok(())
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
