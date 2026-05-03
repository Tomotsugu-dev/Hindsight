use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use tauri::State;

use crate::repo::app_icons::{icon_cache_path, write_cache_file};
use crate::repo::{app_icons, process_paths};
use crate::storage::DbPool;

#[tauri::command]
pub async fn get_app_icon(
    pool: State<'_, DbPool>,
    process_name: String,
) -> Result<Option<String>, String> {
    let cache_path = icon_cache_path(&process_name).map_err(|e| e.to_string())?;

    // 1) 文件 cache
    if cache_path.exists() {
        return Ok(Some(read_as_data_url(&cache_path).map_err(|e| e.to_string())?));
    }

    // 2) DB BLOB —— 同步过来的图标走这里。Win 上传的 chrome.exe 字节，Mac 拉到本地后
    //    没有可执行文件可提取，直接读 app_icons 表的 BLOB 就能渲染。
    if let Some(bytes) = app_icons::get_blob(&pool, &process_name)
        .await
        .map_err(|e| e.to_string())?
    {
        write_cache_file(&cache_path, &bytes);
        return Ok(Some(format!(
            "data:image/png;base64,{}",
            BASE64.encode(&bytes)
        )));
    }

    // 3) 本机 exe 提取（仅当 process_paths 里有可执行文件路径才能走通）
    let exe_path = match process_paths::get_path(&pool, &process_name)
        .await
        .map_err(|e| e.to_string())?
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

    Ok(Some(format!(
        "data:image/png;base64,{}",
        BASE64.encode(&png)
    )))
}

fn read_as_data_url(path: &std::path::Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(format!("data:image/png;base64,{}", BASE64.encode(&bytes)))
}
