use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use tauri::State;

use crate::repo::app_icons::{icon_cache_path, write_cache_file};
use crate::repo::{app_icons, process_paths};
use crate::storage::DbPool;

/// 解析某 process_name 的 PNG 图标字节。三层 fallback（顺序不变）：
/// 1. 文件 cache（icons/<sanitized>.png）—— 直接读文件
/// 2. DB blob（同步过来的图标）—— 写文件 cache
/// 3. 本机 exe 提取（GDI / plist）—— 提取后写 DB + outbox + 文件 cache
///
/// 全部落空返回 None。
async fn resolve_icon_png(pool: &DbPool, process_name: &str) -> Result<Option<Vec<u8>>, String> {
    let cache_path = icon_cache_path(process_name).map_err(String::from)?;

    // 1) 文件 cache（读出失败就当不存在，继续往下走）
    if let Ok(bytes) = std::fs::read(&cache_path) {
        return Ok(Some(bytes));
    }

    // 2) DB BLOB —— 同步过来的图标走这里。Win 上传的 chrome.exe 字节，Mac 拉到本地后
    //    没有可执行文件可提取，直接读 app_icons 表的 BLOB 写到文件 cache 就够。
    if let Some(bytes) = app_icons::get_blob(pool, process_name)
        .await
        .map_err(String::from)?
    {
        write_cache_file(&cache_path, &bytes);
        return Ok(Some(bytes));
    }

    // 3) 本机 exe 提取（仅当 process_paths 里有可执行文件路径才能走通）
    let exe_path = match process_paths::get_path(pool, process_name)
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
    if let Err(e) = app_icons::upsert_local(pool, process_name, &png).await {
        log::warn!("app_icons upsert 失败 process={process_name}: {e}");
    }

    Ok(Some(png))
}

/// 拉某 process_name 的图标，返回文件**绝对路径字符串**（前端 convertFileSrc 转 asset:// URL）。
///
/// 之前返回 `data:image/png;base64,...` data URI —— WKWebView JS heap 永久持有
/// 几十到几百 KB 的字符串，加上 React state / 缓存放大，WebContent 进程吃掉
/// 大量内存。改成返回路径后，图像数据由 WKWebView 自己的 image cache 管，
/// 自动响应系统内存压力。
///
/// 全部落空返回 None，前端显示默认图标。
#[tauri::command]
pub async fn get_app_icon(
    pool: State<'_, DbPool>,
    process_name: String,
) -> Result<Option<String>, String> {
    let cache_path = icon_cache_path(&process_name).map_err(String::from)?;
    match resolve_icon_png(&pool, &process_name).await? {
        Some(_) => Ok(Some(cache_path.to_string_lossy().into_owned())),
        None => Ok(None),
    }
}

/// 导出用：拉某 process_name 的图标，返回 `data:image/png;base64,...` data URL。
///
/// HTML 使用统计报告要自包含单文件（离线可看、可分享），应用图标必须内嵌成
/// data URL。与 [`get_app_icon`] 走同一套三层 fallback；这里只在用户导出时对
/// Top N 应用各调一次，不会像 UI 那样高频调用，内存压力可忽略。
#[tauri::command]
pub async fn get_app_icon_data_url(
    pool: State<'_, DbPool>,
    process_name: String,
) -> Result<Option<String>, String> {
    let Some(bytes) = resolve_icon_png(&pool, &process_name).await? else {
        return Ok(None);
    };
    // 解码 + 重编码是 CPU 活,别占着 tauri runtime 的线程
    let small = tokio::task::spawn_blocking(move || shrink_icon_png(&bytes))
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some(format!(
        "data:image/png;base64,{}",
        BASE64.encode(small)
    )))
}

/// 报告里图标画 26 px,给 2 倍留给 Retina。
const EXPORT_ICON_PX: u32 = 64;

/// 把图标缩到导出规格再重编码 PNG。
///
/// 图标原图常见 256~1024 px、单张能到 900 KB,而报告里只画 26 px —— 一份月报
/// 涉及几十个应用,不缩的话光图标 base64 就能把 HTML 顶到几十 MB(实测 32 MB)。
///
/// 已经够小的、以及解码 / 编码失败的,原样返回:报告宁可大一点,也不该丢图标。
fn shrink_icon_png(bytes: &[u8]) -> Vec<u8> {
    let Ok(img) = image::load_from_memory(bytes) else {
        return bytes.to_vec();
    };
    if img.width() <= EXPORT_ICON_PX && img.height() <= EXPORT_ICON_PX {
        return bytes.to_vec();
    }
    let small = img.resize(
        EXPORT_ICON_PX,
        EXPORT_ICON_PX,
        image::imageops::FilterType::Lanczos3,
    );
    let mut out = Vec::new();
    match small.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png) {
        Ok(()) => out,
        Err(_) => bytes.to_vec(),
    }
}
