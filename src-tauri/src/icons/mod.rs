use std::path::Path;

use crate::error::Result;

#[cfg(target_os = "windows")]
mod windows_impl;
#[cfg(target_os = "macos")]
mod macos_impl;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod stub_impl {
    pub fn extract_png(_: &std::path::Path) -> crate::error::Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

#[cfg(target_os = "windows")]
use windows_impl as imp;
#[cfg(target_os = "macos")]
use macos_impl as imp;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use stub_impl as imp;

pub fn extract_png(exe_path: &Path) -> Result<Option<Vec<u8>>> {
    imp::extract_png(exe_path)
}
