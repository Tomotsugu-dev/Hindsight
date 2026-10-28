//! Linux: captures the focused window. Only X11 for now ([`x11`]); a Wayland session uses it too
//! and only sees XWayland windows.

use image::RgbaImage;

mod x11;

/// Captures the focused window, or returns None when that fails or the foreground is no longer
/// `expected_pid` (unless it is 0).
pub(super) fn grab_focused(expected_pid: u32) -> Option<RgbaImage> {
    x11::grab_focused(expected_pid)
}
