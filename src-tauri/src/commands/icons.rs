use tauri::State;

use crate::repo::app_icons::{icon_cache_path, write_cache_file};
use crate::repo::{app_icons, process_paths};
use crate::storage::DbPool;

/// 拉某 process_name 的图标，返回文件**绝对路径字符串**（前端 convertFileSrc 转 asset:// URL）。
///
/// 之前返回 `data:image/png;base64,...` data URI —— WKWebView JS heap 永久持有
/// 几十到几百 KB 的字符串，加上 React state / 缓存放大，WebContent 进程吃掉
/// 大量内存。改成返回路径后，图像数据由 WKWebView 自己的 image cache 管，
/// 自动响应系统内存压力。
///
/// 三层 fallback 不变：
/// 1. 文件 cache（icons/<sanitized>.png）—— 直接返路径
/// 2. DB blob（同步过来的图标）—— 写文件 cache，返路径
/// 3. 本机 exe 提取（GDI / plist）—— 提取后写 DB + outbox + 文件 cache，返路径
///
/// 全部落空返回 None，前端显示默认图标。
#[tauri::command]
pub async fn get_app_icon(
    pool: State<'_, DbPool>,
    process_name: String,
) -> Result<Option<String>, String> {
    let cache_path = icon_cache_path(&process_name).map_err(String::from)?;

    // 1) 文件 cache
    if cache_path.exists() {
        return Ok(Some(cache_path.to_string_lossy().into_owned()));
    }

    // 2) DB BLOB —— 同步过来的图标走这里。Win 上传的 chrome.exe 字节，Mac 拉到本地后
    //    没有可执行文件可提取，直接读 app_icons 表的 BLOB 写到文件 cache 就够。
    if let Some(bytes) = app_icons::get_blob(&pool, &process_name)
        .await
        .map_err(String::from)?
    {
        write_cache_file(&cache_path, &bytes);
        return Ok(Some(cache_path.to_string_lossy().into_owned()));
    }

    // 3) 本机 exe 提取（仅当 process_paths 里有可执行文件路径才能走通）
    let exe_path = match process_paths::get_path(&pool, &process_name)
        .await
        .map_err(String::from)?
    {
        Some(p) => p,
        None => return Ok(None),
    };

    // GDI / plist 解析是同步阻塞 IO，不应阻塞 Tauri runtime
    let exe = std::path::PathBuf::from(exe_path);
    let png = match tokio::task::spawn_blocking(move || crate::icons::extract_png(&exe))
        .await
        .map_err(|e| e.to_string())?
    {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };

    write_cache_file(&cache_path, &png);

    // 写 DB + outbox：让其它设备拉得到这张图。失败不影响 UI 返回（log 一下）。
    if let Err(e) = app_icons::upsert_local(&pool, &process_name, &png).await {
        log::warn!("app_icons upsert 失败 process={process_name}: {e}");
    }

    Ok(Some(cache_path.to_string_lossy().into_owned()))
}
