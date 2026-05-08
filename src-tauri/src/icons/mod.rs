use std::path::Path;

use crate::error::Result;

#[cfg(target_os = "macos")]
mod macos_impl;
#[cfg(target_os = "windows")]
mod windows_impl;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod stub_impl {
    /// 其它平台 stub：始终返回 None，上层走默认图标。
    pub fn extract_png(_: &std::path::Path) -> crate::error::Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

#[cfg(target_os = "macos")]
use macos_impl as imp;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use stub_impl as imp;
#[cfg(target_os = "windows")]
use windows_impl as imp;

/// 从可执行文件提取图标（Windows: GDI；macOS: plist+icns；其它平台: no-op 返回 None）。
/// 失败返回 Err；exe 没有图标返回 `Ok(None)` 让调用方走默认图标。
pub fn extract_png(exe_path: &Path) -> Result<Option<Vec<u8>>> {
    imp::extract_png(exe_path)
}
