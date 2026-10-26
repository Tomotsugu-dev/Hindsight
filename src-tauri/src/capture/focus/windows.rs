//! Windows：在 xcap 的窗口列表里找有焦点的那个窗口。

use std::sync::Arc;

use super::{basename, resolve_exe_path, FocusSource, WindowInfo};
use crate::error::{Error, Result};

/// 返回 Windows 的焦点来源。
pub(super) fn detect() -> Arc<dyn FocusSource> {
    Arc::new(WindowsFocus)
}

struct WindowsFocus;

impl FocusSource for WindowsFocus {
    fn focused_window(&self) -> Result<WindowInfo> {
        let windows = xcap::Window::all().map_err(|e| Error::Capture(e.to_string()))?;

        let focused = windows
            .iter()
            .find(|w| w.is_focused().unwrap_or(false))
            .ok_or_else(|| Error::Capture("没有焦点窗口".into()))?;

        // xcap 对 UWP 应用会把完整安装路径塞进 app_name，basename 取最后一段
        let raw_name = focused.app_name().unwrap_or_default().to_string();
        let app_name = basename(&raw_name);
        let title = focused.title().unwrap_or_default().to_string();
        let pid = focused.pid().unwrap_or(0);

        let app_path = if pid > 0 { resolve_exe_path(pid) } else { None };

        Ok(WindowInfo {
            app_name,
            title,
            app_path,
            pid,
        })
    }
}
