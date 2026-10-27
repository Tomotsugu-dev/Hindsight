//! 应用图标：每个系统一个实现，编译时选。

use std::path::Path;

use crate::error::Result;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod stub {
    /// 其它平台 stub：始终返回 None，上层走默认图标。
    pub fn extract_png(_: &str, _: &std::path::Path) -> crate::error::Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use stub as imp;
#[cfg(target_os = "windows")]
use windows as imp;

/// 返回应用的图标（PNG）。`process_name` 是记录下来的应用名，`exe_path` 是它的程序路径。
/// Windows（GDI）和 macOS（plist + icns）只看程序路径；其它平台还没有实现，返回 None。
/// 失败返回 Err；没有图标返回 `Ok(None)`，调用方显示默认图标。
pub fn extract_png(process_name: &str, exe_path: &Path) -> Result<Option<Vec<u8>>> {
    imp::extract_png(process_name, exe_path)
}
