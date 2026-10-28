//! X11 会话：xcap 读 `_NET_ACTIVE_WINDOW` 找焦点窗口，应用名取窗口 `WM_CLASS` 的类名。
//!
//! Wayland 下 xcap 只看得见 XWayland 窗口，原生 Wayland 窗口在前台时它找不到焦点。

use super::super::{basename, resolve_exe_path, FocusSource, WindowInfo};
use crate::error::{Error, Result};

pub(super) struct X11Focus;

impl FocusSource for X11Focus {
    fn focused_window(&self) -> Result<WindowInfo> {
        let windows = xcap::Window::all().map_err(|e| Error::Capture(e.to_string()))?;

        let focused = windows
            .iter()
            .find(|w| w.is_focused().unwrap_or(false))
            .ok_or_else(|| Error::Capture("no focused window".into()))?;

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
