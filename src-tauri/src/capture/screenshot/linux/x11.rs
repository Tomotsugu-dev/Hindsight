//! X11: captures the focused window with xcap. xcap asks the X server to list the windows, find
//! the focused one and capture it, so every desktop works the same way. In a Wayland session it
//! only sees XWayland windows.

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

#[cfg(test)]
mod tests {
    /// 手动验证：在 X11 桌面会话里跑，截下现在的焦点窗口，存到 `SHOT_OUT` 指定的 PNG。
    ///
    /// 跑法：`SHOT_OUT=/tmp/shot.png cargo test --lib linux_screenshot_manual -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn linux_screenshot_manual() {
        let windows = xcap::Window::all().expect("列不出窗口：要在 X11 会话里跑");
        let focused = windows.iter().find(|w| w.is_focused().unwrap_or(false));
        println!(
            "窗口 {} 个，焦点窗口: {:?}",
            windows.len(),
            focused.map(|w| (w.app_name().ok(), w.title().ok(), w.pid().ok()))
        );
        let img = super::grab_focused(0).expect("没截到焦点窗口");
        println!("截到 {}×{}", img.width(), img.height());
        if let Ok(out) = std::env::var("SHOT_OUT") {
            img.save(&out).unwrap();
            println!("存到 {out}");
        }
    }
}
