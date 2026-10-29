//! GNOME Wayland: asks the Hindsight GNOME extension to capture the focused window.
//! `CaptureFocusedWindow` on `com.tomotsugu.hindsight.Focus` returns a PNG and the window's pid.
//! The interface is described at the top of `extension.js`. An extension from before version 2
//! does not have the method: the call fails and no screenshot is taken until the extension is
//! updated and the user logs in again.

use std::sync::{Mutex, Once};
use std::time::Duration;

use image::RgbaImage;
use zbus::blocking::Connection;

const BUS_NAME: &str = "com.tomotsugu.hindsight.Focus";
const OBJECT_PATH: &str = "/com/tomotsugu/hindsight/Focus";
const INTERFACE: &str = "com.tomotsugu.hindsight.Focus";
/// The error a version 1 extension returns: it has `GetFocusedWindow` but not this method.
const UNKNOWN_METHOD: &str = "org.freedesktop.DBus.Error.UnknownMethod";
/// Upper bound for one call. The shell captures and encodes the window here, so this is longer
/// than the focus query. A stuck gnome-shell must not hold the capture tick past this.
const CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// Captures the focused window, or returns None when that fails or the foreground is no longer
/// `expected_pid` (unless it is 0).
pub(super) fn grab_focused(expected_pid: u32) -> Option<RgbaImage> {
    match capture() {
        Ok((png, pid)) => image_from_capture(&png, pid, expected_pid),
        Err(e) => {
            log_capture_failure(&e);
            None
        }
    }
}

fn capture() -> zbus::Result<(Vec<u8>, u32)> {
    let reply = connection()?.call_method(
        Some(BUS_NAME),
        OBJECT_PATH,
        Some(INTERFACE),
        "CaptureFocusedWindow",
        &(),
    )?;
    reply.body().deserialize()
}

fn connection() -> zbus::Result<Connection> {
    static CONN: Mutex<Option<Connection>> = Mutex::new(None);
    let mut conn = CONN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(open) = conn.as_ref() {
        return Ok(open.clone());
    }
    let open = zbus::blocking::connection::Builder::session()?
        .method_timeout(CALL_TIMEOUT)
        .build()?;
    *conn = Some(open.clone());
    Ok(open)
}

/// A version 1 extension is warned about once. Any other failure is a debug line: the next
/// screenshot tries again.
fn log_capture_failure(error: &zbus::Error) {
    let missing_method = matches!(
        error,
        zbus::Error::MethodError(name, _, _) if name.as_str() == UNKNOWN_METHOD
    );
    if missing_method {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            log::warn!(
                "The Hindsight GNOME extension cannot capture windows yet; update it and log in again"
            );
        });
        return;
    }
    log::debug!("GNOME screenshot failed: {error}");
}

/// Decodes the PNG the extension returned.
///
/// An empty `png` means there was no window or the capture failed. `pid` is the window that was
/// captured. When `expected_pid` is not 0 and both pids are
/// known, a mismatch means the foreground moved on and the image is dropped.
fn image_from_capture(png: &[u8], pid: u32, expected_pid: u32) -> Option<RgbaImage> {
    if png.is_empty() {
        return None;
    }
    if expected_pid != 0 && pid != 0 && pid != expected_pid {
        log::debug!(
            "Screenshot skipped: the foreground switched from pid={expected_pid} to pid={pid}"
        );
        return None;
    }
    match image::load_from_memory_with_format(png, image::ImageFormat::Png) {
        Ok(img) => Some(img.to_rgba8()),
        Err(e) => {
            log::debug!("GNOME screenshot was not a PNG: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Cursor;

    fn tiny_png() -> Vec<u8> {
        let image = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([9, 8, 7, 255]),
        ));
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        png.into_inner()
    }

    #[test]
    fn empty_capture_is_none() {
        assert!(image_from_capture(&[], 42, 42).is_none());
    }

    #[test]
    fn pid_mismatch_is_dropped() {
        assert!(image_from_capture(&tiny_png(), 7, 8).is_none());
    }

    #[test]
    fn unknown_pid_is_kept() {
        // 扩展没给出 pid 时挡不住，和 X11 一样留下这张图
        let img = image_from_capture(&tiny_png(), 0, 8).unwrap();
        assert_eq!((img.width(), img.height()), (2, 2));
    }

    #[test]
    fn expected_pid_zero_keeps_any_pid() {
        let img = image_from_capture(&tiny_png(), 7, 0).unwrap();
        assert_eq!((img.width(), img.height()), (2, 2));
    }

    #[test]
    fn png_roundtrip() {
        let img = image_from_capture(&tiny_png(), 7, 7).unwrap();
        assert_eq!(img.get_pixel(0, 0).0, [9, 8, 7, 255]);
    }

    #[test]
    fn garbage_is_none() {
        assert!(image_from_capture(b"not a png", 7, 7).is_none());
    }

    /// 手动验证：GNOME Wayland 上扩展已启用时跑，截下现在的焦点窗口，存到 `SHOT_OUT` 指定的 PNG。
    ///
    /// 跑法：`SHOT_OUT=/tmp/shot.png cargo test --lib gnome_screenshot_manual -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn gnome_screenshot_manual() {
        let img = super::grab_focused(0).expect("没截到焦点窗口：要在 GNOME Wayland 上启用扩展");
        println!("截到 {}×{}", img.width(), img.height());
        if let Ok(out) = std::env::var("SHOT_OUT") {
            img.save(&out).unwrap();
            println!("存到 {out}");
        }
    }
}
