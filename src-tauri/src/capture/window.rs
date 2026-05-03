use serde::Serialize;
use sysinfo::{Pid, System};

use crate::error::{Error, Result};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub app_name: String,
    pub title: String,
    pub app_path: Option<String>,
}

pub fn current_window() -> Result<WindowInfo> {
    let windows = xcap::Window::all().map_err(|e| Error::Capture(e.to_string()))?;
    let focused = windows
        .iter()
        .find(|w| w.is_focused().unwrap_or(false))
        .ok_or_else(|| Error::Capture("没有焦点窗口".into()))?;

    let raw_name = focused.app_name().unwrap_or_default().to_string();
    let app_name = basename(&raw_name);
    let title = focused.title().unwrap_or_default().to_string();
    let pid = focused.pid().unwrap_or(0);

    let app_path = if pid > 0 {
        resolve_exe_path(pid as u32)
    } else {
        None
    };

    Ok(WindowInfo {
        app_name,
        title,
        app_path,
    })
}

/// xcap 在某些情况下（特别是 UWP 应用）会把完整路径塞进 app_name。
/// 取最后一段斜杠后的内容作为真正的进程名。
fn basename(s: &str) -> String {
    let trimmed = s.trim();
    if let Some(idx) = trimmed.rfind(|c| c == '\\' || c == '/') {
        trimmed[idx + 1..].to_string()
    } else {
        trimmed.to_string()
    }
}

fn resolve_exe_path(pid: u32) -> Option<String> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
        true,
        sysinfo::ProcessRefreshKind::new().with_exe(sysinfo::UpdateKind::OnlyIfNotSet),
    );
    sys.process(Pid::from_u32(pid))
        .and_then(|p| p.exe().map(|p| p.to_string_lossy().to_string()))
}
