//! App icons: one implementation per system, chosen at compile time.

use std::path::Path;

use crate::error::Result;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

/// Returns the app's icon as a PNG.
///
/// `process_name` is the recorded app name and `exe_path` is its program path. Windows (GDI)
/// and macOS (plist + icns) use only the program path; Linux uses both to find the app's
/// `.desktop` file. Returns `Ok(None)` when there is no icon, and the caller shows the default
/// icon.
///
/// # Errors
///
/// An icon was found, but handling it failed:
/// - It could not be converted to PNG (Windows, macOS): [`crate::error::Error::Capture`]
/// - The `.icns` file could not be opened (macOS): [`crate::error::Error::Io`]
///
/// Linux never returns Err; any failure there counts as no icon.
pub fn extract_png(process_name: &str, exe_path: &Path) -> Result<Option<Vec<u8>>> {
    imp::extract_png(process_name, exe_path)
}
