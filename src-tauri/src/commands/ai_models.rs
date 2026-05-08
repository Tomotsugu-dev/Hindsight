//! 本地模型 (GGUF) 的列表 / 删除 / 下载 / 推荐表命令（Phase 1B-β β.1）。
//!
//! 下载通过 [`MODEL_PROGRESS_EVENT`] 流式推送进度；命令本身在下载完成后才 resolve。

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::ai::models::{self, ModelEntry, PartialEntry};
use crate::ai::recommended::{recommended, Recommended};
use crate::repo::settings;
use crate::storage::DbPool;

/// 列当前 settings 配的模型目录里所有 `.gguf` 文件。
/// 目录不存在或为空都返回 `[]`，不当错误。
#[tauri::command]
pub async fn list_local_models(pool: State<'_, DbPool>) -> Result<Vec<ModelEntry>, String> {
    let cfg = settings::load(&pool).await.map_err(String::from)?;
    models::list_local(&cfg.ai).await.map_err(String::from)
}

/// 删除一个本地 GGUF 文件。`filename` 必须是文件名（不含路径），后端
/// 会再次校验防 `..` / 分隔符注入。
///
/// 顺手清理：被删文件如果是当前 active_main / active_mmproj，把 settings
/// 里那项清掉——下次 `start_engine` 才不会拿一个不存在的文件名报错。
#[tauri::command]
pub async fn delete_model(pool: State<'_, DbPool>, filename: String) -> Result<(), String> {
    let mut cfg = settings::load(&pool).await.map_err(String::from)?;
    models::delete(&cfg.ai, &filename)
        .await
        .map_err(String::from)?;

    let mut dirty = false;
    for slot in [
        &mut cfg.ai.active_main,
        &mut cfg.ai.active_mmproj,
        &mut cfg.ai.describe_main,
        &mut cfg.ai.describe_mmproj,
        &mut cfg.ai.summary_main,
        &mut cfg.ai.summary_mmproj,
    ] {
        if *slot == filename {
            slot.clear();
            dirty = true;
        }
    }
    if dirty {
        settings::save(&pool, &cfg).await.map_err(String::from)?;
    }
    Ok(())
}

/// 返回 Hindsight 内置的推荐模型清单——前端拿来渲染推荐卡片。
/// 静态数据，不查 DB / 网络。
#[tauri::command]
pub async fn list_recommended_models() -> Result<Vec<Recommended>, String> {
    Ok(recommended().to_vec())
}

/// 下载 GGUF 时进度事件 payload。前端 listen 这条事件名拿到。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelDownloadProgressPayload {
    /// 文件名，方便前端区分多个并发下载（main / mmproj）属于谁
    file: String,
    /// 已下载字节数
    downloaded: u64,
    /// 总字节数；HF 一般给 content-length
    total: Option<u64>,
}

const MODEL_PROGRESS_EVENT: &str = "ai://model-download-progress";

/// 从 HuggingFace 下载一个 GGUF 文件到当前 settings.ai.modelsPath。
///
/// - `repo` 形如 `ggml-org/Qwen2.5-VL-3B-Instruct-GGUF`
/// - `file` 是文件名（不含路径分隔符）
/// - `expected_bytes` 是预期字节数；用来判断"是否已下完整"的容差比对，
///    传 0 关闭这个检查
///
/// 进度通过 [`MODEL_PROGRESS_EVENT`] 流式推送给前端，命令本身在下载完才 resolve。
/// 返回值是落盘后的完整路径。
#[tauri::command]
pub async fn download_model(
    app: AppHandle,
    pool: State<'_, DbPool>,
    repo: String,
    file: String,
    save_as: Option<String>,
    expected_bytes: u64,
) -> Result<String, String> {
    let cfg = settings::load(&pool).await.map_err(String::from)?;
    let app_for_emit = app.clone();
    // emit 用的是落盘名——前端按相同名字索引 progress / 调 cancel
    let local_name = save_as
        .as_deref()
        .map(str::to_string)
        .unwrap_or_else(|| file.clone());
    let file_for_emit = local_name.clone();
    let path = models::download_from_hf(
        &cfg.ai,
        &repo,
        &file,
        save_as.as_deref(),
        expected_bytes,
        move |downloaded, total| {
            let payload = ModelDownloadProgressPayload {
                file: file_for_emit.clone(),
                downloaded,
                total,
            };
            if let Err(e) = app_for_emit.emit(MODEL_PROGRESS_EVENT, &payload) {
                log::warn!("emit {MODEL_PROGRESS_EVENT} 失败: {e}");
            }
        },
    )
    .await
    .map_err(String::from)?;
    Ok(path.to_string_lossy().into_owned())
}

/// 暂停某个正在进行中的下载——翻 cancel flag。`<file>.partial` 被保留，
/// 用户下次再调 `download_model` 同 file 名时会走 Range 续传。
///
/// 文件没在下载（或已经下完）时静默成功（idempotent）。前端不需要预先判断。
#[tauri::command]
pub async fn cancel_model_download(file: String) -> Result<(), String> {
    models::set_cancel(&file);
    Ok(())
}

/// 列扫描模型目录里所有 `<file>.partial` 半成品——给前端渲染"继续"按钮 + 当前进度。
/// 目录不存在或没有 partial 时返回 `[]`，不当错误。
#[tauri::command]
pub async fn list_partial_downloads(
    pool: State<'_, DbPool>,
) -> Result<Vec<PartialEntry>, String> {
    let cfg = settings::load(&pool).await.map_err(String::from)?;
    models::list_partials(&cfg.ai).await.map_err(String::from)
}
