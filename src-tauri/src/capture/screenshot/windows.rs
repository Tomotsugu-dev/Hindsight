//! Windows: captures the focused window with xcap.

use image::RgbaImage;

/// Captures the focused window, or returns None when that fails or the foreground is no longer
/// `expected_pid` (unless it is 0).
pub(super) fn grab_focused(expected_pid: u32) -> Option<RgbaImage> {
    let windows = xcap::Window::all().ok()?;
    let focused = windows.iter().find(|w| w.is_focused().unwrap_or(false))?;
    if expected_pid != 0 {
        let pid = focused.pid().unwrap_or(0);
        if pid != 0 && pid != expected_pid {
            log::debug!(
                "Screenshot skipped: the foreground switched from pid={expected_pid} to pid={pid}"
            );
            return None;
        }
    }
    focused.capture_image().ok()
}
