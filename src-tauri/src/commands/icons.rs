use std::path::PathBuf;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use tauri::State;

use crate::repo::process_paths;
use crate::storage::{db_path_dir, DbPool};

#[tauri::command]
pub async fn get_app_icon(
    pool: State<'_, DbPool>,
    process_name: String,
) -> Result<Option<String>, String> {
    let cache_path = match icon_cache_path(&process_name) {
        Ok(p) => p,
        Err(e) => return Err(e.to_string()),
    };

    if cache_path.exists() {
        return Ok(Some(read_as_data_url(&cache_path).map_err(|e| e.to_string())?));
    }

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

    if let Some(parent) = cache_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&cache_path, &png);

    Ok(Some(format!(
        "data:image/png;base64,{}",
        BASE64.encode(&png)
    )))
}

fn icon_cache_path(process_name: &str) -> crate::error::Result<PathBuf> {
    let dir = db_path_dir()?.join("icons");
    Ok(dir.join(format!("{}.png", sanitize(process_name))))
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn read_as_data_url(path: &std::path::Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(format!("data:image/png;base64,{}", BASE64.encode(&bytes)))
}
