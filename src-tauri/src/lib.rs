use::serde::Serialize;
use::xcap::Window;

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[derive(Serialize)]
struct WindowInfo {
    app_name: String,
    title: String,
}

#[tauri::command]
fn get_current_window() -> Result<WindowInfo, String> {
    let windows = Window::all().map_err(|e| e.to_string())?;

    let focused = windows
        .iter()
        .find(|w| w.is_focused().unwrap_or(false))
        .ok_or("没有焦点窗口".to_string())?;

    Ok(WindowInfo {
        app_name: focused.app_name().unwrap_or_default().to_string(),
        title: focused.title().unwrap_or_default().to_string(),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![greet, get_current_window])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
