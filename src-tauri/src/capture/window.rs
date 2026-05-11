use serde::Serialize;
use sysinfo::{Pid, System};

use crate::error::{Error, Result};

/// 当前焦点窗口的元信息（用于判断是否切焦点 / 写 activities 行）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInfo {
    pub app_name: String,
    pub title: String,
    pub app_path: Option<String>,
}

/// 拉当前焦点窗口的 [`WindowInfo`]。取不到（屏幕权限缺失 / 没有窗口在前 等）
/// 返回 `Err`，调用方 log debug 跳过本次 tick。
///
/// macOS 多屏场景下 xcap 的 `is_focused()` 不可靠——它常把 Hindsight 自己挑出来，
/// 因为 Cocoa 层各屏都有自己的 "main window" 概念，xcap 遍历返回的顺序里
/// Hindsight 可能排在前。改成先用 `NSWorkspace.frontmostApplication` 拿到系统
/// 真正的 frontmost PID，再用它 filter xcap 窗口列表拿标题。
pub fn current_window() -> Result<WindowInfo> {
    let windows = xcap::Window::all().map_err(|e| Error::Capture(e.to_string()))?;

    #[cfg(target_os = "macos")]
    let (focused, diag_nswks_pid) = {
        let target_pid = macos_frontmost_pid();
        let focused = match target_pid {
            Some(p) => windows
                .iter()
                .find(|w| w.pid().ok() == Some(p))
                .ok_or_else(|| {
                    macos_log_focus_decision(
                        Some(p),
                        "<no-xcap-window-for-pid>",
                        0,
                        "",
                    );
                    Error::Capture(format!("找不到 PID={} 的窗口", p))
                })?,
            // NSWorkspace 拿不到（极少见，比如登录窗口 / Screen Recording 权限缺失）→
            // 落到 xcap 自带 heuristic 兜底，至少不直接挂
            None => windows
                .iter()
                .find(|w| w.is_focused().unwrap_or(false))
                .ok_or_else(|| {
                    macos_log_focus_decision(None, "<nothing-focused>", 0, "");
                    Error::Capture("没有焦点窗口".into())
                })?,
        };
        (focused, target_pid)
    };
    #[cfg(not(target_os = "macos"))]
    let focused = windows
        .iter()
        .find(|w| w.is_focused().unwrap_or(false))
        .ok_or_else(|| Error::Capture("没有焦点窗口".into()))?;

    let raw_name = focused.app_name().unwrap_or_default().to_string();
    let app_name = basename(&raw_name);
    let title = focused.title().unwrap_or_default().to_string();
    let pid = focused.pid().unwrap_or(0);

    #[cfg(target_os = "macos")]
    macos_log_focus_decision(diag_nswks_pid, &app_name, pid, &title);

    let app_path = if pid > 0 { resolve_exe_path(pid) } else { None };

    Ok(WindowInfo {
        app_name,
        title,
        app_path,
    })
}

/// 调试用：把每次焦点采集决定写一行到 `<data_root>/focus-debug.log`。
/// 排查"为什么 macOS 多屏下时间总是被算到 Hindsight 自己"专用，**v0.5.7-beta
/// 之后会下线**。文件无大小限制，用户做完测试后自行删除。
///
/// 格式：`HH:MM:SS | nswks_pid=<NSWorkspace 报的最前 app PID> | resolved=<app>/<pid> | title=<...>`
#[cfg(target_os = "macos")]
fn macos_log_focus_decision(nswks_pid: Option<u32>, app: &str, pid: u32, title: &str) {
    use std::io::Write;
    let path = crate::bootstrap::data_root().join("focus-debug.log");
    let now = chrono::Local::now().format("%H:%M:%S");
    let title_trim: String = title.chars().take(80).collect();
    let line = format!(
        "{} | nswks_pid={:?} | resolved={}/{} | title={}\n",
        now, nswks_pid, app, pid, title_trim,
    );
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// 调用 `NSWorkspace.sharedWorkspace().frontmostApplication()` 拿系统层"最前"
/// app 的 PID。返回 None 表示当前没有 frontmost（罕见——锁屏 / 登录窗 等）。
/// 不抛错——拿不到时上层落回 xcap heuristic。
#[cfg(target_os = "macos")]
fn macos_frontmost_pid() -> Option<u32> {
    use objc2_app_kit::NSWorkspace;
    // sharedWorkspace 永远返非空；frontmostApplication 在极个别状态下返 nil
    let workspace = NSWorkspace::sharedWorkspace();
    let app = workspace.frontmostApplication()?;
    let pid = app.processIdentifier();
    if pid > 0 { Some(pid as u32) } else { None }
}

/// xcap 在某些情况下（特别是 UWP 应用）会把完整路径塞进 app_name。
/// 取最后一段斜杠后的内容作为真正的进程名。
fn basename(s: &str) -> String {
    let trimmed = s.trim();
    if let Some(idx) = trimmed.rfind(['\\', '/']) {
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
