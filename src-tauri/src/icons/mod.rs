use std::path::Path;

use crate::error::Result;

#[cfg(target_os = "windows")]
mod windows_impl;
#[cfg(target_os = "macos")]
mod macos_impl;

pub fn extract_png(exe_path: &Path) -> Result<Option<Vec<u8>>> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::extract_png(exe_path)
    }
    #[cfg(target_os = "macos")]
    {
        macos_impl::extract_png(exe_path)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = exe_path;
        Ok(None)
    }
}
