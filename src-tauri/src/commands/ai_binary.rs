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

/// 下载 llama.cpp 引擎(按平台 ~50-500MB)。**只管 llama.cpp**——OCR 的
/// onnxruntime 是独立组件,走 [`download_ocr_runtime`]。
///
/// 已安装且版本与 PIN 一致时幂等快速返回;`force=true`(「重新下载」按钮)
/// 强制重下,修复损坏安装用。
///
/// 进度通过 [`ENGINE_DOWNLOAD_PROGRESS_EVENT`] 事件流式推送(stage="engine"),
/// 命令在下载结束后才 resolve。失败时返回错误字符串(前端 toast 展示)。
///
/// 下载前会主动 stop 当前引擎实例——Windows 上 .exe 在运行时无法被覆盖 / 删除,
/// 不停先就会让 download() 内部 `remove_dir_all` 失败。
#[tauri::command]
pub async fn download_binary(
    app: AppHandle,
    supervisor: State<'_, Arc<EngineSupervisor>>,
    force: Option<bool>,
) -> Result<(), String> {
    let force = force.unwrap_or(false);
    if let Err(e) = supervisor.stop().await {
        log::warn!("download_binary 前 stop 引擎失败（可能本就没跑）: {e}");
    }

    let binary_ok = !force
        && binary::status()
            .map(|s| s.installed && s.installed_version.as_deref() == Some(s.current_pin.as_str()))
            .unwrap_or(false);
    if binary_ok {
        log::info!("llama.cpp 已是 PIN 版本,跳过下载");
        return Ok(());
    }
    let app_for_engine = app.clone();
    binary::download(move |phase, downloaded, total| {
        emit_progress(&app_for_engine, phase, downloaded, total, "engine");
    })
    .await
    .map_err(Into::into)
}

/// 下载文字识别(OCR)组件——onnxruntime 运行时,与 llama.cpp 完全独立。
/// 屏幕记忆的 OCR 专用;Windows 上是 DirectML 构建(~40MB),macOS 走系统
/// Vision 用不到本命令(前端 OCR 卡直接不显示下载入口)。
/// 已装且版本匹配时幂等快速返回(force 强制重下,修复损坏安装用)。
/// 进度复用 [`ENGINE_DOWNLOAD_PROGRESS_EVENT`],stage="runtime"。
#[tauri::command]
pub async fn download_ocr_runtime(app: AppHandle, force: Option<bool>) -> Result<(), String> {
    let force = force.unwrap_or(false);
    // Windows 上 installed 已含 DirectML.dll 检查:旧 CPU 构建即使版本号
    // 相同也判未装,迁移场景必然走到下载。
    let runtime_ok = !force
        && embedding_runtime::status()
            .map(|s| s.installed && s.installed_version.as_deref() == Some(s.current_pin.as_str()))
            .unwrap_or(false);
    if runtime_ok {
        log::info!("onnxruntime 已是 PIN 版本,跳过下载");
        return Ok(());
    }
    let app_for_runtime = app.clone();
    embedding_runtime::download(move |phase, downloaded, total| {
        emit_progress(&app_for_runtime, phase, downloaded, total, "runtime");
    })
    .await
    .map_err(Into::into)
}

/// 删除文字识别(OCR)组件。删除失败仅报错——可能因为 ort 已 dlopen 了 dll
/// 而 Windows 拒 unlink,那时提示用户重启 app 后再删。
#[tauri::command]
pub async fn delete_ocr_runtime() -> Result<(), String> {
    embedding_runtime::delete().await.map_err(String::from)
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

/// 删除已安装的 llama.cpp binary。OCR 组件独立,走 [`delete_ocr_runtime`]。
///
/// 要先 stop 引擎，否则 Windows 锁住 .exe 删不掉。
#[tauri::command]
pub async fn delete_binary(supervisor: State<'_, Arc<EngineSupervisor>>) -> Result<(), String> {
    if let Err(e) = supervisor.stop().await {
        log::warn!("delete_binary 前 stop 引擎失败（可能本就没跑）: {e}");
    }
    binary::delete().await.map_err(String::from)
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
