//! Linux: captures the focused window. GNOME on Wayland asks the Hindsight GNOME extension
//! ([`gnome`]); an X11 session uses [`x11`] on every desktop. Other Wayland desktops are not
//! captured. xcap is not used on Wayland: it only sees XWayland windows.

use image::RgbaImage;

use crate::capture::focus::linux::{source, Source};

mod gnome;
mod x11;

/// Captures the focused window, or returns None when that fails or the foreground is no longer
/// `expected_pid` (unless it is 0).
pub(super) fn grab_focused(expected_pid: u32) -> Option<RgbaImage> {
    match source() {
        Some(Source::GnomeWayland) => gnome::grab_focused(expected_pid),
        Some(Source::X11) => x11::grab_focused(expected_pid),
        None => None,
    }
}
